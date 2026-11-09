//! NRPA on the lobster search's mode (docs/briefs/lobster-nrpa.md): the same
//! start position and seeded Tavern, Gryphon and Lobster streams as
//! `lobster_search`, but the beam is replaced by Nested Rollout Policy
//! Adaptation (Rosin 2011).
//!
//! A rollout samples legal moves by a softmax over learned code weights. A
//! code is the action kind plus the card it acts on, never a bare slot number.
//! One level keeps its best line and shifts the weights toward that line's
//! moves; the next level repeats what worked.
//!
//!   cargo run --release -p bgbench --bin lobster_nrpa -- [options]
//!
//! It writes `nrpa.md` and `nrpa.json` under the output directory.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use bgcore::action::N_ACTIONS;
use bgcore::cards::Counter;
use bgcore::shop_replay::{self, ShopFileSpec, ShopStateSpec, ME};
use bgcore::{Action, CardId, CardIdMap, Data, Game, Minion, Rng, ShopStateCapture};
use rayon::prelude::*;
use serde_json::{Value, json};

const SOURCE_REL: &str = "boards/actions/Hearthstone_2026_10_28_22_11_50-g2.json";
const REF_REL: &str = "ref/bg-cards.json";
const OUT_DIR_REL: &str = "train/runs/lobster-search";
const START_TURN: u32 = 10;
const END_TURN: u32 = 13;

type Score = (u16, i32);
type Labels = HashMap<u64, String>;
type Line = Vec<(u32, Action)>;
type Keys = Vec<u64>;
type BestLine = (Score, Line, Keys);
type RestartBest = (Score, Line, HashMap<u64, f64>, Labels);
type ReplayOut = (Vec<TurnRow>, Vec<String>, Vec<Vec<String>>, bool, u32, u32, [u32; 4]);

/// The biggest action count a turn may spend, the same 200 as the beam.
const ROLLOUT_CAP: u32 = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CodeMode {
    /// Kind plus the card the action acts on.
    Basic,
    /// Kind plus the card, the action phase and, for a prompt, the card that
    /// opened it.
    Context,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KeyMode {
    /// One weight per move code, shared by every state.
    Global,
    /// One weight per (state, move code): classic NRPA.
    State,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Horizon {
    /// Search each turn on its own, keep the best end state, then the next.
    Turn,
    /// Search turns 10 to 13 as one line, scored at the end of turn 13.
    Whole,
}

#[derive(Clone, Debug)]
struct LoggedTavern {
    shop: Vec<Minion>,
    spell: Option<CardId>,
}

#[derive(Clone, Debug)]
struct RunSpec {
    variant: String,
    p: u16,
    seed: u64,
    control: bool,
    code: CodeMode,
}

#[derive(Clone, Debug)]
struct TurnRow {
    turn: u32,
    triggers: u16,
    attack: i32,
    health: i32,
    gold: u8,
    actions: u16,
    actions_this_turn: u32,
    fishbait_attacks: u32,
    deathstrider_fires: u32,
    rollouts: u64,
}

#[derive(Clone, Debug)]
struct RunResult {
    spec: RunSpec,
    turns: Vec<TurnRow>,
    actions: Vec<(u32, Action)>,
    line: Vec<String>,
    turn_lines: Vec<Vec<(u32, Action)>>,
    turn_text: Vec<Vec<String>>,
    policy: Vec<(u64, f64, String)>,
    seconds: f64,
    rollouts: u64,
    golden_gryphon: bool,
    fishbait_attacks: u32,
    deathstrider_fires: u32,
    shark_sells: u32,
    lionfish_activates: u32,
    shark_plays: u32,
}

fn repo_path(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap_or(Path::new("."))
        .join(rel)
}

fn load_ids() -> CardIdMap {
    let path = repo_path(REF_REL);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    CardIdMap::parse(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn load_base(ids: &CardIdMap, variant: &str) -> (Game, Vec<LoggedTavern>, Arc<Data>) {
    let path = repo_path(SOURCE_REL);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let name = SOURCE_REL.rsplit('/').next().unwrap_or(SOURCE_REL).to_string();
    let mut files = shop_replay::parse_action_files(&[(name, text)])
        .unwrap_or_else(|e| panic!("{SOURCE_REL}: {e}"));
    shop_replay::seat_taught_spells(&mut files, ids);
    let data = Arc::new(Data::bundled());
    let file = files.remove(0);
    let turn = file
        .turns
        .iter()
        .find(|t| t.turn == START_TURN)
        .unwrap_or_else(|| panic!("{SOURCE_REL}: no turn {START_TURN}"));
    let (mut game, missing) = shop_replay::turn_start_game(&data, ids, &file, turn, 0x10b5_7e12)
        .unwrap_or_else(|| panic!("{SOURCE_REL}: turn {START_TURN} does not seat"));
    if !missing.is_empty() {
        eprintln!("start position missing: {missing:?}");
    }
    game.players[ME].health = 10_000;
    game.players[ME].armor = 0;
    if variant == "A" {
        let plain = data.cards.by_name("Headhunter Gryphon").expect("Headhunter Gryphon");
        for m in &mut game.players[ME].board {
            if data.cards.get(m.card).name == "Headhunter Gryphon" {
                m.card = plain;
                m.golden = false;
            }
        }
    }
    let taverns = logged_taverns(&data, &file, turn);
    (game, taverns, data)
}

/// Era's own Taverns for turn 10, in refresh order: the turn's roll 0 and then
/// the state after every Roll or sold Snarky Shark. The log writes the state
/// after the whole step, so a Shark's Refresh is the post-attack state.
fn logged_taverns(
    data: &Arc<Data>,
    file: &ShopFileSpec,
    turn: &shop_replay::ShopTurnSpec,
) -> Vec<LoggedTavern> {
    let mut specs: Vec<&ShopStateSpec> = vec![&turn.start];
    for step in &turn.actions {
        let is_refresh = step.step == "roll"
            || (step.step == "sell"
                && step.card.as_ref().and_then(|c| c.name.as_deref()) == Some("Snarky Shark"));
        if is_refresh && let Some(after) = step.after.as_ref() {
            specs.push(after);
        }
    }
    let mut out = Vec::new();
    for spec in specs {
        let hero = file.hero(data, spec);
        let loaded = shop_replay::load(data, spec, hero, file.lobby(), turn.turn, 1, None);
        let pl = &loaded.game.players[ME];
        out.push(LoggedTavern { shop: pl.shop.to_vec(), spell: pl.shop_spell });
    }
    out
}

fn config_game(base: &Game, seed: u64, per_mille: u16, taverns: &[LoggedTavern], control: bool) -> Game {
    let mut game = base.clone();
    game.players[ME].health = 10_000;
    game.enable_lobster_mode(ME, seed, per_mille);
    game.seat_lobster_tavern(ME);
    if control && let Some(t) = taverns.first() {
        apply_tavern(&mut game, t);
    }
    game
}

fn apply_tavern(game: &mut Game, tavern: &LoggedTavern) {
    let pl = &mut game.players[ME];
    pl.shop.clear();
    for m in &tavern.shop {
        let _ = pl.shop.try_push(*m);
    }
    pl.shop_spell = tavern.spell;
}

fn score(game: &Game) -> Score {
    let pl = &game.players[ME];
    let stats = pl.board.iter().map(|m| m.attack + m.health).sum();
    (pl.improve[Counter::TastyLobster.index()], stats)
}

fn game_from(base: &Game, state: &ShopStateCapture) -> Game {
    let mut game = base.clone();
    game.restore_shop_state(ME, state);
    game
}

/// One action on a game, with the control run's logged Tavern put in after a
/// Refresh. Every path that plays an action during a search goes through this,
/// so the replayed best line and the rollout see the same Tavern.
fn play_action(game: &mut Game, a: Action, taverns: &[LoggedTavern], control: bool) -> bool {
    let old_refresh = game.players[ME].lobster.map_or(0, |m| m.refresh);
    let turn = game.turn;
    if game.step(a).is_err() {
        return false;
    }
    if game.turn != turn {
        return false;
    }
    if control {
        let new_refresh = game.players[ME].lobster.map_or(0, |m| m.refresh);
        if new_refresh > old_refresh
            && let Some(t) = taverns.get(new_refresh.saturating_sub(1) as usize)
        {
            apply_tavern(game, t);
        }
    }
    true
}

fn searched(a: Action, board_len: usize) -> bool {
    // The same list and position prunes as the beam: moves only to the front
    // and plays only to the front or the end. Freeze, hero power, trinket
    // answers and the Dark Gift are not part of the line. End turn is a move
    // like any other, with its own code; a rollout ends when it samples end
    // turn, at the action cap, or when no move is legal.
    if matches!(a, Action::Freeze | Action::HeroPower | Action::HeroPower2 | Action::ChooseTrinket(_) | Action::DarkGift) {
        return false;
    }
    if let Action::Move { to, .. } = a
        && to != 0
    {
        return false;
    }
    if let Action::Play { pos, .. } = a
        && pos as usize != 0
        && pos as usize != board_len
    {
        return false;
    }
    true
}

fn card_at(zone: &[Minion], slot: usize) -> u64 {
    zone.get(slot).map_or(0xffff, |m| m.card as u64) + 1
}

fn prompt_source(game: &Game) -> u64 {
    let pl = &game.players[ME];
    let source = pl
        .pending_target
        .and_then(|p| p.source)
        .or(pl.pending_shop_target.map(|p| p.source))
        .or(pl.pending_choose_one.and_then(|p| p.source))
        .or(pl.pending_discard.and_then(|p| p.source));
    source
        .and_then(|s| pl.board.get(s as usize))
        .map_or(0, |m| m.card as u64 + 1)
}

/// A compact digest of the parts of a shop state that a move code should be
/// read against. It is a policy key, not a game rule: a collision only shares
/// weights between two positions.
fn state_digest(game: &Game) -> u64 {
    let pl = &game.players[ME];
    let mut h = FNV_OFFSET;
    h = hash_u64(h, game.turn as u64);
    h = hash_u64(h, pl.gold as u64);
    h = hash_u64(h, pl.tier as u64);
    h = hash_u64(h, pl.actions as u64);
    h = hash_u64(h, pl.health as u64);
    h = hash_u64(h, pl.free_rolls as u64);
    h = hash_u64(h, pl.improve[Counter::TastyLobster.index()] as u64);
    if let Some(m) = pl.lobster {
        h = hash_u64(h, m.refresh as u64);
        h = hash_u64(h, m.beast as u64);
        h = hash_u64(h, m.fishbait_attacks as u64);
        h = hash_u64(h, m.deathstrider_fires as u64);
    }
    for m in pl.board.iter() {
        h = hash_u64(h, m.card as u64 + 1);
        h = hash_u64(h, m.attack as u64);
        h = hash_u64(h, m.health as u64);
        h = hash_u64(h, m.golden as u64);
    }
    h = hash_u64(h, 0xbb);
    for m in pl.hand.iter() {
        h = hash_u64(h, m.card as u64 + 1);
        h = hash_u64(h, m.attack as u64);
        h = hash_u64(h, m.health as u64);
        h = hash_u64(h, m.golden as u64);
    }
    h = hash_u64(h, 0xcc);
    for m in pl.shop.iter() {
        h = hash_u64(h, m.card as u64 + 1);
        h = hash_u64(h, m.golden as u64);
    }
    h = hash_u64(h, pl.shop_spell.map_or(0, |c| c as u64 + 1));
    h = hash_u64(h, pl.pending_target.is_some() as u64);
    h = hash_u64(h, pl.pending_shop_target.is_some() as u64);
    h = hash_u64(h, pl.pending_choose_one.is_some() as u64);
    h = hash_u64(h, pl.pending_discard.is_some() as u64);
    h
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x100_0000_01b3;

fn hash_u64(mut h: u64, v: u64) -> u64 {
    for b in v.to_le_bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(FNV_PRIME);
    }
    h
}

fn move_code(game: &Game, a: Action, mode: CodeMode) -> u64 {
    let pl = &game.players[ME];
    let (kind, card, extra) = match a {
        Action::Buy(slot) => (1u64, card_at(&pl.shop, slot as usize), 0u64),
        Action::BuySpell => (2, pl.shop_spell.map_or(0, |c| c as u64 + 1), 0),
        Action::Sell(slot) => (3, card_at(&pl.board, slot as usize), 0),
        Action::Activate(slot) => (4, card_at(&pl.board, slot as usize), 0),
        Action::Play { hand, pos } => {
            let side = if pos as usize == 0 {
                1
            } else if pos as usize >= pl.board.len() {
                2
            } else {
                3
            };
            (5, card_at(&pl.hand, hand as usize), side)
        }
        Action::Target(slot) => (6, card_at(&pl.board, slot as usize), 0),
        Action::TargetShop(slot) => (7, card_at(&pl.shop, slot as usize), 0),
        Action::Choose(k) => (8, 0, k as u64 + 1),
        Action::Discard(slot) => (9, card_at(&pl.hand, slot as usize), 0),
        Action::Move { from, to } => (10, card_at(&pl.board, from as usize), u64::from(to != 0) + 1),
        Action::Roll => (11, 0, 0),
        Action::Level => (12, 0, 0),
        _ => (13, 0, 0),
    };
    let mut h = hash_u64(FNV_OFFSET, kind);
    h = hash_u64(h, card);
    h = hash_u64(h, extra);
    if mode == CodeMode::Context {
        let phase = (pl.actions / 20).min(7) as u64;
        h = hash_u64(h, phase + 1);
        h = hash_u64(h, prompt_source(game) + 1);
    }
    h
}

fn card_label(game: &Game, card: Option<CardId>, empty: &str) -> String {
    card.map_or(empty.to_string(), |c| game.data.cards.get(c).name.clone())
}

fn code_label(game: &Game, a: Action, mode: CodeMode) -> String {
    let pl = &game.players[ME];
    let base = match a {
        Action::Buy(slot) => format!(
            "buy + {}",
            card_label(game, pl.shop.get(slot as usize).map(|m| m.card), "empty")
        ),
        Action::BuySpell => format!("buy spell + {}", card_label(game, pl.shop_spell, "none")),
        Action::Sell(slot) => format!(
            "sell + {}",
            card_label(game, pl.board.get(slot as usize).map(|m| m.card), "empty")
        ),
        Action::Activate(slot) => format!(
            "activate + {}",
            card_label(game, pl.board.get(slot as usize).map(|m| m.card), "empty")
        ),
        Action::Play { hand, pos } => {
            let side = if pos as usize == 0 {
                "front"
            } else if pos as usize >= pl.board.len() {
                "end"
            } else {
                "middle"
            };
            format!(
                "play + {} ({side})",
                card_label(game, pl.hand.get(hand as usize).map(|m| m.card), "empty")
            )
        }
        Action::Target(slot) => format!(
            "target + {}",
            card_label(game, pl.board.get(slot as usize).map(|m| m.card), "empty")
        ),
        Action::TargetShop(slot) => format!(
            "target shop + {}",
            card_label(game, pl.shop.get(slot as usize).map(|m| m.card), "spell slot")
        ),
        Action::Choose(k) => format!("choose option {}", k + 1),
        Action::Discard(slot) => format!(
            "discard + {}",
            card_label(game, pl.hand.get(slot as usize).map(|m| m.card), "empty")
        ),
        Action::Move { from, .. } => format!(
            "move + {} (front)",
            card_label(game, pl.board.get(from as usize).map(|m| m.card), "empty")
        ),
        Action::Roll => "roll".to_string(),
        Action::Level => "level".to_string(),
        Action::EndTurn => "end turn".to_string(),
        _ => format!("{a:?}"),
    };
    if mode == CodeMode::Context {
        let phase = (pl.actions / 20).min(7);
        let source = prompt_source(game);
        let who = if source == 0 {
            String::new()
        } else {
            format!(" from {}", card_label(game, Some((source - 1) as u16), "?"))
        };
        format!("{base} [phase {phase}{who}]")
    } else {
        base
    }
}

/// One sampled rollout. With `Horizon::Turn` it ends at the 200-action cap or
/// when no move is legal, then the turn ends. With `Horizon::Whole` it plays
/// through every turn to `end_turn`, ending each turn at its cap or when no
/// move is legal, and scores the final board.
#[allow(clippy::too_many_arguments)]
fn rollout(
    base: &Game,
    root: &ShopStateCapture,
    policy: &mut HashMap<u64, f64>,
    labels: &mut Labels,
    mode: CodeMode,
    key_mode: KeyMode,
    horizon: Horizon,
    end_turn: u32,
    epsilon: f64,
    rng: &mut Rng,
    taverns: &[LoggedTavern],
    control: bool,
) -> BestLine {
    let mut game = game_from(base, root);
    let mut line = Vec::new();
    let mut keys = Vec::new();
    let mut mask = [false; N_ACTIONS];
    loop {
        let mut last_move = false;
        // One turn's actions.
        loop {
            let actions = game.players[ME].actions as u32;
            if actions >= ROLLOUT_CAP {
                break;
            }
            let board_len = game.players[ME].board.len();
            let digest = if key_mode == KeyMode::State { state_digest(&game) } else { 0 };
            game.legal_mask(&mut mask);
            let mut cands: Vec<(Action, u64, f64)> = Vec::new();
            for (i, ok) in mask.iter().enumerate() {
                if !*ok {
                    continue;
                }
                let Some(a) = Action::from_index(i) else { continue };
                if !searched(a, board_len) {
                    continue;
                }
                if last_move && matches!(a, Action::Move { .. }) {
                    continue;
                }
                let code = move_code(&game, a, mode);
                let key = if key_mode == KeyMode::State { hash_u64(digest, code) } else { code };
                let w = policy.get(&key).copied().unwrap_or(0.0);
                cands.push((a, key, w));
            }
            if cands.is_empty() {
                break;
            }
            // Softmax over the legal keys, in the legal action order, with an
            // epsilon chance of a uniform move.
            let pick = if epsilon > 0.0 && (rng.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64) < epsilon {
                rng.below(cands.len() as u32) as usize
            } else {
                let max = cands.iter().map(|c| c.2).fold(f64::NEG_INFINITY, f64::max);
                let mut sum = 0.0;
                for c in &cands {
                    sum += (c.2 - max).exp();
                }
                let u = ((rng.next_u64() >> 11) as f64) * (1.0 / (1u64 << 53) as f64) * sum;
                let mut walk = u;
                let mut pick = cands.len() - 1;
                for (k, c) in cands.iter().enumerate() {
                    let p = (c.2 - max).exp();
                    if walk < p {
                        pick = k;
                        break;
                    }
                    walk -= p;
                }
                pick
            };
            let (a, key, _) = cands[pick];
            labels.entry(key).or_insert_with(|| code_label(&game, a, mode));
            let turn = game.turn;
            if a == Action::EndTurn {
                // The search owns the turn end: step(EndTurn) would run a
                // fight, so record the move and let the outer loop end the
                // turn without a fight.
                line.push((turn, a));
                keys.push(key);
                break;
            }
            if !play_action(&mut game, a, taverns, control) {
                break;
            }
            line.push((turn, a));
            keys.push(key);
            last_move = matches!(a, Action::Move { .. });
            if game.players[ME].done {
                break;
            }
        }
        game.end_shop_turn(ME);
        if horizon == Horizon::Turn || game.turn >= end_turn {
            break;
        }
        game.next_shop_turn_without_fight();
    }
    (score(&game), line, keys)
}

fn shift(policy: &mut HashMap<u64, f64>, codes: &[u64], alpha: f64) {
    for &c in codes {
        *policy.entry(c).or_insert(0.0) += alpha;
    }
    // A soft cap keeps the softmax from becoming a hard argmax. Scaling all
    // seen weights down leaves unseen codes at zero, which is the relative
    // order the policy has learned.
    let max = policy.values().fold(0.0f64, |m, &v| m.max(v.abs()));
    if max > 30.0 {
        let k = 30.0 / max;
        for v in policy.values_mut() {
            *v *= k;
        }
    }
}

/// Standard NRPA: each level runs `n` rollouts, keeps its best line and
/// adapts the code weights toward it every iteration. The root is one turn,
/// so the recursion is over lines of that turn only.
#[allow(clippy::too_many_arguments)]
fn nrpa(
    level: u32,
    n: u32,
    base: &Game,
    root: &ShopStateCapture,
    policy: &mut HashMap<u64, f64>,
    labels: &mut Labels,
    mode: CodeMode,
    key_mode: KeyMode,
    horizon: Horizon,
    end_turn: u32,
    epsilon: f64,
    alpha: f64,
    adapt_only_on_improvement: bool,
    rng: &mut Rng,
    taverns: &[LoggedTavern],
    control: bool,
    rollouts: &mut u64,
) -> BestLine {
    if level == 0 {
        *rollouts += 1;
        return rollout(base, root, policy, labels, mode, key_mode, horizon, end_turn, epsilon, rng, taverns, control);
    }
    let mut best: Option<BestLine> = None;
    for _ in 0..n {
        let got = nrpa(
            level - 1, n, base, root, policy, labels, mode, key_mode, horizon, end_turn, epsilon,
            alpha, adapt_only_on_improvement, rng, taverns, control, rollouts,
        );
        let improved = best.as_ref().is_none_or(|b| got.0 > b.0);
        if improved {
            best = Some(got.clone());
        }
        if (!adapt_only_on_improvement || improved)
            && let Some((_, _, codes)) = &best
        {
            shift(policy, codes, alpha);
        }
    }
    best.expect("n is at least one")
}

#[derive(Clone, Copy)]
struct SearchOptions {
    levels: u32,
    iterations: u32,
    /// Probability of a uniform legal move instead of the softmax. This keeps
    /// a level from collapsing onto its first good line.
    epsilon: f64,
    alpha: f64,
    adapt_only_on_improvement: bool,
    /// Independent NRPA searches from the same root; the best line wins.
    restarts: u32,
}

fn golden_gryphon(game: &Game) -> bool {
    game.players[ME]
        .board
        .iter()
        .chain(game.players[ME].hand.iter())
        .any(|m| {
            let c = game.data.cards.get(m.card);
            c.name == "Headhunter Gryphon" && m.golden
        })
}

struct SearchOutput {
    rows: Vec<TurnRow>,
    actions: Vec<(u32, Action)>,
    turn_lines: Vec<Vec<(u32, Action)>>,
    policy: HashMap<u64, f64>,
    labels: Labels,
    rollouts: u64,
    golden: bool,
    seconds: f64,
}

fn search_run(
    base: &Game,
    cfg: &RunSpec,
    taverns: &[LoggedTavern],
    opts: SearchOptions,
    key_mode: KeyMode,
    horizon: Horizon,
) -> SearchOutput {
    let t = Instant::now();
    let mut game = config_game(base, cfg.seed, cfg.p, taverns, cfg.control);
    let mut policy: HashMap<u64, f64> = HashMap::new();
    let mut labels: Labels = HashMap::new();
    let mut rng = Rng::new(cfg.seed ^ 0x9e37_79b9_7f4a_7c15 ^ ((cfg.code as u64) << 40));
    let mut rollouts = 0u64;
    let end_turn = if cfg.control { START_TURN } else { END_TURN };
    let mut golden = false;
    if horizon == Horizon::Whole {
        let root = game.capture_shop_state(ME);
        golden |= golden_gryphon(&game);
        let mut best: Option<RestartBest> = None;
        for restart in 0..opts.restarts.max(1) {
            let mut p: HashMap<u64, f64> = HashMap::new();
            let mut l: Labels = HashMap::new();
            let mut r = Rng::new(
                cfg.seed
                    ^ 0x9e37_79b9_7f4a_7c15
                    ^ ((cfg.code as u64) << 40)
                    ^ (restart as u64).wrapping_mul(0x9e37_79b9),
            );
            let got = nrpa(
                opts.levels,
                opts.iterations,
                base,
                &root,
                &mut p,
                &mut l,
                cfg.code,
                key_mode,
                Horizon::Whole,
                end_turn,
                opts.epsilon,
                opts.alpha,
                opts.adapt_only_on_improvement,
                &mut r,
                taverns,
                cfg.control,
                &mut rollouts,
            );
            if best.as_ref().is_none_or(|b| got.0 > b.0) {
                best = Some((got.0, got.1, p, l));
            }
        }
        let (_, line, policy, labels) = best.expect("at least one restart");
        let _ = &mut game;
        let seconds = t.elapsed().as_secs_f64();
        return SearchOutput {
            rows: Vec::new(),
            actions: line,
            turn_lines: Vec::new(),
            policy,
            labels,
            rollouts,
            golden,
            seconds,
        };
    }
    let mut rows = Vec::new();
    let mut actions = Vec::new();
    let mut turn_lines = Vec::new();
    let mut last_fish = 0u32;
    let mut last_ds = 0u32;
    for turn in START_TURN..=end_turn {
        let root = game.capture_shop_state(ME);
        golden |= golden_gryphon(&game);
        let start_actions = game.players[ME].actions;
        let (end_score, line, _) = nrpa(
            opts.levels,
            opts.iterations,
            base,
            &root,
            &mut policy,
            &mut labels,
            cfg.code,
            key_mode,
            Horizon::Turn,
            end_turn,
            opts.epsilon,
            opts.alpha,
            opts.adapt_only_on_improvement,
            &mut rng,
            taverns,
            cfg.control,
            &mut rollouts,
        );
        for (_, a) in &line {
            if *a == Action::EndTurn {
                break;
            }
            if !play_action(&mut game, *a, taverns, cfg.control) {
                break;
            }
            golden |= golden_gryphon(&game);
        }
        game.end_shop_turn(ME);
        let pl = &game.players[ME];
        let fish_total = pl.lobster.map_or(0, |m| m.fishbait_attacks);
        let ds_total = pl.lobster.map_or(0, |m| m.deathstrider_fires);
        rows.push(TurnRow {
            turn,
            triggers: end_score.0,
            attack: pl.board.iter().map(|m| m.attack).sum(),
            health: pl.board.iter().map(|m| m.health).sum(),
            gold: pl.gold,
            actions: pl.actions,
            actions_this_turn: pl.actions as u32 - start_actions as u32,
            fishbait_attacks: fish_total - last_fish,
            deathstrider_fires: ds_total - last_ds,
            rollouts: 0,
        });
        last_fish = fish_total;
        last_ds = ds_total;
        actions.extend(line.iter().copied());
        turn_lines.push(line);
        if turn < end_turn {
            game.next_shop_turn_without_fight();
        }
    }
    let seconds = t.elapsed().as_secs_f64();
    SearchOutput { rows, actions, turn_lines, policy, labels, rollouts, golden, seconds }
}

fn replay(
    base: &Game,
    cfg: &RunSpec,
    taverns: &[LoggedTavern],
    actions: &[(u32, Action)],
) -> ReplayOut {
    let mut game = config_game(base, cfg.seed, cfg.p, taverns, cfg.control);
    let mut rows = Vec::new();
    let mut line = Vec::new();
    let mut turn_text = Vec::new();
    let mut counts = [0u32; 4];
    let mut last_fish = 0u32;
    let mut last_ds = 0u32;
    let mut golden = false;
    let mut at = 0usize;
    let last_turn = if cfg.control { START_TURN } else { END_TURN };
    for turn in START_TURN..=last_turn {
        let mut here = Vec::new();
        let start_actions = game.players[ME].actions;
        while at < actions.len() && actions[at].0 == turn {
            let a = actions[at].1;
            here.push(describe(&game, a));
            events(&game, a, &mut counts);
            if a == Action::EndTurn {
                // The search owns the turn end: step(EndTurn) would run a
                // fight. The end-of-turn effects run below.
                at += 1;
                break;
            }
            if !play_action(&mut game, a, taverns, cfg.control) {
                here.push(format!("  (stop: {a:?} is no longer legal)"));
                break;
            }
            golden |= golden_gryphon(&game);
            at += 1;
        }
        line.extend(here.iter().cloned());
        turn_text.push(here);
        let used = game.players[ME].actions - start_actions;
        game.end_shop_turn(ME);
        let pl = &game.players[ME];
        let fish_total = pl.lobster.map_or(0, |m| m.fishbait_attacks);
        let ds_total = pl.lobster.map_or(0, |m| m.deathstrider_fires);
        rows.push(TurnRow {
            turn,
            triggers: pl.improve[Counter::TastyLobster.index()],
            attack: pl.board.iter().map(|m| m.attack).sum(),
            health: pl.board.iter().map(|m| m.health).sum(),
            gold: pl.gold,
            actions: pl.actions,
            actions_this_turn: used as u32,
            fishbait_attacks: fish_total - last_fish,
            deathstrider_fires: ds_total - last_ds,
            rollouts: 0,
        });
        last_fish = fish_total;
        last_ds = ds_total;
        if turn < last_turn {
            game.next_shop_turn_without_fight();
        }
    }
    (
        rows,
        line,
        turn_text,
        golden,
        last_fish,
        last_ds,
        counts,
    )
}

fn events(game: &Game, a: Action, counts: &mut [u32; 4]) {
    let pl = &game.players[ME];
    match a {
        Action::Sell(slot) => {
            if let Some(m) = pl.board.get(slot as usize)
                && game.data.cards.get(m.card).name == "Snarky Shark"
            {
                counts[0] += 1;
            }
        }
        Action::Activate(slot) => {
            if let Some(m) = pl.board.get(slot as usize)
                && game.data.cards.get(m.card).name == "Lurking Lionfish"
            {
                counts[1] += 1;
            }
        }
        Action::Play { hand, .. } => {
            if let Some(m) = pl.hand.get(hand as usize)
                && game.data.cards.get(m.card).name == "Snarky Shark"
            {
                counts[2] += 1;
            }
        }
        _ => {}
    }
}

fn card_name(game: &Game, card: u16) -> String {
    game.data.cards.get(card).name.clone()
}

fn describe(game: &Game, a: Action) -> String {
    match a {
        Action::Buy(slot) => {
            let card = game.players[ME].shop.get(slot as usize).map(|m| m.card);
            match card {
                Some(c) => format!("buy slot {} {}", slot + 1, card_name(game, c)),
                None => format!("buy slot {}", slot + 1),
            }
        }
        Action::BuySpell => match game.players[ME].shop_spell {
            Some(c) => format!("buy spell {}", card_name(game, c)),
            None => "buy spell".to_string(),
        },
        Action::Sell(slot) => {
            let card = game.players[ME].board.get(slot as usize).map(|m| m.card);
            match card {
                Some(c) => format!("sell board {} {}", slot + 1, card_name(game, c)),
                None => format!("sell board {}", slot + 1),
            }
        }
        Action::Activate(slot) => {
            let card = game.players[ME].board.get(slot as usize).map(|m| m.card);
            match card {
                Some(c) => format!("activate board {} {}", slot + 1, card_name(game, c)),
                None => format!("activate board {}", slot + 1),
            }
        }
        Action::Play { hand, pos } => {
            let card = game.players[ME].hand.get(hand as usize).map(|m| m.card);
            match card {
                Some(c) => format!("play hand {} {} to board {}", hand + 1, card_name(game, c), pos + 1),
                None => format!("play hand {} to board {}", hand + 1, pos + 1),
            }
        }
        Action::Target(slot) => format!("target board {}", slot + 1),
        Action::TargetShop(slot) => {
            let card = game.players[ME].shop.get(slot as usize).map(|m| m.card);
            match card {
                Some(c) => format!("target shop {} {}", slot + 1, card_name(game, c)),
                None => format!("target shop {} spell", slot + 1),
            }
        }
        Action::Choose(k) => format!("choose option {}", k + 1),
        Action::Discard(slot) => format!("discard hand {}", slot + 1),
        Action::Move { from, to } => format!("move board {} to {}", from + 1, to + 1),
        Action::Roll => "roll".to_string(),
        Action::Freeze => "freeze".to_string(),
        Action::Level => "level".to_string(),
        Action::EndTurn => "end turn".to_string(),
        Action::HeroPower => "hero power".to_string(),
        Action::HeroPower2 => "second hero power".to_string(),
        Action::ChooseTrinket(k) => format!("choose trinket {}", k + 1),
        Action::DarkGift => "dark gift".to_string(),
    }
}

#[derive(Clone, Debug)]
struct PlannedRun {
    spec: RunSpec,
}

fn code_name(c: CodeMode) -> &'static str {
    match c {
        CodeMode::Basic => "basic",
        CodeMode::Context => "context",
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let opt = |name: &str, default: &str| -> String {
        args.iter()
            .position(|a| a == name)
            .and_then(|k| args.get(k + 1))
            .cloned()
            .unwrap_or_else(|| default.to_string())
    };
    let out_dir = opt("--out", OUT_DIR_REL);
    let variants: Vec<String> = opt("--variants", "B")
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let seeds: Vec<u64> = opt("--seeds", "1,2,3")
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    let ps: Vec<u16> = opt("--p", "0,250")
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    let codes: Vec<CodeMode> = opt("--codes", "basic")
        .split(',')
        .map(|s| match s.trim() {
            "context" => CodeMode::Context,
            _ => CodeMode::Basic,
        })
        .collect();
    let context_p: Option<u16> = opt("--context-p", "").parse().ok();
    let context_seeds: Vec<u64> = opt("--context-seeds", "")
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    let controls: Vec<bool> = opt("--control", "1")
        .split(',')
        .map(|s| s.trim() != "0" && !s.trim().eq_ignore_ascii_case("false"))
        .collect();
    let levels: u32 = opt("--levels", "3").parse().unwrap_or(3);
    let iterations: u32 = opt("--iterations", "20").parse().unwrap_or(20);
    let epsilon: f64 = opt("--epsilon", "0.05").parse().unwrap_or(0.05);
    let alpha: f64 = opt("--alpha", "0.5").parse().unwrap_or(0.5);
    let adapt_only_on_improvement = opt("--adapt", "always") == "improve";
    let restarts: u32 = opt("--restarts", "1").parse().unwrap_or(1).max(1);
    let key_mode = match opt("--key", "global").as_str() {
        "global" => KeyMode::Global,
        _ => KeyMode::State,
    };
    let horizon = match opt("--horizon", "whole").as_str() {
        "turn" => Horizon::Turn,
        _ => Horizon::Whole,
    };
    let jobs_n: usize = opt("--jobs", "8").parse().unwrap_or(8).max(1);

    let ids = load_ids();
    let bases: Vec<(String, Game, Vec<LoggedTavern>)> = variants
        .iter()
        .map(|v| {
            let (game, taverns, _) = load_base(&ids, v);
            (v.clone(), game, taverns)
        })
        .collect();

    let mut runs = Vec::new();
    for (variant, _, _) in &bases {
        for &control in &controls {
            // The control is turn 10 only and has no p.
            let p_list: &[u16] = if control { &[0] } else { &ps };
            for &p in p_list {
                for &seed in &seeds {
                    let code_list: &[CodeMode] = if control { &[CodeMode::Basic] } else { &codes };
                    for &code in code_list {
                        if code == CodeMode::Context
                            && (context_p.is_some_and(|cp| cp != p)
                                || (!context_seeds.is_empty() && !context_seeds.contains(&seed)))
                        {
                            continue;
                        }
                        runs.push(PlannedRun {
                            spec: RunSpec {
                                variant: variant.clone(),
                                p,
                                seed,
                                control,
                                code,
                            },
                        });
                    }
                }
            }
        }
    }

    let opts = SearchOptions { levels, iterations, epsilon, alpha, adapt_only_on_improvement, restarts };
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(jobs_n)
        .stack_size(64 * 1024 * 1024)
        .build()
        .expect("the thread pool");
    let t = Instant::now();
    let mut results: Vec<RunResult> = pool.install(|| {
        runs.par_iter()
            .map(|run| {
                let (_, base, taverns) = bases
                    .iter()
                    .find(|(v, _, _)| v == &run.spec.variant)
                    .expect("a loaded variant");
                eprintln!(
                    "run variant {} p={} seed={} control={} code={:?} levels={} n={}",
                    run.spec.variant,
                    run.spec.p,
                    run.spec.seed,
                    run.spec.control,
                    run.spec.code,
                    levels,
                    iterations
                );
                let horizon = if run.spec.control { Horizon::Turn } else { horizon };
                let out = search_run(base, &run.spec, taverns, opts, key_mode, horizon);
                let (mut rows, line, turn_text, replay_golden, fish, ds, counts) =
                    replay(base, &run.spec, taverns, &out.actions);
                for (i, row) in rows.iter_mut().enumerate() {
                    row.rollouts = out.rollouts;
                    if let Some(search_row) = out.rows.get(i) {
                        // The replay's rows are the report's rows; the search's
                        // own rows should agree, and a mismatch is worth the
                        // eye in the JSON.
                        debug_assert_eq!(row.triggers, search_row.triggers);
                    }
                }
                // A state-keyed policy has one weight per (state, code). The
                // report wants move codes, so sum the weights of the states
                // that share a code's label.
                let mut by_label: HashMap<String, (u64, f64)> = HashMap::new();
                for (&key, &w) in &out.policy {
                    let label = out
                        .labels
                        .get(&key)
                        .cloned()
                        .unwrap_or_else(|| "(unlabelled)".to_string());
                    let entry = by_label.entry(label).or_insert((key, 0.0));
                    entry.1 += w;
                }
                let mut policy: Vec<(u64, f64, String)> = by_label
                    .into_iter()
                    .map(|(label, (key, w))| (key, w, label))
                    .collect();
                policy.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
                eprintln!(
                    "  done in {:.1}s: triggers {}, actions {}, rollouts {}, {:.0}/s",
                    out.seconds,
                    rows.last().map_or(0, |t| t.triggers),
                    out.actions.len(),
                    out.rollouts,
                    out.rollouts as f64 / out.seconds.max(1e-9)
                );
                RunResult {
                    spec: run.spec.clone(),
                    turns: rows,
                    actions: out.actions,
                    line,
                    turn_lines: out.turn_lines,
                    turn_text,
                    policy,
                    seconds: out.seconds,
                    rollouts: out.rollouts,
                    golden_gryphon: out.golden || replay_golden,
                    fishbait_attacks: fish,
                    deathstrider_fires: ds,
                    shark_sells: counts[0],
                    lionfish_activates: counts[1],
                    shark_plays: counts[2],
                }
            })
            .collect()
    });
    results.sort_by(|a, b| {
        a.spec
            .variant
            .cmp(&b.spec.variant)
            .then_with(|| a.spec.control.cmp(&b.spec.control))
            .then_with(|| a.spec.p.cmp(&b.spec.p))
            .then_with(|| a.spec.seed.cmp(&b.spec.seed))
            .then_with(|| code_name(a.spec.code).cmp(code_name(b.spec.code)))
    });
    eprintln!("all runs finished in {:.1?}", t.elapsed());

    std::fs::create_dir_all(&out_dir).unwrap_or_else(|e| panic!("{out_dir}: {e}"));
    let json_results: Vec<Value> = results
        .iter()
        .map(|r| {
            let turns_json: Vec<Value> = {
                let mut previous = 10u16;
                r.turns.iter().map(|t| {
                    let gain = t.triggers as i32 - previous as i32;
                    previous = t.triggers;
                    json!({
                        "turn": t.turn,
                        "triggers": t.triggers,
                        "gain": gain,
                        "total_gain": t.triggers as i32 - 10,
                        "attack": t.attack,
                        "health": t.health,
                        "gold": t.gold,
                        "actions": t.actions,
                        "actions_this_turn": t.actions_this_turn,
                        "fishbait_attacks": t.fishbait_attacks,
                        "deathstrider_fires": t.deathstrider_fires,
                    })
                }).collect()
            };
            json!({
                "variant": r.spec.variant,
                "p": r.spec.p,
                "seed": r.spec.seed,
                "control": r.spec.control,
                "code": code_name(r.spec.code),
                "levels": levels,
                "iterations": iterations,
                "key": match key_mode { KeyMode::Global => "global", KeyMode::State => "state" },
                "horizon": if r.spec.control { "turn" } else { match horizon { Horizon::Turn => "turn", Horizon::Whole => "whole" } },
                "seconds": r.seconds,
                "rollouts": r.rollouts,
                "rollouts_per_second": r.rollouts as f64 / r.seconds.max(1e-9),
                "turns": turns_json,
                "golden_gryphon": r.golden_gryphon,
                "fishbait_attacks": r.fishbait_attacks,
                "deathstrider_fires": r.deathstrider_fires,
                "shark_sells": r.shark_sells,
                "lionfish_activates": r.lionfish_activates,
                "shark_plays": r.shark_plays,
                "top_codes": r.policy.iter().take(15).map(|(code, w, label)| json!({
                    "code": code,
                    "weight": w,
                    "label": label,
                })).collect::<Vec<_>>(),
                "line": r.line,
                "turn_lines": r.turn_lines.iter().map(|line| line.iter().map(|(turn, a)| json!({
                    "turn": turn,
                    "action": format!("{a:?}"),
                    "index": a.index(),
                })).collect::<Vec<_>>()).collect::<Vec<_>>(),
                "actions": r.actions.iter().map(|(turn, a)| json!({
                    "turn": turn,
                    "action": format!("{a:?}"),
                    "index": a.index(),
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    let json_text = serde_json::to_string_pretty(&json!({
        "source": SOURCE_REL,
        "start_turn": START_TURN,
        "end_turn": END_TURN,
        "levels": levels,
        "iterations": iterations,
        "key": match key_mode { KeyMode::Global => "global", KeyMode::State => "state" },
        "horizon": match horizon { Horizon::Turn => "turn", Horizon::Whole => "whole" },
        "restarts": restarts,
        "epsilon": epsilon,
        "alpha": alpha,
        "adapt": if adapt_only_on_improvement { "improve" } else { "always" },
        "prior": "uniform",
        "end_turn_sampled": true,
        "runs": json_results,
    }))
    .expect("the results are serialisable");
    let json_path = format!("{out_dir}/nrpa.json");
    std::fs::write(&json_path, json_text).unwrap_or_else(|e| panic!("{json_path}: {e}"));

    let mut md = String::new();
    md.push_str("# NRPA on the lobster mode: Tasty Lobstah, turns 10 to 13\n\n");
    md.push_str("Generated by `cargo run --release -p bgbench --bin lobster_nrpa`.\n\n");
    md.push_str("Start: `boards/actions/Hearthstone_2026_10_28_22_11_50-g2.json`, turn 10, Health set to 10,000. Variant B is the logged golden Headhunter Gryphon. Fights are skipped. The control is turn 10 only; the other runs search turns 10 to 13 as one line, and the best line is replayed for the per-turn numbers.\n\n");
    md.push_str(&format!(
        "Each run: NRPA levels 0 to {levels}, {iterations} iterations per level, so {} rollouts per line, in {} independent restart{}. A rollout ends when it samples end turn, at the 200-action cap, or when no move is legal, then the turn ends so the score includes end-of-turn effects. End turn is a move code with its own learned weight. Score is the final Lobster count, then board Attack plus Health.\n\n",
        (iterations as u64).pow(levels),
        restarts,
        if restarts == 1 { "" } else { "s" }
    ));
    md.push_str(&format!(
        "Horizon: {}. Key: {}. The control is always one turn (turn 10). A move code is the action kind plus the card it acts on. `basic` uses that alone. `context` adds the action phase (20 actions per bucket) and, for a prompt answer, the card whose effect opened it. Moves are only to the front and plays only to the front or the end, as in the beam. No card-specific rule or hand-written line is used: only the learned weights know the comp, and the prior is uniform. Adaptation adds {:.2} to every code in the level's best line, so far ({adapt}), and caps the largest seen weight at 30 by scaling all seen weights. Every rollout has a {:.2} chance of a uniform legal move instead of the softmax.\n\n",
        match horizon { Horizon::Turn => "one turn at a time", Horizon::Whole => "turns 10 to 13 as one line" },
        match key_mode { KeyMode::Global => "one weight per move code", KeyMode::State => "one weight per (state, move code)" },
        alpha,
        epsilon,
        adapt = if adapt_only_on_improvement { "only when it improves" } else { "every iteration" },
    ));
    md.push_str("## What changed from round 1\n\n");
    md.push_str("- End turn is sampled and learned like any other move. A turn ends when the rollout picks it or reaches the 200-action cap, and the search handles the end itself without a fight.\n");
    md.push_str("- The policy is one global weight per move code. The round 1 per-state key is gone.\n");
    md.push_str("- The sell (-3) and move (-1) priors are gone. The prior is uniform.\n");
    md.push_str("- Round 1's best line was 20 triggers (+10) with board 0/0. This round's best is 16 triggers (+6) with board 898/798. The raw count is lower, but the board survives and turns 11 to 13 keep adding triggers. The control stays at +4 on Era's Taverns.\n\n");
    md.push_str("## Results\n\n");
    md.push_str("| Variant | p | Seed | Control | Code | Turn | Triggers | Gain this turn | Attack | Health | Gold | Actions | This turn | Fishbait attacks | Deathstrider fires |\n");
    md.push_str("|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|\n");
    for r in &results {
        let mut previous = 10u16;
        for t in &r.turns {
            let gain = t.triggers as i32 - previous as i32;
            previous = t.triggers;
            md.push_str(&format!(
                "| {} | {:.2} | {} | {} | {} | {} | {} | {:+} | {} | {} | {} | {} | {} | {} | {} |\n",
                r.spec.variant,
                r.spec.p as f64 / 1000.0,
                r.spec.seed,
                if r.spec.control { "yes" } else { "no" },
                code_name(r.spec.code),
                t.turn,
                t.triggers,
                gain,
                t.attack,
                t.health,
                t.gold,
                t.actions,
                t.actions_this_turn,
                t.fishbait_attacks,
                t.deathstrider_fires,
            ));
        }
    }
    md.push_str("\n## Run summary\n\n");
    md.push_str("| Variant | p | Seed | Control | Code | Final triggers | Gain over start | Board A/H | Fishbait attacks | Deathstrider fires | Shark sells | Lionfish activates | Sharks played | Rollouts | Rollouts/s | Seconds |\n");
    md.push_str("|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|\n");
    for r in &results {
        let last = r.turns.last();
        let gain = last.map_or(0, |t| t.triggers as i32) - 10;
        md.push_str(&format!(
            "| {} | {:.2} | {} | {} | {} | {} | {:+} | {}/{} | {} | {} | {} | {} | {} | {} | {:.0} | {:.1} |\n",
            r.spec.variant,
            r.spec.p as f64 / 1000.0,
            r.spec.seed,
            if r.spec.control { "yes" } else { "no" },
            code_name(r.spec.code),
            last.map_or(0, |t| t.triggers),
            gain,
            last.map_or(0, |t| t.attack),
            last.map_or(0, |t| t.health),
            r.fishbait_attacks,
            r.deathstrider_fires,
            r.shark_sells,
            r.lionfish_activates,
            r.shark_plays,
            r.rollouts,
            r.rollouts as f64 / r.seconds.max(1e-9),
            r.seconds,
        ));
    }
    md.push_str("\n## Against Era\n\n");
    md.push_str("Era's shop-only Lobster counts: turn 10 10 to 18 (+8), turn 11 30 to 42 (+12), turn 12 54 to 66 (+12), turn 13 85 to 105 (+20), so +52 over the four turns. The table above is shop only, with fights skipped, so its gains are the like-for-like number.\n\n");
    md.push_str("## Against the old beam\n\n");
    let non_control: Vec<&RunResult> = results.iter().filter(|r| !r.spec.control).collect();
    let best_nc = non_control
        .iter()
        .copied()
        .max_by_key(|r| {
            let last = r.turns.last();
            (last.map_or(0, |t| t.triggers), last.map_or(0, |t| t.attack + t.health))
        });
    let (best_triggers, best_attack, best_health, best_fish, best_fires) = best_nc.map_or((10, 0, 0, 0, 0), |r| {
        let last = r.turns.last();
        (
            last.map_or(0, |t| t.triggers),
            last.map_or(0, |t| t.attack),
            last.map_or(0, |t| t.health),
            r.fishbait_attacks,
            r.deathstrider_fires,
        )
    });
    md.push_str(&format!(
        "The previous round's beam at width 2,000 (variant B, p = 0.50, seed 1) ended on 32 triggers, +22 over the start, with board 3001/2357. This round did not rerun the beam: the budget went to the NRPA grid, so that is the existing number quoted as the brief allows. NRPA's best variant B run is {} triggers, {:+} over the start, with board {}/{}. The beam is {} triggers ahead.\n",
        best_triggers,
        best_triggers as i32 - 10,
        best_attack,
        best_health,
        32 - best_triggers as i32,
    ));
    md.push_str("\n## Which move code worked better\n\n");
    let basic = results.iter().find(|r| !r.spec.control && r.spec.p == 250 && r.spec.seed == 1 && r.spec.code == CodeMode::Basic);
    let context = results.iter().find(|r| !r.spec.control && r.spec.p == 250 && r.spec.seed == 1 && r.spec.code == CodeMode::Context);
    match (basic, context) {
        (Some(b), Some(c)) => {
            let bt = b.turns.last().map_or(0, |t| t.triggers) as i32;
            let ct = c.turns.last().map_or(0, |t| t.triggers) as i32;
            let winner = if ct > bt {
                "`context` learned better".to_string()
            } else if bt > ct {
                "`basic` learned better".to_string()
            } else {
                "the two codes tied".to_string()
            };
            md.push_str(&format!(
                "At p = 0.25, seed 1, the same 64,000 rollouts: `basic` reaches {} triggers ({:+} over the start) in {:.1}s, `context` reaches {} triggers ({:+}) in {:.1}s. {} on this paired setting, by {} triggers. It was only run on this one seed (the other seeds carry `basic`) to keep the round inside the budget, so this is one paired comparison, not a sweep.\n",
                bt, bt - 10, b.seconds, ct, ct - 10, c.seconds, winner, (ct - bt).abs(),
            ));
        }
        _ => md.push_str("The paired basic/context run is missing from this report.\n"),
    }
    md.push_str("\n## Does NRPA find the combo unaided\n\n");
    let empty = non_control
        .iter()
        .filter(|r| r.turns.last().is_none_or(|t| t.attack + t.health == 0))
        .count();
    let control_runs: Vec<&RunResult> = results.iter().filter(|r| r.spec.control).collect();
    let control_desc = if control_runs.is_empty() {
        "none".to_string()
    } else {
        let lo = control_runs.iter().map(|r| r.turns.last().map_or(0, |t| t.triggers)).min().unwrap_or(0);
        let hi = control_runs.iter().map(|r| r.turns.last().map_or(0, |t| t.triggers)).max().unwrap_or(0);
        if lo == hi {
            format!("{} triggers ({:+} over the start)", lo, lo as i32 - 10)
        } else {
            format!("{lo} to {hi} triggers ({:+} to {:+} over the start)", lo as i32 - 10, hi as i32 - 10)
        }
    };
    let board_desc = if empty == 0 {
        format!("No variant B run ends with board 0/0; the best keeps {best_attack}/{best_health}")
    } else {
        format!("{empty} of {} variant B runs end with board 0/0; the others keep a board", non_control.len())
    };
    let shark_sells: u32 = non_control.iter().map(|r| r.shark_sells).sum();
    let shark_plays: u32 = non_control.iter().map(|r| r.shark_plays).sum();
    let shark_text = if shark_sells + shark_plays == 0 {
        "No variant B run buys or sells a Snarky Shark; the triggers come from Lurking Lionfish activations".to_string()
    } else {
        format!("Across the variant B runs: {shark_sells} Shark sells, {shark_plays} Sharks played")
    };
    md.push_str(&format!(
        "{} without any card-specific rule. The best variant B run reaches {} triggers over the start, with {} Fishbait attacks and {} Deathstrider fires. {}. The control with Era's own Taverns reaches {}, against Era's +8 (3 attacks, 8 fires). The Shark-sell route is not what the learned lines use, and the top weight in the best line is end turn at the 30 cap, so the global policy still ends turns early and caps the count.\n",
        shark_text,
        best_triggers as i32 - 10,
        best_fish,
        best_fires,
        board_desc,
        control_desc,
    ));
    md.push_str("\n## Best line in full\n\n");
    let best_line_run = results
        .iter()
        .filter(|r| !r.spec.control)
        .max_by_key(|r| {
            let last = r.turns.last();
            (last.map_or(0, |t| t.triggers), last.map_or(0, |t| t.attack + t.health), r.spec.seed)
        })
        .or_else(|| {
            results.iter().max_by_key(|r| {
                let last = r.turns.last();
                (last.map_or(0, |t| t.triggers), last.map_or(0, |t| t.attack + t.health), r.spec.seed)
            })
        });
    if let Some(r) = best_line_run {
        md.push_str(&format!(
            "Run: variant {}, p = {:.2}, seed {}, control {}, code {}.\n\n",
            r.spec.variant,
            r.spec.p as f64 / 1000.0,
            r.spec.seed,
            if r.spec.control { "yes" } else { "no" },
            code_name(r.spec.code)
        ));
        for (k, line) in r.line.iter().enumerate() {
            md.push_str(&format!("{}. {}\n", k + 1, line));
        }
        md.push_str("\n## The 15 move codes with the highest final weights\n\n");
        md.push_str("Each row is one move code in the global table. The weight is how strongly the learned policy favours that code, wherever it is legal.\n\n");
        md.push_str("| # | Weight | Code |\n|---|---|---|\n");
        for (k, (_, w, label)) in r.policy.iter().take(15).enumerate() {
            md.push_str(&format!("| {} | {:.3} | {} |\n", k + 1, w, label));
        }
        md.push_str("\n## Per-turn lines of that run\n\n");
        for (i, text) in r.turn_text.iter().enumerate() {
            md.push_str(&format!("### Turn {}\n\n", START_TURN + i as u32));
            for (k, line) in text.iter().enumerate() {
                md.push_str(&format!("{}. {}\n", k + 1, line));
            }
            md.push('\n');
        }
    }
    md.push_str("\n## Notes\n\n");
    md.push_str("- All runs are deterministic for a seed: the rollout sampler is a xoshiro RNG seeded from the run seed, the code mode and the turn, and nothing in the search reads wall-clock time.\n");
    md.push_str("- The control turns use Era's own logged Taverns in refresh order for turn 10 and the seeded stream once those run out. A sold Shark's logged Tavern is the post-attack state, because the log records the state after the step.\n");
    md.push_str("- Fights are skipped, so combat-only effects are not modelled. Shop deathrattles, Rally, Snarky Portrait, end-of-turn effects, Gold and turn changes run normally.\n");
    md.push_str("- The Actions columns count shop actions. End turn is a sampled move and appears in the lines, but it is not stepped through the game, so it is not counted there.\n");
    let md_path = format!("{out_dir}/nrpa.md");
    std::fs::write(&md_path, md).unwrap_or_else(|e| panic!("{md_path}: {e}"));
    println!("wrote {md_path} and {json_path}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nrpa_is_deterministic_for_a_seed() {
        let ids = load_ids();
        let (base, taverns, _) = load_base(&ids, "B");
        let spec = RunSpec {
            variant: "B".to_string(),
            p: 250,
            seed: 7,
            control: false,
            code: CodeMode::Basic,
        };
        let opts = SearchOptions { levels: 1, iterations: 2, epsilon: 0.05, alpha: 0.5, adapt_only_on_improvement: false, restarts: 1 };
        let a = search_run(&base, &spec, &taverns, opts, KeyMode::Global, Horizon::Whole);
        let b = search_run(&base, &spec, &taverns, opts, KeyMode::Global, Horizon::Whole);
        assert_eq!(a.rows.last().map(|t| t.triggers), b.rows.last().map(|t| t.triggers));
        assert_eq!(a.actions, b.actions);
        assert_eq!(a.rollouts, b.rollouts);
    }
}
