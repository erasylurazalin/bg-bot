//! Benchmarks and a fuzzer for bgcore, with random legal play.
//!
//!   bgbench fight [n]       time n fights between random boards on one core
//!   bgbench games [secs]    full games on every core, games per second
//!   bgbench fuzz [games]    games on every core, invariants after every step
//!   bgbench match [games]   curve bots against random bots, average placement
//!   bgbench peak [n]        peak board balance matrix (n fights each way per pair)

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use bgcore::bots::{random_action, rush_action, standard_action};
use bgcore::{Attached, BOARD, Counters, Data, Game, Keywords, Minion, Rng, fight, fight_with};
use rayon::prelude::*;
use serde::Deserialize;

fn random_board(data: &Data, rng: &mut Rng) -> Vec<Minion> {
    let n = 1 + rng.below(BOARD as u32) as usize;
    (0..n)
        .map(|_| {
            let c = rng.below(data.cards.pool_cards as u32) as u8;
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

/// Plays one game with random actions. Returns (steps, turns).
fn play(data: &Arc<Data>, seed: u64, check: bool) -> Result<(u64, u32), String> {
    let mut g = Game::new(data.clone(), seed);
    let mut rng = Rng::new(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15));
    let mut steps = 0;
    while !g.over {
        let a = random_action(&g, &mut rng);
        g.step(a).map_err(|_| format!("seed {seed}: random_action picked illegal {a:?}"))?;
        steps += 1;
        if check {
            g.check_invariants()
                .map_err(|e| format!("seed {seed}, step {steps}, turn {}: {e}", g.turn))?;
        }
    }
    Ok((steps, g.turn))
}

fn bench_games(secs: u64) {
    let data = Arc::new(Data::bundled());
    // one core first, for the per-game time
    let t = Instant::now();
    let mut steps = 0;
    let mut turns = 0;
    let n = 2000;
    for seed in 0..n {
        let (s, tu) = play(&data, seed, false).unwrap();
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
            play(&data, seed, false).unwrap();
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

fn fuzz(n: u64) {
    let data = Arc::new(Data::bundled());
    let t = Instant::now();
    let results: Vec<Result<(u64, u32), String>> =
        (0..n).into_par_iter().map(|seed| play(&data, seed, true)).collect();
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
                    let mut g = Game::new(data.clone(), seed);
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

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let num = |default: u64| args.get(2).and_then(|s| s.parse().ok()).unwrap_or(default);
    match args.get(1).map(String::as_str) {
        Some("fight") => bench_fights(num(1_000_000) as usize),
        Some("games") => bench_games(num(10)),
        Some("fuzz") => fuzz(num(10_000)),
        Some("match") => bot_match(num(10_000)),
        Some("peak") => bench_peak(num(10_000) as usize),
        _ => {
            eprintln!("usage: bgbench fight [n] | games [secs] | fuzz [games] | match [games] | peak [n]");
            std::process::exit(2);
        }
    }
}
