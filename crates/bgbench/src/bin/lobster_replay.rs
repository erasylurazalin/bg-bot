//! Replay Era's logged turn 10 of the Tasty Lobstah game step by step in the
//! lobster mode and write `train/runs/lobster-search/replay-turn10.md`.
//!
//! Each step is seated from the log's own state before it (board, hand,
//! Tavern, Gold and the player-wide counters), with the shop replay's action
//! mapping. The seeded Tavern and Gryphon streams are off; only the mode's
//! Fishbait and Deathstrider counters are on. The engine's Lobster count
//! after a step is compared with the log's after state, so one step's trigger
//! count is judged on the log's exact position.

use std::sync::Arc;

use bgcore::cards::Counter;
use bgcore::shop_replay::{self, ShopStateSpec, ShopStepSpec, ME};
use bgcore::{Action, CardIdMap, Data, Game, Minion};

const SOURCE: &str = "boards/actions/Hearthstone_2026_10_28_22_11_50-g2.json";
const REF: &str = "ref/bg-cards.json";
const OUT: &str = "train/runs/lobster-search/replay-turn10.md";
const TURN: u32 = 10;
const SEED: u64 = 1;

fn logged_count(numbers: &[i32], golden: bool) -> Option<u16> {
    let attack = *numbers.first()?;
    // Plain prints [2(c+1), c+1], golden [4(c+1), 2(c+1)].
    let per = if golden { 4 } else { 2 };
    Some(((attack / per - 1).max(0)).min(u16::MAX as i32) as u16)
}

#[derive(Clone)]
struct LobsterView {
    zone: &'static str,
    attack: i32,
    health: i32,
    golden: bool,
    numbers: Option<Vec<i32>>,
    count: Option<u16>,
}

fn log_lobster(state: &ShopStateSpec) -> Option<LobsterView> {
    for (zone, zone_minions) in [("board", &state.board), ("hand", &state.hand)] {
        if let Some(m) = zone_minions.iter().find(|m| m.name.as_deref() == Some("Tasty Lobster")) {
            return Some(LobsterView {
                zone,
                attack: m.attack,
                health: m.health,
                golden: m.golden,
                numbers: Some(m.numbers.clone()),
                count: logged_count(&m.numbers, m.golden),
            });
        }
    }
    None
}

fn engine_lobster(game: &Game) -> Option<LobsterView> {
    let pl = &game.players[ME];
    let is_lobster = |m: &&Minion| game.data.cards.get(m.card).name == "Tasty Lobster";
    let found = pl
        .board
        .iter()
        .find(is_lobster)
        .map(|m| ("board", m))
        .or_else(|| pl.hand.iter().find(is_lobster).map(|m| ("hand", m)));
    found.map(|(zone, m)| LobsterView {
        zone,
        attack: m.attack,
        health: m.health,
        golden: m.golden,
        numbers: None,
        count: Some(pl.improve[Counter::TastyLobster.index()]),
    })
}

fn logged_action(step: &ShopStepSpec) -> String {
    let mut s = step.step.clone();
    let card = step
        .card
        .as_ref()
        .or(step.target.as_ref())
        .or(step.picked.as_ref())
        .or(step.source.as_ref());
    if let Some(name) = card.and_then(|c| c.name.as_deref()) {
        s.push(' ');
        s.push_str(name);
    }
    if let Some(slot) = step.slot {
        s.push_str(&format!(" slot {slot}"));
    }
    if let Some(index) = step.index {
        s.push_str(&format!(" hand {index}"));
    }
    if let Some(position) = step.position {
        s.push_str(&format!(" board {position}"));
    }
    if let Some(to) = step.to {
        s.push_str(&format!(" to {to}"));
    }
    s
}

fn engine_actions(actions: &[Action]) -> String {
    if actions.is_empty() {
        return "end of turn effects".to_string();
    }
    actions.iter().map(|a| format!("{a:?}")).collect::<Vec<_>>().join(", ")
}

fn view_text(v: &Option<LobsterView>) -> String {
    match v {
        None => "none".to_string(),
        Some(v) => {
            let numbers = v
                .numbers
                .as_ref()
                .map(|n| format!(" nums {:?}", n))
                .unwrap_or_default();
            format!(
                "{} {}/{} {} count {}{}",
                v.zone,
                v.attack,
                v.health,
                if v.golden { "golden" } else { "plain" },
                v.count.map_or("?".to_string(), |c| c.to_string()),
                numbers
            )
        }
    }
}

fn state_before(
    turn: &shop_replay::ShopTurnSpec,
    i: usize,
) -> Option<&ShopStateSpec> {
    if i == 0 {
        Some(&turn.start)
    } else {
        turn.actions.get(i - 1).and_then(|a| a.after.as_ref())
    }
}

fn main() {
    let text = std::fs::read_to_string(SOURCE).unwrap_or_else(|e| panic!("{SOURCE}: {e}"));
    let name = SOURCE.rsplit('/').next().unwrap_or(SOURCE).to_string();
    let mut files = shop_replay::parse_action_files(&[(name, text)])
        .unwrap_or_else(|e| panic!("{SOURCE}: {e}"));
    let ids_text = std::fs::read_to_string(REF).unwrap_or_else(|e| panic!("{REF}: {e}"));
    let ids = CardIdMap::parse(&ids_text).unwrap_or_else(|e| panic!("{REF}: {e}"));
    shop_replay::seat_taught_spells(&mut files, &ids);
    let file = &files[0];
    let turn = file
        .turns
        .iter()
        .find(|t| t.turn == TURN)
        .unwrap_or_else(|| panic!("{SOURCE}: no turn {TURN}"));
    let data = Arc::new(Data::bundled());

    let start_log = log_lobster(&turn.start);
    let start_count = start_log.as_ref().and_then(|v| v.count).unwrap_or(0);
    let (_start_game, start_missing) = shop_replay::turn_start_game(&data, &ids, file, turn, SEED)
        .unwrap_or_else(|| panic!("{SOURCE}: turn {TURN} does not seat"));

    let mut rows = Vec::new();
    let mut first_count_divergence: Option<String> = None;
    let mut first_stat_divergence: Option<String> = None;
    let mut stopped: Option<String> = None;
    let mut cumulative_engine = start_count as i32;
    let mut cumulative_fish = 0u32;
    let mut cumulative_ds = 0u32;
    let mut last_engine = None;
    let mut last_log = start_log.clone();
    let mut last_board = (0i32, 0i32);

    for i in 0..turn.actions.len() {
        let seated = match shop_replay::seat_step(&data, &ids, file, turn, i, SEED) {
            Ok(s) => s,
            Err((why, missing)) => {
                stopped = Some(format!("step {} does not seat: {why} ({missing:?})", i + 1));
                break;
            }
        };
        let step = seated.step.clone();
        let actions = seated.actions.clone();
        let seated_missing = seated.missing.clone();
        let before = state_before(turn, i);
        let before_view = before.and_then(log_lobster);
        let before_count = before_view.as_ref().and_then(|v| v.count).unwrap_or(0);
        let mut step_game = seated.game.clone();
        step_game.enable_lobster_mode(ME, SEED, 0);
        if let Some(mode) = &mut step_game.players[ME].lobster {
            mode.stream_tavern = false;
            mode.stream_beasts = false;
        }
        let res = seated.play_on(&data, &ids, step_game);
        match res {
            Ok((next, play_missing, _action_text)) => {
                let engine_after = engine_lobster(&next);
                last_board = (
                    next.players[ME].board.iter().map(|m| m.attack).sum(),
                    next.players[ME].board.iter().map(|m| m.health).sum(),
                );
                let log_after = step.after.as_ref().and_then(log_lobster);
                let engine_count = engine_after
                    .as_ref()
                    .and_then(|v| v.count)
                    .unwrap_or(before_count);
                let log_count = log_after.as_ref().and_then(|v| v.count);
                let engine_delta = engine_count as i32 - before_count as i32;
                let log_delta = log_count.map_or(0, |c| c as i32 - before_count as i32);
                cumulative_engine += engine_delta;
                let (fish_step, ds_step) = next.players[ME]
                    .lobster
                    .map_or((0, 0), |m| (m.fishbait_attacks, m.deathstrider_fires));
                cumulative_fish += fish_step;
                cumulative_ds += ds_step;

                if first_count_divergence.is_none()
                    && let Some(log_count) = log_count
                    && engine_delta != log_delta
                {
                    first_count_divergence = Some(format!(
                        "step {} ({}): engine count {} -> {} (+{}), log {} -> {} (+{})",
                        i + 1,
                        logged_action(&step),
                        before_count,
                        engine_count,
                        engine_delta,
                        before_count,
                        log_count,
                        log_delta
                    ));
                }
                if first_stat_divergence.is_none()
                    && let (Some(e), Some(l)) = (&engine_after, &log_after)
                    && (e.attack != l.attack || e.health != l.health)
                {
                    let why = if engine_count == l.count.unwrap_or(engine_count) {
                        "same count, different random Lobster target"
                    } else {
                        "different count"
                    };
                    first_stat_divergence = Some(format!(
                        "step {} ({}): engine Lobster {} {}/{} vs log {}/{} ({why})",
                        i + 1,
                        logged_action(&step),
                        e.zone,
                        e.attack,
                        e.health,
                        l.attack,
                        l.health
                    ));
                }
                rows.push(format!(
                    "| {} | {} | {} | {} | {} | {} | +{}/+{} | {}/{} | {}/{} |",
                    i + 1,
                    logged_action(&step),
                    engine_actions(&actions),
                    if play_missing.is_empty() {
                        "yes".to_string()
                    } else {
                        format!("missing: {play_missing:?}")
                    },
                    view_text(&engine_after),
                    view_text(&log_after),
                    engine_delta,
                    log_delta,
                    fish_step,
                    cumulative_fish,
                    ds_step,
                    cumulative_ds,
                ));
                last_engine = engine_after;
                last_log = log_after;
            }
            Err((why, play_missing)) => {
                stopped = Some(format!(
                    "step {} ({}): the engine does not allow it: {why} (missing {:?}{:?})",
                    i + 1,
                    logged_action(&step),
                    seated_missing,
                    play_missing
                ));
                break;
            }
        }
    }

    let end_log_state = turn.end.as_ref().or_else(|| {
        turn.actions.last().and_then(|a| a.after.as_ref())
    });
    let end_log_lobster = end_log_state.and_then(log_lobster);
    let log_attack: i32 = end_log_state
        .map(|s| s.board.iter().map(|m| m.attack).sum())
        .unwrap_or(0);
    let log_health: i32 = end_log_state
        .map(|s| s.board.iter().map(|m| m.health).sum())
        .unwrap_or(0);
    let log_count = end_log_lobster.as_ref().and_then(|v| v.count);

    let mut md = String::new();
    md.push_str("# Replay of Era's logged turn 10 in the lobster mode\n\n");
    md.push_str("Source: `");
    md.push_str(SOURCE);
    md.push_str("`, turn 10, all 47 logged steps. Variant B (the logged golden Headhunter Gryphon). Each step is seated from the log's own state before it (board, hand, Tavern, Gold and the player-wide counters) with `shop_replay::seat_step`, and played with `SeatedStep::play_on`. The seeded Tavern and Gryphon streams are off. The engine's Lobster count after the step is compared with the log's `after` state, so each step is judged on the position the log shows.\n\n");
    md.push_str("The count is the player-wide Tasty Lobster count. A plain Lobster prints `[2(c+1), c+1]`, so its count is `attack / 2 - 1`; a golden Lobster prints `[4(c+1), 2(c+1)]`, so its count is `attack / 4 - 1`. The engine count is `Player.improve[TastyLobster]`.\n\n");
    md.push_str("## Start\n\n");
    md.push_str(&format!(
        "- Log: {}.\n- Engine: the same seated state (missing: {:?}).\n- Log board Attack/Health: {}/{}.\n\n",
        view_text(&start_log),
        start_missing,
        turn.start.board.iter().map(|m| m.attack).sum::<i32>(),
        turn.start.board.iter().map(|m| m.health).sum::<i32>(),
    ));
    md.push_str("## Steps\n\n");
    md.push_str("| # | Logged action | Engine action | Played | Engine Lobster (A/H count) | Log Lobster (A/H nums count) | Trigger delta (engine/log) | Fishbait attacks (step/cum.) | Deathstrider fires (step/cum.) |\n");
    md.push_str("|---|---|---|---|---|---|---|---|---|\n");
    for row in &rows {
        md.push_str(row);
        md.push('\n');
    }
    md.push_str("\n## End of turn 10\n\n");
    md.push_str("| | Engine | Log |\n|---|---|---|\n");
    md.push_str(&format!(
        "| Summed engine trigger count (start {} plus every step's delta) | {} | {} |\n",
        start_count,
        cumulative_engine,
        log_count.map_or("?".to_string(), |c| c.to_string())
    ));
    md.push_str(&format!(
        "| Last step's engine count | {} | |\n",
        last_engine.as_ref().and_then(|v| v.count).map_or("?".to_string(), |c| c.to_string())
    ));
    md.push_str(&format!(
        "| Board Attack (engine after the last step) | {} | {} |\n| Board Health (engine after the last step) | {} | {} |\n",
        last_board.0, log_attack, last_board.1, log_health
    ));
    md.push_str(&format!(
        "| Fishbait attacks | {} | not logged |\n| Deathstrider fires | {} | not logged |\n",
        cumulative_fish, cumulative_ds
    ));
    md.push_str(&format!(
        "| Final Lobster | {} | {} |\n",
        view_text(&last_engine),
        view_text(&last_log)
    ));
    md.push_str("\n## Divergence\n\n");
    match &first_count_divergence {
        Some(d) => md.push_str(&format!("First count divergence: {d}.\n")),
        None => md.push_str("No count divergence: every step's engine trigger delta matches the log's count change.\n"),
    }
    if let Some(d) = &first_stat_divergence {
        md.push_str(&format!("\nFirst Lobster-stat divergence: {d}.\n"));
    }
    if let Some(s) = &stopped {
        md.push_str(&format!("\nReplay stopped: {s}.\n"));
    } else {
        md.push_str("\nAll 47 steps replayed.\n");
    }
    md.push_str("\n## Why this matters for the search\n\n");
    if first_count_divergence.is_none() && log_count == Some(18) && cumulative_engine == 18 {
        md.push_str("The engine adds the same 8 triggers as the log over turn 10 when it is given the log's own positions and actions, ending at count 18. The low counts in the stopped search therefore come from the search's lines and its random streams, not from a rule that fails to fire on this comp.\n");
    } else {
        md.push_str("The engine does not reproduce the log's turn 10 trigger count on the log's own positions. The first count divergence above is the rule to chase before blaming the search.\n");
    }
    md.push_str("\n## What this leaves out\n\n");
    md.push_str("- Fights are skipped; the count jumps between logged turns are fights (Era's shop-only counts are 10 to 18, 30 to 42, 54 to 66, 85 to 105).\n");
    md.push_str("- The engine RNG is not the log's RNG for the Gryphon's random Beasts or the Lobster's random targets, so a stat difference at equal count is a random-target difference, not a rule difference.\n");
    std::fs::create_dir_all("train/runs/lobster-search")
        .unwrap_or_else(|e| panic!("train/runs/lobster-search: {e}"));
    std::fs::write(OUT, &md).unwrap_or_else(|e| panic!("{OUT}: {e}"));
    println!("wrote {OUT}");
    if let Some(d) = first_count_divergence {
        println!("first count divergence: {d}");
    }
    if let Some(d) = first_stat_divergence {
        println!("first stat divergence: {d}");
    }
    if let Some(s) = stopped {
        println!("stopped: {s}");
    }
}
