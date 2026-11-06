//! `bgbench record`: Era's own shop decisions as training examples, the data
//! the copy phase learns from (docs/notes/training-plans.md "Plan A",
//! docs/real-boards.md "Recording").
//!
//! Every step of every finished game in `boards/actions/` is replayed from the
//! logged state before it (`bgcore::shop_replay`, which already maps the step to
//! an engine `Action` and judges it). A step whose verdict is ok, ok-random or
//! ok-shape and whose action is legal in the state it starts from becomes one
//! example: the observation the bot would have had in Era's seat
//! (`bgcore::obs::write`, the very numbers `bgpy` hands the training), the legal
//! moves (`Game::legal_mask`) and the index of Era's own action.
//!
//! The log's own `end_turn` step carries the end of turn effects, not a
//! decision, so a turn contributes one `EndTurn` example from the state Era
//! pressed the button in (the state before that step) instead: the bot has to
//! learn when to stop as much as what to buy.
//!
//! The replay's other seats are made up by the engine (a sandbox game whose
//! opponents are dead stand-ins), so the example's opponent numbers are blanked
//! where the log cannot say them: see `blank_invented`.

use std::collections::BTreeMap;
use std::io::BufRead;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use bgcore::obs::{
    self, OBS_SIZE, OPP_BLOCK, OPP_SCALARS, OPP_TRIBE_COUNT, OPP_TRIBE_MIXED, OPP_TOP_TRIBE, OWN_END,
    SCALAR_ALIVE,
};
use bgcore::real_boards::CardIdMap;
use bgcore::shop_replay::{self, ReplayOptions, ShopFileSpec, ShopTurnSpec, StepResult, Verdict};
use bgcore::{Action, Data, Leaderboard, N_ACTIONS, Opponent, PLAYERS, Tribe};
use rayon::prelude::*;
use serde::Deserialize;

/// Where the games' final places come from: the boards of Era's real games,
/// named exactly like the action files (`docs/real-boards.md`).
const PLACES: &str = "boards/real";
/// Where the recording goes.
const OUT: &str = "train/runs/clone-data";
/// The card id mapping (a Magicfin token's taught spell is a dbf id in the log).
const REF: &str = "ref/bg-cards.json";
/// The client build line of a game's own log, which says its patch: build
/// 254477 is patch 37.0 and 255790 is patch 37.0.2
/// (docs/briefs/era-comps.md "The patch of each game").
const BUILD_LINE: &str = "GameState.DebugPrintGame() - BuildNumber=";

/// One example: Era's decision in one state.
struct Row {
    /// `<source>-g<n>`, the game's name in `boards/actions/` and `boards/real/`.
    game: String,
    turn: u32,
    /// The step's place in its turn, -1 for the added End Turn example.
    step: i32,
    /// The log step's kind, or `end` for the added one.
    kind: String,
    /// Era's action, as an index into the flat action space.
    action: usize,
    place: u8,
    patch: &'static str,
    obs: Vec<f32>,
    mask: [bool; N_ACTIONS],
}

/// A game that is recorded, with what `boards/real/` and its log say about it.
struct Game<'a> {
    spec: &'a ShopFileSpec,
    name: String,
    place: u8,
    patch: &'static str,
}

/// What one run made.
struct Recorder {
    rows: Vec<Row>,
    /// Why steps gave no example, most common first when printed.
    skipped: BTreeMap<&'static str, usize>,
    /// Games that gave at least one example, and games the folder holds.
    games: usize,
    games_read: usize,
}

/// The place of every game in `dir`, by `<source>` and game number: the
/// action files carry no place, and a game still going (or one the board
/// collector has not written yet) has none to copy.
fn read_places(dir: &str) -> Result<BTreeMap<(String, u32), u8>, String> {
    #[derive(Deserialize)]
    struct Place {
        source: String,
        game: u32,
        #[serde(default)]
        place: u8,
    }
    let mut out = BTreeMap::new();
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{dir}: {e}"))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("{dir}: {e}"))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.ends_with(".json") || name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let place: Place = serde_json::from_str(&text)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        out.insert((place.source, place.game), place.place);
    }
    Ok(out)
}

/// The log folders of the machines whose games are in the folders, as
/// `train/llm/digest.py` has them.
fn log_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    vec![
        home.join("bg-logs"),
        home.join("bg-logs-laptop"),
        home.join("hearthstone-linux/hearthstone/Logs"),
    ]
}

/// The patch a game was played on, from the client build its own `Power.log`
/// names: `37.0`, `37.0.2`, or `unknown` where no log is left.
fn patch_of(source: &str) -> &'static str {
    let folder = source.strip_prefix("laptop-").unwrap_or(source);
    for dir in log_dirs() {
        let path = dir.join(folder).join("Power.log");
        let Ok(file) = std::fs::File::open(&path) else { continue };
        let mut reader = std::io::BufReader::new(file);
        let mut line = String::new();
        // The line stands near the top of the file, in the first game the
        // client started: 20,000 lines is far past it and keeps a 400 MB log
        // from being read whole.
        for _ in 0..20_000 {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            if let Some(at) = line.find(BUILD_LINE) {
                let digits: String = line[at + BUILD_LINE.len()..]
                    .chars()
                    .take_while(char::is_ascii_digit)
                    .collect();
                return match digits.as_str() {
                    "254477" => "37.0",
                    "255790" => "37.0.2",
                    _ => "unknown",
                };
            }
        }
    }
    "unknown"
}

/// The observation of the seated state and its legal moves, with the parts the
/// log cannot say blanked.
fn state_row(game: &bgcore::Game) -> (Vec<f32>, [bool; N_ACTIONS]) {
    let mut obs = vec![0.0f32; OBS_SIZE];
    obs::write(game, shop_replay::ME, &mut obs);
    blank_invented(&mut obs, game);
    let mut mask = [false; N_ACTIONS];
    game.legal_mask(&mut mask);
    (obs, mask)
}

/// Blank the numbers `obs::write` takes from the seats the engine made up.
///
/// A replay is a sandbox game: player 0 is Era and the other seven are dead
/// stand-ins with a placement, a Health and a tier the engine wrote itself. The
/// log says nothing about them, and an example that kept them would teach the
/// bot about opponents that never existed: seven dead players and one player
/// alive, every time. So:
/// - `SCALAR_ALIVE` (own scalar 7, the living players) is set to 0, unknown.
/// - Every opponent block's `OPP_SCALARS` values, 54 of them (alive, health,
///   armor, tier, hero, seen, turns since, next, last, placement, the last
///   board's tribes, Deity, Blood Gem and Tavern spell bonus, trinkets, the
///   leaderboard's tribe line and the last three fights), are set to 0, but
///   the leaderboard tribe line of the one seat whose board is kept (below),
///   which the log's board says: Power.log carries no leaderboard tribe tag
///   (`bgbench powerlog` dumps every tag; there is none), and the tribe the
///   client shows is that of the board the player fought with, which the log
///   has for the opponent Era fought and for no other seat.
/// - Every opponent's board slots are set to 0, except the first opponent
///   block, the seat `load` gives the warband the log really has
///   (`last_opponent_board`, the opponent Era last fought, as that seat's seen
///   board): those slots keep the real cards, their stats and their keywords.
///   Which of the seven seats that is comes from the log saying nothing about
///   seat order, so the first block is as good as any.
///
/// Every opponent's last three fights stay 0. Power.log carries
/// `BACON_WON_LAST_COMBAT` on my own hero and on the hero of the one
/// opponent I fought (that is all of it: a dump of a real log has no other
/// entity with the tag, and there is no loss, tie or damage tag at all), so
/// for the other six seats there is no result at all, and for the seventh a
/// won flag that cannot tell a loss from a tie and no damage: not a fight
/// result.
///
/// `SCALAR_WON_LAST_COMBAT` and `SCALAR_LOST_LAST_COMBAT` are left alone: the
/// log has them (`won_last_combat`), so they are Era's own state, not an
/// opponent's. `SCALAR_GHOST` stays 0, which is what a game with no pairing
/// writes: the log does not say whether Era fought a ghost that turn.
fn blank_invented(out: &mut [f32], game: &bgcore::Game) {
    out[SCALAR_ALIVE] = 0.0;
    let real = match game.players[shop_replay::ME].last_opponent {
        Some(Opponent::Player(q)) | Some(Opponent::Ghost(q)) => Some(q as usize),
        None => None,
    };
    for k in 0..PLAYERS - 1 {
        // The blocks are the seats after mine, in seat order: block 0 is the
        // seat right behind me, which is the one `load` gives the warband to.
        let q = (shop_replay::ME + k + 1) % PLAYERS;
        let at = OWN_END + k * OPP_BLOCK;
        let end = if real == Some(q) { at + OPP_SCALARS } else { at + OPP_BLOCK };
        out[at..end].fill(0.0);
        if real == Some(q) {
            // The one board the log carries is the board that fight went in
            // with, which is where the leaderboard's snapshot is taken.
            let seen = &game.players[shop_replay::ME].seen[q];
            let line = Leaderboard::of(&game.data.cards, &seen.board);
            if let Some(t) = line.top {
                let j = Tribe::EVERY.iter().position(|&x| x == t).unwrap_or(0);
                out[at + OPP_TOP_TRIBE + j] = 1.0;
            }
            out[at + OPP_TRIBE_MIXED] = line.mixed as u8 as f32;
            out[at + OPP_TRIBE_COUNT] = line.count as f32;
        }
    }
}

/// Count one step or one game that gave no example.
fn skip(why: &'static str, skipped: &mut BTreeMap<&'static str, usize>) {
    *skipped.entry(why).or_default() += 1;
}

/// Whether the replay judged a step one this recording can copy.
fn kept(v: Verdict) -> bool {
    matches!(v, Verdict::Ok | Verdict::OkRandom | Verdict::OkShape)
}

/// Why a step the replay did not keep gives no example. `unmapped` is a step
/// kind the engine has no action for, which the replay counts as incomplete.
fn reason(result: &StepResult) -> &'static str {
    match result.verdict {
        Verdict::Flagged => "flagged",
        Verdict::Illegal => "illegal",
        Verdict::Incomplete => seat_reason(&result.first()),
        _ => "kept",
    }
}

/// The same reasons for a step the engine refuses while it is seated.
fn seat_reason(why: &str) -> &'static str {
    if why.starts_with("the engine does not allow") {
        "illegal"
    } else if why.starts_with("a step the engine has no action for") {
        "unmapped"
    } else {
        "incomplete"
    }
}

/// One logged step as an example: seat it with the seed the replay runs first,
/// and take the state's observation, its legal moves and the index of the
/// step's own action. The error is why the step gives none.
fn step_example(
    data: &Arc<Data>,
    ids: &CardIdMap,
    game: &Game,
    turn: &ShopTurnSpec,
    i: usize,
    seed: u64,
) -> Result<Row, &'static str> {
    let seated =
        shop_replay::seat_step(data, ids, game.spec, turn, i, seed).map_err(|(why, _)| seat_reason(&why))?;
    if !seated.missing.is_empty() {
        return Err("incomplete");
    }
    let Some(action) = seated.decision() else { return Err("unmapped") };
    let (obs, mask) = state_row(&seated.game);
    // The action the bot could not have picked teaches nothing.
    if !mask[action.index()] {
        return Err("illegal");
    }
    Ok(Row {
        game: game.name.clone(),
        turn: turn.turn,
        step: i as i32,
        kind: turn.actions[i].step.clone(),
        action: action.index(),
        place: game.place,
        patch: game.patch,
        obs,
        mask,
    })
}

/// The turn's End Turn example: the state before the log's own `end_turn` step,
/// which is the state Era pressed the button in (the step itself carries the end
/// of turn effects, which are the game's, not a decision).
fn end_example(
    data: &Arc<Data>,
    ids: &CardIdMap,
    game: &Game,
    turn: &ShopTurnSpec,
    at: usize,
    seed: u64,
) -> Result<Row, &'static str> {
    let seated =
        shop_replay::seat_step(data, ids, game.spec, turn, at, seed).map_err(|(why, _)| seat_reason(&why))?;
    if !seated.missing.is_empty() {
        return Err("incomplete");
    }
    let (obs, mask) = state_row(&seated.game);
    let action = Action::EndTurn.index();
    // A choice still open at the end of the turn ("Answer the Discover") leaves
    // End Turn illegal, and the example would teach an action the bot cannot
    // take there.
    if !mask[action] {
        return Err("illegal");
    }
    Ok(Row {
        game: game.name.clone(),
        turn: turn.turn,
        step: -1,
        kind: "end".to_string(),
        action,
        place: game.place,
        patch: game.patch,
        obs,
        mask,
    })
}

/// Every example of one turn, in step order with the End Turn example last, and
/// why its other steps gave none.
fn record_turn(
    data: &Arc<Data>,
    ids: &CardIdMap,
    game: &Game,
    turn: &ShopTurnSpec,
    opts: &ReplayOptions,
) -> (Vec<Row>, BTreeMap<&'static str, usize>) {
    let mut rows = Vec::new();
    let mut skipped: BTreeMap<&'static str, usize> = BTreeMap::new();
    let last_real = turn.actions.iter().rposition(|s| s.step != "end_turn");
    let mut last_kept = false;
    for (i, step) in turn.actions.iter().enumerate() {
        if step.step == "end_turn" {
            continue;
        }
        let result = shop_replay::replay_step(data, ids, game.spec, turn, i, opts);
        let mut here = false;
        if !kept(result.verdict) {
            skip(reason(&result), &mut skipped);
        } else {
            match step_example(data, ids, game, turn, i, opts.seed) {
                Ok(row) => {
                    rows.push(row);
                    here = true;
                }
                Err(why) => skip(why, &mut skipped),
            }
        }
        if Some(i) == last_real {
            last_kept = here;
        }
    }
    // The turn's end: the bot has to learn when to stop. Only a turn whose last
    // decision replayed ok ends where the bot would have ended.
    match turn.actions.iter().rposition(|s| s.step == "end_turn") {
        None => skip("end: the log has no end turn step", &mut skipped),
        Some(_) if last_real.is_none() => skip("end: a turn with no step of its own", &mut skipped),
        Some(_) if !last_kept => skip("end: the last step was not ok", &mut skipped),
        Some(at) => match end_example(data, ids, game, turn, at, opts.seed) {
            Ok(row) => rows.push(row),
            Err(why) => skip(why, &mut skipped),
        },
    }
    (rows, skipped)
}

/// Every example of the folder, in game, turn and step order.
///
/// A `Game` is about 157 KB by value and a turn seats one for the replay's
/// verdict and one for the example, which is more than a default worker stack
/// holds, so the turns run on a pool of its own with room for them.
fn record(
    data: &Arc<Data>,
    ids: &CardIdMap,
    files: &[ShopFileSpec],
    places: &BTreeMap<(String, u32), u8>,
    opts: &ReplayOptions,
) -> Recorder {
    const STACK: usize = 32 << 20;
    let pool = rayon::ThreadPoolBuilder::new()
        .stack_size(STACK)
        .build()
        .expect("the recording's thread pool");
    pool.install(|| record_on(data, ids, files, places, opts))
}

/// `record`'s own work, on whatever thread runs it.
fn record_on(
    data: &Arc<Data>,
    ids: &CardIdMap,
    files: &[ShopFileSpec],
    places: &BTreeMap<(String, u32), u8>,
    opts: &ReplayOptions,
) -> Recorder {
    let mut patches: BTreeMap<String, &'static str> = BTreeMap::new();
    let mut games: Vec<Game> = Vec::new();
    let mut out = Recorder {
        rows: Vec::new(),
        skipped: BTreeMap::new(),
        games: 0,
        games_read: files.len(),
    };
    for spec in files {
        let Some(&place) = places.get(&(spec.source.clone(), spec.game)) else {
            *out.skipped.entry("no board file (place unknown)").or_default() += 1;
            continue;
        };
        if place == 0 {
            *out.skipped.entry("place 0 (the game is still going)").or_default() += 1;
            continue;
        }
        let patch = *patches.entry(spec.source.clone()).or_insert_with(|| patch_of(&spec.source));
        games.push(Game {
            spec,
            name: format!("{}-g{}", spec.source, spec.game),
            place,
            patch,
        });
    }
    let turns: Vec<(&Game, &ShopTurnSpec)> = games
        .iter()
        .flat_map(|g| g.spec.turns.iter().map(move |t| (g, t)))
        .collect();
    let recorded: Vec<(Vec<Row>, BTreeMap<&'static str, usize>)> = turns
        .par_iter()
        .map(|(game, turn)| record_turn(data, ids, game, turn, opts))
        .collect();
    for (rows, skipped) in recorded {
        out.rows.extend(rows);
        for (why, n) in skipped {
            *out.skipped.entry(why).or_default() += n;
        }
    }
    out.games = out.rows.iter().map(|r| r.game.as_str()).collect::<std::collections::BTreeSet<_>>().len();
    out
}

/// Write the three files and the header of one recording.
fn write_files(out: &str, rec: &Recorder) -> Result<(), String> {
    std::fs::create_dir_all(out).map_err(|e| format!("{out}: {e}"))?;
    let path = |name: &str| format!("{out}/{name}");
    let obs_path = path("obs.f32");
    let mask_path = path("mask.u8");
    let meta_path = path("meta.tsv");
    let header_path = path("header.json");
    let file = |p: &str| {
        std::fs::File::create(p).map(std::io::BufWriter::new).map_err(|e| format!("{p}: {e}"))
    };
    let mut obs_out = file(&obs_path)?;
    let mut mask_out = file(&mask_path)?;
    let mut meta = file(&meta_path)?;
    writeln!(meta, "i\taction\tgame\tturn\tstep\tkind\tplace\tpatch")
        .map_err(|e| format!("{meta_path}: {e}"))?;
    for (i, row) in rec.rows.iter().enumerate() {
        let mut bytes = Vec::with_capacity(row.obs.len() * 4);
        for v in &row.obs {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        obs_out.write_all(&bytes).map_err(|e| format!("{obs_path}: {e}"))?;
        let bits: Vec<u8> = row.mask.iter().map(|&b| b as u8).collect();
        mask_out.write_all(&bits).map_err(|e| format!("{mask_path}: {e}"))?;
        writeln!(
            meta,
            "{i}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            row.action, row.game, row.turn, row.step, row.kind, row.place, row.patch
        )
        .map_err(|e| format!("{meta_path}: {e}"))?;
    }
    for (mut w, p) in [(obs_out, &obs_path), (mask_out, &mask_path), (meta, &meta_path)] {
        w.flush().map_err(|e| format!("{p}: {e}"))?;
    }
    let skipped: BTreeMap<&str, usize> =
        rec.skipped.iter().map(|(why, n)| (*why, *n)).collect();
    let header = serde_json::json!({
        "obs_size": OBS_SIZE,
        "n_actions": N_ACTIONS,
        "count": rec.rows.len(),
        "games": rec.games,
        "git": git_rev(),
        "skipped": skipped,
    });
    std::fs::write(&header_path, format!("{header:#}\n"))
        .map_err(|e| format!("{header_path}: {e}"))?;
    println!("wrote {obs_path}, {mask_path}, {meta_path}, {header_path}");
    Ok(())
}

/// The commit the recording was made from, or `unknown` where git is not there.
fn git_rev() -> String {
    std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

/// The most common actions: how many examples each, most common first.
fn top_actions(rows: &[Row], n: usize) -> Vec<(usize, usize)> {
    let mut counts: BTreeMap<usize, usize> = BTreeMap::new();
    for row in rows {
        *counts.entry(row.action).or_default() += 1;
    }
    let mut out: Vec<(usize, usize)> = counts.into_iter().collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    out.truncate(n);
    out
}

/// The action's name as `Action::kind` numbers it (`ACTION_KINDS`).
fn action_kind(index: usize) -> &'static str {
    match Action::from_index(index) {
        Some(a) => bgcore::ACTION_KINDS[a.kind()].0,
        None => "?",
    }
}

/// `bgbench record`: write Era's logged shop decisions as training examples.
pub fn run(args: &[String]) {
    let t = Instant::now();
    let opt = |name: &str, default: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|k| args.get(k + 1))
            .cloned()
            .unwrap_or_else(|| default.to_string())
    };
    let actions = opt("--actions", "boards/actions");
    let places = opt("--places", PLACES);
    let out = opt("--out", OUT);
    let seeds: u32 = opt("--seeds", "64").parse().unwrap_or_else(|_| {
        eprintln!("--seeds needs a number");
        std::process::exit(2);
    });
    let files = crate::read_board_files(&actions).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(2);
    });
    let ids = CardIdMap::parse(
        &std::fs::read_to_string(REF).unwrap_or_else(|e| panic!("{REF} (the card id mapping): {e}")),
    )
    .unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(2);
    });
    let mut specs = shop_replay::parse_action_files(&files).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(2);
    });
    // A Magicfin token's taught spell is a dbf id in the log; the ref card map
    // is the only thing that can say which card that is.
    shop_replay::seat_taught_spells(&mut specs, &ids);
    let places = read_places(&places).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(2);
    });
    let data = Arc::new(Data::bundled());
    let opts = ReplayOptions { seeds: seeds.max(1), seed: 1 };
    let rec = record(&data, &ids, &specs, &places, &opts);
    if let Err(e) = write_files(&out, &rec) {
        eprintln!("{e}");
        std::process::exit(2);
    }
    let el = t.elapsed();
    let n = rec.rows.len();
    let ends = rec.rows.iter().filter(|r| r.kind == "end").count();
    println!(
        "{n} examples from {} games ({} games read, {}.{}% of them End Turn)",
        rec.games,
        rec.games_read,
        ends * 100 / n.max(1),
        ends * 1000 / n.max(1) % 1000
    );
    let mut patches: BTreeMap<&'static str, usize> = BTreeMap::new();
    for row in &rec.rows {
        *patches.entry(row.patch).or_default() += 1;
    }
    let by_patch: Vec<String> =
        patches.iter().map(|(p, n)| format!("{p} {n}")).collect();
    println!("by patch: {}", by_patch.join(", "));
    let skipped: Vec<String> =
        rec.skipped.iter().map(|(why, n)| format!("{why} {n}")).collect();
    println!("skipped: {}", skipped.join(", "));
    println!("the 10 most common actions:");
    let names: Vec<String> = top_actions(&rec.rows, 10)
        .into_iter()
        .map(|(i, n)| {
            let action = Action::from_index(i).map_or_else(|| "?".to_string(), |a| format!("{a:?}"));
            format!("  {n:6}  {:<15} {action} ({i})", action_kind(i))
        })
        .collect();
    for line in names {
        println!("{line}");
    }
    println!("{el:.1?} on {} cores", rayon::current_num_threads());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// No card id map: the loader falls back to the name in the file for an id
    /// no map knows, and no step of the fixtures read is a Magicfin token's
    /// taught spell, the one thing the map is needed for.
    fn ids() -> CardIdMap {
        CardIdMap::parse(r#"{"cards": []}"#).unwrap()
    }

    fn fixture() -> Vec<(String, String)> {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../bgcore/tests/data/actions");
        let mut files: Vec<(String, String)> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap())
            .filter(|e| e.file_name().to_str().is_some_and(|n| n.ends_with(".json")))
            .map(|e| {
                (
                    e.file_name().to_str().unwrap().to_string(),
                    std::fs::read_to_string(e.path()).unwrap(),
                )
            })
            .collect();
        files.sort();
        files
    }

    #[test]
    fn a_row_is_the_seated_state() {
        let data = Arc::new(Data::bundled());
        let ids = ids();
        let mut specs = shop_replay::parse_action_files(&fixture()).unwrap();
        shop_replay::seat_taught_spells(&mut specs, &ids);
        // Every fixture game finished: the places are the test's own.
        let places: BTreeMap<(String, u32), u8> = specs
            .iter()
            .map(|f| ((f.source.clone(), f.game), 3))
            .collect();
        let opts = ReplayOptions { seeds: 1, seed: 1 };
        let rec = record(&data, &ids, &specs, &places, &opts);
        assert!(!rec.rows.is_empty(), "the fixtures record no example: {:?}", rec.skipped);

        // Row 0, read back the way the training reads it.
        let dir = std::env::temp_dir().join(format!("bgbench-record-{}", std::process::id()));
        write_files(dir.to_str().unwrap(), &rec).unwrap();
        let obs_bytes = std::fs::read(dir.join("obs.f32")).unwrap();
        let mask_bytes = std::fs::read(dir.join("mask.u8")).unwrap();
        let meta = std::fs::read_to_string(dir.join("meta.tsv")).unwrap();
        let header: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("header.json")).unwrap()).unwrap();
        assert_eq!(obs_bytes.len(), rec.rows.len() * OBS_SIZE * 4);
        assert_eq!(mask_bytes.len(), rec.rows.len() * N_ACTIONS);
        assert_eq!(header["count"], rec.rows.len());
        assert_eq!(header["obs_size"], OBS_SIZE);
        assert_eq!(header["n_actions"], N_ACTIONS);
        let first = meta.lines().nth(1).unwrap().split('\t').collect::<Vec<_>>();
        assert_eq!(first.len(), 8, "the meta line: {first:?}");
        let action: usize = first[1].parse().unwrap();
        let game = first[2].to_string();
        let turn: u32 = first[3].parse().unwrap();
        let step: i32 = first[4].parse().unwrap();
        // The mask says the action is legal.
        assert_eq!(mask_bytes[action], 1);
        // The observation is the seated state's, and nothing else's.
        let spec = specs.iter().find(|f| format!("{}-g{}", f.source, f.game) == game).unwrap();
        let turn_spec = spec.turns.iter().find(|t| t.turn == turn).unwrap();
        let seated = if step >= 0 {
            shop_replay::seat_step(&data, &ids, spec, turn_spec, step as usize, opts.seed).unwrap()
        } else {
            let at = turn_spec.actions.iter().rposition(|s| s.step == "end_turn").unwrap();
            shop_replay::seat_step(&data, &ids, spec, turn_spec, at, opts.seed).unwrap()
        };
        let mut obs = vec![0.0f32; OBS_SIZE];
        obs::write(&seated.game, shop_replay::ME, &mut obs);
        blank_invented(&mut obs, &seated.game);
        // The block whose board the log carries keeps the leaderboard's tribe
        // line, taken from that board: blank_invented zeroes the rest of the
        // block's scalars and must not zero this one with them.
        let real = match seated.game.players[shop_replay::ME].last_opponent {
            Some(bgcore::Opponent::Player(q)) | Some(bgcore::Opponent::Ghost(q)) => Some(q as usize),
            None => None,
        };
        if let Some(q) = real {
            let seen = &seated.game.players[shop_replay::ME].seen[q];
            let line = bgcore::Leaderboard::of(&seated.game.data.cards, &seen.board);
            let at = OWN_END;
            assert_eq!(obs[at + obs::OPP_TRIBE_COUNT], line.count as f32);
            assert_eq!(obs[at + obs::OPP_TRIBE_MIXED], line.mixed as u8 as f32);
            for (j, t) in bgcore::Tribe::EVERY.into_iter().enumerate() {
                assert_eq!(obs[at + obs::OPP_TOP_TRIBE + j], (line.top == Some(t)) as u8 as f32);
            }
        }
        let mut mask = [false; N_ACTIONS];
        seated.game.legal_mask(&mut mask);
        assert!(mask[action]);
        for (k, v) in obs.iter().enumerate() {
            let at = k * 4;
            let read = f32::from_le_bytes(obs_bytes[at..at + 4].try_into().unwrap());
            assert_eq!(read, *v, "observation value {k}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
