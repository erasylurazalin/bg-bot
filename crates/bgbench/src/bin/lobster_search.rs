//! The lobster search (docs/briefs/lobster-search.md): a deterministic beam
//! search over the shop of Era's turn 10 Tasty Lobstah board, turns 10 to 13,
//! with the Tavern and the Gryphon's random Beasts drawn from seeded streams.
//! Fights are skipped. Run it with:
//!
//!   cargo run --release -p bgbench --bin lobster_search -- [options]
//!
//! It writes `report.md` and `results.json` under the output directory.

use std::collections::{BinaryHeap, HashMap};
use std::cmp::Reverse;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Instant;

use bgcore::action::{ACTION_KINDS, N_ACTIONS};
use bgcore::cards::Counter;
use bgcore::shop_replay::{self, ShopFileSpec, ME};
use bgcore::{Action, CardIdMap, Data, Game, ShopStateCapture};
use rayon::prelude::*;
use serde_json::{Value, json};

const SOURCE: &str = "boards/actions/Hearthstone_2026_10_28_22_11_50-g2.json";
const REF: &str = "ref/bg-cards.json";
const START_TURN: u32 = 10;
const END_TURN: u32 = 13;

type Score = (u16, i32, i32);

#[derive(Clone)]
struct Node {
    state: ShopStateCapture,
    key: u64,
    score: Score,
    path: Option<Arc<Step>>,
    /// Actions since triggers or board stats last improved. A state that has
    /// found nothing for `STAGNANT` actions is an end candidate but is not
    /// expanded.
    stagnant: u16,
    /// Whether the last action was a board move, so a line cannot spend a
    /// whole turn shuffling bodies with no score change.
    last_move: bool,
}

const STAGNANT: u16 = 60;

struct Step {
    prev: Option<Arc<Step>>,
    action: Action,
    turn: u32,
}

impl Drop for Step {
    fn drop(&mut self) {
        // A path can be hundreds of steps long. Dropping it as a chain of
        // `Arc`s would recurse once per step and overflow the stack, so walk
        // the unique tail by hand.
        let mut prev = self.prev.take();
        while let Some(arc) = prev {
            match Arc::try_unwrap(arc) {
                Ok(mut step) => prev = step.prev.take(),
                Err(shared) => {
                    drop(shared);
                    break;
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct HeapItem {
    score: Score,
    seq: u64,
    key: u64,
}

impl Ord for HeapItem {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.score
            .cmp(&other.score)
            .then_with(|| self.seq.cmp(&other.seq))
    }
}

impl PartialOrd for HeapItem {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// A beam layer: the best `width` distinct states under `state_key`, with a
/// bounded heap so a wide layer never holds every successor at once.
struct Trimmer {
    width: usize,
    seq: u64,
    map: HashMap<u64, (u64, Node)>,
    heap: BinaryHeap<Reverse<HeapItem>>,
}

impl Trimmer {
    fn new(width: usize) -> Trimmer {
        Trimmer { width: width.max(1), seq: 0, map: HashMap::new(), heap: BinaryHeap::new() }
    }

    fn insert(&mut self, key: u64, node: Node) {
        if let Some((seq, old)) = self.map.get_mut(&key) {
            if old.score >= node.score {
                return;
            }
            self.seq += 1;
            *seq = self.seq;
            *old = node;
            let score = old.score;
            self.heap.push(Reverse(HeapItem { score, seq: self.seq, key }));
        } else {
            self.seq += 1;
            let score = node.score;
            self.map.insert(key, (self.seq, node));
            self.heap.push(Reverse(HeapItem { score, seq: self.seq, key }));
        }
        while self.map.len() > self.width {
            let Some(Reverse(item)) = self.heap.pop() else { break };
            if self.map.get(&item.key).is_some_and(|(seq, _)| *seq == item.seq) {
                self.map.remove(&item.key);
            }
        }
    }

    fn len(&self) -> usize {
        self.map.len()
    }

    fn into_nodes(mut self) -> Vec<Node> {
        let mut out: Vec<Node> = self.map.drain().map(|(_, (_, node))| node).collect();
        out.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then_with(|| b.key.cmp(&a.key))
        });
        out
    }
}

fn score(game: &Game) -> Score {
    let pl = &game.players[ME];
    let stats = pl.board.iter().map(|m| m.attack + m.health).sum();
    // The brief's score is triggers, then board stats. Fewer actions is only
    // a tie-break, so a line does not shuffle the board for nothing.
    (pl.improve[Counter::TastyLobster.index()], stats, -(pl.actions as i32))
}

fn improved(a: Score, b: Score) -> bool {
    a.0 > b.0 || (a.0 == b.0 && a.1 > b.1)
}

fn state_key(base: &Game, state: &ShopStateCapture) -> u64 {
    let mut h = DefaultHasher::new();
    base.lobby.hash(&mut h);
    state.hash(&mut h);
    h.finish()
}

fn game_from(base: &Game, state: &ShopStateCapture) -> Game {
    let mut game = base.clone();
    game.restore_shop_state(ME, state);
    game
}

fn searched(a: Action, board_len: usize) -> bool {
    // The brief's list: buy, sell, play, activate, move, roll, level, targets,
    // choose and end turn. End turn is handled by the turn step itself, and
    // Freeze, powers, trinket answers and the Dark Gift button are not part of
    // this mode's line.
    //
    // Two generic position prunes keep the branching factor sane. A move is
    // only tried to the front: every order can still be built by repeated
    // front moves. A minion is only played to the front or the end: those are
    // the two places that decide which body attacks and which body is the
    // left-most Deathrattle, and the front moves can arrange anything else.
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
    !matches!(
        a,
        Action::EndTurn
            | Action::Freeze
            | Action::HeroPower
            | Action::HeroPower2
            | Action::ChooseTrinket(_)
            | Action::DarkGift
    )
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
                None => format!("target shop {}", slot + 1),
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

fn load_base(ids: &CardIdMap, variant: &str) -> (ShopFileSpec, Game) {
    let text = std::fs::read_to_string(SOURCE).unwrap_or_else(|e| panic!("{SOURCE}: {e}"));
    let name = SOURCE.rsplit('/').next().unwrap_or(SOURCE).to_string();
    let mut files = shop_replay::parse_action_files(&[(name, text)])
        .unwrap_or_else(|e| panic!("{SOURCE}: {e}"));
    shop_replay::seat_taught_spells(&mut files, ids);
    let data = Arc::new(Data::bundled());
    let file = files.remove(0);
    let turn = file
        .turns
        .iter()
        .find(|t| t.turn == START_TURN)
        .unwrap_or_else(|| panic!("{SOURCE}: no turn {START_TURN}"));
    let (mut game, missing) = shop_replay::turn_start_game(&data, ids, &file, turn, 0x10b5_7e12)
        .unwrap_or_else(|| panic!("{SOURCE}: turn {START_TURN} does not seat"));
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
    (file, game)
}

fn config_game(base: &Game, seed: u64, per_mille: u16) -> Game {
    let mut game = base.clone();
    game.players[ME].health = 10_000;
    game.enable_lobster_mode(ME, seed, per_mille);
    game.seat_lobster_tavern(ME);
    game
}

fn successors(node: &Node, base: &Game, cap: u32) -> Vec<Node> {
    if node.state.player.actions >= cap as u16 || node.stagnant >= STAGNANT {
        return Vec::new();
    }
    let saved = node.state.clone();
    let mut game = game_from(base, &saved);
    let mut mask = [false; N_ACTIONS];
    game.legal_mask(&mut mask);
    let mut out = Vec::new();
    for (i, ok) in mask.iter().enumerate() {
        if !*ok {
            continue;
        }
        let Some(a) = Action::from_index(i) else { continue };
        if !searched(a, saved.player.board.len()) {
            continue;
        }
        if node.last_move && matches!(a, Action::Move { .. }) {
            continue;
        }
        // One mutable game is restored to the parent state for each action,
        // which copies the 18 KiB player rather than the 157 KiB game.
        game.restore_shop_state(ME, &saved);
        if game.step(a).is_err() {
            continue;
        }
        if game.turn != saved.turn {
            continue;
        }
        let actions = game.players[ME].actions;
        if actions <= saved.player.actions {
            continue;
        }
        let score = score(&game);
        let state = game.capture_shop_state(ME);
        let key = state_key(base, &state);
        let stagnant = if improved(score, node.score) { 0 } else { node.stagnant.saturating_add(1) };
        out.push(Node {
            state,
            key,
            score,
            path: Some(Arc::new(Step {
                prev: node.path.clone(),
                action: a,
                turn: game.turn,
            })),
            stagnant,
            last_move: matches!(a, Action::Move { .. }),
        });
    }
    out
}

/// One turn's beam. Every state is also a candidate for ending the turn; the
/// best `width` end states become the next turn's roots.
fn search_turn(
    roots: Vec<Node>,
    base: &Game,
    width: usize,
    cap: u32,
    end_top: usize,
) -> Vec<Node> {
    let mut beam = roots;
    // The beam is `width`; the end-of-turn carry is smaller, since only the
    // best distinct end states are worth starting the next turn from.
    let mut ends = Trimmer::new(width.min(4096));
    let mut depth = 0u32;
    loop {
        if verbose() && depth < 2 {
            let mut kinds = [0usize; bgcore::N_KINDS];
            for node in &beam {
                let game = game_from(base, &node.state);
                let mut mask = [false; N_ACTIONS];
                game.legal_mask(&mut mask);
                for (i, ok) in mask.iter().enumerate() {
                    if !*ok {
                        continue;
                    }
                    let mut at = 0;
                    for (k, (_, n)) in ACTION_KINDS.iter().enumerate() {
                        if i < at + n {
                            kinds[k] += 1;
                            break;
                        }
                        at += n;
                    }
                }
            }
            eprintln!("  turn {} depth {} action kinds {:?}", beam[0].state.turn, depth, kinds);
        }
        let end_n = beam.len().min(end_top.max(1));
        for node in beam.iter().take(end_n) {
            let mut game = game_from(base, &node.state);
            game.end_shop_turn(ME);
            let score = score(&game);
            let state = game.capture_shop_state(ME);
            let key = state_key(base, &state);
            let end = Node {
                state,
                key,
                score,
                path: node.path.clone(),
                stagnant: node.stagnant,
                last_move: node.last_move,
            };
            ends.insert(key, end);
        }
        if beam.is_empty() || depth >= cap {
            break;
        }
        let mut next = Trimmer::new(width);
        let mut generated = 0usize;
        for node in &beam {
            let succ = successors(node, base, cap);
            generated += succ.len();
            for succ in succ {
                let key = succ.key;
                next.insert(key, succ);
            }
        }
        if next.len() == 0 {
            break;
        }
        beam = next.into_nodes();
        depth += 1;
        if depth.is_multiple_of(25) || verbose() {
            eprintln!(
                "  depth {depth}, beam {}, ends {}, generated {} ({:.1} per state)",
                beam.len(),
                ends.len(),
                generated,
                generated as f64 / beam.len().max(1) as f64
            );
        }
    }
    ends.into_nodes()
}

fn path_actions(path: &Option<Arc<Step>>) -> Vec<(u32, Action)> {
    let mut out = Vec::new();
    let mut cur = path.as_ref();
    while let Some(step) = cur {
        out.push((step.turn, step.action));
        cur = step.prev.as_ref();
    }
    out.reverse();
    out
}

fn has_lobster(game: &Game) -> bool {
    game.players[ME]
        .board
        .iter()
        .any(|m| game.data.cards.get(m.card).name == "Tasty Lobster")
}

fn has_fishbait_source(game: &Game) -> bool {
    let pl = &game.players[ME];
    let lionfish = pl.board.iter().any(|m| {
        let c = game.data.cards.get(m.card);
        c.name == "Lurking Lionfish" && pl.gold >= c.activate.map_or(u8::MAX, |a| a.cost)
    });
    let shark = pl
        .board
        .iter()
        .chain(pl.hand.iter())
        .any(|m| game.data.cards.get(m.card).name == "Snarky Shark");
    let fish = pl.shop.iter().any(|m| game.data.cards.get(m.card).name == "Fishbait");
    lionfish || shark || fish
}

fn stop_reason(game: &Game, cap: u32) -> &'static str {
    let actions = game.players[ME].actions as u32;
    if actions >= cap {
        return "cap reached";
    }
    if !has_lobster(game) {
        return "no triggers left";
    }
    if !has_fishbait_source(game) {
        return "no triggers left";
    }
    if game.players[ME].gold == 0 {
        return "Gold out";
    }
    "search pruned"
}

fn events(game_before: &Game, a: Action, counts: &mut [u32; 4]) {
    match a {
        Action::Sell(slot) => {
            if let Some(m) = game_before.players[ME].board.get(slot as usize)
                && game_before.data.cards.get(m.card).name == "Snarky Shark"
            {
                counts[0] += 1;
            }
        }
        Action::Activate(slot) => {
            if let Some(m) = game_before.players[ME].board.get(slot as usize)
                && game_before.data.cards.get(m.card).name == "Lurking Lionfish"
            {
                counts[1] += 1;
            }
        }
        Action::Play { hand, .. } => {
            if let Some(m) = game_before.players[ME].hand.get(hand as usize)
                && game_before.data.cards.get(m.card).name == "Snarky Shark"
            {
                counts[2] += 1;
            }
        }
        Action::TargetShop(slot) => {
            if let Some(m) = game_before.players[ME].shop.get(slot as usize)
                && game_before.data.cards.get(m.card).name == "Fishbait"
            {
                counts[3] += 1;
            }
        }
        _ => {}
    }
}

#[derive(Clone, Debug)]
struct TurnRow {
    turn: u32,
    triggers: u16,
    attack: i32,
    health: i32,
    gold: u8,
    actions: u16,
    stop: &'static str,
    actions_this_turn: u32,
    fishbait_attacks: u32,
    deathstrider_fires: u32,
}

#[derive(Clone, Debug)]
struct RunResult {
    variant: String,
    p: u16,
    width: usize,
    seed: u64,
    turns: Vec<TurnRow>,
    actions: Vec<(u32, Action)>,
    line: Vec<String>,
    shark_sells: u32,
    lionfish_activates: u32,
    shark_plays: u32,
    fishbait_picks: u32,
    golden_gryphon: bool,
    fishbait_attacks: u32,
    deathstrider_fires: u32,
    seconds: f64,
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

fn replay(base: &Game, cfg: &RunResult) -> RunResult {
    let mut game = config_game(base, cfg.seed, cfg.p);
    let mut counts = [0u32; 4];
    let mut line = Vec::new();
    let mut rows = Vec::new();
    let mut last_fishbait = 0u32;
    let mut last_deathstrider = 0u32;
    let mut golden_seen = golden_gryphon(&game);
    let cap = game.data.rules.max_actions_per_turn as u32;
    let mut at = 0usize;
    for turn in START_TURN..=END_TURN {
        let start_actions = game.players[ME].actions;
        while at < cfg.actions.len() && cfg.actions[at].0 == turn {
            let a = cfg.actions[at].1;
            line.push(describe(&game, a));
            events(&game, a, &mut counts);
            if game.step(a).is_err() {
                line.push(format!("  (stop: {a:?} is no longer legal)"));
                break;
            }
            golden_seen |= golden_gryphon(&game);
            at += 1;
        }
        let used = game.players[ME].actions - start_actions;
        let stop = stop_reason(&game, cap);
        game.end_shop_turn(ME);
        let pl = &game.players[ME];
        let attack: i32 = pl.board.iter().map(|m| m.attack).sum();
        let health: i32 = pl.board.iter().map(|m| m.health).sum();
        let fishbait_total = pl.lobster.map_or(0, |m| m.fishbait_attacks);
        let deathstrider_total = pl.lobster.map_or(0, |m| m.deathstrider_fires);
        let fishbait_attacks = fishbait_total - last_fishbait;
        let deathstrider_fires = deathstrider_total - last_deathstrider;
        last_fishbait = fishbait_total;
        last_deathstrider = deathstrider_total;
        rows.push(TurnRow {
            turn,
            triggers: pl.improve[Counter::TastyLobster.index()],
            attack,
            health,
            gold: pl.gold,
            actions: pl.actions,
            stop,
            actions_this_turn: used as u32,
            fishbait_attacks,
            deathstrider_fires,
        });
        if turn < END_TURN {
            game.next_shop_turn_without_fight();
        }
    }
    RunResult {
        turns: rows,
        actions: cfg.actions.clone(),
        line,
        shark_sells: counts[0],
        lionfish_activates: counts[1],
        shark_plays: counts[2],
        fishbait_picks: counts[3],
        golden_gryphon: golden_seen,
        fishbait_attacks: last_fishbait,
        deathstrider_fires: last_deathstrider,
        ..cfg.clone()
    }
}

fn run(base: &Game, variant: &str, p: u16, width: usize, seed: u64, cap_opt: Option<u32>, end_top: usize) -> RunResult {
    let t = Instant::now();
    let start = config_game(base, seed, p);
    let cap = cap_opt.unwrap_or(start.data.rules.max_actions_per_turn as u32);
    let root_state = start.capture_shop_state(ME);
    let root = Node {
        key: state_key(&start, &root_state),
        score: score(&start),
        state: root_state,
        path: None,
        stagnant: 0,
        last_move: false,
    };
    let mut roots = vec![root];
    let mut final_ends = Vec::new();
    for turn in START_TURN..=END_TURN {
        let ends = search_turn(roots, &start, width, cap, end_top);
        if turn == END_TURN {
            final_ends = ends;
            break;
        }
        let mut next = Vec::new();
        for mut node in ends {
            let mut game = game_from(&start, &node.state);
            game.next_shop_turn_without_fight();
            node.score = score(&game);
            node.state = game.capture_shop_state(ME);
            node.key = state_key(&start, &node.state);
            next.push(node);
        }
        roots = next;
        if roots.is_empty() {
            eprintln!("no end states after turn {turn} ({variant} p={p} seed={seed})");
            break;
        }
    }
    let best = final_ends
        .into_iter()
        .max_by(|a, b| a.score.cmp(&b.score).then_with(|| a.key.cmp(&b.key)))
        .expect("at least one end state");
    let actions = path_actions(&best.path);
    let shell = RunResult {
        variant: variant.to_string(),
        p,
        width,
        seed,
        turns: Vec::new(),
        actions,
        line: Vec::new(),
        shark_sells: 0,
        lionfish_activates: 0,
        shark_plays: 0,
        fishbait_picks: 0,
        golden_gryphon: false,
        fishbait_attacks: 0,
        deathstrider_fires: 0,
        seconds: 0.0,
    };
    let mut out = replay(base, &shell);
    out.seconds = t.elapsed().as_secs_f64();
    out
}

fn verbose() -> bool {
    std::env::var_os("LOBSTER_VERBOSE").is_some()
}

fn p_label(p: u16) -> String {
    if p == 0 { "0".to_string() } else { format!("{:.2}", p as f64 / 1000.0) }
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
    let out_dir = opt("--out", "train/runs/lobster-search");
    let widths: Vec<usize> = opt("--widths", "2000,20000")
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    let seeds: Vec<u64> = opt("--seeds", "1,2,3")
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    let ps: Vec<u16> = opt("--p", "0,100,250,500")
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    let cap_opt = args
        .iter()
        .position(|a| a == "--cap")
        .and_then(|k| args.get(k + 1))
        .and_then(|s| s.parse::<u32>().ok());
    let end_top: usize = opt("--end-top", "64").parse().unwrap_or(64);
    let variants: Vec<String> = opt("--variants", "A,B")
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let ids = CardIdMap::parse(
        &std::fs::read_to_string(REF).unwrap_or_else(|e| panic!("{REF}: {e}")),
    )
    .unwrap_or_else(|e| panic!("{REF}: {e}"));
    let start = Instant::now();
    let bases: Vec<(String, Game)> = variants
        .iter()
        .map(|v| (v.clone(), load_base(&ids, v).1))
        .collect();
    eprintln!(
        "start positions loaded in {:.1?}; Game {} bytes, Player {} bytes, Node {} bytes",
        start.elapsed(),
        std::mem::size_of::<Game>(),
        std::mem::size_of::<bgcore::Player>(),
        std::mem::size_of::<Node>()
    );

    #[derive(Clone)]
    struct Job {
        base: usize,
        variant: String,
        p: u16,
        width: usize,
        seed: u64,
    }
    let mut jobs = Vec::new();
    for (base, (variant, _)) in bases.iter().enumerate() {
        for &p in &ps {
            for &width in &widths {
                for &seed in &seeds {
                    jobs.push(Job {
                        base,
                        variant: variant.clone(),
                        p,
                        width,
                        seed,
                    });
                }
            }
        }
    }
    let jobs_n: usize = opt("--jobs", "1").parse().unwrap_or(1).max(1);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(jobs_n)
        // A Game holds eight Players and is moved by value in a few places;
        // the default 2 MiB worker stack is not enough for a wide search.
        .stack_size(32 * 1024 * 1024)
        .build()
        .expect("the thread pool");
    let start_runs = Instant::now();
    let results: Vec<RunResult> = pool.install(|| {
        jobs.par_iter()
            .map(|job| {
                eprintln!(
                    "run variant {} p={} width={} seed={}",
                    job.variant,
                    p_label(job.p),
                    job.width,
                    job.seed
                );
                let r = run(
                    &bases[job.base].1,
                    &job.variant,
                    job.p,
                    job.width,
                    job.seed,
                    cap_opt,
                    end_top,
                );
                eprintln!(
                    "  done in {:.1}s: triggers {}, actions {}, line {}",
                    r.seconds,
                    r.turns.last().map_or(0, |t| t.triggers),
                    r.turns.last().map_or(0, |t| t.actions),
                    r.actions.len()
                );
                r
            })
            .collect()
    });
    eprintln!("all runs finished in {:.1?}", start_runs.elapsed());

    std::fs::create_dir_all(&out_dir).unwrap_or_else(|e| panic!("{out_dir}: {e}"));
    let json_results: Vec<Value> = results
        .iter()
        .map(|r| {
            json!({
                "variant": r.variant,
                "p": p_label(r.p),
                "width": r.width,
                "seed": r.seed,
                "seconds": r.seconds,
                "turns": r.turns.iter().map(|t| json!({
                    "turn": t.turn,
                    "triggers": t.triggers,
                    "attack": t.attack,
                    "health": t.health,
                    "gold": t.gold,
                    "actions": t.actions,
                    "actions_this_turn": t.actions_this_turn,
                    "fishbait_attacks": t.fishbait_attacks,
                    "deathstrider_fires": t.deathstrider_fires,
                    "stop": t.stop,
                })).collect::<Vec<_>>(),
                "golden_gryphon": r.golden_gryphon,
                "fishbait_attacks": r.fishbait_attacks,
                "deathstrider_fires": r.deathstrider_fires,
                "shark_sells": r.shark_sells,
                "lionfish_activates": r.lionfish_activates,
                "shark_plays": r.shark_plays,
                "fishbait_picks": r.fishbait_picks,
                "line": r.line,
                "actions": r.actions.iter().map(|(turn, a)| json!({
                    "turn": turn,
                    "action": format!("{a:?}"),
                    "index": a.index(),
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    let json_text = serde_json::to_string_pretty(&json!({
        "source": SOURCE,
        "start_turn": START_TURN,
        "end_turn": END_TURN,
        "beam_widths": widths,
        "seeds": seeds,
        "p": ps.iter().map(|&p| p_label(p)).collect::<Vec<_>>(),
        "runs": json_results,
    }))
    .expect("the results are serialisable");
    let json_path = format!("{out_dir}/results.json");
    std::fs::write(&json_path, json_text).unwrap_or_else(|e| panic!("{json_path}: {e}"));

    let mut md = String::new();
    md.push_str("# Lobster search: Tasty Lobstah, turns 10 to 13\n\n");
    md.push_str("Generated by `cargo run --release -p bgbench --bin lobster_search`.\n\n");
    md.push_str("Start: `boards/actions/Hearthstone_2026_10_28_22_11_50-g2.json`, turn 10, Health set to 10,000. Lobster count read from its `numbers` pair (22/11) as 22 / 2 - 1 = 10 for a plain Lobster; a golden Lobster prints `[4(c+1), 2(c+1)]` and reads as `attack / 4 - 1`, the same player-wide count. Variant A is the logged board with the Headhunter Gryphon made plain (its logged 65/63 kept); variant B is the logged golden Gryphon.\n\n");
    md.push_str("## Results\n\n");
    md.push_str("| Variant | p | Width | Seed | Turn | Triggers | Attack | Health | Gold | Actions | This turn | Fishbait attacks | Deathstrider fires | Stopped |\n");
    md.push_str("|---|---|---|---|---|---|---|---|---|---|---|---|---|---|\n");
    for r in &results {
        for t in &r.turns {
            md.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
                r.variant,
                p_label(r.p),
                r.width,
                r.seed,
                t.turn,
                t.triggers,
                t.attack,
                t.health,
                t.gold,
                t.actions,
                t.actions_this_turn,
                t.fishbait_attacks,
                t.deathstrider_fires,
                t.stop
            ));
        }
    }
    md.push_str("\n## Final lines and what the search found\n\n");
    md.push_str("| Variant | p | Width | Seed | Triggers | Attack | Health | Actions | Golden Gryphon | Fishbait attacks | Deathstrider fires | Shark sells | Lionfish activates | Sharks played | Fishbait picks |\n");
    md.push_str("|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|\n");
    for r in &results {
        let last = r.turns.last().cloned().unwrap_or(TurnRow {
            turn: END_TURN,
            triggers: 0,
            attack: 0,
            health: 0,
            gold: 0,
            actions: 0,
            stop: "",
            actions_this_turn: 0,
            fishbait_attacks: 0,
            deathstrider_fires: 0,
        });
        md.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
            r.variant,
            p_label(r.p),
            r.width,
            r.seed,
            last.triggers,
            last.attack,
            last.health,
            last.actions,
            if r.golden_gryphon { "yes" } else { "no" },
            r.fishbait_attacks,
            r.deathstrider_fires,
            r.shark_sells,
            r.lionfish_activates,
            r.shark_plays,
            r.fishbait_picks
        ));
    }
    let widest = widths.iter().copied().max().unwrap_or(0);
    md.push_str("\n## What the search found\n\n");
    let a_runs: Vec<&RunResult> =
        results.iter().filter(|r| r.variant == "A" && r.width == widest).collect();
    let a_gold: Vec<&RunResult> = a_runs.iter().copied().filter(|r| r.golden_gryphon).collect();
    md.push_str(&format!(
        "- Variant A at the widest beam ({}): a golden Headhunter Gryphon appears in {} of {} best lines{}.\n",
        widest,
        a_gold.len(),
        a_runs.len(),
        if a_gold.is_empty() {
            String::new()
        } else {
            let mut ps: Vec<String> = a_gold.iter().map(|r| p_label(r.p)).collect();
            ps.sort();
            ps.dedup();
            format!(" (p = {})", ps.join(", "))
        }
    ));
    let mut cap_ps = Vec::new();
    for &p in &ps {
        let hits = results
            .iter()
            .filter(|r| r.variant == "B" && r.p == p && r.turns.last().is_some_and(|t| t.actions >= 200))
            .count();
        let n = results.iter().filter(|r| r.variant == "B" && r.p == p).count();
        if hits > 0 && n > 0 {
            cap_ps.push((p, hits, n));
        }
    }
    if let Some((p, hits, n)) = cap_ps.first() {
        md.push_str(&format!(
            "- Variant B first hits the 200-action cap at p = {} ({} of {} runs).\n",
            p_label(*p),
            hits,
            n
        ));
    } else {
        md.push_str("- Variant B did not hit the 200-action cap at any p in these runs.\n");
    }
    let b_best = results
        .iter()
        .filter(|r| r.variant == "B")
        .max_by_key(|r| (r.turns.last().map_or(0, |t| t.triggers), r.width, r.seed));
    if let Some(r) = b_best {
        md.push_str(&format!(
            "- The best variant B line (p = {}, width {}, seed {}) has {} Shark sells, {} Lionfish activates, {} Sharks played and {} Fishbait picks, for {} triggers.\n",
            p_label(r.p),
            r.width,
            r.seed,
            r.shark_sells,
            r.lionfish_activates,
            r.shark_plays,
            r.fishbait_picks,
            r.turns.last().map_or(0, |t| t.triggers)
        ));
    }
    md.push_str("\n## Full best line, variant A, p = 0.5, widest beam, seed 1\n\n");
    if let Some(r) = results.iter().find(|r| {
        r.variant == "A" && r.p == 500 && r.width == widest && r.seed == 1
    }) {
        for (k, line) in r.line.iter().enumerate() {
            md.push_str(&format!("{}. {}\n", k + 1, line));
        }
    } else {
        md.push_str("(that setting was not run)\n");
    }
    md.push_str("\n## Full best line, variant B, p = 0.5, widest beam, seed 1\n\n");
    if let Some(r) = results.iter().find(|r| {
        r.variant == "B" && r.p == 500 && r.width == widest && r.seed == 1
    }) {
        for (k, line) in r.line.iter().enumerate() {
            md.push_str(&format!("{}. {}\n", k + 1, line));
        }
    } else {
        md.push_str("(that setting was not run)\n");
    }
    md.push_str("\n## Notes\n\n");
    md.push_str("- A move is pruned only by the beam width and by duplicate-state merging. Every legal action from the brief's list is tried; the game's own `is_legal` is the filter, and no card-specific rule is used.\n");
    md.push_str("- The Tavern stream is per turn and per refresh number; the Gryphon Beast stream is per turn and per trigger. Both are pure functions of the seed and the index, so the path taken does not change them.\n");
    md.push_str("- The mode does not read or consume the pool, and the extra Sharks and Lionfish are generated tokens. Normal cards in the stream are weighted like a full pool at their tier; a bought or sold card never changes what a later stream index holds.\n");
    md.push_str("- Fights are skipped, so combat-only effects (Windfury, Divine Shield, attack damage, deaths in combat, hero damage) are not modelled. Shop deathrattles, Rally, Snarky Portrait, end-of-turn effects, Gold and turn changes run normally.\n");
    let md_path = format!("{out_dir}/report.md");
    std::fs::write(&md_path, md).unwrap_or_else(|e| panic!("{md_path}: {e}"));
    println!("wrote {md_path} and {json_path}");
}
