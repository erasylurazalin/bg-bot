//! `bgbench collect-boards`: the real boards, one JSON file per game.
//!
//! Ported from the Python `tools/collect_boards.py` (removed 2026-11-03); a
//! user timer runs it every 15 minutes. The game writes every Battlegrounds
//! game to `Logs/Hearthstone_<date>/Power.log` and deletes old log folders
//! after a few sessions, so the command first copies the log folders into an
//! archive outside the repo (`--no-archive` skips that), then reads every new
//! or grown `Power.log` with [`Tracker`] and writes one file per game:
//!
//!   boards/real/<log folder>-g<N>.json
//!
//! Every fight gives two boards, each marked with `who`: `me` is my board, one
//! per turn, and `opp` the opponent I fought that turn. Player names are not
//! stored. A second machine keeps its own logs and syncs them into another
//! archive; its boards get a `laptop-` prefix.
//!
//!   bgbench collect-boards [--out DIR] [--cards FILE] [--no-archive]
//!                          [--logs DIR] [--archive DIR] [--laptop DIR]
//!
//! `--out` is where the files and the `.state.json` incremental state go
//! (`boards/real` by default). `--logs`, `--archive` and `--laptop` are the
//! client's log folders, the archive and the second machine's archive, for
//! testing with fake folders; they default to folders in the home directory.
//!
//! The state file maps a log folder's name (with the `laptop-` prefix) to the
//! size of the `Power.log` last read from it, so a rerun only reads logs that
//! are new or have grown. Deleting it and running with `--no-archive` rewrites
//! every file (docs/real-boards.md "The file").

use std::collections::{BTreeMap, HashMap};
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::exit;

use serde::Deserializer;

use bglog::{Cards, Fight, Minion, PLAYER_ENCHANTS, Seat, TagValue, Tracker};

/// The `CREATE_GAME` line the collectors feed themselves when a spectator
/// stretch starts or ends: it closes the game in hand without reading any of
/// the spectated state, so the session numbers after it stay the log's own.
/// It carries no clock, so the reader's clock (the files' `time`) stays the
/// log's own too.
pub(crate) const SYNTHETIC_CREATE_GAME: &str =
    "D synthetic GameState.DebugPrintPower() - CREATE_GAME";

/// The line the client prints when a spectated game starts. It names the
/// player watched ("1st player", and so on), so only the start is keyed on.
pub(crate) fn begins_spectating(line: &str) -> bool {
    line.contains("================== Begin Spectating ")
}

/// The line the client prints when spectating ends. `End Spectator Game` is
/// only one game of the watch, not the end of it, so it is not this line.
pub(crate) fn ends_spectating(line: &str) -> bool {
    line.contains("================== End Spectator Mode ==================")
}

/// A game's own `CREATE_GAME`, not the `PowerTaskList` copy printed later.
pub(crate) fn is_create_game(line: &str) -> bool {
    line.contains("GameState.DebugPrintPower() - CREATE_GAME")
}

/// The Python's `sys.argv[1:]` handling: a flag's value, or a default.
pub(crate) fn flag(args: &[String], name: &str, default: &str) -> String {
    args.iter()
        .position(|a| a == name)
        .and_then(|k| args.get(k + 1))
        .cloned()
        .unwrap_or_else(|| default.to_string())
}

/// The home folder, for the paths the command shares with the Python.
pub(crate) fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

/// A path as a flag value.
pub(crate) fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

pub fn run(args: &[String]) {
    let home = home();
    let out = flag(args, "--out", "boards/real");
    let cards = flag(args, "--cards", "ref/hsjson-cards.json");
    let logs = flag(args, "--logs", &text(&home.join("hearthstone-linux/hearthstone/Logs")));
    let archive = flag(args, "--archive", &text(&home.join("bg-logs")));
    let laptop = flag(args, "--laptop", &text(&home.join("bg-logs-laptop")));
    let archiving = !args.iter().any(|a| a == "--no-archive");
    let rewrite = args.iter().any(|a| a == "--rewrite");
    match collect(
        Path::new(&out),
        &cards,
        Path::new(&logs),
        Path::new(&archive),
        Path::new(&laptop),
        archiving,
        rewrite,
    ) {
        Ok(summary) => println!(
            "{} log folders archived, {} logs read, {} skipped, {} games written to {}",
            summary.archived, summary.read, summary.skipped, summary.games, out
        ),
        Err(e) => {
            eprintln!("{e}");
            exit(2);
        }
    }
}

#[derive(Default)]
struct Summary {
    archived: usize,
    read: usize,
    skipped: usize,
    games: usize,
}

/// The whole run: archive, then read every new or grown log into `out`.
fn collect(
    out: &Path,
    cards: &str,
    logs: &Path,
    archive: &Path,
    laptop: &Path,
    archiving: bool,
    rewrite: bool,
) -> Result<Summary, String> {
    let cards = Cards::load(cards)?;
    let mut summary = Summary::default();
    if archiving {
        summary.archived = archive_logs(logs, archive)?;
    }
    fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let state_path = out.join(".state.json");
    let mut state = State::read(&state_path);
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    for path in log_files(archive) {
        files.push((String::new(), path));
    }
    for path in log_files(laptop) {
        files.push(("laptop-".to_string(), path));
    }
    let mut log_entries: Vec<LogEntry> = Vec::with_capacity(files.len());
    for (prefix, path) in files {
        let Some(folder) = path.parent().and_then(|p| p.file_name()) else {
            continue;
        };
        let name = format!("{prefix}{}", folder.to_string_lossy());
        let is_laptop = !prefix.is_empty();
        let size = fs::metadata(&path)
            .map_err(|e| format!("{}: {e}", path.display()))?
            .len();
        if !rewrite && state.get(&name) == Some(size) {
            summary.skipped += 1;
            log_entries.push(LogEntry {
                name,
                is_laptop,
                path,
                sessions: None,
            });
            continue;
        }
        summary.read += 1;
        let games = read_log(&path, &cards)?;
        let mut sessions = Vec::with_capacity(games.len());
        for (gi, game) in games.into_iter().enumerate() {
            let gi = gi + 1;
            sessions.push(game.session_for(&name, gi));
            let Some(text) = game.render(&name, gi) else {
                continue;
            };
            let file = out.join(format!("{name}-g{gi}.json"));
            fs::write(&file, text).map_err(|e| format!("{}: {e}", file.display()))?;
            summary.games += 1;
        }
        log_entries.push(LogEntry {
            name: name.clone(),
            is_laptop,
            path,
            sessions: Some(sessions),
        });
        state.set(name, size);
        let text = state.render();
        fs::write(&state_path, text)
            .map_err(|e| format!("{}: {e}", state_path.display()))?;
    }
    link_collected_files(out, &mut log_entries)?;
    Ok(summary)
}

/// Copy every log folder to the archive; a folder that grew is copied again.
fn archive_logs(logs: &Path, archive: &Path) -> Result<usize, String> {
    fs::create_dir_all(archive).map_err(|e| format!("{}: {e}", archive.display()))?;
    let mut copied = 0;
    for dir in folders(logs) {
        let src = dir.join("Power.log");
        if !src.exists() {
            continue;
        }
        let Some(name) = dir.file_name() else {
            continue;
        };
        let dst = archive.join(name).join("Power.log");
        if let (Ok(done), Ok(now)) = (fs::metadata(&dst), fs::metadata(&src))
            && done.len() == now.len()
        {
            continue;
        }
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        copy2(&src, &dst)?;
        copied += 1;
    }
    Ok(copied)
}

/// `shutil.copy2`: the bytes and the mode, then the source's access and
/// modification times on the copy.
fn copy2(src: &Path, dst: &Path) -> Result<(), String> {
    let fail = |what: &Path, e: std::io::Error| format!("{}: {e}", what.display());
    fs::copy(src, dst).map_err(|e| fail(dst, e))?;
    let meta = fs::metadata(src).map_err(|e| fail(src, e))?;
    let mut times = fs::FileTimes::new();
    if let Ok(time) = meta.modified() {
        times = times.set_modified(time);
    }
    if let Ok(time) = meta.accessed() {
        times = times.set_accessed(time);
    }
    let file = fs::OpenOptions::new()
        .write(true)
        .open(dst)
        .map_err(|e| fail(dst, e))?;
    file.set_times(times).map_err(|e| fail(dst, e))?;
    Ok(())
}

/// The folders under `dir` whose name starts with `Hearthstone_`, sorted. A
/// folder that is not there has none (the Python's `glob` on a missing folder).
fn folders(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_dir()
                && path
                    .file_name()
                    .map(|name| name.to_string_lossy().starts_with("Hearthstone_"))
                    .unwrap_or(false)
        })
        .collect();
    out.sort();
    out
}

/// The `Power.log` of every log folder under `dir`, sorted.
pub(crate) fn log_files(dir: &Path) -> Vec<PathBuf> {
    folders(dir)
        .into_iter()
        .map(|dir| dir.join("Power.log"))
        .filter(|path| path.exists())
        .collect()
}

/// The `.state.json` incremental state: log folder name to the size of the
/// `Power.log` last read from it, in the order the Python wrote it.
#[derive(Default)]
pub(crate) struct State {
    entries: Vec<(String, u64)>,
}

impl State {
    /// The state in `path`, or an empty one when it is missing or unreadable
    /// (the Python's `except (OSError, ValueError): state = {}`).
    pub(crate) fn read(path: &Path) -> State {
        let Ok(text) = fs::read_to_string(path) else {
            return State::default();
        };
        serde_json::Deserializer::from_str(&text)
            .deserialize_map(StateVisitor)
            .unwrap_or_default()
    }

    pub(crate) fn get(&self, name: &str) -> Option<u64> {
        self.entries
            .iter()
            .find(|(have, _)| have == name)
            .map(|(_, size)| *size)
    }

    /// Set a log's size, keeping a name it already knows in its place.
    pub(crate) fn set(&mut self, name: String, size: u64) {
        match self.entries.iter_mut().find(|(have, _)| *have == name) {
            Some(slot) => slot.1 = size,
            None => self.entries.push((name, size)),
        }
    }

    pub(crate) fn render(&self) -> String {
        render(&Json::Obj(
            self.entries
                .iter()
                .map(|(name, size)| (name.clone(), Json::Int(*size as i64)))
                .collect(),
        ))
    }
}

/// Reads the state file's object into name/size pairs, in the file's order.
struct StateVisitor;

impl<'de> serde::de::Visitor<'de> for StateVisitor {
    type Value = State;

    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("a JSON object of log folder names to file sizes")
    }

    fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<State, A::Error> {
        let mut state = State::default();
        while let Some((name, size)) = map.next_entry::<String, u64>()? {
            state.set(name, size);
        }
        Ok(state)
    }
}

/// What identifies one Battlegrounds session for linking split games across
/// reconnects and client restarts.
#[derive(Clone, Debug, Default)]
pub(crate) struct SessionInfo {
    pub(crate) stem: String,
    pub(crate) is_laptop: bool,
    /// The game's `BuildNumber` line, 0 when the session had none.
    pub(crate) build: i64,
    pub(crate) stand_in: Option<(i32, String)>,
    pub(crate) my_hero: Option<(i32, String)>,
    pub(crate) lobby: BTreeMap<i32, String>,
    pub(crate) init_turn: i64,
    pub(crate) max_turn: i64,
    pub(crate) complete: bool,
    pub(crate) final_place: i64,
}

/// One log folder in the collection order, with its sessions once read.
pub(crate) struct LogEntry {
    pub(crate) name: String,
    pub(crate) is_laptop: bool,
    pub(crate) path: PathBuf,
    pub(crate) sessions: Option<Vec<SessionInfo>>,
}

/// Normalize a hero card id across skins (`_SKIN_`) and mid-game hero
/// transformations (`TB_BaconShop_HERO_59t`).
fn hero_base_id(card_id: &str) -> String {
    let base = card_id.split("_SKIN_").next().unwrap_or_default();
    base.strip_suffix('t').unwrap_or(base).to_string()
}

/// Build a session's linking info from its finished `bglog::Game`, its
/// `Tracker` seats, and the `GameEntity` `TURN` tag from its `CREATE_GAME`
/// dump before any `TAG_CHANGE` on `TURN`.
pub(crate) fn session_info(
    stem: String,
    game: &bglog::Game,
    seats: &BTreeMap<i32, Seat>,
    init_turn: i64,
) -> SessionInfo {
    let is_laptop = stem.starts_with("laptop-");
    let stand_in = game
        .player_names
        .iter()
        .find(|(pid, name)| !game.real_players.contains(pid) && !name.is_empty())
        .cloned();
    let me = game.real_players.iter().copied().min();
    let mut my_hero = None;
    let mut lobby = BTreeMap::new();
    for e in game.entities_in_order() {
        if e.text("CARDTYPE") != "HERO"
            || e.card_id.is_empty()
            || e.card_id.starts_with("TB_BaconShop_HERO_PH")
            || e.card_id == "BaconPHhero"
        {
            continue;
        }
        let pid = e.int("PLAYER_ID") as i32;
        if pid <= 0 {
            continue;
        }
        let base = hero_base_id(&e.card_id);
        lobby.entry(pid).or_insert_with(|| base.clone());
        if Some(pid) == me && my_hero.is_none() {
            my_hero = Some((e.id, base));
        }
    }
    let max_turn = game.game_tag("TURN").max(init_turn);
    let complete = game.game_text("STATE") == "COMPLETE";
    let final_place = if complete {
        let won = me
            .and_then(|pid| game.player_entity(pid))
            .is_some_and(|p| p.text("PLAYSTATE") == "WON");
        if won {
            1
        } else {
            me.and_then(|pid| seats.get(&pid)).map_or(0, |s| s.place)
        }
    } else {
        0
    };
    SessionInfo {
        stem,
        is_laptop,
        build: game.build,
        stand_in,
        my_hero,
        lobby,
        init_turn,
        max_turn,
        complete,
        final_place,
    }
}

/// Whether `next` is a reconnect continuation of the same Battlegrounds game
/// as `prev`: the previous session did not see `STATE value=COMPLETE`, the new
/// session opens with a reconnect dump past turn 1 at or after the previous
/// session's last turn, both sessions have the same stand-in player in the
/// `DebugPrintGame()` header and the same hero entity and card for me, and
/// every lobby seat present in both carries the same base hero.
pub(crate) fn continues(prev: &SessionInfo, next: &SessionInfo) -> bool {
    if prev.is_laptop != next.is_laptop || prev.complete {
        return false;
    }
    if next.init_turn <= 1 || next.init_turn < prev.max_turn {
        return false;
    }
    if prev.stand_in.is_none() || prev.stand_in != next.stand_in {
        return false;
    }
    if prev.my_hero != next.my_hero {
        return false;
    }
    for (pid, hero) in &prev.lobby {
        if let Some(other) = next.lobby.get(pid)
            && hero != other
        {
            return false;
        }
    }
    true
}

/// Read `("match", "final_place")` from the header of a collected board or
/// action file without parsing its body. `None` when either key is absent.
pub(crate) fn read_file_link(path: &Path) -> Option<(String, i64)> {
    let file = File::open(path).ok()?;
    let reader = BufReader::new(file);
    let mut match_stem = None;
    let mut final_place = None;
    for line in reader.lines().take(16) {
        let line = line.ok()?;
        let trimmed = line.trim();
        if trimmed.starts_with("\"boards\":") || trimmed.starts_with("\"turns\":") {
            break;
        }
        if let Some(rest) = trimmed.strip_prefix("\"match\": ") {
            let raw = rest.strip_suffix(',').unwrap_or(rest);
            match_stem = serde_json::from_str::<String>(raw).ok();
        } else if let Some(rest) = trimmed.strip_prefix("\"final_place\": ") {
            let raw = rest.strip_suffix(',').unwrap_or(rest);
            final_place = raw.parse::<i64>().ok();
        }
    }
    match_stem.zip(final_place)
}

/// Set or replace `"match"` and `"final_place"` in a rendered board or action
/// JSON text, leaving every other key and value byte-for-byte unchanged.
pub(crate) fn set_file_link(text: &str, match_stem: &str, final_place: i64) -> Option<String> {
    let split_at = text
        .find(",\n \"boards\": ")
        .or_else(|| text.find(",\n \"turns\": "))?;
    let head = &text[..split_at];
    let tail = &text[split_at..];
    let head = match head.find(",\n \"match\": ") {
        Some(at) => &head[..at],
        None => head,
    };
    let match_json = render(&Json::Str(match_stem.to_string()));
    Some(format!(
        "{head},\n \"match\": {match_json},\n \"final_place\": {final_place}{tail}"
    ))
}

fn stem_belongs_to_log(stem: &str, log_name: &str) -> bool {
    let Some(rest) = stem.strip_prefix(log_name) else {
        return false;
    };
    let Some(digits) = rest.strip_prefix("-g") else {
        return false;
    };
    !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
}

fn scan_log_sessions(path: &Path, name: &str) -> Result<Vec<SessionInfo>, String> {
    let games = read_log(path, &Cards::default())?;
    Ok(games
        .into_iter()
        .enumerate()
        .map(|(idx, g)| g.session_for(name, idx + 1))
        .collect())
}

/// Link all collected files in `out` across sessions and log folders, writing
/// `"match"` and `"final_place"` into any file whose link changed or was not
/// yet known when the file was first written.
pub(crate) fn link_collected_files(
    out: &Path,
    log_entries: &mut [LogEntry],
) -> Result<(), String> {
    let entries = fs::read_dir(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let mut existing: BTreeMap<String, Option<(String, i64)>> = BTreeMap::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("{}: {e}", out.display()))?;
        let file_name = entry.file_name();
        let file_name = file_name.to_string_lossy();
        if file_name.starts_with('.') || !file_name.ends_with(".json") {
            continue;
        }
        let stem = file_name.trim_end_matches(".json").to_string();
        let link = read_file_link(&entry.path());
        existing.insert(stem, link);
    }
    if existing.is_empty() {
        return Ok(());
    }

    for entry in log_entries.iter_mut() {
        if entry.sessions.is_some() {
            continue;
        }
        let needs_scan = existing.iter().any(|(stem, link)| {
            stem_belongs_to_log(stem, &entry.name)
                && match link {
                    None => true,
                    Some((m, fp)) => *fp == 0 || m != stem,
                }
        });
        if needs_scan {
            entry.sessions = Some(scan_log_sessions(&entry.path, &entry.name)?);
        }
    }

    loop {
        let mut changed = false;
        for i in 1..log_entries.len() {
            if log_entries[i].is_laptop != log_entries[i - 1].is_laptop {
                continue;
            }
            let back = log_entries[i - 1].sessions.is_none()
                && log_entries[i]
                    .sessions
                    .as_ref()
                    .is_some_and(|s| s.iter().any(|x| x.init_turn > 1));
            if back {
                let scanned =
                    scan_log_sessions(&log_entries[i - 1].path, &log_entries[i - 1].name)?;
                log_entries[i - 1].sessions = Some(scanned);
                changed = true;
            }
            let fwd = log_entries[i].sessions.is_none()
                && log_entries[i - 1]
                    .sessions
                    .as_ref()
                    .is_some_and(|s| s.last().is_some_and(|x| !x.complete));
            if fwd {
                let scanned = scan_log_sessions(&log_entries[i].path, &log_entries[i].name)?;
                log_entries[i].sessions = Some(scanned);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    let mut groups: Vec<Vec<SessionInfo>> = Vec::new();
    let mut chain_open = false;
    let mut prev_laptop = false;
    for entry in log_entries.iter() {
        let Some(sessions) = &entry.sessions else {
            chain_open = false;
            continue;
        };
        if entry.is_laptop != prev_laptop {
            chain_open = false;
            prev_laptop = entry.is_laptop;
        }
        for s in sessions {
            if !existing.contains_key(&s.stem) {
                continue;
            }
            if chain_open
                && let Some(group) = groups.last_mut()
                && let Some(prev) = group.last()
                && continues(prev, s)
            {
                group.push(s.clone());
                continue;
            }
            groups.push(vec![s.clone()]);
            chain_open = true;
        }
    }

    for group in &groups {
        let match_stem = &group[0].stem;
        let final_place = group.last().map_or(0, |s| s.final_place);
        for s in group {
            if existing.get(&s.stem) == Some(&Some((match_stem.clone(), final_place))) {
                continue;
            }
            let path = out.join(format!("{}.json", s.stem));
            let text =
                fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let updated = set_file_link(&text, match_stem, final_place)
                .ok_or_else(|| format!("{}: unexpected JSON layout", path.display()))?;
            if updated != text {
                fs::write(&path, updated).map_err(|e| format!("{}: {e}", path.display()))?;
            }
        }
    }
    Ok(())
}

/// One game of a log: its fights with the seats as they were when each was
/// staged, and the lobby as it was when the game ended.
#[derive(Default)]
struct Game {
    fights: Vec<(Fight, SeatInfo, SeatInfo)>,
    seats: BTreeMap<i32, Seat>,
    /// My PlayerID as the last game of the log had it (the Python reads
    /// `place` off the lobby of the game being read, for every game of the
    /// log).
    me: Option<i32>,
    session: SessionInfo,
    match_gi: usize,
    final_place: i64,
}

/// A seat as the file holds it (the Python's `seat_info`).
#[derive(Clone, Default)]
struct SeatInfo {
    hero: String,
    hero_card: String,
    hero_base: String,
    tier: i64,
    health: i64,
    armor: i64,
}

impl SeatInfo {
    fn of(seat: Option<&Seat>) -> SeatInfo {
        match seat {
            None => SeatInfo::default(),
            Some(seat) => SeatInfo {
                hero: seat.hero.clone(),
                // A hero card can be a skin (BG23_HERO_303_SKIN_D); the base is
                // the hero.
                hero_base: seat
                    .hero_card
                    .split("_SKIN_")
                    .next()
                    .unwrap_or_default()
                    .to_string(),
                hero_card: seat.hero_card.clone(),
                tier: seat.tier,
                health: seat.health,
                armor: seat.armor,
            },
        }
    }
}

/// Every Battlegrounds game of a log, in order: its fights and its lobby.
///
/// The Python's `read_log`: the fights of a game are taken as they are staged
/// (their seats as they stand then), and the game is closed when the next one
/// starts, or at the end of the log.
fn read_log(path: &Path, cards: &Cards) -> Result<Vec<Game>, String> {
    let file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut reader = BufReader::with_capacity(1 << 20, file);
    read_log_reader(&mut reader, cards).map_err(|e| format!("{}: {e}", path.display()))
}

/// The same reader over any stream, so a test can feed a trimmed log.
fn read_log_reader(reader: &mut impl BufRead, cards: &Cards) -> io::Result<Vec<Game>> {
    let mut tracker = Tracker::new(cards.clone());
    let mut games: Vec<Game> = Vec::new();
    let mut current: Vec<(usize, SeatInfo, SeatInfo)> = Vec::new();
    let mut seen_past = 0;
    let mut n_fights = 0;
    let mut line = String::new();
    let mut spectating = false;
    let mut spectator_games = 0;
    let mut last_me = None;
    let mut init_turn = None;
    let mut ended_game = None;
    loop {
        line.clear();
        let read = reader.read_line(&mut line)?;
        if read == 0 {
            break;
        }
        if spectating {
            if ends_spectating(&line) {
                // The spectated session is skipped but keeps its number: later
                // games must still be g3, g4 and so on. A synthetic CREATE_GAME
                // closes the empty game the start left behind.
                spectating = false;
                init_turn = None;
                tracker.feed(SYNTHETIC_CREATE_GAME);
                for _ in 0..spectator_games {
                    games.push(Game::default());
                }
                spectator_games = 0;
            } else if is_create_game(&line) {
                spectator_games += 1;
            }
        } else if begins_spectating(&line) {
            spectating = true;
            if tracker.is_battlegrounds() {
                ended_game = tracker.game().cloned().map(|g| {
                    let init_t = init_turn.unwrap_or_else(|| g.game_tag("TURN"));
                    (g, init_t)
                });
            }
            init_turn = None;
            // Close the game in hand at the marker, as its next CREATE_GAME in
            // the spectator block would.
            tracker.feed(SYNTHETIC_CREATE_GAME);
        } else {
            if is_create_game(&line) {
                if tracker.is_battlegrounds() {
                    ended_game = tracker.game().cloned().map(|g| {
                        let init_t = init_turn.unwrap_or_else(|| g.game_tag("TURN"));
                        (g, init_t)
                    });
                }
                init_turn = None;
            } else if init_turn.is_none()
                && line.contains("TAG_CHANGE")
                && line.contains("tag=TURN ")
            {
                init_turn = Some(tracker.game().map_or(0, |g| g.game_tag("TURN")));
            }
            tracker.feed(&line);
            if let Some(me) = tracker.me() {
                last_me = Some(me);
            }
        }
        if tracker.past().len() > seen_past {
            // A game ended: its fights are done.
            seen_past = tracker.past().len();
            let (seats, fights) = &tracker.past()[seen_past - 1];
            let session = match ended_game.take() {
                Some((g, init_t)) => session_info(String::new(), &g, seats, init_t),
                None => SessionInfo::default(),
            };
            let final_place = session.final_place;
            games.push(Game {
                fights: current
                    .drain(..)
                    .map(|(index, mine, theirs)| (fights[index].clone(), mine, theirs))
                    .collect(),
                seats: seats.clone(),
                me: last_me,
                session,
                match_gi: 0,
                final_place,
            });
            n_fights = 0;
        }
        if tracker.fights().len() > n_fights {
            n_fights = tracker.fights().len();
            let fight = &tracker.fights()[n_fights - 1];
            // The seats as they are at the first attack: tier, health, armor.
            let mine = SeatInfo::of(tracker.me().and_then(|me| tracker.seats().get(&me)));
            let theirs = SeatInfo::of(tracker.seats().get(&fight.opponent));
            current.push((n_fights - 1, mine, theirs));
        }
    }
    if spectating {
        // The log ended during the watch: skip the unfinished session too.
        tracker.feed(SYNTHETIC_CREATE_GAME);
        for _ in 0..spectator_games {
            games.push(Game::default());
        }
    }
    if !current.is_empty() && tracker.is_battlegrounds() {
        tracker.fill_missing_seats();
        let session = match tracker.game() {
            Some(g) => {
                let init_t = init_turn.unwrap_or_else(|| g.game_tag("TURN"));
                session_info(String::new(), g, tracker.seats(), init_t)
            }
            None => SessionInfo::default(),
        };
        let final_place = session.final_place;
        let fights = tracker.fights();
        let fights = current
            .drain(..)
            .map(|(index, mine, theirs)| (fights[index].clone(), mine, theirs))
            .collect();
        games.push(Game {
            fights,
            seats: tracker.seats().clone(),
            me: last_me,
            session,
            match_gi: 0,
            final_place,
        });
    }
    // Each game reads `place` off its own seat: my PlayerID changes from game
    // to game in one log (6, 3 and 5 in Hearthstone_2026_11_20_15_06_37), so
    // the last game's PlayerID read another player's place for the others.
    // The Python did that, and so did this port until 2026-11-22.
    link_games_in_log(&mut games);
    Ok(games)
}

fn link_games_in_log(games: &mut [Game]) {
    let collected: Vec<usize> = games
        .iter()
        .enumerate()
        .filter_map(|(i, g)| g.has_finished_fight().then_some(i))
        .collect();
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for idx in collected {
        if let Some(group) = groups.last_mut()
            && let Some(&prev) = group.last()
            && continues(&games[prev].session, &games[idx].session)
        {
            group.push(idx);
            continue;
        }
        groups.push(vec![idx]);
    }
    for group in groups {
        let match_gi = group[0] + 1;
        let final_place = group
            .last()
            .map_or(0, |&last| games[last].session.final_place);
        for idx in group {
            games[idx].match_gi = match_gi;
            games[idx].final_place = final_place;
        }
    }
}

impl Game {
    fn has_finished_fight(&self) -> bool {
        self.fights.iter().any(|(f, _, _)| !f.result.is_empty())
    }

    fn session_for(&self, source: &str, gi: usize) -> SessionInfo {
        let mut s = self.session.clone();
        s.stem = format!("{source}-g{gi}");
        s.is_laptop = source.starts_with("laptop-");
        s
    }

    /// The JSON of one game, `{source, game, place, match, final_place, boards}`,
    /// ready to write. `None` for a game with no finished fight: the Python
    /// writes no file for it.
    fn render(&self, source: &str, gi: usize) -> Option<String> {
        let mut boards = Vec::new();
        for (fight, mine, theirs) in &self.fights {
            if fight.result.is_empty() {
                continue;
            }
            for (index, seat, who, result) in [
                (0, mine, "me", fight.result.clone()),
                (1, theirs, "opp", flip(&fight.result).to_string()),
            ] {
                let mut fields: Vec<(String, Json)> = vec![
                    ("who".to_string(), Json::Str(who.to_string())),
                    ("turn".to_string(), Json::Int(fight.turn)),
                    ("result".to_string(), Json::Str(result)),
                ];
                // When the fight was staged, local time (bglog::local_time).
                let time = bglog::local_time(source, &fight.clock, fight.days);
                if !time.is_empty() {
                    fields.push(("time".to_string(), Json::Str(time)));
                }
                fields.extend(side(fight, index, seat));
                boards.push(Json::Obj(fields));
            }
        }
        if boards.is_empty() {
            return None;
        }
        let place = self.place();
        let match_gi = if self.match_gi > 0 { self.match_gi } else { gi };
        let match_stem = format!("{source}-g{match_gi}");
        Some(render(&Json::Obj(vec![
            ("source".to_string(), Json::Str(source.to_string())),
            ("game".to_string(), Json::Int(gi as i64)),
            ("build".to_string(), Json::Int(self.session.build)),
            ("place".to_string(), Json::Int(place)),
            ("match".to_string(), Json::Str(match_stem)),
            ("final_place".to_string(), Json::Int(self.final_place)),
            ("boards".to_string(), Json::Arr(boards)),
        ])))
    }

    /// My place in the game: read off my seat in the lobby as the game ended.
    fn place(&self) -> i64 {
        self.me
            .and_then(|me| self.seats.get(&me))
            .map_or(0, |seat| seat.place)
    }
}

/// The Python's `FLIP`: the opponent's result for my result.
fn flip(result: &str) -> &'static str {
    match result {
        "win" => "lose",
        "lose" => "win",
        "tie" => "tie",
        _ => "",
    }
}

/// One side's board as the file holds it: the seat's hero, tier, health and
/// armor, its hero power, trinkets and rewards, its minions, its Secrets, and
/// what the log shows of the side itself at the first attack.
fn side(fight: &Fight, index: usize, seat: &SeatInfo) -> Vec<(String, Json)> {
    let mut fields: Vec<(String, Json)> = vec![
        ("hero".to_string(), Json::Str(seat.hero.clone())),
        ("hero_card".to_string(), Json::Str(seat.hero_card.clone())),
        ("hero_base".to_string(), Json::Str(seat.hero_base.clone())),
        ("tier".to_string(), Json::Int(seat.tier)),
        ("health".to_string(), Json::Int(seat.health)),
        ("armor".to_string(), Json::Int(seat.armor)),
    ];
    fields.extend(extras(fight, index));
    let marks = if index == 0 {
        &fight.reanimates.0
    } else {
        &fight.reanimates.1
    };
    let board = if index == 0 { &fight.mine } else { &fight.theirs };
    fields.push(("minions".to_string(), minions(board, marks)));
    let secrets = if index == 0 { &fight.secrets.0 } else { &fight.secrets.1 };
    fields.push((
        "secrets".to_string(),
        Json::Arr(secrets.iter().map(|s| Json::Str(s.clone())).collect()),
    ));
    if fight.at_attack.is_some() {
        let hand = if index == 0 { &fight.hand } else { &fight.their_hand };
        fields.push(("counters".to_string(), counters(fight, index)));
        fields.push(("deity".to_string(), deity(fight, index)));
        fields.push((
            "hand".to_string(),
            minions(hand.as_deref().unwrap_or_default(), &[]),
        ));
        let hidden = if index == 0 { fight.hidden.0 } else { fight.hidden.1 };
        fields.push(("hand_hidden".to_string(), Json::Int(hidden)));
    }
    fields
}

/// What a side had at the first attack besides its board: hero power, trinkets
/// and quest rewards. The hero power also carries the minion Lock and Load
/// still owes the board, as `waiting` (null when it owes none, which is also
/// every power that is not Lock and Load).
fn extras(fight: &Fight, index: usize) -> Vec<(String, Json)> {
    let hp = if index == 0 { &fight.hero_powers.0 } else { &fight.hero_powers.1 };
    let waiting = if index == 0 { &fight.waiting.0 } else { &fight.waiting.1 };
    let hero_power = match hp {
        None => Json::Null,
        Some(hp) => Json::Obj(vec![
            ("card_id".to_string(), Json::Str(hp.card_id.clone())),
            ("name".to_string(), Json::Str(hp.name.clone())),
            ("numbers".to_string(), numbers_json(&hp.numbers)),
            (
                "waiting".to_string(),
                waiting.as_ref().map_or(Json::Null, |m| minion_json(m, false)),
            ),
        ]),
    };
    let trinkets = if index == 0 { &fight.trinkets.0 } else { &fight.trinkets.1 };
    let rewards = if index == 0 { &fight.rewards.0 } else { &fight.rewards.1 };
    vec![
        ("hero_power".to_string(), hero_power),
        (
            "trinkets".to_string(),
            Json::Arr(
                trinkets
                    .iter()
                    .map(|t| {
                        Json::Obj(vec![
                            ("card_id".to_string(), Json::Str(t.card_id.clone())),
                            ("name".to_string(), Json::Str(t.name.clone())),
                            ("greater".to_string(), Json::Bool(t.greater)),
                            ("numbers".to_string(), numbers_json(&t.numbers)),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "rewards".to_string(),
            Json::Arr(
                rewards
                    .iter()
                    .map(|r| {
                        Json::Obj(vec![
                            ("card_id".to_string(), Json::Str(r.card_id.clone())),
                            ("name".to_string(), Json::Str(r.name.clone())),
                            ("numbers".to_string(), numbers_json(&r.numbers)),
                        ])
                    })
                    .collect(),
            ),
        ),
    ]
}

/// A side's player-level numbers at the first attack, the keys the file holds.
/// A number the log does not show is left out: the loader then uses the base
/// value (0 spells cast, no spell bonus, Blood Gems +1/+1).
fn counters(fight: &Fight, index: usize) -> Json {
    let side = if index == 0 { &fight.side_numbers.0 } else { &fight.side_numbers.1 };
    let mut fields: Vec<(String, Json)> = Vec::new();
    let mut push = |key: &str, value: i64| {
        fields.push((key.to_string(), Json::Int(value)));
    };
    for key in ["spell_attack", "spell_health"] {
        if let Some(&value) = side.get(key) {
            push(key, value);
        }
    }
    for (_, attack_key, health_key) in PLAYER_ENCHANTS {
        for key in [attack_key, health_key].into_iter().flatten() {
            if let Some(&value) = side.get(key) {
                push(key, value);
            }
        }
    }
    let upper = if index == 0 { fight.upper_hands.0 } else { fight.upper_hands.1 };
    if upper != 0 {
        push("upper_hand", upper);
    }
    // Blood Gems (BACON_BLOODGEMBUFFATKVALUE / _HEALTHVALUE on the player).
    let (gem_attack, gem_health) = if index == 0 { fight.gems.0 } else { fight.gems.1 };
    if gem_attack != 1 || gem_health != 1 {
        push("gem_attack", gem_attack);
        push("gem_health", gem_health);
    }
    // The tracker's counts(): spells cast (mine only: the stand-in player
    // carries none) and Eternal Knight deaths.
    let counts = if index == 0 { &fight.counters.0 } else { &fight.counters.1 };
    if let Some(&value) = counts.get("spells")
        && value != 0
    {
        push("spells_cast", value);
    }
    if let Some(&value) = counts.get("knights")
        && value != 0
    {
        push("knight_deaths", value);
    }
    Json::Obj(fields)
}

/// The side's Deity at the first attack: the minion it awakens and the stats it
/// has now.
fn deity(fight: &Fight, index: usize) -> Json {
    let d = if index == 0 { &fight.deities.0 } else { &fight.deities.1 };
    match d {
        None => Json::Null,
        Some(d) => Json::Obj(vec![
            ("card_id".to_string(), Json::Str(d.card_id.clone())),
            ("name".to_string(), Json::Str(d.name.clone())),
            ("attack".to_string(), Json::Int(d.attack)),
            ("health".to_string(), Json::Int(d.health)),
        ]),
    }
}

/// The four `TAG_SCRIPT_DATA_NUM_1..4` slots of a minion, hero power, trinket
/// or reward, as a JSON array.
pub(crate) fn numbers_json(numbers: &[i64; 4]) -> Json {
    Json::Arr(numbers.iter().map(|n| Json::Int(*n)).collect())
}

/// The `{0} {1} {2} {3}` of a minion's text as the file holds it, or `None`
/// when it has none.
pub(crate) fn minion_numbers(tags: &HashMap<String, TagValue>) -> Option<[i64; 4]> {
    let mut sd = [0; 4];
    for (i, slot) in sd.iter_mut().enumerate() {
        *slot = tags
            .get(&format!("TAG_SCRIPT_DATA_NUM_{}", i + 1))
            .map(|value| value.as_int())
            .unwrap_or(0);
    }
    if sd.iter().any(|n| *n != 0) { Some(sd) } else { None }
}

/// A board as the file holds it, with Rapid Reanimation's marks by position.
fn minions(board: &[Minion], marks: &[bool]) -> Json {
    Json::Arr(
        board
            .iter()
            .enumerate()
            .map(|(i, m)| minion_json(m, marks.get(i).copied().unwrap_or(false)))
            .collect(),
    )
}

fn minion_json(m: &Minion, reanimate: bool) -> Json {
    let mut fields: Vec<(String, Json)> = vec![
        ("card_id".to_string(), Json::Str(m.card_id.clone())),
        ("name".to_string(), Json::Str(m.name.clone())),
        ("attack".to_string(), Json::Int(m.attack)),
        ("health".to_string(), Json::Int(m.health)),
        ("golden".to_string(), Json::Bool(m.golden)),
        (
            "keywords".to_string(),
            Json::Arr(
                m.keywords
                    .iter()
                    .map(|k| Json::Str(k.to_string()))
                    .collect(),
            ),
        ),
        (
            "dark_gifts".to_string(),
            Json::Arr(m.dark_gifts.iter().map(|g| Json::Str(g.clone())).collect()),
        ),
    ];
    if let Some(sd) = minion_numbers(&m.tags) {
        fields.push(("numbers".to_string(), numbers_json(&sd)));
    }
    // Holy Vanguard's "+30/+30 if you have 15 or less Health" is in the card's
    // logged ATK/HEALTH wherever it is, and the log keeps it on the minion's
    // `Vanguard's Valor` enchantment: the file writes the pair beside the
    // stats, and the loader records it on `Minion::vanguard_aura` so a later
    // Health crossing can take it off exactly once. Written only when the
    // minion has one; a file from before the key existed reads as no aura,
    // and its stats keep what the log counted in them.
    if let Some((attack, health)) = m.vanguard_aura {
        fields.push((
            "vanguard_aura".to_string(),
            Json::Arr(vec![Json::Int(attack), Json::Int(health)]),
        ));
    }
    if reanimate {
        fields.push(("reanimate".to_string(), Json::Bool(true)));
    }
    Json::Obj(fields)
}

/// A JSON value whose object keys keep the order they were added in, so the
/// files read as the Python's do (and a diff of the two runs shows only what
/// differs).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) enum Json {
    #[default]
    Null,
    Bool(bool),
    Int(i64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

/// `json.dumps(value, indent=1)`, the Python's own text for the same data:
/// one space of indent per level, `": "` after a key, `,` and a newline between
/// items, `{}` and `[]` for empty ones, and every non-ASCII byte escaped.
pub(crate) fn render(value: &Json) -> String {
    let mut out = String::new();
    write_json(&mut out, value, 0);
    out
}

fn write_json(out: &mut String, value: &Json, depth: usize) {
    match value {
        Json::Null => out.push_str("null"),
        Json::Bool(true) => out.push_str("true"),
        Json::Bool(false) => out.push_str("false"),
        Json::Int(n) => out.push_str(&n.to_string()),
        Json::Str(s) => write_string(out, s),
        Json::Arr(items) => {
            if items.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push('\n');
                write_indent(out, depth + 1);
                write_json(out, item, depth + 1);
            }
            out.push('\n');
            write_indent(out, depth);
            out.push(']');
        }
        Json::Obj(fields) => {
            if fields.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push('{');
            for (i, (key, item)) in fields.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push('\n');
                write_indent(out, depth + 1);
                write_string(out, key);
                out.push_str(": ");
                write_json(out, item, depth + 1);
            }
            out.push('\n');
            write_indent(out, depth);
            out.push('}');
        }
    }
}

fn write_indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push(' ');
    }
}

/// A JSON string, escaped as `json.dumps` escapes one (ensure_ascii on).
fn write_string(out: &mut String, text: &str) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c if (' '..='~').contains(&c) => out.push(c),
            c if (c as u32) < 0x10000 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => {
                let n = c as u32 - 0x10000;
                let high = 0xd800 + (n >> 10);
                let low = 0xdc00 + (n & 0x3ff);
                out.push_str(&format!("\\u{high:04x}\\u{low:04x}"));
            }
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_text_is_the_python_json_dumps_indent_1() {
        // `python3 -c 'json.dumps({"a": 1, "b": [], "c": "N\'Zoth \"x\" \\\\ é",
        // "d": None, "e": {"f": [1, 2]}}, indent=1)'`
        let value = Json::Obj(vec![
            ("a".to_string(), Json::Int(1)),
            ("b".to_string(), Json::Arr(Vec::new())),
            ("c".to_string(), Json::Str("N'Zoth \"x\" \\ é".to_string())),
            ("d".to_string(), Json::Null),
            (
                "e".to_string(),
                Json::Obj(vec![(
                    "f".to_string(),
                    Json::Arr(vec![Json::Int(1), Json::Int(2)]),
                )]),
            ),
        ]);
        assert_eq!(
            render(&value),
            "{\n \"a\": 1,\n \"b\": [],\n \"c\": \"N'Zoth \\\"x\\\" \\\\ \\u00e9\",\n \
             \"d\": null,\n \"e\": {\n  \"f\": [\n   1,\n   2\n  ]\n }\n}"
        );
    }

    /// `crates/bglog/tests/data/fight.log`, a trimmed real log, with the turn
    /// moved into the full-state dump a reconnect prints, instead of the
    /// `TURN` tag change a normal turn starts with. The fight that follows is
    /// still staged, so its board is written, not lost or moved to another
    /// file.
    fn reconnect_excerpt() -> String {
        let text = include_str!("../../bglog/tests/data/fight.log");
        let text = text.replacen(
            "D 00:57:53.6478530 GameState.DebugPrintPower() -     GameEntity EntityID=1\n",
            "D 00:57:53.6478530 GameState.DebugPrintPower() -     GameEntity EntityID=1\n\
             D 00:57:53.6478530 GameState.DebugPrintPower() -         tag=TURN value=3\n",
            1,
        );
        text.replacen(
            "D 00:58:25.1783940 GameState.DebugPrintPower() -     TAG_CHANGE Entity=GameEntity tag=TURN value=1 \n",
            "",
            1,
        )
    }

    #[test]
    fn a_mid_turn_reconnect_keeps_its_board() {
        let cards = Cards::from_pairs(Vec::<(String, String)>::new());
        let mut reader = std::io::Cursor::new(reconnect_excerpt());
        let games = read_log_reader(&mut reader, &cards).unwrap();
        assert_eq!(games.len(), 1, "one game");
        assert_eq!(games[0].fights.len(), 1, "its fight is there");
        assert_eq!(games[0].fights[0].0.turn, 2, "the dump's TURN names it");
        assert!(!games[0].fights[0].0.mine.is_empty(), "my board is recorded");
        assert!(games[0].render("test", 1).is_some(), "and written");
    }

    /// The same trimmed log with the game step already past `MAIN_START`
    /// (at `MAIN_ACTION`) when the setup block stages the fight, as in 7 of
    /// the archived games (bg-vision's notes, 2026-11-20). The fight is
    /// staged when its setup block closes, not lost.
    #[test]
    fn a_fight_staged_after_main_action_is_kept() {
        let text = include_str!("../../bglog/tests/data/fight.log").replacen(
            "D 00:59:12.6839750 GameState.DebugPrintPower() - TAG_CHANGE Entity=GameEntity tag=STEP value=MAIN_START \n",
            "D 00:59:12.6839750 GameState.DebugPrintPower() - TAG_CHANGE Entity=GameEntity tag=STEP value=MAIN_ACTION \n",
            1,
        );
        assert!(!text.contains("tag=STEP value=MAIN_START "), "the excerpt changed");
        let cards = Cards::from_pairs(Vec::<(String, String)>::new());
        let mut reader = std::io::Cursor::new(text);
        let games = read_log_reader(&mut reader, &cards).unwrap();
        assert_eq!(games[0].fights.len(), 1, "the fight is there");
        assert!(!games[0].fights[0].0.mine.is_empty(), "with my board");
    }

    /// Each board carries the local time its fight was staged: the log's
    /// clock on the date of the log's folder.
    #[test]
    fn a_board_carries_the_time_its_fight_was_staged() {
        let cards = Cards::from_pairs(Vec::<(String, String)>::new());
        let mut reader = std::io::Cursor::new(include_str!("../../bglog/tests/data/fight.log"));
        let games = read_log_reader(&mut reader, &cards).unwrap();
        let text = games[0].render("Hearthstone_2026_11_17_00_57_00", 1).unwrap();
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        let time = json["boards"][0]["time"].as_str().unwrap();
        assert!(time.starts_with("2026-11-17T00:59:12."), "{time}");
        assert_eq!(json["boards"][1]["time"], json["boards"][0]["time"], "both sides of one fight");
    }

    /// Every board file carries its game's own `BuildNumber`: 254477 in the
    /// trimmed log, and 0 when the session has no such line.
    #[test]
    fn a_board_file_carries_the_games_build() {
        let cards = Cards::from_pairs(Vec::<(String, String)>::new());
        let text = include_str!("../../bglog/tests/data/fight.log");
        let mut reader = std::io::Cursor::new(text);
        let games = read_log_reader(&mut reader, &cards).unwrap();
        let json: serde_json::Value =
            serde_json::from_str(&games[0].render("test", 1).unwrap()).unwrap();
        assert_eq!(json["build"], 254477);

        let no_build = text.replacen(
            "D 00:57:53.6478530 GameState.DebugPrintGame() - BuildNumber=254477\n",
            "",
            1,
        );
        let mut reader = std::io::Cursor::new(no_build);
        let games = read_log_reader(&mut reader, &cards).unwrap();
        let json: serde_json::Value =
            serde_json::from_str(&games[0].render("test", 1).unwrap()).unwrap();
        assert_eq!(json["build"], 0);
    }

    /// The side-wide numbers are read as the fight is staged, before Start
    /// of Combat, not at the first attack: the engine runs Start of Combat
    /// itself (bg-overlay's Beetle bonus case, 2026_11_11_02_43_24 game 1
    /// turn 9). The trimmed log gets my Tavern spell count set before the
    /// fight is staged and raised again before the first attack.
    #[test]
    fn the_counters_are_read_before_start_of_combat() {
        let text = include_str!("../../bglog/tests/data/fight.log");
        let staged = "D 00:59:12.6839750 GameState.DebugPrintPower() - TAG_CHANGE Entity=GameEntity tag=STEP value=MAIN_START \n";
        let attack = "D 00:59:12.8785010 GameState.DebugPrintPower() - TAG_CHANGE Entity=GameEntity tag=STEP value=MAIN_ACTION \n";
        assert_eq!((text.matches(staged).count(), text.matches(attack).count()), (1, 1));
        let set = |n: u32| format!("D 00:59:12.6839750 GameState.DebugPrintPower() - TAG_CHANGE Entity=Alice#1919 tag=3088 value={n} \n");
        let text = text
            .replacen(staged, &format!("{}{staged}", set(5)), 1)
            .replacen(attack, &format!("{attack}{}", set(9)), 1);
        let cards = Cards::from_pairs(Vec::<(String, String)>::new());
        let mut reader = std::io::Cursor::new(text);
        let games = read_log_reader(&mut reader, &cards).unwrap();
        let fight = &games[0].fights[0].0;
        assert!(fight.at_attack.is_some(), "the first attack was read");
        assert_eq!(fight.counters.0.get("spells"), Some(&5));
    }

    /// A session that starts with a reconnect's full-state dump has my hero
    /// and its place in the dump only, and the place may never change again
    /// (the last session of Hearthstone_2026_11_22_03_22_48, which I won,
    /// wrote place 0). The place is read off the hero as the log ends.
    #[test]
    fn a_rejoined_session_keeps_my_place_from_the_dump() {
        let dump_end = "D 00:57:53.6478530 GameState.DebugPrintGame() - BuildNumber=254477\n";
        let text = include_str!("../../bglog/tests/data/fight.log");
        assert_eq!(text.matches(dump_end).count(), 1);
        let p = "D 00:57:53.6478530 GameState.DebugPrintPower() - ";
        let hero = format!(
            "{p}    FULL_ENTITY - Creating ID=118 CardID=TB_BaconShop_HERO_15\n\
             {p}        tag=CONTROLLER value=1\n\
             {p}        tag=CARDTYPE value=HERO\n\
             {p}        tag=PLAYER_ID value=1\n\
             {p}        tag=HEALTH value=30\n\
             {p}        tag=PLAYER_LEADERBOARD_PLACE value=1\n"
        );
        let text = text.replacen(dump_end, &format!("{hero}{dump_end}"), 1);
        let cards = Cards::from_pairs(Vec::<(String, String)>::new());
        let mut reader = std::io::Cursor::new(text);
        let games = read_log_reader(&mut reader, &cards).unwrap();
        let json: serde_json::Value =
            serde_json::from_str(&games[0].render("test", 1).unwrap()).unwrap();
        assert_eq!(json["place"], 1);
    }

    /// My PlayerID changes from game to game in one log, and each game reads
    /// its place through its own: here 7 in the first game and 1 in the second
    /// (Hearthstone_2026_11_20_15_06_37 had 6, 3 and 5, and the first game's
    /// file wrote another player's place 1 for my 7th).
    #[test]
    fn each_game_reads_its_place_through_its_own_player_id() {
        let cards = Cards::from_pairs(Vec::<(String, String)>::new());
        // The excerpt has no GameType line. The log ends inside game 2, as a
        // log does while the next game is played: game 1 is the one written.
        let bg = "D 18:00:39.2695330 GameState.DebugPrintGame() - GameType=GT_BATTLEGROUNDS\n";
        let text = include_str!("../../bglog/tests/data/two_games.log")
            .replace("PlayerName=Eve\n", &format!("PlayerName=Eve\n{bg}"))
            .replace("PlayerName=Carol\n", &format!("PlayerName=Carol\n{bg}"));
        let mut reader = std::io::Cursor::new(text);
        let games = read_log_reader(&mut reader, &cards).unwrap();
        let mes: Vec<Option<i32>> = games.iter().map(|g| g.me).collect();
        assert_eq!(mes, [Some(7)], "game 1 keeps its own PlayerID, not game 2's 1");
    }

    /// A whole trimmed real game between the spectator markers is another
    /// player's game: it writes no board file, and still takes its session
    /// number so later games keep theirs.
    #[test]
    fn a_spectated_stretch_writes_no_board_file() {
        let text = format!(
            "D 00:00:00.0000000 ================== Begin Spectating 1st player ==================\n\
             {}D 00:00:00.0000000 ================== End Spectator Mode ==================\n",
            include_str!("../../bglog/tests/data/fight.log")
        );
        let cards = Cards::from_pairs(Vec::<(String, String)>::new());
        let mut reader = std::io::Cursor::new(text);
        let games = read_log_reader(&mut reader, &cards).unwrap();
        assert_eq!(games.len(), 1, "the spectated session keeps its number");
        assert!(
            games[0].render("test", 1).is_none(),
            "and has no board file"
        );
    }

    fn with_hero(log: &str, id: i32, card_id: &str, place: i64) -> String {
        let dump_end = "D 00:57:53.6478530 GameState.DebugPrintGame() - BuildNumber=254477\n";
        let p = "D 00:57:53.6478530 GameState.DebugPrintPower() - ";
        let hero = format!(
            "{p}    FULL_ENTITY - Creating ID={id} CardID={card_id}\n\
             {p}        tag=CONTROLLER value=1\n\
             {p}        tag=CARDTYPE value=HERO\n\
             {p}        tag=PLAYER_ID value=1\n\
             {p}        tag=HEALTH value=30\n\
             {p}        tag=PLAYER_LEADERBOARD_PLACE value={place}\n"
        );
        log.replacen(dump_end, &format!("{hero}{dump_end}"), 1)
    }

    #[test]
    fn two_sessions_of_one_game_share_match_and_final_place() {
        let s1 = with_hero(
            include_str!("../../bglog/tests/data/fight.log"),
            118,
            "TB_BaconShop_HERO_15",
            3,
        );
        let s2 = format!(
            "{}D 01:00:00.0000000 GameState.DebugPrintPower() - TAG_CHANGE Entity=Alice#1919 tag=PLAYSTATE value=WON \n\
             D 01:00:00.0000000 GameState.DebugPrintPower() - TAG_CHANGE Entity=GameEntity tag=STATE value=COMPLETE \n",
            with_hero(&reconnect_excerpt(), 118, "TB_BaconShop_HERO_15", 3)
        );
        let cards = Cards::from_pairs(Vec::<(String, String)>::new());
        let mut reader = std::io::Cursor::new(format!("{s1}{s2}"));
        let games = read_log_reader(&mut reader, &cards).unwrap();
        assert_eq!(games.len(), 2);
        let g1: serde_json::Value =
            serde_json::from_str(&games[0].render("Hearthstone_2026_11_22_02_57_33", 1).unwrap())
                .unwrap();
        let g2: serde_json::Value =
            serde_json::from_str(&games[1].render("Hearthstone_2026_11_22_02_57_33", 2).unwrap())
                .unwrap();
        assert_eq!(g1["match"], "Hearthstone_2026_11_22_02_57_33-g1");
        assert_eq!(g2["match"], "Hearthstone_2026_11_22_02_57_33-g1");
        assert_eq!(g1["place"], 3);
        assert_eq!(g1["final_place"], 1);
        assert_eq!(g2["final_place"], 1);

        // Across two log folders and incremental runs: the first folder is
        // collected before the game finishes, then the restart folder finishes
        // it and updates the first folder's file.
        let tmp = std::env::temp_dir().join(format!("bgbench_board_links_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let archive = tmp.join("archive");
        let laptop = tmp.join("laptop");
        let out = tmp.join("out");
        let cards_file = tmp.join("cards.json");
        let f1 = archive.join("Hearthstone_2026_11_22_02_57_33");
        let f2 = archive.join("Hearthstone_2026_11_22_03_22_48");
        fs::create_dir_all(&f1).unwrap();
        fs::write(&cards_file, "[]").unwrap();
        fs::write(f1.join("Power.log"), &s1).unwrap();

        collect(
            &out,
            &text(&cards_file),
            &laptop,
            &archive,
            &laptop,
            false,
            false,
        )
        .unwrap();
        let before: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(out.join("Hearthstone_2026_11_22_02_57_33-g1.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(before["match"], "Hearthstone_2026_11_22_02_57_33-g1");
        assert_eq!(before["final_place"], 0);

        fs::create_dir_all(&f2).unwrap();
        fs::write(f2.join("Power.log"), &s2).unwrap();
        collect(
            &out,
            &text(&cards_file),
            &laptop,
            &archive,
            &laptop,
            false,
            false,
        )
        .unwrap();
        let after1: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(out.join("Hearthstone_2026_11_22_02_57_33-g1.json")).unwrap(),
        )
        .unwrap();
        let after2: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(out.join("Hearthstone_2026_11_22_03_22_48-g1.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(after1["match"], "Hearthstone_2026_11_22_02_57_33-g1");
        assert_eq!(after1["final_place"], 1);
        assert_eq!(after1["place"], 3);
        assert_eq!(after2["match"], "Hearthstone_2026_11_22_02_57_33-g1");
        assert_eq!(after2["final_place"], 1);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn two_different_games_with_the_same_stand_in_name_are_not_joined() {
        // Both games have `PlayerID=9, PlayerName=Eve`, but the first game
        // finishes and the second starts a new game with a different hero;
        // even a third game that starts with a reconnect dump and the same
        // stand-in name is not joined when its hero differs.
        let g1 = format!(
            "{}D 01:00:00.0000000 GameState.DebugPrintPower() - TAG_CHANGE Entity=GameEntity tag=STATE value=COMPLETE \n",
            with_hero(
                include_str!("../../bglog/tests/data/fight.log"),
                118,
                "TB_BaconShop_HERO_15",
                4,
            )
        );
        let g2 = with_hero(
            include_str!("../../bglog/tests/data/fight.log"),
            118,
            "TB_BaconShop_HERO_15",
            2,
        );
        let g3 = with_hero(&reconnect_excerpt(), 118, "TB_BaconShop_HERO_16", 5);
        let cards = Cards::from_pairs(Vec::<(String, String)>::new());
        let mut reader = std::io::Cursor::new(format!("{g1}{g2}{g3}"));
        let games = read_log_reader(&mut reader, &cards).unwrap();
        assert_eq!(games.len(), 3);
        let j1: serde_json::Value =
            serde_json::from_str(&games[0].render("test", 1).unwrap()).unwrap();
        let j2: serde_json::Value =
            serde_json::from_str(&games[1].render("test", 2).unwrap()).unwrap();
        let j3: serde_json::Value =
            serde_json::from_str(&games[2].render("test", 3).unwrap()).unwrap();
        assert_eq!(j1["match"], "test-g1");
        assert_eq!(j1["final_place"], 4);
        assert_eq!(j2["match"], "test-g2");
        assert_eq!(j2["final_place"], 0);
        assert_eq!(j3["match"], "test-g3");
        assert_eq!(j3["final_place"], 0);
    }
}
