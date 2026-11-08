//! `bgbench eraseat`: one seat plays Era's own boards of a finished game,
//! the other seven play random real boards with that game left out.
//!
//! `--runs N` (the default 32) repeats every finished game; `--games N`
//! instead plays N games, each with a finished game drawn at random. The
//! lobby filter can be switched on for the opponents' pool. Output is one
//! TSV row per game played, plus a summary; `--out FILE` also writes the TSV.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use bgcore::real_boards::{
    CardIdMap, MIN_FILTERED_BOARDS, RealBoardSpec, RealBoards, parse_board_files,
};
use bgcore::{Action, Data, Game, PLAYERS, Rng};

fn opt(args: &[String], name: &str, default: &str) -> String {
    args.iter()
        .position(|a| a == name)
        .and_then(|k| args.get(k + 1))
        .cloned()
        .unwrap_or_else(|| default.to_string())
}

fn load(dir: &str) -> (Arc<Data>, Vec<RealBoardSpec>) {
    let files = match super::read_board_files(dir) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    let ref_json = match std::fs::read_to_string("ref/bg-cards.json") {
        Ok(t) => t,
        Err(e) => {
            eprintln!("ref/bg-cards.json (the card id mapping): {e}");
            std::process::exit(2);
        }
    };
    let ids = match CardIdMap::parse(&ref_json) {
        Ok(ids) => ids,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    let specs = match parse_board_files(&files, &ids) {
        Ok((specs, unknown)) => {
            if !unknown.is_empty() {
                let mut names = unknown;
                names.sort();
                names.dedup();
                eprintln!("  unmatched minions: {}", names.join(", "));
            }
            specs
        }
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    (Arc::new(Data::bundled()), specs)
}

/// The finished games that have Era's own boards, by the order their files
/// first name them.
fn finished_games(specs: &[RealBoardSpec]) -> Vec<(String, u32)> {
    let mut games = BTreeMap::new();
    for s in specs.iter().filter(|s| s.side == bgcore::RealSide::Me && s.place != 0) {
        games.entry((s.source.clone(), s.game)).or_insert(s.place);
    }
    games.into_keys().collect()
}

/// One game with `era_seat` playing Era's boards of `source`/`game`.
fn play(seed: u64, data: &Arc<Data>, boards: &Arc<RealBoards>, source: &str, game: u32) -> Game {
    let real: Vec<usize> = (0..PLAYERS).collect();
    let mut g = Game::new_random_heroes_era(
        data.clone(),
        seed,
        &real,
        boards.clone(),
        0,
        source.to_string(),
        game,
    );
    while !g.over {
        let Some(p) = g.current_player() else { break };
        assert!(g.players[p].real, "an all-real game has no shopper");
        assert!(!g.players[p].done, "a done real seat was asked to act");
        if g.step(Action::EndTurn).is_err() {
            panic!("seed {seed}: a real seat could not end its turn");
        }
    }
    g
}

pub fn run(args: &[String]) {
    let dir = opt(args, "--boards", "boards/real");
    let seed: u64 = opt(args, "--seed", "14606").parse().unwrap_or_else(|_| {
        eprintln!("--seed needs a number");
        std::process::exit(2);
    });
    let runs: usize = opt(args, "--runs", "32").parse().unwrap_or_else(|_| {
        eprintln!("--runs needs a number");
        std::process::exit(2);
    });
    let games: Option<usize> = args
        .iter()
        .position(|a| a == "--games")
        .and_then(|k| args.get(k + 1))
        .map(|s| {
            s.parse().unwrap_or_else(|_| {
                eprintln!("--games needs a number");
                std::process::exit(2);
            })
        });
    let filter = args.iter().any(|a| a == "--real-lobby-filter");
    let out = args
        .iter()
        .position(|a| a == "--out")
        .and_then(|k| args.get(k + 1))
        .cloned();

    let (data, specs) = load(&dir);
    let (mut boards, unmatched) = RealBoards::build(&specs, &data.cards, &data.gifts);
    let games_with_era = finished_games(&specs);
    if games_with_era.is_empty() {
        eprintln!("{dir}: no finished game has Era's own boards");
        std::process::exit(2);
    }
    println!(
        "{} boards, {} finished games with Era's own boards, filter {}",
        boards.len(),
        games_with_era.len(),
        if filter { "on" } else { "off" }
    );
    println!("widening threshold: fewer than {MIN_FILTERED_BOARDS} eligible boards");
    if !unmatched.is_empty() {
        let mut names = unmatched;
        names.sort();
        names.dedup();
        println!("unmatched minions: {}", names.join(", "));
    }
    boards.set_lobby_filter(filter);
    let boards = Arc::new(boards);

    struct Row {
        source: String,
        game: u32,
        run: usize,
        place: u8,
        last_real: u32,
        turns_added: u32,
        outlived: bool,
        final_turn: u32,
    }
    let mut rows = Vec::new();
    let started = Instant::now();
    match games {
        None => {
            for (gi, (source, game)) in games_with_era.iter().enumerate() {
                let last_real = boards.era_last_turn(source, *game).unwrap_or(0);
                for run in 0..runs {
                    let s = seed
                        .wrapping_add((gi as u64).wrapping_mul(1_000_003))
                        .wrapping_add(run as u64);
                    let g = play(s, &data, &boards, source, *game);
                    rows.push(Row {
                        source: source.clone(),
                        game: *game,
                        run,
                        place: g.players[0].placement,
                        last_real,
                        turns_added: g.era_turns_added,
                        outlived: g.era_outlived,
                        final_turn: g.turn,
                    });
                }
                let mine: Vec<&Row> = rows.iter().filter(|r| &r.source == source && r.game == *game).collect();
                let n = mine.len().max(1) as f64;
                let place = mine.iter().map(|r| r.place as f64).sum::<f64>() / n;
                let top4 = mine.iter().filter(|r| r.place <= 4).count() as f64 / n;
                let added: u32 = mine.iter().map(|r| r.turns_added).sum();
                let out = mine.iter().filter(|r| r.outlived).count();
                println!(
                    "{source}-g{game}: place {place:.3}, top4 {:.1}%, added {added} turns, outlived {out}/{n:.0}",
                    top4 * 100.0
                );
            }
        }
        Some(n_games) => {
            let mut pick = Rng::new(seed ^ 0x6572_6173_6561_7400);
            for run in 0..n_games {
                let (source, game) = &games_with_era[pick.below(games_with_era.len() as u32) as usize];
                let last_real = boards.era_last_turn(source, *game).unwrap_or(0);
                let s = seed.wrapping_add(run as u64);
                let g = play(s, &data, &boards, source, *game);
                rows.push(Row {
                    source: source.clone(),
                    game: *game,
                    run,
                    place: g.players[0].placement,
                    last_real,
                    turns_added: g.era_turns_added,
                    outlived: g.era_outlived,
                    final_turn: g.turn,
                });
            }
        }
    }

    let n = rows.len().max(1) as f64;
    let place = rows.iter().map(|r| r.place as f64).sum::<f64>() / n;
    let top4 = rows.iter().filter(|r| r.place <= 4).count() as f64 / n;
    let first = rows.iter().filter(|r| r.place == 1).count() as f64 / n;
    let added: u32 = rows.iter().map(|r| r.turns_added).sum();
    let outlived = rows.iter().filter(|r| r.outlived).count();
    let stats = boards.filter_stats();
    println!(
        "{} games: place {place:.3}, top4 {:.1}%, first {:.1}%, \
         added {added} game-turns, outlived {outlived}/{:.0}, {:.1?}",
        rows.len(),
        top4 * 100.0,
        first * 100.0,
        n,
        started.elapsed()
    );
    println!(
        "filter: {} draws, {} own turn, {} widened, {} unfiltered fallback",
        stats.draws, stats.current, stats.widened, stats.unfiltered
    );

    if let Some(path) = out {
        let mut text = String::from(
            "source\tgame\trun\tplace\tlast_real\tturns_added\toutlived\tfinal_turn\n",
        );
        for r in &rows {
            text.push_str(&format!(
                "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                r.source, r.game, r.run, r.place, r.last_real, r.turns_added, r.outlived, r.final_turn
            ));
        }
        if let Err(e) = std::fs::write(&path, text) {
            eprintln!("{path}: {e}");
            std::process::exit(2);
        }
        println!("wrote {path}");
    }
}
