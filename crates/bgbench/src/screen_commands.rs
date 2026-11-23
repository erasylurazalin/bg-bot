//! `bgbench screenreplay` and `bgbench screen-from-actions`: the screen file
//! replay and the check that writes a screen file from an action file
//! (docs/real-boards.md "The screen file" and "The screen replay").

use std::collections::BTreeMap;
use std::sync::Arc;

use bgcore::real_boards::CardIdMap;
use bgcore::screen_replay::{self, ScreenTurnOutcome, ScreenVerdict};
use bgcore::{Data, shop_replay};

fn opt(args: &[String], name: &str, default: &str) -> String {
    args.iter()
        .position(|a| a == name)
        .and_then(|k| args.get(k + 1))
        .cloned()
        .unwrap_or_else(|| default.to_string())
}

fn json_files(dir: &str) -> Vec<(String, String)> {
    let mut files: Vec<(String, String)> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{dir}: {e}"))
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_str().is_some_and(|n| n.ends_with(".json")))
        .map(|e| {
            let p = e.path();
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            let text =
                std::fs::read_to_string(&p).unwrap_or_else(|err| panic!("{}: {err}", p.display()));
            (name, text)
        })
        .collect();
    files.sort();
    files
}

fn read_ids() -> CardIdMap {
    let text = std::fs::read_to_string("ref/bg-cards.json")
        .unwrap_or_else(|e| panic!("ref/bg-cards.json (the card id mapping): {e}"));
    CardIdMap::parse(&text).unwrap_or_else(|e| panic!("ref/bg-cards.json: {e}"))
}

fn clean(text: &str) -> String {
    text.replace(['\t', '\n', '\r'], " ")
}

pub fn screenreplay(args: &[String]) {
    let in_dir = opt(args, "--in", "boards/screens");
    let out = opt(args, "--out", "boards/checks");
    let seeds: u64 = opt(args, "--seeds", "8").parse().unwrap_or_else(|_| {
        eprintln!("--seeds needs a number");
        std::process::exit(2);
    });
    let chain = args.iter().any(|a| a == "--chain");
    let files = json_files(&in_dir);
    if files.is_empty() {
        eprintln!("{in_dir}: no screen files");
        std::process::exit(2);
    }
    let ids = read_ids();
    let data = Arc::new(Data::bundled());
    let specs =
        screen_replay::parse_screen_files(&files).unwrap_or_else(|e| panic!("{in_dir}: {e}"));
    let mut outcomes: Vec<ScreenTurnOutcome> = Vec::new();
    for file in &specs {
        if chain {
            outcomes.extend(screen_replay::replay_screen_file_chain(
                &data, &ids, file, seeds,
            ));
        } else {
            for turn in &file.turns {
                outcomes.push(screen_replay::replay_screen_turn_seeds(
                    &data, &ids, file, turn, seeds,
                ));
            }
        }
    }
    let mut totals: BTreeMap<&'static str, usize> = BTreeMap::new();
    for o in &outcomes {
        *totals.entry(o.verdict.as_str()).or_default() += 1;
    }
    std::fs::create_dir_all(&out).unwrap_or_else(|e| panic!("{out}: {e}"));
    let mut report = String::new();
    report.push_str("# Screen replay\n\n");
    report.push_str(&format!(
        "{} screen files, {} turns, {} seeds each{}.\n\n",
        specs.len(),
        outcomes.len(),
        seeds.max(1),
        if chain { ", chained" } else { "" }
    ));
    report.push_str("| verdict | turns |\n|---|---:|\n");
    for verdict in [
        ScreenVerdict::Ok,
        ScreenVerdict::OkRandom,
        ScreenVerdict::Flagged,
        ScreenVerdict::Stopped,
    ] {
        report.push_str(&format!(
            "| {} | {} |\n",
            verdict.as_str(),
            totals.get(verdict.as_str()).copied().unwrap_or(0)
        ));
    }
    let between_turns = outcomes
        .iter()
        .filter(|o| !o.between_turns.is_empty())
        .count();
    let between_lines: usize = outcomes.iter().map(|o| o.between_turns.len()).sum();
    let reseated = outcomes.iter().filter(|o| o.reseated).count();
    if chain {
        report.push_str(&format!(
            "\n{between_turns} turns had a between-turn difference ({between_lines} lines); \
             {reseated} turns were reseated from their `start`.\n"
        ));
    }
    let mut tsv = String::from("file\tgame\tturn\tverdict\tsteps_played\tfirst_difference\n");
    for o in &outcomes {
        let played = o.seeds.first().map_or(0, |s| s.played);
        let first = if let Some(first) = o.between_turns.first() {
            first.clone()
        } else {
            match o.verdict {
                ScreenVerdict::Flagged => o.differences.first().cloned().unwrap_or_default(),
                ScreenVerdict::Stopped => o
                    .stop
                    .as_ref()
                    .map(|s| format!("step {} {}: {}", s.step + 1, s.kind, s.reason))
                    .unwrap_or_default(),
                ScreenVerdict::OkRandom => o
                    .randomness_at
                    .as_ref()
                    .map(|s| format!("random at step {} ({})", s.step + 1, s.kind))
                    .unwrap_or_default(),
                ScreenVerdict::Ok => String::new(),
            }
        };
        tsv.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\n",
            clean(&o.file),
            o.game,
            o.turn,
            o.verdict.as_str(),
            played,
            clean(&first)
        ));
    }
    let flagged: Vec<&ScreenTurnOutcome> = outcomes
        .iter()
        .filter(|o| o.verdict == ScreenVerdict::Flagged)
        .collect();
    let stopped: Vec<&ScreenTurnOutcome> = outcomes
        .iter()
        .filter(|o| o.verdict == ScreenVerdict::Stopped)
        .collect();
    let random: Vec<&ScreenTurnOutcome> = outcomes
        .iter()
        .filter(|o| o.verdict == ScreenVerdict::OkRandom)
        .collect();
    if !random.is_empty() {
        report.push_str("\n## Random splits\n\n");
        for o in &random {
            let at = o
                .randomness_at
                .as_ref()
                .map(|s| format!("step {} ({})", s.step + 1, s.kind))
                .unwrap_or_else(|| "an unknown step".to_string());
            report.push_str(&format!("- {} turn {}: split at {at}\n", o.file, o.turn));
        }
    }
    let between: Vec<&ScreenTurnOutcome> = outcomes
        .iter()
        .filter(|o| !o.between_turns.is_empty())
        .collect();
    if !between.is_empty() {
        report.push_str("\n## Between turns\n\n");
        for o in &between {
            report.push_str(&format!("### {} turn {}\n\n", o.file, o.turn));
            for d in &o.between_turns {
                report.push_str(&format!("- {d}\n"));
            }
            report.push('\n');
        }
    }
    if !flagged.is_empty() {
        report.push_str("\n## Flagged\n\n");
        for o in &flagged {
            report.push_str(&format!("### {} turn {}\n\n", o.file, o.turn));
            for d in &o.differences {
                report.push_str(&format!("- {d}\n"));
            }
            report.push('\n');
        }
    }
    if !stopped.is_empty() {
        report.push_str("\n## Stopped\n\n");
        for o in &stopped {
            match &o.stop {
                Some(s) => report.push_str(&format!(
                    "- {} turn {}: step {} {} ({}): {}\n",
                    o.file,
                    o.turn,
                    s.step + 1,
                    s.kind,
                    s.class,
                    s.reason
                )),
                None => report.push_str(&format!(
                    "- {} turn {}: stopped without a reason\n",
                    o.file, o.turn
                )),
            }
        }
        report.push('\n');
    }
    let report_path = format!("{out}/screen-report.md");
    std::fs::write(&report_path, report).unwrap_or_else(|e| panic!("{report_path}: {e}"));
    let tsv_path = format!("{out}/turns.tsv");
    std::fs::write(&tsv_path, tsv).unwrap_or_else(|e| panic!("{tsv_path}: {e}"));
    let json_path = format!("{out}/turns.json");
    let text = serde_json::to_string_pretty(&outcomes).expect("the outcomes serialize");
    std::fs::write(&json_path, text).unwrap_or_else(|e| panic!("{json_path}: {e}"));
    println!(
        "{} screen files, {} turns: {} ok, {} ok-random, {} flagged, {} stopped",
        specs.len(),
        outcomes.len(),
        totals.get("ok").copied().unwrap_or(0),
        totals.get("ok-random").copied().unwrap_or(0),
        totals.get("flagged").copied().unwrap_or(0),
        totals.get("stopped").copied().unwrap_or(0)
    );
    println!("wrote {report_path}, {tsv_path}, {json_path}");
}

pub fn screen_from_actions(args: &[String]) {
    let actions = opt(args, "--actions", "boards/actions");
    let out = opt(args, "--out", "boards/screens");
    let current_patch = args.iter().any(|a| a == "--current-patch");
    let files = json_files(&actions);
    let ids = read_ids();
    let game_build = ids.game_build();
    let mut specs =
        shop_replay::parse_action_files(&files).unwrap_or_else(|e| panic!("{actions}: {e}"));
    shop_replay::seat_taught_spells(&mut specs, &ids);
    let data = Data::bundled();
    std::fs::create_dir_all(&out).unwrap_or_else(|e| panic!("{out}: {e}"));
    let mut written = 0usize;
    let mut skipped = 0usize;
    for file in &specs {
        if current_patch
            && let Some(build) = game_build
            && file.build.is_some_and(|b| b != build)
        {
            skipped += 1;
            continue;
        }
        let screen = screen_replay::screen_from_action_file(&data, &ids, file);
        let path = format!("{out}/{}", screen.file());
        let text = serde_json::to_string_pretty(&screen).expect("a screen file serializes");
        std::fs::write(&path, text).unwrap_or_else(|e| panic!("{path}: {e}"));
        written += 1;
    }
    println!(
        "{written} screen files written to {out}{}",
        if skipped > 0 {
            format!(" ({skipped} other-build files skipped)")
        } else {
            String::new()
        }
    );
}
