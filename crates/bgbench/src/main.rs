//! Benchmarks and a fuzzer for bgcore, with random legal play.
//!
//!   bgbench fight [n]       time n fights between random boards on one core
//!   bgbench games [secs]    full games on every core, games per second
//!   bgbench fuzz [games]    games on every core, invariants after every step
//!   bgbench match [games]   curve bots against random bots, average placement
//!   bgbench peak [n]        peak board balance matrix (n fights each way per pair)
//!   bgbench fightcheck      play Era's real fights N times each, write boards/checks/
//!                           [--boards DIR] [--runs N] [--out DIR] [--flag PCT]
//!   bgbench powerlog FILE   every game of a Power.log as its entities and tags stood
//!                           [--out FILE] [--time]
//!   bgbench tracker FILE    the Battlegrounds seats and fights of a Power.log
//!                           [--out FILE] [--cards FILE] [--time]
//!   bgbench record          my logged shop decisions as training examples
//!                           [--actions DIR] [--out DIR] [--seeds N]
//!
//! `fuzz` and `games` also take --real-boards DIR [--real-seats N] to seat
//! real-board seats playing the boards of DIR (docs/real-boards.md), the
//! small fixture crate bgcore/tests/data/real-boards included.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use bgcore::bots::{random_action, rush_action, standard_action};
use bgcore::comps::Comps;
use bgcore::planner::Planner;
use bgcore::fight_check::{CheckOptions, CheckReport, check_fight};
use bgcore::real_boards::{CardIdMap, RealBoards, pair_fights, parse_board_files};
use bgcore::{
    Attached, BOARD, CardId, Counters, Data, Game, Keywords, Minion, PLAYERS, RealBoardSpec, Rng,
    fight, fight_with,
};
use rayon::prelude::*;
use serde::Deserialize;

mod check_actions;
mod collect_actions;
mod collect_boards;
mod powerlog;
mod record_examples;
mod tracker;

/// The boards real seats play, and how many seats of a game are real.
type Real = Option<(Arc<RealBoards>, usize)>;

/// The board files of `dir`: (name, text) per game, in name order.
fn read_board_files(dir: &str) -> Result<Vec<(String, String)>, String> {
    let mut files: Vec<(String, String)> = std::fs::read_dir(dir)
        .map_err(|e| format!("{dir}: {e}"))?
        .map(|e| e.map_err(|e| format!("{dir}: {e}")))
        .filter_map(|e| e.ok())
        .filter(|e| {
            let n = e.file_name();
            let n = n.to_string_lossy();
            n.ends_with(".json") && !n.starts_with('.')
        })
        .map(|e| {
            let p = e.path();
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            let text = std::fs::read_to_string(&p)
                .unwrap_or_else(|err| panic!("{}: {err}", p.display()));
            (name, text)
        })
        .collect();
    files.sort();
    if files.is_empty() {
        return Err(format!("{dir}: no board files"));
    }
    Ok(files)
}

/// Read the board files in `dir` (docs/real-boards.md) and resolve them
/// against the card set, mapping card ids through ref/bg-cards.json. Prints
/// what was loaded: boards per turn and the unmatched minions by name.
fn load_real_boards(dir: &str) -> Result<Arc<RealBoards>, String> {
    let files = read_board_files(dir)?;
    let ref_json = std::fs::read_to_string("ref/bg-cards.json")
        .map_err(|e| format!("ref/bg-cards.json (the card id mapping): {e}"))?;
    let ids = CardIdMap::parse(&ref_json)?;
    let (specs, unknown): (Vec<RealBoardSpec>, Vec<String>) = parse_board_files(&files, &ids)?;
    let data = Data::bundled();
    let (boards, unmatched) = RealBoards::build(&specs, &data.cards, &data.gifts);
    let mut by_name = unknown;
    by_name.extend(unmatched);
    println!(
        "{} boards from {} games in {dir}",
        boards.len(),
        files.len()
    );
    let counts = boards.counts_by_turn();
    let per_turn: Vec<String> = counts[..counts.len() - 1]
        .iter()
        .enumerate()
        .filter(|&(_, &n)| n > 0)
        .map(|(t, &n)| format!("t{}: {n}", t + 1))
        .chain(std::iter::once(format!(
            "{}+: {}",
            bgcore::LATE_TURN,
            counts[counts.len() - 1]
        )))
        .collect();
    println!("  boards per turn: {}", per_turn.join(", "));
    if !by_name.is_empty() {
        by_name.sort();
        by_name.dedup();
        println!("  unmatched minions: {}", by_name.join(", "));
    }
    Ok(Arc::new(boards))
}

fn random_board(data: &Data, rng: &mut Rng) -> Vec<Minion> {
    let n = 1 + rng.below(BOARD as u32) as usize;
    (0..n)
        .map(|_| {
            let c = rng.below(data.cards.pool_cards as u32) as CardId;
            let mut m = data.cards.make(c, rng.below(8) == 0);
            let bonus = rng.below(4) as i32;
            m.attack += bonus;
            m.health += bonus;
            m
        })
        .collect()
}

fn bench_fights(n: usize) {
    let data = Data::bundled();
    let mut rng = Rng::new(1);
    let boards: Vec<(Vec<Minion>, Vec<Minion>)> =
        (0..1000).map(|_| (random_board(&data, &mut rng), random_board(&data, &mut rng))).collect();
    let mut wins = [0u64; 3];
    let mut attacks = 0u64;
    let t = Instant::now();
    for i in 0..n {
        let (a, b) = &boards[i % boards.len()];
        let o = fight(&data.cards, a, b, data.rules.max_attacks, &mut rng);
        wins[o.winner.map_or(2, |w| w)] += 1;
        attacks += o.attacks as u64;
    }
    let el = t.elapsed();
    println!(
        "{n} fights in {:.2?}: {:.0} ns per fight, {:.1} attacks per fight",
        el,
        el.as_nanos() as f64 / n as f64,
        attacks as f64 / n as f64
    );
    println!(
        "first board won {}, second {}, ties {} (a check that the boards are mixed)",
        wins[0], wins[1], wins[2]
    );
}

/// Plays one game with random actions. Returns (steps, turns). With `real`,
/// that many seats are real-board seats instead (players 0.., or 1.. when a
/// planner seat is present). With `planner`, player 0 plays the comp planner.
fn play(
    data: &Arc<Data>,
    seed: u64,
    check: bool,
    real: Real,
    planner: Option<Arc<Comps>>,
) -> Result<(u64, u32), String> {
    let mut g = match &real {
        Some((boards, n)) => {
            // The planner always takes seat 0; real seats start at 1.
            let first = if planner.is_some() { 1 } else { 0 };
            assert!(*n + first <= bgcore::PLAYERS, "a planner seat leaves at most 7 real seats");
            let seats: Vec<usize> = (first..first + *n).collect();
            Game::new_random_heroes_real(data.clone(), seed, &seats, boards.clone())
        }
        None => Game::new_random_heroes(data.clone(), seed),
    };
    let mut rng = Rng::new(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15));
    let mut bot = planner.map(|c| Planner::new((*c).clone(), seed ^ 0x51ed_2a17, data));
    let mut steps = 0;
    if let Some(bot) = bot.as_mut() {
        while !g.over {
            let a = if g.current_player() == Some(0) {
                bot.action(&g, 0)
            } else {
                random_action(&g, &mut rng)
            };
            g.step(a).map_err(|_| format!("seed {seed}: picked illegal {a:?}"))?;
            steps += 1;
            for (p, pl) in g.players.iter().enumerate() {
                if pl.pending_cycle.is_some() && !g.choice_waiting(p) {
                    return Err(format!(
                        "seed {seed}, step {steps}, turn {}: player {p} has a paused cycle with no choice open: {:?}, gold {}, board {:?}, hand {:?}",
                        g.turn,
                        pl.pending_cycle,
                        pl.gold,
                        pl.board.iter().map(|m| g.data.cards.get(m.card).name.clone()).collect::<Vec<_>>(),
                        pl.hand.iter().map(|m| g.data.cards.get(m.card).name.clone()).collect::<Vec<_>>(),
                    ));
                }
            }
            if check {
                g.check_invariants()
                    .map_err(|e| format!("seed {seed}, step {steps}, turn {}: {e}", g.turn))?;
            }
        }
    } else {
        // The default path is the original loop, inlined here: `games` and
        // `fuzz` keep the speed they had before the planner seat existed.
        while !g.over {
            let a = random_action(&g, &mut rng);
            g.step(a).map_err(|_| format!("seed {seed}: picked illegal {a:?}"))?;
            steps += 1;
            for (p, pl) in g.players.iter().enumerate() {
                if pl.pending_cycle.is_some() && !g.choice_waiting(p) {
                    return Err(format!(
                        "seed {seed}, step {steps}, turn {}: player {p} has a paused cycle with no choice open: {:?}, gold {}, board {:?}, hand {:?}",
                        g.turn,
                        pl.pending_cycle,
                        pl.gold,
                        pl.board.iter().map(|m| g.data.cards.get(m.card).name.clone()).collect::<Vec<_>>(),
                        pl.hand.iter().map(|m| g.data.cards.get(m.card).name.clone()).collect::<Vec<_>>(),
                    ));
                }
            }
            if check {
                g.check_invariants()
                    .map_err(|e| format!("seed {seed}, step {steps}, turn {}: {e}", g.turn))?;
            }
        }
    }
    Ok((steps, g.turn))
}

fn bench_games(secs: u64, real: Real, planner: Option<Arc<Comps>>) {
    let data = Arc::new(Data::bundled());
    // one core first, for the per-game time
    let t = Instant::now();
    let mut steps = 0;
    let mut turns = 0;
    let n = 2000;
    for seed in 0..n {
        let (s, tu) = play(&data, seed, false, real.clone(), planner.clone()).unwrap();
        steps += s;
        turns += tu as u64;
    }
    let el = t.elapsed();
    println!(
        "one core: {:.0} µs per game, {:.0} actions and {:.1} turns per game, {:.0} ns per action",
        el.as_micros() as f64 / n as f64,
        steps as f64 / n as f64,
        turns as f64 / n as f64,
        el.as_nanos() as f64 / steps as f64,
    );

    let threads = rayon::current_num_threads();
    let games = AtomicU64::new(0);
    let stop = AtomicBool::new(false);
    let t = Instant::now();
    let deadline = Duration::from_secs(secs);
    (0..threads as u64).into_par_iter().for_each(|th| {
        let mut seed = th << 40;
        while !stop.load(Ordering::Relaxed) {
            play(&data, seed, false, real.clone(), planner.clone()).unwrap();
            seed += 1;
            games.fetch_add(1, Ordering::Relaxed);
            if t.elapsed() >= deadline {
                stop.store(true, Ordering::Relaxed);
            }
        }
    });
    let el = t.elapsed().as_secs_f64();
    let g = games.load(Ordering::Relaxed);
    println!("{threads} threads: {g} games in {el:.1} s, {:.0} games per second", g as f64 / el);
}

fn fuzz(n: u64, real: Real, planner: Option<Arc<Comps>>) {
    let data = Arc::new(Data::bundled());
    let t = Instant::now();
    let results: Vec<Result<(u64, u32), String>> =
        (0..n).into_par_iter().map(|seed| play(&data, seed, true, real.clone(), planner.clone())).collect();
    let steps: u64 = results.iter().filter_map(|r| r.as_ref().ok()).map(|r| r.0).sum();
    let errors: Vec<&String> = results.iter().filter_map(|r| r.as_ref().err()).collect();
    println!("{n} games, {steps} actions, invariants checked after each, in {:.1?}", t.elapsed());
    if errors.is_empty() {
        println!("no failures");
    } else {
        println!("{} games failed, first ones:", errors.len());
        for e in errors.iter().take(5) {
            println!("  {e}");
        }
        std::process::exit(1);
    }
}

/// Seats `rule` curve bots and fills the rest with random bots, in random
/// seats. Prints the average placement of each kind.
fn bot_match(n: u64) {
    let data = Arc::new(Data::bundled());
    for (label, bot_fn) in [
        ("standard", standard_action as fn(&Game) -> bgcore::Action),
        ("rush", rush_action as fn(&Game) -> bgcore::Action),
    ] {
        for rule in [1usize, 4] {
            let sums: Vec<[f64; 2]> = (0..n)
                .into_par_iter()
                .map(|seed| {
                    let mut g = Game::new_random_heroes(data.clone(), seed);
                    let mut rng = Rng::new(seed ^ 0x5eed);
                    let mut seats: Vec<bool> = (0..bgcore::PLAYERS).map(|i| i < rule).collect();
                    rng.shuffle(&mut seats);
                    while let Some(p) = g.current_player() {
                        let a = if seats[p] { bot_fn(&g) } else { random_action(&g, &mut rng) };
                        g.step(a).expect("bot picked an illegal action");
                    }
                    let mut s = [0.0; 2];
                    for (p, &place) in g.placements().iter().enumerate() {
                        s[seats[p] as usize] += place as f64;
                    }
                    s
                })
                .collect();
            let (rand_sum, rule_sum) =
                sums.iter().fold((0.0, 0.0), |acc, s| (acc.0 + s[0], acc.1 + s[1]));
            let k = n as f64;
            println!(
                "{rule} {label} bot(s) vs {} random, {n} games: {label} average place {:.2}, random {:.2}",
                bgcore::PLAYERS - rule,
                rule_sum / (k * rule as f64),
                rand_sum / (k * (bgcore::PLAYERS - rule) as f64),
            );
        }
    }
}

/// Which player the `planner` command puts in seat 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Seat {
    Planner,
    Standard,
    Rush,
}

impl Seat {
    fn name(self) -> &'static str {
        match self {
            Seat::Planner => "planner",
            Seat::Standard => "standard",
            Seat::Rush => "rush",
        }
    }
}

/// One game of the `planner` command: the seat 0 result and, for the planner,
/// what it committed to and did.
struct GameRec {
    seed: u64,
    place: u8,
    tier: u8,
    health: i32,
    comp: Option<String>,
    commit_turn: Option<u32>,
    switched_turn: Option<u32>,
    core_share: f32,
    core_cards: usize,
    shell_chances: usize,
    shell_cycles: usize,
    level_turns: Vec<(u32, u8)>,
    tier_turns: [u8; 6],
    board: Vec<String>,
    hand: Vec<String>,
    decisions: u64,
    nanos: u128,
}

fn minion_text(game: &Game, m: &Minion) -> String {
    let c = game.data.cards.get(m.card);
    let mut s = format!("{} {}/{}", c.name, m.attack, m.health);
    if m.golden {
        s.push_str(" golden");
    }
    let kw = bgcore::search::keyword_names(m);
    if !kw.is_empty() {
        s.push_str(&format!(" [{}]", kw.join(",")));
    }
    if m.improvements != 0 || m.magnetizations != 0 {
        s.push_str(&format!(" (improvements {}, magnetizations {})", m.improvements, m.magnetizations));
    }
    s
}

fn load_comps(path: &str, data: &Data) -> Result<Arc<Comps>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    Ok(Arc::new(Comps::from_toml(&text, data)?))
}

/// `--planner PATH` on `fuzz` and `games`: the comp library for the seat 0
/// planner, None when the run is all random play.
fn load_planner_arg(args: &[String]) -> Option<Arc<Comps>> {
    let k = args.iter().position(|a| a == "--planner")?;
    let Some(path) = args.get(k + 1) else {
        eprintln!("--planner needs a comp library path");
        std::process::exit(2);
    };
    let data = Data::bundled();
    Some(load_comps(path, &data).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(2);
    }))
}

fn play_one(
    seed: u64,
    data: Arc<Data>,
    boards: Arc<RealBoards>,
    comps: Option<Arc<Comps>>,
    seat: Seat,
) -> GameRec {
    // Seat 0 is the bot; seats 1..7 play real boards.
    let real: Vec<usize> = (1..PLAYERS).collect();
    let mut g = Game::new_random_heroes_real(data.clone(), seed, &real, boards);
    let mut planner = comps.as_ref().map(|c| Planner::new((**c).clone(), seed ^ 0x51ed_2a17, &data));
    let mut nanos = 0u128;
    let mut decisions = 0u64;
    let mut last_board: Vec<Minion> = Vec::new();
    let mut last_hand: Vec<Minion> = Vec::new();
    while let Some(p) = g.current_player() {
        assert_eq!(p, 0, "seed {seed}: a real-board seat is the current player");
        if !g.players[0].board.is_empty() {
            last_board = g.players[0].board.to_vec();
            last_hand = g.players[0].hand.to_vec();
        }
        let a = match seat {
            Seat::Planner => {
                let t = Instant::now();
                let a = planner.as_mut().expect("the planner seat").action(&g, 0);
                nanos += t.elapsed().as_nanos();
                decisions += 1;
                a
            }
            Seat::Standard => standard_action(&g),
            Seat::Rush => rush_action(&g),
        };
        g.step(a).unwrap_or_else(|_| panic!("seed {seed}: {} picked illegal {a:?}", seat.name()));
    }
    let pl = &g.players[0];
    if last_board.is_empty() {
        last_board = pl.board.to_vec();
        last_hand = pl.hand.to_vec();
    }
    let comp_index = planner.as_ref().and_then(|p| p.state.comp);
    let (core_share, core_cards) = match (comps.as_ref(), comp_index) {
        (Some(c), Some(ci)) => {
            let comp = &c.comps[ci];
            let total: f32 = comp.core.iter().map(|c| c.weight).sum();
            let mut on = 0.0;
            let mut n = 0;
            for core in &comp.core {
                if last_board.iter().any(|m| m.card == core.id) {
                    on += core.weight;
                    n += 1;
                }
            }
            (if total > 0.0 { on / total } else { 0.0 }, n)
        }
        _ => (0.0, 0),
    };
    GameRec {
        seed,
        place: pl.placement,
        tier: pl.tier,
        health: pl.health,
        comp: planner.as_ref().and_then(|p| p.comp_name().map(str::to_string)),
        commit_turn: planner.as_ref().and_then(|p| p.state.committed_turn),
        switched_turn: planner.as_ref().and_then(|p| p.state.switched_turn),
        core_share,
        core_cards,
        shell_chances: planner.as_ref().map_or(0, |p| p.state.shell_chance_turns.len()),
        shell_cycles: planner.as_ref().map_or(0, |p| p.state.shell_cycle_turns.len()),
        level_turns: planner.as_ref().map_or_else(Vec::new, |p| p.state.level_turns.clone()),
        tier_turns: pl.tier_turns,
        board: last_board.iter().map(|m| minion_text(&g, m)).collect(),
        hand: last_hand.iter().map(|m| minion_text(&g, m)).collect(),
        decisions,
        nanos,
    }
}

fn bench_planner(comps_path: &str, games: usize, seat: Seat, boards_dir: &str) {
    let games = games.max(1);
    let data = Arc::new(Data::bundled());
    let boards = load_real_boards(boards_dir).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(2);
    });
    let comps = if seat == Seat::Planner {
        Some(load_comps(comps_path, &data).unwrap_or_else(|e| {
            eprintln!("{e}");
            std::process::exit(2);
        }))
    } else {
        None
    };
    // The candidate copies and the fight sim are large; this command's own
    // pool gets a bigger stack so a 512-game run does not overflow. The other
    // commands keep rayon's default stack, and their speed is unchanged.
    let pool = rayon::ThreadPoolBuilder::new()
        .stack_size(8 * 1024 * 1024)
        .build()
        .expect("the planner's thread pool");
    let t = Instant::now();
    let recs: Vec<GameRec> = pool.install(|| {
        (0..games as u64)
            .into_par_iter()
            .map(|seed| play_one(seed, data.clone(), boards.clone(), comps.clone(), seat))
            .collect()
    });
    let secs = t.elapsed().as_secs_f64();
    let n = recs.len() as f64;
    let avg = recs.iter().map(|r| r.place as f64).sum::<f64>() / n;
    let top4 = recs.iter().filter(|r| r.place <= 4).count() as f64 / n;
    let first = recs.iter().filter(|r| r.place == 1).count() as f64 / n;
    println!("{} games, one {} seat against 7 real boards of {boards_dir}", games, seat.name());
    println!("average place {avg:.2}, top-4 {:.1}%, first {:.1}%", top4 * 100.0, first * 100.0);

    if seat == Seat::Planner {
        let mut commits: BTreeMap<String, (usize, u64)> = BTreeMap::new();
        for r in &recs {
            if let (Some(c), Some(t)) = (&r.comp, r.commit_turn) {
                let e = commits.entry(c.clone()).or_insert((0, 0));
                e.0 += 1;
                e.1 += t as u64;
            }
        }
        let never = recs.iter().filter(|r| r.comp.is_none()).count();
        println!("comp committed to (games, average commit turn):");
        for (c, (k, sum)) in &commits {
            println!("  {c}: {k} games, average turn {:.2}", *sum as f64 / *k as f64);
        }
        if never > 0 {
            println!("  none: {never} games");
        }
        let switched = recs.iter().filter(|r| r.switched_turn.is_some()).count();
        println!("give-up switches: {switched} games");
        let comp_boards = recs
            .iter()
            .filter(|r| r.core_share >= 0.5 && r.core_cards >= 2)
            .count();
        println!(
            "final boards that are the committed comp (core share >= 0.5 and at least 2 core cards): {} of {} ({:.1}%)",
            comp_boards,
            recs.len(),
            comp_boards as f64 / n * 100.0
        );
        let chances: usize = recs.iter().map(|r| r.shell_chances).sum();
        let cycles: usize = recs.iter().map(|r| r.shell_cycles).sum();
        println!(
            "Shell cycles: {cycles} on {chances} turns where the rule applied ({:.1}%)",
            if chances == 0 { 0.0 } else { cycles as f64 / chances as f64 * 100.0 }
        );
        for tier in 3..=6 {
            let mut turns: Vec<u32> = recs
                .iter()
                .map(|r| r.tier_turns[tier - 1] as u32)
                .filter(|&t| t > 0)
                .collect();
            turns.sort_unstable();
            if !turns.is_empty() {
                println!(
                    "tier {tier}: median turn {} ({} games reached it)",
                    turns[turns.len() / 2],
                    turns.len()
                );
            }
        }
        let decisions: u64 = recs.iter().map(|r| r.decisions).sum();
        let nanos: u128 = recs.iter().map(|r| r.nanos).sum();
        if decisions > 0 {
            println!(
                "decision time: {:.3} ms average over {decisions} decisions",
                nanos as f64 / decisions as f64 / 1e6
            );
        }
    }
    println!("{games} games in {secs:.1} s");
    println!("\n10 final boards:");
    for r in recs.iter().take(10) {
        println!(
            "\nseed {}: place {}, tier {}, health {}, comp {:?} (commit {:?}, switch {:?}), core share {:.2} ({} cards)",
            r.seed, r.place, r.tier, r.health, r.comp, r.commit_turn, r.switched_turn, r.core_share, r.core_cards
        );
        for (i, line) in r.board.iter().enumerate() {
            println!("  {}. {line}", i + 1);
        }
        if !r.hand.is_empty() {
            println!("  hand: {}", r.hand.join(", "));
        }
        if !r.level_turns.is_empty() {
            println!("  level presses: {:?}", r.level_turns);
        }
    }
}

#[derive(Deserialize)]
struct RawPeakFile {
    board: Vec<RawPeakBoard>,
}

#[derive(Deserialize)]
struct RawPeakBoard {
    name: String,
    justification: String,
    #[serde(default)]
    beetle_bonus: [i32; 2],
    #[serde(default)]
    volumizer_bonus: [i32; 2],
    #[serde(default)]
    boon_of_beetles: u8,
    #[serde(default)]
    brood_of_nozdormu: u8,
    #[serde(default)]
    upper_hand: u8,
    minions: Vec<RawPeakMinion>,
}

#[derive(Deserialize)]
struct RawAttached {
    name: String,
    #[serde(default)]
    golden: bool,
}

#[derive(Deserialize)]
struct RawPeakMinion {
    name: String,
    #[serde(default)]
    golden: bool,
    #[serde(default)]
    buff: [i32; 2],
    #[serde(default)]
    keywords: Vec<String>,
    #[serde(default)]
    improvements: u16,
    #[serde(default)]
    magnetizations: u16,
    #[serde(default)]
    volumizers: u16,
    #[serde(default)]
    attached: Vec<RawAttached>,
}

struct PeakBoard {
    name: String,
    justification: String,
    counters: Counters,
    minions: Vec<Minion>,
}

fn load_peak_boards(data: &Data) -> Vec<PeakBoard> {
    let text = std::fs::read_to_string("peak_boards.toml")
        .unwrap_or_else(|_| include_str!("../../../peak_boards.toml").to_string());
    let raw: RawPeakFile = toml::from_str(&text).expect("invalid peak_boards.toml");
    raw.board
        .into_iter()
        .map(|b| {
            let counters = Counters {
                beetle_attack: b.beetle_bonus[0],
                beetle_health: b.beetle_bonus[1],
                boon_of_beetles: b.boon_of_beetles,
                brood_of_nozdormu: b.brood_of_nozdormu,
                upper_hand: b.upper_hand,
                ..Counters::default()
            };
            let minions = b
                .minions
                .into_iter()
                .map(|rm| {
                    let id = data
                        .cards
                        .by_name(&rm.name)
                        .unwrap_or_else(|| panic!("unknown card '{}' in peak_boards.toml", rm.name));
                    let mut m = data.cards.make(id, rm.golden);
                    m.attack += rm.buff[0];
                    m.health += rm.buff[1];
                    m.volumizers = m.volumizers.max(rm.volumizers);
                    if m.volumizers > 0 {
                        m.attack += (m.volumizers as i32) * b.volumizer_bonus[0];
                        m.health += (m.volumizers as i32) * b.volumizer_bonus[1];
                    }
                    if data.cards.get(id).is_beetle {
                        m.attack += b.beetle_bonus[0];
                        m.health += b.beetle_bonus[1];
                    }
                    m.magnetizations = m.magnetizations.max(rm.magnetizations);
                    m.improvements = rm.improvements;
                    for kw in &rm.keywords {
                        let &(bit, _) = Keywords::ALL
                            .iter()
                            .find(|(_, name)| *name == kw)
                            .unwrap_or_else(|| panic!("unknown keyword '{kw}' in peak_boards.toml"));
                        m.keywords.set(bit);
                    }
                    for att in &rm.attached {
                        let aid = data
                            .cards
                            .by_name(&att.name)
                            .unwrap_or_else(|| panic!("unknown attached card '{}'", att.name));
                        m.push_attached(Attached { card: aid, golden: att.golden });
                    }
                    m
                })
                .collect();
            PeakBoard { name: b.name, justification: b.justification, counters, minions }
        })
        .collect()
}

fn bench_peak(n: usize) {
    let data = Data::bundled();
    let boards = load_peak_boards(&data);
    let k = boards.len();

    println!("Peak boards ({n} fights each way per pair, {} total per pair):", 2 * n);
    for b in &boards {
        let summary: Vec<String> = b
            .minions
            .iter()
            .map(|m| {
                let g = if m.golden { "G " } else { "" };
                format!("{g}{} {}/{}", data.cards.get(m.card).name, m.attack, m.health)
            })
            .collect();
        println!("  {:<16} {}", b.name, b.justification);
        println!("  {:<16} [{}]", "", summary.join(", "));
    }
    println!();

    // dir_wtl[i][j] = [wins_i, ties, losses_i] when i is side 0 and j is side 1 over `n` fights
    let mut dir_wtl = vec![vec![[0u64; 3]; k]; k];
    for i in 0..k {
        for j in 0..k {
            if i == j {
                continue;
            }
            let mut rng = Rng::new(((i as u64) << 16) ^ (j as u64) ^ 0x9e37_79b9);
            let mut wtl = [0u64; 3];
            for _ in 0..n {
                let out = fight_with(
                    &data.cards,
                    &boards[i].minions,
                    &boards[j].minions,
                    [boards[i].counters, boards[j].counters],
                    data.rules.max_attacks,
                    &mut rng,
                    &mut (),
                );
                match out.winner {
                    Some(0) => wtl[0] += 1,
                    None => wtl[1] += 1,
                    Some(_) => wtl[2] += 1,
                }
            }
            dir_wtl[i][j] = wtl;
        }
    }

    println!("Combined win/tie/loss matrix (row vs column, {} fights = {n} each way):", 2 * n);
    print!("{:<16}", "board");
    for b in &boards {
        print!(" {:>18}", b.name);
    }
    println!(" {:>10}", "overall_w");
    #[allow(clippy::needless_range_loop)]
    for i in 0..k {
        print!("{:<16}", boards[i].name);
        let mut tot_w = 0u64;
        let mut tot_t = 0u64;
        let mut tot_g = 0u64;
        for j in 0..k {
            if i == j {
                print!(" {:>18}", "same");
            } else {
                let w = dir_wtl[i][j][0] + dir_wtl[j][i][2];
                let t = dir_wtl[i][j][1] + dir_wtl[j][i][1];
                let l = dir_wtl[i][j][2] + dir_wtl[j][i][0];
                let total = (w + t + l) as f64;
                tot_w += w;
                tot_t += t;
                tot_g += w + t + l;
                let cell = format!(
                    "{:4.1}/{:4.1}/{:4.1}%",
                    100.0 * w as f64 / total,
                    100.0 * t as f64 / total,
                    100.0 * l as f64 / total,
                );
                print!(" {:>18}", cell);
            }
        }
        let ov = 100.0 * (tot_w as f64 + 0.5 * tot_t as f64) / tot_g as f64;
        println!(" {:>9.1}%", ov);
    }
}

/// `bgbench fightcheck`: play every paired fight of Era's real games `runs`
/// times, on every core, and write `fights.tsv`, `cards.tsv` and `report.md`
/// to the output folder (docs/real-boards.md "The fight check"). Nothing is
/// fixed here: the output says which real results the engine finds unlikely.
fn fight_check(boards: &str, runs: u32, out: &str, flag_below: f64) {
    let t = Instant::now();
    let files = read_board_files(boards).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(2);
    });
    let ids = CardIdMap::parse(
        &std::fs::read_to_string("ref/bg-cards.json")
            .unwrap_or_else(|e| panic!("ref/bg-cards.json (the card id mapping): {e}")),
    )
    .unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(2);
    });
    let (specs, unknown) = parse_board_files(&files, &ids).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(2);
    });
    let boards_count = specs.len();
    let (fights, unpaired) = pair_fights(specs);
    println!(
        "{} boards from {} games in {boards}, {} paired fights, {} boards with no opponent",
        boards_count,
        files.len(),
        fights.len(),
        unpaired.len()
    );
    let data = Data::bundled();
    let opts = CheckOptions { runs, flag_below, ..CheckOptions::default() };
    let entries: Vec<_> = fights
        .par_iter()
        .enumerate()
        .map(|(i, f)| check_fight(&data, f, opts.runs, opts.seed.wrapping_add(i as u64)))
        .collect();
    let mut report = CheckReport::new(&data, entries, opts);
    report.unpaired = unpaired;
    report.unknown_minions = unknown;
    let el = t.elapsed();

    std::fs::create_dir_all(out).unwrap_or_else(|e| panic!("{out}: {e}"));
    for (name, text) in [
        ("fights.tsv", report.fights_tsv()),
        ("cards.tsv", report.cards_tsv()),
        ("report.md", report.report_md(&data)),
    ] {
        let path = format!("{out}/{name}");
        std::fs::write(&path, text).unwrap_or_else(|e| panic!("{path}: {e}"));
    }

    let total = report.entries.len();
    let complete = report.complete();
    println!(
        "{total} fights, {complete} complete, {} flagged (complete, real result under {flag_below:.1}%)",
        report.flagged()
    );
    let mut reasons: Vec<(String, u32)> = Vec::new();
    for e in &report.entries {
        for line in &e.check.missing {
            let head = bgcore::fight_check::missing_kind(line);
            match reasons.iter_mut().find(|(r, _)| r == head) {
                Some((_, c)) => *c += 1,
                None => reasons.push((head.to_string(), 1)),
            }
        }
    }
    reasons.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    println!("why incomplete, most common first:");
    for (name, count) in reasons.iter().take(8) {
        println!("  {count:5}  {name}");
    }
    if !report.unknown_minions.is_empty() {
        let mut names = report.unknown_minions.clone();
        names.sort();
        names.dedup();
        println!("card ids no source knows, {} distinct: {}", names.len(), names.join(", "));
    }
    println!("the 15 most suspicious cards (flagged / complete fights):");
    for t in report.pieces.iter().filter(|t| t.flagged > 0).take(15) {
        println!(
            "  {:>4}/{:<4} {:>5.1}%  {} ({})",
            t.flagged,
            t.complete,
            t.ratio(),
            t.name,
            t.kind.map_or("?", bgcore::fight_check::PieceKind::name)
        );
    }
    println!("{el:.1?} for {total} fights x {runs} runs on {} cores", rayon::current_num_threads());
    println!("wrote {out}/fights.tsv, {out}/cards.tsv, {out}/report.md");
}

/// `bgbench shopreplay`: replay every step of Era's real shop phases from the
/// logged state before it, on every core, and write `steps.tsv`,
/// `shop-cards.tsv` and `shop-report.md` (docs/real-boards.md "The shop
/// replay"). Nothing is fixed here: the output says which steps the engine
/// does not reproduce.
fn shop_replay(dir: &str, seeds: u32, out: &str, card: Option<&str>) {
    let t = Instant::now();
    let files = read_board_files(dir).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(2);
    });
    let ids = CardIdMap::parse(
        &std::fs::read_to_string("ref/bg-cards.json")
            .unwrap_or_else(|e| panic!("ref/bg-cards.json (the card id mapping): {e}")),
    )
    .unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(2);
    });
    let mut specs = bgcore::shop_replay::parse_action_files(&files).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(2);
    });
    // A Magicfin token's taught spell is a dbf id in the log; the ref card map
    // is the only thing that can say which card that is.
    bgcore::shop_replay::seat_taught_spells(&mut specs, &ids);
    let data = Arc::new(Data::bundled());
    let opts = bgcore::shop_replay::ReplayOptions { seeds: seeds.max(1), seed: 1 };
    let turns: Vec<(&bgcore::shop_replay::ShopFileSpec, &bgcore::shop_replay::ShopTurnSpec)> =
        specs.iter().flat_map(|f| f.turns.iter().map(move |t| (f, t))).collect();
    let mut results: Vec<Vec<bgcore::shop_replay::StepResult>> = turns
        .par_iter()
        .map(|(f, turn)| bgcore::shop_replay::replay_turn(&data, &ids, f, turn, &opts, card))
        .collect();
    let mut report = bgcore::shop_replay::ShopReplayReport {
        files: specs.len(),
        turns: turns.len(),
        ..Default::default()
    };
    for mut steps in results.drain(..) {
        report.entries.append(&mut steps);
    }
    for f in &specs {
        for u in bgcore::shop_replay::unknown_minions(&data, &f.turns.first().map(|t| t.start.clone()).unwrap_or_default()) {
            if !report.unknown.contains(&u) {
                report.unknown.push(u);
            }
        }
    }
    let el = t.elapsed();
    std::fs::create_dir_all(out).unwrap_or_else(|e| panic!("{out}: {e}"));
    for (name, text) in [
        ("steps.tsv", report.steps_tsv()),
        ("shop-cards.tsv", report.cards_tsv(&data)),
        ("shop-report.md", report.report_md(&data)),
    ] {
        let path = format!("{out}/{name}");
        std::fs::write(&path, text).unwrap_or_else(|e| panic!("{path}: {e}"));
    }
    let steps = report.steps();
    let deterministic = report.deterministic();
    println!(
        "{} games, {} turns, {steps} steps replayed ({} deterministic, {} random)",
        report.files,
        report.turns,
        deterministic,
        steps - deterministic
    );
    let counts: Vec<String> = report
        .verdict_counts()
        .into_iter()
        .filter(|(_, n)| *n > 0)
        .map(|(name, n)| format!("{name} {n}"))
        .collect();
    println!("{}", counts.join(", "));
    if let Some(card) = card {
        println!("limited to steps involving {card}");
    }
    println!("the 20 most suspicious cards (flagged / steps):");
    for t in report.pieces(&data).iter().take(20) {
        println!(
            "  {:>4}/{:<4} {:>5.1}%  {} ({})",
            t.flagged,
            t.steps,
            t.ratio() * 100.0,
            t.name,
            t.kind
        );
    }
    println!("{el:.1?} on {} cores", rayon::current_num_threads());
    println!("wrote {out}/steps.tsv, {out}/shop-cards.tsv, {out}/shop-report.md");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let num = |default: u64| args.get(2).and_then(|s| s.parse().ok()).unwrap_or(default);
    // --real-boards DIR [--real-seats N] on fuzz and games: seats that play
    // boards from DIR instead of shopping.
    let real: Real = if let Some(k) = args.iter().position(|a| a == "--real-boards") {
        let Some(dir) = args.get(k + 1) else {
            eprintln!("--real-boards needs a directory");
            std::process::exit(2);
        };
        let seats = args
            .iter()
            .position(|a| a == "--real-seats")
            .and_then(|k| args.get(k + 1))
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(2);
        assert!((1..=bgcore::PLAYERS).contains(&seats), "real seats must be 1 to 8");
        let boards = load_real_boards(dir).unwrap_or_else(|e| {
            eprintln!("{e}");
            std::process::exit(2);
        });
        Some((boards, seats))
    } else {
        if args.iter().any(|a| a == "--real-seats") {
            eprintln!("--real-seats needs --real-boards DIR");
            std::process::exit(2);
        }
        None
    };
    match args.get(1).map(String::as_str) {
        Some("fight") => bench_fights(num(1_000_000) as usize),
        Some("fightcheck") => {
            let opt = |name: &str, default: &str| {
                args.iter()
                    .position(|a| a == name)
                    .and_then(|k| args.get(k + 1))
                    .cloned()
                    .unwrap_or_else(|| default.to_string())
            };
            let boards = opt("--boards", "boards/real");
            let out = opt("--out", "boards/checks");
            let runs: u32 = opt("--runs", "1000").parse().unwrap_or_else(|_| {
                eprintln!("--runs needs a number");
                std::process::exit(2);
            });
            let flag_below: f64 = opt("--flag", "2.0").parse().unwrap_or_else(|_| {
                eprintln!("--flag needs a percent, like 2 or 0.5");
                std::process::exit(2);
            });
            fight_check(&boards, runs.max(1), &out, flag_below);
        }
        Some("shopreplay") => {
            let opt = |name: &str, default: &str| {
                args.iter()
                    .position(|a| a == name)
                    .and_then(|k| args.get(k + 1))
                    .cloned()
                    .unwrap_or_else(|| default.to_string())
            };
            let actions = opt("--actions", "boards/actions");
            let out = opt("--out", "boards/checks");
            let seeds: u32 = opt("--seeds", "64").parse().unwrap_or_else(|_| {
                eprintln!("--seeds needs a number");
                std::process::exit(2);
            });
            let card = args
                .iter()
                .position(|a| a == "--card")
                .and_then(|k| args.get(k + 1))
                .cloned();
            shop_replay(&actions, seeds, &out, card.as_deref());
        }
        Some("record") => record_examples::run(&args),
        Some("powerlog") => powerlog::run(&args),
        Some("tracker") => tracker::run(&args),
        Some("check-actions") => check_actions::run(&args),
        Some("collect-actions") => collect_actions::run(&args),
        Some("collect-boards") => collect_boards::run(&args),
        Some("games") => bench_games(num(10), real, load_planner_arg(&args)),
        Some("fuzz") => fuzz(num(10_000), real, load_planner_arg(&args)),
        Some("planner") => {
            let opt = |name: &str, default: &str| {
                args.iter()
                    .position(|a| a == name)
                    .and_then(|k| args.get(k + 1))
                    .cloned()
                    .unwrap_or_else(|| default.to_string())
            };
            let comps = opt("--comps", "data/comps/comps.toml");
            let boards = opt("--real-boards", "boards/real");
            let games: usize = opt("--games", "512").parse().unwrap_or_else(|_| {
                eprintln!("--games needs a number");
                std::process::exit(2);
            });
            let vs = opt("--vs", "real");
            if vs != "real" {
                eprintln!("--vs only takes real (7 real-board seats)");
                std::process::exit(2);
            }
            let seat = match opt("--seat", "planner").as_str() {
                "planner" => Seat::Planner,
                "standard" => Seat::Standard,
                "rush" => Seat::Rush,
                other => {
                    eprintln!("no seat {other}: planner, standard or rush");
                    std::process::exit(2);
                }
            };
            bench_planner(&comps, games, seat, &boards);
        }
        Some("match") => bot_match(num(10_000)),
        Some("peak") => bench_peak(num(10_000) as usize),
        _ => {
            eprintln!(
                "usage: bgbench fight [n] | games [secs] | fuzz [games] | match [games] | peak [n]\n\
                 \x20      bgbench fightcheck [--boards DIR] [--runs N] [--out DIR] [--flag PCT]\n\
                 \x20      bgbench shopreplay [--actions DIR] [--seeds N] [--out DIR] [--card NAME]\n\
                 \x20      bgbench powerlog FILE [--out FILE] [--time]\n\
                 \x20      bgbench tracker FILE [--out FILE] [--cards FILE] [--time]\n\
                 \x20      bgbench collect-boards [--out DIR] [--cards FILE] [--no-archive]\n\
                 \x20      bgbench collect-actions [--out DIR] [--cards FILE] [--rewrite]\n\
                 \x20      bgbench check-actions [--verbose] [PATTERN] [--actions DIR]\n\
                 \x20      bgbench record [--actions DIR] [--out DIR] [--seeds N]\n\
                 \x20      bgbench fuzz|games N --real-boards DIR [--real-seats N]"
            );
            std::process::exit(2);
        }
    }
}
