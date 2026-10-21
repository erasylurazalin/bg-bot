//! Benchmarks and a fuzzer for bgcore, with random legal play.
//!
//!   bgbench fight [n]       time n fights between random boards on one core
//!   bgbench games [secs]    full games on every core, games per second
//!   bgbench fuzz [games]    games on every core, invariants after every step
//!   bgbench match [games]   rule bots against random bots, average placement

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use bgcore::bots::{random_action, rule_action};
use bgcore::{BOARD, Data, Game, Minion, Rng, fight};
use rayon::prelude::*;

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
    println!("{n} fights in {:.2?}: {:.0} ns per fight, {:.1} attacks per fight", el, el.as_nanos() as f64 / n as f64, attacks as f64 / n as f64);
    println!("first board won {}, second {}, ties {} (a check that the boards are mixed)", wins[0], wins[1], wins[2]);
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
            g.check_invariants().map_err(|e| format!("seed {seed}, step {steps}, turn {}: {e}", g.turn))?;
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
    let results: Vec<Result<(u64, u32), String>> = (0..n).into_par_iter().map(|seed| play(&data, seed, true)).collect();
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

/// Seats `rule` rule bots and fills the rest with random bots, in random
/// seats. Returns the average placement of each kind.
fn bot_match(n: u64) {
    let data = Arc::new(Data::bundled());
    for rule in [1usize, 4] {
        let sums: Vec<[f64; 2]> = (0..n)
            .into_par_iter()
            .map(|seed| {
                let mut g = Game::new(data.clone(), seed);
                let mut rng = Rng::new(seed ^ 0x5eed);
                let mut seats: Vec<bool> = (0..bgcore::PLAYERS).map(|i| i < rule).collect();
                rng.shuffle(&mut seats);
                while let Some(p) = g.current_player() {
                    let a = if seats[p] { rule_action(&g) } else { random_action(&g, &mut rng) };
                    g.step(a).expect("bot picked an illegal action");
                }
                let mut s = [0.0; 2];
                for (p, &place) in g.placements().iter().enumerate() {
                    s[seats[p] as usize] += place as f64;
                }
                s
            })
            .collect();
        let (rand_sum, rule_sum) = sums.iter().fold((0.0, 0.0), |acc, s| (acc.0 + s[0], acc.1 + s[1]));
        let k = n as f64;
        println!(
            "{rule} rule bot(s) vs {} random, {n} games: rule bot average place {:.2}, random {:.2}",
            bgcore::PLAYERS - rule,
            rule_sum / (k * rule as f64),
            rand_sum / (k * (bgcore::PLAYERS - rule) as f64),
        );
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
        _ => {
            eprintln!("usage: bgbench fight [n] | games [secs] | fuzz [games] | match [games]");
            std::process::exit(2);
        }
    }
}
