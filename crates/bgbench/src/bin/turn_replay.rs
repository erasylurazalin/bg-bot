//! Whole-turn replay: seat each logged turn once from its own start, play
//! every logged step in order on that one game, and write the engine's end
//! board against the log's. Measurement only; no game rule changes.
//!
//!   bgbench turn_replay [--actions DIR] [--out DIR] [--seeds N]
//!                       [--turns MIN,MAX] [--only SUBSTR]
//!
//! `train/runs/engine-pays/analyze.py` turns `turns.json` into the report.

use std::sync::Arc;

use bgcore::real_boards::CardIdMap;
use bgcore::shop_replay::{self, ME, ShopStateSpec};
use bgcore::{Data, Game};
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
struct CardRow {
    name: String,
    attack: i32,
    health: i32,
    golden: bool,
}

fn engine_board(data: &Data, game: &Game) -> Vec<CardRow> {
    game.players[ME]
        .board
        .iter()
        .map(|m| CardRow {
            name: data.cards.get(m.card).name.clone(),
            attack: m.attack,
            health: m.health,
            golden: m.golden,
        })
        .collect()
}

fn log_board(state: &ShopStateSpec) -> Vec<CardRow> {
    state
        .board
        .iter()
        .map(|m| CardRow {
            name: m
                .name
                .clone()
                .or_else(|| m.card_id.clone())
                .unwrap_or_default(),
            attack: m.attack,
            health: m.health,
            golden: m.golden,
        })
        .collect()
}

fn log_zone(state: &ShopStateSpec, zone: &str) -> Vec<CardRow> {
    let cards = match zone {
        "board" => &state.board,
        "hand" => &state.hand,
        _ => return Vec::new(),
    };
    cards
        .iter()
        .map(|m| CardRow {
            name: m
                .name
                .clone()
                .or_else(|| m.card_id.clone())
                .unwrap_or_default(),
            attack: m.attack,
            health: m.health,
            golden: m.golden,
        })
        .collect()
}

fn total(board: &[CardRow]) -> i32 {
    board.iter().map(|c| c.attack + c.health).sum()
}

#[derive(Clone, Debug, Serialize)]
struct StopRow {
    step: usize,
    step_number: usize,
    kind: String,
    card: Option<String>,
    class: String,
    reason: String,
}

#[derive(Debug, Serialize)]
struct TurnRow {
    file: String,
    source: String,
    game: u32,
    turn: u32,
    seed: u64,
    steps: usize,
    played: usize,
    complete: bool,
    stop: Option<StopRow>,
    missing: Vec<String>,
    engine_board: Vec<CardRow>,
    log_board: Vec<CardRow>,
    start_board: Vec<CardRow>,
    start_hand: Vec<CardRow>,
    engine_total: i32,
    log_total: i32,
}

fn turn_row(
    data: &Arc<Data>,
    ids: &CardIdMap,
    file: &shop_replay::ShopFileSpec,
    turn: &shop_replay::ShopTurnSpec,
    seed: u64,
) -> TurnRow {
    let start_board = log_zone(&turn.start, "board");
    let start_hand = log_zone(&turn.start, "hand");
    match shop_replay::replay_whole_turn(data, ids, file, turn, seed) {
        Ok(run) => {
            let engine_board = engine_board(data, &run.game);
            let log_board = run.log_state(turn).map(log_board).unwrap_or_default();
            let complete = run.complete();
            let stop = run.stop.map(|s| StopRow {
                step: s.step,
                step_number: s.step + 1,
                kind: s.kind,
                card: s.card,
                class: s.class,
                reason: s.reason,
            });
            TurnRow {
                file: file.file(),
                source: file.source.clone(),
                game: file.game,
                turn: turn.turn,
                seed,
                steps: run.steps,
                played: run.played,
                complete,
                stop,
                missing: run.missing,
                engine_total: total(&engine_board),
                log_total: total(&log_board),
                engine_board,
                log_board,
                start_board,
                start_hand,
            }
        }
        Err((why, missing)) => {
            // The turn's own start does not seat: no engine board exists to
            // compare, so the row says why at step 1 of the turn.
            let kind = turn
                .actions
                .first()
                .map(|s| s.step.clone())
                .unwrap_or_default();
            TurnRow {
                file: file.file(),
                source: file.source.clone(),
                game: file.game,
                turn: turn.turn,
                seed,
                steps: turn.actions.len(),
                played: 0,
                complete: false,
                stop: Some(StopRow {
                    step: 0,
                    step_number: 1,
                    kind,
                    card: turn
                        .actions
                        .first()
                        .and_then(|s| s.card.as_ref())
                        .and_then(|c| c.name.clone()),
                    class: "start".to_string(),
                    reason: why,
                }),
                missing,
                engine_board: Vec::new(),
                log_board: start_board.clone(),
                engine_total: 0,
                log_total: total(&start_board),
                start_board,
                start_hand,
            }
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let opt = |name: &str, default: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|k| args.get(k + 1))
            .cloned()
            .unwrap_or_else(|| default.to_string())
    };
    let actions = opt("--actions", "boards/actions");
    let out = opt("--out", "train/runs/engine-pays");
    let seeds: u64 = opt("--seeds", "5").parse().unwrap_or_else(|_| {
        eprintln!("--seeds needs a number");
        std::process::exit(2);
    });
    let (turn_min, turn_max) = opt("--turns", "7,12")
        .split_once(',')
        .and_then(|(a, b)| Some((a.trim().parse::<u32>().ok()?, b.trim().parse::<u32>().ok()?)))
        .unwrap_or_else(|| {
            eprintln!("--turns needs MIN,MAX, like 7,12");
            std::process::exit(2);
        });
    let only = Some(opt("--only", "")).filter(|s| !s.is_empty());
    let base = seeds.max(1);

    let mut files: Vec<(String, String)> = std::fs::read_dir(&actions)
        .unwrap_or_else(|e| panic!("{actions}: {e}"))
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_str().is_some_and(|n| n.ends_with(".json")))
        .map(|e| {
            let p = e.path();
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            let text = std::fs::read_to_string(&p).unwrap_or_else(|err| panic!("{}: {err}", p.display()));
            (name, text)
        })
        .collect();
    files.sort();
    if let Some(only) = &only {
        files.retain(|(name, _)| name.contains(only));
    }
    let ids_text = std::fs::read_to_string("ref/bg-cards.json")
        .unwrap_or_else(|e| panic!("ref/bg-cards.json (the card id mapping): {e}"));
    let ids = CardIdMap::parse(&ids_text).unwrap_or_else(|e| panic!("ref/bg-cards.json: {e}"));
    let mut specs = shop_replay::parse_action_files(&files)
        .unwrap_or_else(|e| panic!("{actions}: {e}"));
    shop_replay::seat_taught_spells(&mut specs, &ids);
    let data = Arc::new(Data::bundled());

    let mut wanted: Vec<(usize, usize)> = Vec::new();
    for (fi, file) in specs.iter().enumerate() {
        for (ti, turn) in file.turns.iter().enumerate() {
            if turn.turn >= turn_min && turn.turn <= turn_max {
                wanted.push((fi, ti));
            }
        }
    }
    let mut rows: Vec<TurnRow> = Vec::new();
    for &(fi, ti) in &wanted {
        let file = &specs[fi];
        let turn = &file.turns[ti];
        for k in 1..=base {
            rows.push(turn_row(&data, &ids, file, turn, k));
        }
    }
    rows.sort_by(|a, b| {
        (a.file.as_str(), a.turn, a.seed).cmp(&(b.file.as_str(), b.turn, b.seed))
    });
    let turns = wanted.len();
    let fully: Vec<&TurnRow> = rows.iter().filter(|r| r.seed == 1 && r.complete).collect();
    let stopped: Vec<&TurnRow> = rows.iter().filter(|r| r.seed == 1 && !r.complete).collect();
    let mut reasons: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for r in &stopped {
        let key = r
            .stop
            .as_ref()
            .map(|s| format!("{} ({})", s.class, s.kind))
            .unwrap_or_else(|| "unknown".to_string());
        *reasons.entry(key).or_default() += 1;
    }
    std::fs::create_dir_all(&out).unwrap_or_else(|e| panic!("{out}: {e}"));
    let path = format!("{out}/turns.json");
    let text = serde_json::to_string(&rows).expect("the rows serialize");
    std::fs::write(&path, text).unwrap_or_else(|e| panic!("{path}: {e}"));
    println!(
        "{} turns 7 to 12, {} runs at {} seeds, {} complete, {} stopped",
        turns,
        rows.len(),
        base,
        fully.len(),
        stopped.len()
    );
    for (reason, n) in reasons {
        println!("  stop {n}: {reason}");
    }
    println!("wrote {path}");
}
