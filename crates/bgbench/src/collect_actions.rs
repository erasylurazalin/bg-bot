//! `bgbench collect-actions`: my shop turns from the real logs, one JSON file
//! per game.
//!
//! Ported from the Python `tools/collect_actions.py` (removed 2026-11-03). It
//! runs from the same timer as collect-boards, after it. It reads the logs
//! the way `collect-boards` does (the same games, numbered the same way), follows
//! the shop phase turn by turn, and writes what I did there:
//!
//!   boards/actions/<log folder>-g<N>.json
//!
//! One file per game,
//! `{"source", "game", "turns", "hero", "unread", "tribes", "lobby"}`;
//! the step kinds and every key of a step are `docs/real-boards.md` "The action
//! file".
//!
//!   bgbench collect-actions [--out DIR] [--cards FILE] [--archive DIR]
//!                           [--laptop DIR] [--rewrite]
//!
//! `--out` is where the files and the `.state.json` incremental state go
//! (`boards/actions` by default, the live folder), `--rewrite`
//! reads every log even when the state says it was read already, and the
//! printed lines are the Python's own:
//! one per log, with the number of games, the step kinds it wrote and what the
//! log could not say (the `unread` counts).
//!
//! The state file maps a log folder's name (with the `laptop-` prefix) to the
//! size of the `Power.log` last read from it, as `collect-boards` does.
//!
//! Almost everything the steps need is read the same way `read_boards` reads a
//! board: [`Tracker`] keeps the lobby, the fights and the views, and the log
//! reader this module owns keeps the game itself, since a reading that is not a
//! view of one moment (the options the client offered, the choice it printed,
//! the block a tag change sits in) has no home in the tracker.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Write};
use std::mem;
use std::path::{Path, PathBuf};
use std::process::exit;

use bglog::{
    Block, Cards, Choice, Entity, Game, Handler, Minion, Offer, Options, PowerLog, REANIMATE_MARK,
    Ref, SECRET_CARDS, TagValue, Tracker, choice_start, enchant_numbers, gifts_by_minion,
    hero_enchants_by_minion, minion, pick_start, taken, trinkets,
};

use crate::collect_boards::{
    SYNTHETIC_CREATE_GAME, Json, State, begins_spectating, ends_spectating, flag, home,
    is_create_game, log_files, minion_numbers, numbers_json, render, text,
};

/// Where the client's logs are copied to, and the laptop's own archive.
const ARCHIVE: &str = "bg-logs";
const LAPTOP_ARCHIVE: &str = "bg-logs-laptop";

/// The card keys a moved card is compared on (the Python's `KEYS`).
const KEYS: [&str; 6] = [
    "card_id",
    "attack",
    "health",
    "golden",
    "keywords",
    "dark_gifts",
];

/// What the stand-in player's Tavern holds.
const SHOP_TYPES: [&str; 2] = ["MINION", "BATTLEGROUND_SPELL"];

/// The minion types a lobby can be made of, in the engine's spelling: the ten
/// names `bglog` reads a card's races as. The game's eleventh, Naga, is not one
/// of them (the engine has no Naga yet), so a Naga-only minion adds no type.
const LOBBY_TRIBES: [&str; 10] = [
    "beast",
    "demon",
    "dragon",
    "elemental",
    "mech",
    "murloc",
    "pirate",
    "quilboar",
    "undead",
    "aberration",
];

/// The buttons and cards a step is named after.
const DRAG_BUY: [&str; 2] = ["TB_BaconShop_DragBuy", "TB_BaconShop_DragBuy_Spell"];
const SELL: &str = "TB_BaconShop_DragSell";
const ROLL: &str = "TB_BaconShop_8p_Reroll_Button";
const FREEZE: &str = "TB_BaconShopLockAll_Button";
const LEVEL: &str = "TB_BaconShopTechUp";
const DARK: &str = "BG36_Button_DarkGift";
/// The Dark Gift Discover a trinket deals (Ominous Stone, Wax Lance, Sinister
/// Invitation). Only Sinister Invitation's offers are Golden.
const DARK_DISCOVERY: &str = "BG36_MidGameEffect_010";
const TRIPLE: &str = "TB_BaconShop_Triples_01";

/// The two trinket slots, by the card that fills them.
const TRINKET_SLOTS: [(&str, &str); 2] = [
    ("BG30_Trinket_1st", "lesser"),
    ("BG30_Trinket_2nd", "greater"),
];

/// The steps that have a price of their own.
const COSTED: [&str; 5] = ["buy", "roll", "level", "hero_power", "dark_discovery"];

/// The log's own entities, whose card ids name no card of mine.
const SYSTEM: [&str; 3] = ["TB_BaconShop_8P_PlayerE", "Bacon_TagTransferPlayerE", "PET_"];

/// The card types that do not sit in my hand as cards.
const NOT_IN_HAND: [&str; 5] = ["ENCHANTMENT", "PLAYER", "HERO", "GAME", "HERO_POWER"];

const FREE_REFRESH: &str = "Bacon_Free_Refresh_Player_Ench";

/// The card types a block can name as the source of what it does.
const SOURCES: [&str; 7] = [
    "MINION",
    "SPELL",
    "BATTLEGROUND_SPELL",
    "BATTLEGROUND_TRINKET",
    "HERO_POWER",
    "ENCHANTMENT",
    "WEAPON",
];

/// The tags `watch` follows.
const WATCHED: [&str; 7] = [
    "BACON_FREE_REFRESH_COUNT",
    "HEALTH",
    "DAMAGE",
    "ARMOR",
    "RESOURCES",
    "TEMP_RESOURCES",
    "RESOURCES_USED",
];

const SILENT_DELIVERER: &str = "BG36_343";

/// `Undead Army`, the enchant an Undead minion carries for the player's "your
/// Undead have +A/+H this game, wherever they are" (`undead_army_numbers`).
const UNDEAD_ARMY: &str = "BG25_011e2";

/// `Volumized`, the enchant every Volumizer of mine carries for "your
/// Volumizers have +A/+H this game (wherever they are)"
/// (`volumizer_bonus_numbers`): its first number is the Attack and its second
/// the Health, and every Volumizer of mine holds the running pair.
const VOLUMIZED: &str = "BG34_170e";

/// Aureate Laureate: `always_golden`, so the game shows it Golden without three
/// copies, and its text says it gives no Triple Reward. The golden card id is
/// listed too, against a log that names that one.
const AUREATE_LAUREATE: [&str; 2] = ["BG32_236", "BG32_236_G"];

/// The Gilding Dark Gift (`darkgifts.toml`): it makes the minion it lands on
/// Golden, and that Golden gives no Triple Reward.
const GILDING: &str = "BG36_MidGameEffect_000t14";

/// The enchant the game keeps the Demon Fodder queue on ("Fodder Player
/// Enchant [DNT]"): its four script numbers are the Fodders each of the next
/// four Refreshes gets (`Player.fodder_refreshes`).
const FODDER_ENCHANT: &str = "BG35_150pe";

/// The pair of tags a minion carries for the Attack and Health its Blood Gems
/// have given it, from every source: `Minion::gem_attack`/`gem_health`, and
/// exactly what Gem Confiscation takes from the neighbours. The client prints
/// this pair as numbers (it has no name for them); the player's own Gem size
/// is a different pair (`BACON_BLOODGEMBUFF*VALUE`, docs/real-boards.md).
const GEM_TAGS: [&str; 2] = ["4270", "4271"];

/// The blocks a click on my own board turns into a step, and the step it makes.
const BOARD_BLOCKS: [&str; 2] = ["PLAY", "MOVE_MINION"];

/// The player enchantments whose two script numbers are a pair of counters, in
/// the order the Python writes them.
const PLAYER_ENCHANTS: [(&str, (&str, Option<&str>)); 8] = [
    ("BG31_808pe", ("beetle_attack", Some("beetle_health"))),
    ("BG25_011pe", ("undead_attack", None)),
    ("BG31_816pe", ("baller", None)),
    // Eternal Knight: how many of them died this game, which every one of them
    // reads "wherever this is".
    ("BG25_008pe", ("knight_deaths", None)),
    ("BG_ShopBuff", ("tavern_attack", Some("tavern_health"))),
    (
        "BG_ShopBuff_Elemental",
        ("tavern_elemental_attack", Some("tavern_elemental_health")),
    ),
    ("BG34_854pe", ("refresh_attack", Some("refresh_health"))),
    ("BG34_689e2", ("barrage_attack", Some("barrage_health"))),
];

/// The player enchantments whose two counters are one number.
const SAME_NUMBER_ENCHANTS: [(&str, (&str, &str)); 1] = [(
    "BG31_873pe2",
    ("tavern_turn_attack", "tavern_turn_health"),
)];

pub fn run(args: &[String]) {
    let home = home();
    let out = flag(args, "--out", "boards/actions");
    let cards = flag(args, "--cards", "ref/hsjson-cards.json");
    let live_log = flag(args, "--live-log", "");
    let live_once = flag(args, "--live-once", "");
    let label = flag(args, "--label", "");
    if !live_log.is_empty() || !live_once.is_empty() {
        let follow = !live_log.is_empty();
        let path = if follow { &live_log } else { &live_once };
        if let Err(e) = collect_live(Path::new(&out), &cards, Path::new(path), &label, follow) {
            eprintln!("{e}");
            exit(2);
        }
        return;
    }
    if args.get(1).map(String::as_str) == Some("live-actions") {
        eprintln!("usage: bgbench live-actions --live-log <path> | --live-once <path> [--out <dir>] [--label <name>] [--cards <path>]");
        exit(2);
    }
    let archive = flag(args, "--archive", &text(&home.join(ARCHIVE)));
    let laptop = flag(args, "--laptop", &text(&home.join(LAPTOP_ARCHIVE)));
    let rewrite = args.iter().any(|a| a == "--rewrite");
    if let Err(e) = collect(
        Path::new(&out),
        &cards,
        Path::new(&archive),
        Path::new(&laptop),
        rewrite,
    ) {
        eprintln!("{e}");
        exit(2);
    }
}

/// The whole run: every log the state has not seen, one file per game.
fn collect(
    out: &Path,
    cards_path: &str,
    archive: &Path,
    laptop: &Path,
    rewrite: bool,
) -> Result<(), String> {
    let cards = Cards::load(cards_path)?;
    let dbf = load_dbf(cards_path)?;
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
    for (prefix, path) in files {
        let Some(folder) = path.parent().and_then(|p| p.file_name()) else {
            continue;
        };
        let name = format!("{prefix}{}", folder.to_string_lossy());
        let size = fs::metadata(&path)
            .map_err(|e| format!("{}: {e}", path.display()))?
            .len();
        if !rewrite && state.get(&name) == Some(size) {
            continue;
        }
        let mut steps = Tally::new();
        let mut unread = Tally::new();
        let mut games = 0;
        for (index, record) in read_log(&path, &cards, &name, &dbf)?.into_iter().enumerate() {
            let Some(record) = record else {
                continue;
            };
            let gi = index + 1;
            games += 1;
            let object = record.as_obj().unwrap();
            for turn in parts_of(object, "turns") {
                for step in parts_of(turn.as_obj().unwrap(), "actions") {
                    steps.bump(text_in(step.as_obj().unwrap(), "step"));
                }
            }
            if let Some(counts) = field(object, "unread").and_then(Json::as_obj) {
                for (key, n) in counts {
                    unread.add(key, n.as_int().unwrap_or(0));
                }
            }
            let mut fields: Obj = Vec::with_capacity(6);
            put(&mut fields, "source", str_(&name));
            put(&mut fields, "game", Json::Int(gi as i64));
            fields.extend(object.clone());
            let file = out.join(format!("{name}-g{gi}.json"));
            fs::write(&file, render(&Json::Obj(fields)))
                .map_err(|e| format!("{}: {e}", file.display()))?;
        }
        println!(
            "{name}: {games} games, {}, unread {}",
            steps.render(),
            unread.render()
        );
        state.set(name, size);
        let text = state.render();
        fs::write(&state_path, text).map_err(|e| format!("{}: {e}", state_path.display()))?;
    }
    Ok(())
}

/// One JSON object whose keys keep their order.
type Obj = Vec<(String, Json)>;

fn str_(text: &str) -> Json {
    Json::Str(text.to_string())
}

/// `object[key] = value`, the Python's own: a key already there keeps its
/// place and takes the new value, and an object never holds one key twice.
fn put(object: &mut Obj, key: &str, value: Json) {
    match object.iter_mut().find(|(have, _)| have == key) {
        Some(slot) => slot.1 = value,
        None => object.push((key.to_string(), value)),
    }
}

/// The same, named for the places the Python replaces a value it set before.
fn set(object: &mut Obj, key: &str, value: Json) {
    put(object, key, value);
}

fn drop_key(object: &mut Obj, key: &str) {
    if let Some(at) = object.iter().position(|(have, _)| have == key) {
        object.remove(at);
    }
}

fn field<'a>(object: &'a Obj, key: &str) -> Option<&'a Json> {
    object.iter().find(|(have, _)| have == key).map(|(_, v)| v)
}

fn field_mut<'a>(object: &'a mut Obj, key: &str) -> Option<&'a mut Json> {
    object.iter_mut().find(|(have, _)| have == key).map(|(_, v)| v)
}

/// `object[key]` where the Python expects a number.
fn number_in(object: &Obj, key: &str) -> i64 {
    field(object, key).and_then(Json::as_int).unwrap_or(0)
}

fn text_in<'a>(object: &'a Obj, key: &str) -> &'a str {
    field(object, key).and_then(Json::as_str).unwrap_or("")
}

/// The cards of a state part (`board`, `hand`, `tavern`).
fn parts<'a>(state: &'a Json, key: &str) -> &'a [Json] {
    state
        .as_obj()
        .and_then(|object| field(object, key))
        .and_then(Json::as_arr)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

/// The last step of a turn, which a late change is written into.
fn actions_last(turn: &mut Obj) -> Option<&mut Obj> {
    field_mut(turn, "actions")?.as_arr_mut()?.last_mut()?.as_obj_mut()
}

impl Json {
    fn as_obj(&self) -> Option<&Obj> {
        match self {
            Json::Obj(object) => Some(object),
            _ => None,
        }
    }

    fn as_obj_mut(&mut self) -> Option<&mut Obj> {
        match self {
            Json::Obj(object) => Some(object),
            _ => None,
        }
    }

    fn as_arr(&self) -> Option<&Vec<Json>> {
        match self {
            Json::Arr(items) => Some(items),
            _ => None,
        }
    }

    fn as_arr_mut(&mut self) -> Option<&mut Vec<Json>> {
        match self {
            Json::Arr(items) => Some(items),
            _ => None,
        }
    }

    fn as_int(&self) -> Option<i64> {
        match self {
            Json::Int(n) => Some(*n),
            _ => None,
        }
    }

    fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(text) => Some(text),
            _ => None,
        }
    }

    fn is_null(&self) -> bool {
        matches!(self, Json::Null)
    }
}

/// Which of my places a card sits in, `off` when it is in none of them.
fn where_of(object: &Obj) -> &str {
    text_in(object, "where")
}

/// A count of something, in the order the Python's `Counter` first saw it, so
/// the printed line reads as the Python's does.
#[derive(Default)]
struct Tally(Vec<(String, i64)>);

impl Tally {
    fn new() -> Tally {
        Tally(Vec::new())
    }

    fn bump(&mut self, key: &str) {
        self.add(key, 1);
    }

    fn add(&mut self, key: &str, n: i64) {
        match self.0.iter_mut().find(|(have, _)| have == key) {
            Some(slot) => slot.1 += n,
            None => self.0.push((key.to_string(), n)),
        }
    }

    fn to_json(&self) -> Json {
        Json::Obj(
            self.0
                .iter()
                .map(|(key, n)| (key.clone(), Json::Int(*n)))
                .collect(),
        )
    }

    /// `dict(counter)`: `{'buy': 3, 'roll': 1}`.
    fn render(&self) -> String {
        let items: Vec<String> = self
            .0
            .iter()
            .map(|(key, n)| format!("'{key}': {n}"))
            .collect();
        format!("{{{}}}", items.join(", "))
    }
}

/// What a step did besides take Gold and spend it, as `watch` reads it.
#[derive(Clone, Default)]
struct Stats {
    floor: i64,
    restored: i64,
    restore_by: Option<Json>,
    gold_gain: i64,
    gold_by: Option<Json>,
}

/// One grant of free Refreshes: how many, and the card that put them there.
#[derive(Clone)]
struct Pool {
    count: i64,
    card: Option<Json>,
}

/// The step being resolved, and the state it started from.
struct Pending {
    step: Obj,
    entity: Option<i32>,
    gold: i64,
    health: i64,
    before: Json,
    free: i64,
    pool: Vec<Json>,
}

/// The card counts a state is compared on.
#[derive(PartialEq)]
struct CardKey {
    id: i64,
    card_id: Option<String>,
    attack: Option<i64>,
    health: Option<i64>,
    golden: Option<bool>,
    keywords: Option<Json>,
    dark_gifts: Option<Json>,
    frozen: Option<bool>,
}

/// One log's worth of the Python's `ActionTracker`.
struct Collector {
    log: PowerLog,
    tracker: Tracker,
    cards: Cards,
    /// The game's dbf id to card id, from the same `--cards` file: a hero's
    /// leaderboard tags name the other players' trinkets by dbf id.
    dbf: HashMap<u32, String>,
    /// One record per game the log ends, `None` for a game with no shop turn.
    games: Vec<Option<Json>>,
    record: Json,
    unread: Tally,
    upper: i64,
    turn: Option<Obj>,
    pending: Option<Pending>,
    options: Options,
    choice: Option<Choice>,
    ended: bool,
    last: Option<Json>,
    last_gold: i64,
    last_health: i64,
    free: i64,
    /// Golden minions this player played from hand this game: the log shows no
    /// number for it, and "After you Discover a card ... improved by Golden
    /// minions you played this game" (Hooktusk, Master Marauder) reads it.
    goldens: i64,
    pool: Vec<Pool>,
    stats: Option<Stats>,
    damage: Option<i64>,
    armor_now: Option<i64>,
    gold_live: Option<i64>,
    waited: BTreeMap<i64, Json>,
    tribes: BTreeSet<String>,
    /// The lobby's types by Era's rule (docs/rules/design.md "The lobby"): for
    /// every type a minion of only that type showed in the Tavern, the ids of
    /// the Tavern minions that showed it. The ids are for the warning a game
    /// with more than five types prints.
    lobby: BTreeMap<String, BTreeSet<String>>,
    /// The log folder this run's files are named after, for that warning.
    label: String,
    game_complete: bool,
    /// A `Begin Spectating` line was read and `End Spectator Mode` has not:
    /// every line between them is skipped. The number of games inside the
    /// stretch is kept so their session numbers can be left empty.
    spectating: bool,
    spectator_games: usize,
}

impl Collector {
    #[cfg(test)]
    fn new(cards: Cards, label: &str) -> Collector {
        Collector::new_with_dbf(cards, label, HashMap::new())
    }

    fn new_with_dbf(cards: Cards, label: &str, dbf: HashMap<u32, String>) -> Collector {
        let mut collector = Collector {
            log: PowerLog::new(),
            tracker: Tracker::new(cards.clone()),
            cards,
            dbf,
            games: Vec::new(),
            record: Json::Null,
            unread: Tally::new(),
            upper: 0,
            turn: None,
            pending: None,
            options: Options::default(),
            choice: None,
            ended: false,
            last: None,
            last_gold: 0,
            last_health: 0,
            free: 0,
            goldens: 0,
            pool: Vec::new(),
            stats: None,
            damage: None,
            armor_now: None,
            gold_live: None,
            waited: BTreeMap::new(),
            tribes: BTreeSet::new(),
            lobby: BTreeMap::new(),
            label: label.to_string(),
            game_complete: false,
            spectating: false,
            spectator_games: 0,
        };
        collector.reset();
        collector
    }

    /// Read one line, into the tracker and into the reader this collector owns.
    ///
    /// A spectated stretch is skipped whole: it is another player's game, and
    /// the collectors only keep Era's. The stretch still takes a session
    /// number, so the games after it keep the numbers `collect-boards` gives
    /// them.
    fn feed(&mut self, line: &str) {
        if self.spectating {
            if ends_spectating(line) {
                self.spectating = false;
                // Close the empty game the start left behind, then keep one
                // empty record for every spectated game in the stretch.
                self.feed_line(SYNTHETIC_CREATE_GAME);
                for _ in 0..self.spectator_games {
                    self.games.push(None);
                }
                self.spectator_games = 0;
            } else if is_create_game(line) {
                self.spectator_games += 1;
            }
            return;
        }
        if begins_spectating(line) {
            self.spectating = true;
            // Close the game in hand at the marker, as its next CREATE_GAME in
            // the spectator block would.
            self.feed_line(SYNTHETIC_CREATE_GAME);
            return;
        }
        self.feed_line(line);
    }

    fn feed_line(&mut self, line: &str) {
        self.tracker.feed(line);
        // The reader reports to this collector and the collector owns it: hand
        // it over for the call and take it back after, as `Tracker::feed` does.
        let mut log = mem::take(&mut self.log);
        log.feed(line, self);
        self.log = log;
    }

    /// The end of a log: an unfinished spectator stretch is skipped too, so
    /// its sessions never become files.
    fn finish_log(&mut self) {
        if self.spectating {
            self.spectating = false;
            self.feed_line(SYNTHETIC_CREATE_GAME);
            for _ in 0..self.spectator_games {
                self.games.push(None);
            }
            self.spectator_games = 0;
        }
    }

    /// The game the log is reading, for the end of a log (the reader's own game
    /// stays until the next `CREATE_GAME`).
    fn last_game(&mut self) {
        let Some(game) = self.log.game().cloned() else {
            return;
        };
        if !game.game_type.starts_with("GT_BATTLEGROUNDS") {
            return;
        }
        self.file_game(&game, true);
    }

    // -- games --------------------------------------------------------------

    fn reset(&mut self) {
        self.record = Json::Obj(vec![
            ("turns".to_string(), Json::Arr(Vec::new())),
            ("hero".to_string(), Json::Null),
        ]);
        // Upper Hands cast this turn: "Start of Combat: set a random enemy
        // minion's Health to 1" hides its pending trigger, so the casts are the
        // count (the board files write it the same way).
        self.upper = 0;
        self.unread = Tally::new();
        self.turn = None;
        self.pending = None;
        self.options = Options::default();
        self.choice = None;
        self.ended = false;
        self.game_complete = false;
        self.last = None;
        self.last_gold = 0;
        self.last_health = 0;
        self.free = 0;
        self.goldens = 0;
        self.pool.clear();
        self.stats = None;
        self.damage = None;
        self.armor_now = None;
        self.gold_live = None;
        self.waited.clear();
        self.tribes.clear();
        self.lobby.clear();
    }

    /// File the game just read, under the number `collect-boards` gives it: one
    /// per game the log ends, `None` for a game with no shop turn. The last game
    /// of a log takes no number unless it got as far as a fight, which is where
    /// `collect-boards` draws the same line.
    fn file_game(&mut self, game: &Game, need_fight: bool) {
        self.close_turn(game, false);
        if need_fight && self.tracker.fights().is_empty() {
            return;
        }
        let mut record = mem::take(&mut self.record);
        let has_turns = record
            .as_obj()
            .map(|object| !parts_of(object, "turns").is_empty())
            .unwrap_or(false);
        if !has_turns {
            self.games.push(None);
            self.reset();
            return;
        }
        let unread = self.unread.to_json();
        let hero = self.my_hero(game);
        let tribes = Json::Arr(self.tribes.iter().map(|t| str_(t)).collect());
        let lobby = Json::Arr(self.lobby.keys().map(|t| str_(t)).collect());
        // A lobby has five types (docs/rules/design.md "The lobby"): more than
        // five of them seen alone in the Tavern is the collector's bug, not the
        // game's, so it says which game and which cards showed them.
        if self.lobby.len() > 5 {
            let mut cards: Vec<String> = Vec::new();
            for (tribe, ids) in &self.lobby {
                let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
                cards.push(format!("{tribe}: {}", ids.join(" ")));
            }
            eprintln!(
                "{}-g{}: {} lobby types, more than the five a lobby has: {}",
                self.label,
                self.games.len() + 1,
                self.lobby.len(),
                cards.join(", ")
            );
        }
        if let Some(object) = record.as_obj_mut() {
            set(object, "hero", hero);
            put(object, "unread", unread);
            put(object, "tribes", tribes);
            put(object, "lobby", lobby);
        }
        self.games.push(Some(record));
        self.reset();
    }

    fn my_hero(&self, game: &Game) -> Json {
        let Some(hero) = hero_entity(game) else {
            return Json::Null;
        };
        if hero.card_id.is_empty() {
            return Json::Null;
        }
        Json::Obj(vec![
            ("card_id".to_string(), str_(&hero.card_id)),
            ("name".to_string(), str_(self.cards.name(&hero.card_id))),
        ])
    }
}

/// The cards of one part of a state object.
fn parts_of<'a>(state: &'a Obj, key: &str) -> &'a [Json] {
    field(state, key)
        .and_then(Json::as_arr)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

/// My PlayerID: the smallest with a real account behind it.
fn me(game: &Game) -> Option<i32> {
    game.real_players.iter().copied().min()
}

/// The stand-in player, which holds Bob's Tavern in the shop phase.
fn dummy(game: &Game) -> Option<i32> {
    let mut ids: Vec<i32> = game.players.keys().copied().collect();
    ids.sort_unstable();
    ids.into_iter().find(|p| !game.real_players.contains(p))
}

fn my_player(game: &Game) -> Option<&Entity> {
    me(game).and_then(|me| game.player_entity(me))
}

/// The "Undead Army" enchant a minion carries for the player's "your Undead
/// have +A/+H this game, wherever they are": its number 1 is the Attack and
/// its number 2 the Health. Every Undead of mine on the board holds the same
/// pair, so the newest one answers for all of them; the game keeps no
/// player-level enchant for the Health half (`BG25_011pe` has the Attack
/// alone), so this is the only place the log shows it. `None` when no Undead
/// of mine is on the board.
fn undead_army_numbers(game: &Game) -> Option<[i64; 4]> {
    let me = me(game)? as i64;
    let mine: HashSet<i64> = game
        .entities_in_order()
        .filter(|e| {
            e.text("CARDTYPE") == "MINION"
                && e.text("ZONE") == "PLAY"
                && e.int("CONTROLLER") == me
        })
        .map(|e| e.id as i64)
        .collect();
    game.entities_in_order()
        .filter(|x| {
            x.card_id == UNDEAD_ARMY
                && x.text("ZONE") == "PLAY"
                && mine.contains(&x.int("ATTACHED"))
        })
        .max_by_key(|x| x.id)
        .map(|x| {
            [
                x.int("TAG_SCRIPT_DATA_NUM_1"),
                x.int("TAG_SCRIPT_DATA_NUM_2"),
                x.int("TAG_SCRIPT_DATA_NUM_3"),
                x.int("TAG_SCRIPT_DATA_NUM_4"),
            ]
        })
}

/// "Your Volumizers have +A/+H this game (wherever they are)": the Attack and
/// Health every Volumizer of mine carries as its `Volumized` enchantment
/// (`BG34_170e`, numbers 1 and 2). The game keeps no pair on the player for it
/// and the player enchantment (`BG34_170pe`) shows no numbers at all, so the
/// minion's own enchantment is the only place the log has it. The pair runs on
/// every Volumizer of mine, wherever it is, so the largest of them answers for
/// all; a Volumizer Magnetized into a host stops getting it and keeps the last
/// pair it had, which is why the largest and not the newest is taken. `None`
/// when I never had a Volumizer.
fn volumizer_bonus_numbers(game: &Game) -> Option<[i64; 2]> {
    let me = me(game)? as i64;
    game.entities_in_order()
        .filter(|x| {
            x.card_id == VOLUMIZED && x.text("ZONE") == "PLAY" && x.int("CONTROLLER") == me
        })
        .map(|x| {
            [
                x.int("TAG_SCRIPT_DATA_NUM_1"),
                x.int("TAG_SCRIPT_DATA_NUM_2"),
            ]
        })
        .max_by_key(|pair| pair[0] + pair[1])
}

/// The tag the game keeps a minion's own Magnetization count in. It has no
/// name in the log (the client shows it as the bare number), and it is the
/// count Utility Drone's "+4/+5 for each Magnetization they have" reads.
/// `Hearthstone_2026_11_02_00_10_22-g2` turn 10, the end of the turn: the
/// Glambot at `3670` 94 takes +376/+470 from the Drone (94 x +4/+5) while the
/// two other minions the Drone hits take their own counts, 6 and 12.
const MAGNETIZED_TAG: &str = "3670";

/// The tag a Drone Duplicator's held "the next Magnetization to this minion
/// this turn happens an extra time" sits in (its Activate (1)). It has no name
/// in the log, and the client shows nothing for it: the Activate sets it to 1
/// and the Magnetization it was waiting for clears it again
/// (`Hearthstone_2026_10_29_22_20_04-g3` turn 14 step 19 sets it on the
/// Duplicator, step 20's Magnetization spends it). A Golden one sets the same 1
/// and applies it twice, so what this file writes is the number of extra times:
/// the tag, times two for a Golden card.
const EXTRA_MAGNETIZATION_TAG: &str = "4945";

/// The extra times the next Magnetization onto this minion happens, as the
/// engine's `Minion.extra_magnetize` holds it: the game's held-extra tag
/// (`EXTRA_MAGNETIZATION_TAG`), and a Golden Drone Duplicator applies its 1
/// twice (`Hearthstone_2026_10_29_22_20_04-g3` turn 14 step 20, a golden one
/// and three merges where the tag says 1).
fn extra_magnetizations(e: &Entity, golden: bool) -> i64 {
    e.int(EXTRA_MAGNETIZATION_TAG) * if golden { 2 } else { 1 }
}

/// How many Magnetizations a minion carries, which is what the game's own
/// Utility Drone reads: the minion's `3670` tag, one for every Magnetic that
/// merged into it and one for the minion itself when the minion is Magnetic
/// (a Blue Volumizer on its own in hand shows 1, one with a Satellite merged
/// into it 2). The engine's `Minion::magnetizations` counts what merged into
/// the minion and not the minion itself, so a Magnetic minion's own one comes
/// off the tag.
///
/// The count is *not* the number of `Magnetic` enchantments attached to the
/// host: the game accumulates a Satellite's stats into a single `BG31_171te`
/// enchantment, so a Glambot with 94 Magnetizations shows one enchantment.
/// That is the count this used to read, and it gave the Drone nothing
/// (`Hearthstone_2026_11_02_00_10_22-g2` turn 10 step 31: Glambot 581/626
/// replayed against the log's 953/1091).
///
/// A log with no `3670` tag at all (every log of Era's has it) falls back to
/// counting the enchantments, one per merge, as the collector always did.
fn magnetization_count(game: &Game, host: &Entity) -> i64 {
    let merged = || {
        game.entities_in_order()
            .filter(|x| {
                x.text("CARDTYPE") == "ENCHANTMENT"
                    && x.text("ZONE") == "PLAY"
                    && x.int("MAGNETIC") != 0
                    && x.int("ATTACHED") == host.id as i64
            })
            .count() as i64
    };
    let Some(count) = host.get(MAGNETIZED_TAG) else {
        return merged();
    };
    let own = i64::from(host.int("MAGNETIC") != 0);
    (count.as_int() - own).max(0)
}

/// The +A/+H Holy Vanguard's aura has put into a minion's logged stats, or
/// `None` when it has none: the pair on the `Vanguard's Valor` enchantment the
/// minion carries while "you have 15 or less Health" holds, 30/30 on a plain
/// one and 60/60 on a Golden one (`bglog::vanguard_aura`). A triple carries one
/// such enchantment per copy over onto the Golden one (a Golden Holy Vanguard
/// ends up with three, two of 30/30 and its own 60/60), so the largest pair is
/// the one the minion's own card adds and the one to take off; the game never
/// adds the copies' pairs up.
///
/// The file writes it beside `attack` and `health`, which already include it,
/// because the stats are the card's wherever it is (docs/rules/design.md
/// "Triples"): the loader records the pair on `Minion::vanguard_aura`, which
/// takes it off again exactly once (a later Health crossing) and which a
/// triple leaves out of its sum, since the Golden one is given its own once
/// (log `Hearthstone_2026_10_28_22_11_50` game 5 turn 10 step 12: the Golden
/// Holy Vanguard 208/116, where counting each copy's aura said 238/146).
fn vanguard_aura_numbers(game: &Game, host: &Entity) -> Option<(i64, i64)> {
    bglog::vanguard_aura(game, host.id)
}

fn hero_entity(game: &Game) -> Option<&Entity> {
    let player = my_player(game)?;
    let hero = player.int("HERO_ENTITY");
    if hero == 0 {
        return None;
    }
    game.entities.get(&(hero as i32))
}

/// The Gold the client shows: what the turn gave me, plus what a sell or a Gold
/// effect put in the temporary pool, minus what I have spent this turn.
fn gold(game: &Game) -> i64 {
    let Some(player) = my_player(game) else {
        return 0;
    };
    player.int("RESOURCES") + player.int("TEMP_RESOURCES") - player.int("RESOURCES_USED")
}

/// My hero's Health as the client shows it, Armor apart (the `health` of a
/// state).
fn hero_health(game: &Game) -> i64 {
    match hero_entity(game) {
        Some(hero) => hero.int("HEALTH") - hero.int("DAMAGE"),
        None => 0,
    }
}

/// Health and Armor together, which is what a card priced in Health takes.
fn health(game: &Game) -> i64 {
    match hero_entity(game) {
        Some(hero) => hero.int("HEALTH") + hero.int("ARMOR") - hero.int("DAMAGE"),
        None => 0,
    }
}

fn armor(game: &Game) -> i64 {
    match hero_entity(game) {
        Some(hero) => hero.int("ARMOR"),
        None => 0,
    }
}

/// The price on the tier up button, or `None` at the top tier.
fn upgrade_cost(game: &Game) -> Json {
    let mine = me(game).map(|me| me as i64);
    let best = game
        .entities_in_order()
        .filter(|x| {
            Some(x.int("CONTROLLER")) == mine
                && x.text("ZONE") == "PLAY"
                && x.card_id.starts_with(LEVEL)
        })
        .min_by_key(|x| x.id);
    match best {
        Some(x) => Json::Int(x.int("COST")),
        None => Json::Null,
    }
}

/// A zone or card type as a `kind`: lower case, or the fallback when the log
/// never named it.
fn lower<'a>(text: &'a str, fallback: &'a str) -> String {
    if text.is_empty() {
        fallback.to_string()
    } else {
        text.to_lowercase()
    }
}

/// Whether a card id names one of the log's own entities.
fn is_system(card_id: &str) -> bool {
    SYSTEM.iter().any(|prefix| card_id.starts_with(prefix))
}

/// A minion as the views read it, with Dark Gifts aside (only the ones a step
/// names are wanted).
fn plain_minion<'a>(game: &'a Game, cards: &Cards, e: &'a Entity) -> bglog::Minion {
    let gifts: HashMap<i32, Vec<String>> = HashMap::new();
    minion(e, game, cards, &gifts, &gifts)
}

/// The warband of the opponent I last fought, as the log last showed it in
/// combat: the board that went into the fight, before start-of-combat effects,
/// in the shape a state's own `board` has. Null before the first fight. The
/// last fight with a result is the one I last fought, so a fight the log has
/// already staged for the round to come is not it (the tracker keeps the
/// latest fight staged, complete or not).
fn last_opponent_board(cards: &Cards, fights: &[bglog::Fight]) -> Json {
    let Some(fight) = fights.iter().rev().find(|f| !f.result.is_empty()) else {
        return Json::Null;
    };
    warband_json(cards, fight)
}

/// The other side of one fight, as a state's `board` writes a board. An empty
/// board is null: the game has no warband to copy, and the powers that copy one
/// then cannot be pressed (`burgle_has_board`, `IspyNextOpponent`).
fn warband_json(cards: &Cards, fight: &bglog::Fight) -> Json {
    if fight.theirs.is_empty() {
        return Json::Null;
    }
    let marks = &fight.reanimates.1;
    let board: Vec<Json> = fight
        .theirs
        .iter()
        .enumerate()
        .map(|(i, m)| Json::Obj(minion_json(cards, i, m, marks.get(i).copied().unwrap_or(false))))
        .collect();
    Json::Arr(board)
}

/// One minion of that board, as `state` writes one of mine, without the entity
/// id: the copy the log staged for the fight is gone by the time the state is
/// written. Its Dark Gifts are the gifts' own card ids, the shape a state's
/// board holds (`ShopGiftSpec`), so the loader reads it the same way.
fn minion_json(cards: &Cards, i: usize, m: &Minion, reanimate: bool) -> Obj {
    let mut d = vec![
        ("position".to_string(), Json::Int(i as i64 + 1)),
        ("card_id".to_string(), str_(&m.card_id)),
        ("name".to_string(), str_(&m.name)),
        ("attack".to_string(), Json::Int(m.attack)),
        ("health".to_string(), Json::Int(m.health)),
        ("golden".to_string(), Json::Bool(m.golden)),
        (
            "keywords".to_string(),
            Json::Arr(m.keywords.iter().map(|k| str_(k)).collect()),
        ),
    ];
    if !m.dark_gifts.is_empty() {
        let gifts: Vec<Json> = m
            .dark_gifts
            .iter()
            .map(|card| {
                Json::Obj(vec![
                    ("card_id".to_string(), str_(card)),
                    ("name".to_string(), str_(cards.name(card))),
                ])
            })
            .collect();
        put(&mut d, "dark_gifts", Json::Arr(gifts));
    }
    if let Some(sd) = minion_numbers(&m.tags) {
        put(&mut d, "numbers", numbers_json(&sd));
    }
    if reanimate {
        put(&mut d, "reanimate", Json::Bool(true));
    }
    d
}

/// Whether a Golden minion gives no Triple Reward when it is played.
///
/// Every Golden minion gives one unless its own card or the effect that made it
/// says it does not (docs/rules/design.md, THE GOLDEN RULE). The ones that say
/// so are the four sources Era knows: Silent Deliverer's Battlecry goldens,
/// Sinister Invitation's (its Dark Gift offer is the only one whose offers are
/// Golden), the Gilding Dark Gift, and Aureate Laureate (`always_golden`).
/// Everything else gives one, a Golden carrying any other Dark Gift, a triple's
/// Golden and a Golden bought included, so the default here is `false`.
fn no_triple_reward(m: &bglog::Minion, creator: Option<&str>) -> bool {
    m.golden
        && (creator == Some(SILENT_DELIVERER)
            || creator == Some(DARK_DISCOVERY)
            || AUREATE_LAUREATE.contains(&m.card_id.as_str())
            || m.dark_gifts.iter().any(|gift| gift == GILDING))
}

/// The Attack and Health this minion's Blood Gems have given it (`GEM_TAGS`),
/// or `None` when the log shows none: the card's own key stays out then.
///
/// The game keeps the pair on the minion itself and moves it as the minion
/// does (a copy carries it, a triple's sum, a steal clears the neighbour's),
/// so one state's tags answer for it: log `2026_10_26_00_48_15-g2` turn 7
/// step 24 has the Sly Infiltrator at 4/4 and 0/0 either side of a Gem
/// Confiscation, `2026_10_29_22_20_04-g1` turn 11 step 23 the Amalgam at
/// 804/469 and then 0/0.
fn gem_stats(tags: &HashMap<String, TagValue>) -> Option<(i64, i64)> {
    let read = |tag: &str| tags.get(tag).map(TagValue::as_int).unwrap_or(0);
    let (attack, health) = (read(GEM_TAGS[0]), read(GEM_TAGS[1]));
    if attack == 0 && health == 0 {
        None
    } else {
        Some((attack, health))
    }
}

impl Handler for Collector {
    /// The game a `CREATE_GAME` ended: file it, as the Python files it in
    /// `_on_new_game` (the reader swaps its game before that hook, so this is
    /// where the finished one is still in hand).
    fn end_game(&mut self, game: &Game) {
        if game.game_type.starts_with("GT_BATTLEGROUNDS") {
            self.file_game(game, false);
        }
    }

    fn new_game(&mut self, _game: &mut Game) {
        self.reset();
    }

    fn tag_change(
        &mut self,
        game: &mut Game,
        _depth: usize,
        entity: i32,
        tag: &str,
        value: &TagValue,
    ) {
        let game = &*game;
        let Some(e) = game.entities.get(&entity) else {
            return;
        };
        self.watch(game, e, tag, value);
        if tag == "COPIED_FROM_ENTITY_ID" && value.as_int() != 0 {
            self.waited_copy(game, e, value.as_int());
        }
        if game.game_entity().map(|x| x.id) != Some(entity) {
            return;
        }
        if tag == "TURN" {
            self.set_turn(game, value.as_int());
        } else if tag == "STEP"
            && value.as_text() == Some("MAIN_END")
            && self.turn.is_some()
            && !self.ended
        {
            // the turn is over: this is where my End Turn click lands, and
            // where the end of turn effects resolve
            self.close_step(game);
            self.ended = true;
            self.open_step(game, step_of("end_turn"), None);
        } else if tag == "STATE" && value.as_text() == Some("COMPLETE") {
            self.game_complete = true;
        }
    }

    fn block_start(&mut self, game: &mut Game, _depth: usize, block: Block<'_>) {
        // The Python reads the block's target off its own reader before its
        // tracker sees the block; nothing in this hook reads the open blocks,
        // so the order the two are told in does not matter.
        self.on_block_start(&*game, block.kind, block.entity, block.target);
    }

    fn line(&mut self, game: Option<&mut Game>, call: &str, body: &str) {
        let Some(game) = game else {
            return;
        };
        let game = &*game;
        match call {
            "DebugPrintOptions" => self.options_line(game, body),
            "SendOption" => self.send_option(game, body),
            "DebugPrintEntityChoices" => self.choice_line(game, body),
            "SendChoices" => self.send_choices(game, body),
            _ => {}
        }
    }
}

/// A step that is only its name.
fn step_of(kind: &str) -> Obj {
    vec![("step".to_string(), str_(kind))]
}

impl Collector {
    // -- the tracker's hooks ------------------------------------------------

    fn on_block_start(&mut self, game: &Game, kind: &str, entity: Option<i32>, target: Option<Ref>) {
        if kind == "PLAY"
            && let Some(e) = entity.and_then(|id| game.entities.get(&id))
            && e.card_id == "BG28_573"
        {
            self.upper += 1;
        }
        {
            let Some(pending) = self.pending.as_ref() else {
                return;
            };
            if text_in(&pending.step, "step") != "board" || pending.entity.is_none() {
                return;
            }
            if Some(pending.entity) != Some(entity) || !BOARD_BLOCKS.contains(&kind) {
                return;
            }
        }
        let named = if kind == "PLAY" { "activate" } else { "move" };
        let empty = self
            .pending
            .as_ref()
            .map(|p| field(&p.step, "target").map(Json::is_null).unwrap_or(true))
            .unwrap_or(false);
        let target = match target {
            Some(target) if empty => Some(self.where_(game, Some(&target))),
            _ => None,
        };
        let pending = self.pending.as_mut().unwrap();
        set(&mut pending.step, "step", str_(named));
        if let Some(target) = target {
            put(&mut pending.step, "target", target);
        }
    }

    fn waited_copy(&mut self, game: &Game, e: &Entity, held: i64) {
        if e.text("CARDTYPE") != "MINION" || e.card_id.is_empty() || self.waited.contains_key(&held)
        {
            return;
        }
        let d = minion_brief(game, &self.cards, e);
        self.waited.insert(held, Json::Obj(d));
    }

    fn watch(&mut self, game: &Game, e: &Entity, tag: &str, value: &TagValue) {
        if !WATCHED.contains(&tag) {
            return;
        }
        if tag == "BACON_FREE_REFRESH_COUNT" && e.card_id.starts_with(FREE_REFRESH) {
            self.free_refreshes(game, value.as_int());
            return;
        }
        let n = value.as_int();
        let hero = hero_entity(game).map(|x| x.id);
        let player = my_player(game).map(|x| x.id);
        if hero == Some(e.id) && matches!(tag, "HEALTH" | "DAMAGE" | "ARMOR") {
            if tag == "DAMAGE" {
                // damage undone shows as my hero's DAMAGE going back down
                if let Some(damage) = self.damage
                    && n < damage
                    && self.stats.is_some()
                {
                    self.add_restore(game, damage - n);
                }
                self.damage = Some(n);
            } else if tag == "ARMOR" {
                // and, when the Armor took it (a Health price is paid out of
                // Armor first), as the Armor coming back
                if let Some(armor) = self.armor_now
                    && n > armor
                    && self.stats.is_some()
                {
                    self.add_restore(game, n - armor);
                }
                self.armor_now = Some(n);
            }
            if let Some(stats) = self.stats.as_mut() {
                // the deepest the Health ever went in this step: what it cost
                // before anything gave it back
                stats.floor = stats.floor.min(health(game));
            }
        } else if player == Some(e.id)
            && matches!(tag, "RESOURCES" | "TEMP_RESOURCES" | "RESOURCES_USED")
        {
            let now = gold(game);
            if self.stats.is_some()
                && let Some(live) = self.gold_live
                && now > live
            {
                let by = self.source(game);
                let stats = self.stats.as_mut().unwrap();
                stats.gold_gain += now - live;
                if by.is_some() {
                    stats.gold_by = by;
                }
            }
            self.gold_live = Some(now);
        }
    }

    /// My hero got Health or Armor back inside a step: remember how much and the
    /// card whose block did it (Soul Rewinder, Ashen Corruptor).
    fn add_restore(&mut self, game: &Game, health: i64) {
        let by = self.source(game);
        if let Some(stats) = self.stats.as_mut() {
            stats.restored += health;
            if by.is_some() {
                stats.restore_by = by;
            }
        }
    }

    /// The free Refresh count changed: remember who put the new ones there, and
    /// drop the ones I have spent (the log keeps only the total).
    fn free_refreshes(&mut self, game: &Game, count: i64) {
        let gain = count - self.free;
        self.free = count;
        if gain > 0 {
            let card = self.source(game);
            self.pool.push(Pool { count: gain, card });
        } else {
            // the log only keeps the total, so the ones I spend come off the
            // oldest grant first
            let mut spend = -gain;
            while spend > 0 && !self.pool.is_empty() {
                let take = spend.min(self.pool[0].count);
                self.pool[0].count -= take;
                spend -= take;
                if self.pool[0].count == 0 {
                    self.pool.remove(0);
                }
            }
        }
        if count == 0 {
            self.pool.clear();
        }
    }

    /// The card behind what is happening now: the innermost open block whose
    /// entity is an effect's own card.
    fn source(&self, game: &Game) -> Option<Json> {
        for (_, id) in self.tracker.blocks().iter().rev() {
            let Some(e) = id.and_then(|id| game.entities.get(&id)) else {
                continue;
            };
            if e.card_id.is_empty() || is_system(&e.card_id) {
                continue;
            }
            if SOURCES.contains(&e.text("CARDTYPE")) {
                return Some(Json::Obj(vec![
                    ("card_id".to_string(), str_(&e.card_id)),
                    ("name".to_string(), str_(self.cards.name(&e.card_id))),
                ]));
            }
        }
        None
    }

    // -- turns --------------------------------------------------------------

    fn set_turn(&mut self, game: &Game, value: i64) {
        if value % 2 == 1 {
            self.upper = 0;
            self.close_turn(game, false);
            self.turn = Some(vec![
                ("turn".to_string(), Json::Int((value + 1) / 2)),
                ("start".to_string(), Json::Null),
                ("actions".to_string(), Json::Arr(Vec::new())),
                ("end".to_string(), Json::Null),
            ]);
            self.ended = false;
        } else {
            self.close_turn(game, true);
        }
    }

    /// A reconnect's full-state dump carries the turn already running, but a
    /// `FULL_ENTITY` tag line fires no `tag_change`, so no `TURN` change opened
    /// it. Open the shop turn the game is in, and mark it: the actions before
    /// the reconnect are not in this log.
    fn join_turn(&mut self, game: &Game) {
        if self.turn.is_some() || !game.game_type.starts_with("GT_BATTLEGROUNDS") {
            return;
        }
        let Some(entity) = game.game_entity() else {
            return;
        };
        let value = entity.int("TURN");
        if value % 2 != 1 {
            return;
        }
        self.upper = 0;
        self.turn = Some(vec![
            ("turn".to_string(), Json::Int((value + 1) / 2)),
            ("joined_mid_turn".to_string(), Json::Bool(true)),
            ("start".to_string(), Json::Null),
            ("actions".to_string(), Json::Arr(Vec::new())),
            ("end".to_string(), Json::Null),
        ]);
        self.ended = false;
    }

    /// Close the turn: the last step, then my hand, board and Gold as they were
    /// when the fight began (`before_fight`), or `None` when the game or the log
    /// ended before one.
    fn close_turn(&mut self, game: &Game, before_fight: bool) {
        if self.turn.is_none() {
            return;
        }
        self.close_step(game);
        let Some(mut turn) = self.turn.take() else {
            return;
        };
        if field(&turn, "start").map(Json::is_null).unwrap_or(true) {
            self.unread.bump("a turn the log gives no decision in");
            return;
        }
        if field(&turn, "actions")
            .and_then(Json::as_arr)
            .map(Vec::is_empty)
            .unwrap_or(true)
        {
            self.unread.bump("a turn I took no step in");
            return;
        }
        let end = self.full_state(game);
        // the game can change things after my last step (an end of turn effect
        // that runs once the turn is over, an effect while I am idle). Those
        // changes belong to the turn, so they are written into its last step and
        // counted, rather than lost.
        if let Some(last) = self.last.clone() {
            let late = self.last_gold - gold(game);
            if late != 0 {
                // Gold the game took with no step of mine (an end of turn
                // effect, a trinket, a hero power that pays by itself)
                if let Some(step) = actions_last(&mut turn) {
                    put(step, "late_gold", Json::Int(late));
                }
                self.unread
                    .bump("Gold the game took with no step of mine");
            }
            if !self.same(&last, &end) {
                if let Some(step) = actions_last(&mut turn) {
                    self.diff(step, &last, &end, &HashSet::new());
                }
                self.unread.bump("a change after my last step");
            }
        }
        self.last = Some(end.clone());
        self.last_gold = gold(game);
        self.last_health = health(game);
        // the turn's end is the state the last step left, with the changes the
        // game made after it: the next turn starts from the same shape
        if let Some(step) = actions_last(&mut turn) {
            set(step, "after", end.clone());
        }
        set(&mut turn, "end", if before_fight { end } else { Json::Null });
        if let Some(object) = self.record.as_obj_mut()
            && let Some(Json::Arr(turns)) = field_mut(object, "turns")
        {
            turns.push(Json::Obj(turn));
        }
        self.turn = None;
    }

    /// Whether two states hold the same cards, numbers and Tavern.
    fn same(&self, a: &Json, b: &Json) -> bool {
        for part in ["hand", "board", "tavern"] {
            let (left, right) = (parts(a, part), parts(b, part));
            if left.len() != right.len() {
                return false;
            }
            for (x, y) in left.iter().zip(right) {
                if self.key(x) != self.key(y) {
                    return false;
                }
            }
        }
        let hidden = |state: &Json| {
            state
                .as_obj()
                .map(|o| number_in(o, "hand_hidden"))
                .unwrap_or(0)
        };
        hidden(a) == hidden(b)
    }

    fn key(&self, card: &Json) -> CardKey {
        let object = card.as_obj().unwrap();
        let opt_int = |key: &str| field(object, key).and_then(Json::as_int);
        CardKey {
            id: number_in(object, "id"),
            card_id: field(object, "card_id")
                .and_then(Json::as_str)
                .map(str::to_string),
            attack: opt_int("attack"),
            health: opt_int("health"),
            golden: match field(object, "golden") {
                Some(Json::Bool(b)) => Some(*b),
                _ => None,
            },
            keywords: field(object, "keywords").cloned(),
            dark_gifts: field(object, "dark_gifts").cloned(),
            frozen: match field(object, "frozen") {
                Some(Json::Bool(b)) => Some(*b),
                _ => None,
            },
        }
    }

    // -- my decisions -------------------------------------------------------

    fn options_line(&mut self, game: &Game, body: &str) {
        if self.options.read(body) {
            self.decision(game);
            self.options.clear();
        }
    }

    /// The game is asking me what to do: close the last step and, in a new turn,
    /// take the state I was first offered.
    fn decision(&mut self, game: &Game) {
        self.join_turn(game);
        self.close_step(game);
        let waiting = self
            .turn
            .as_ref()
            .map(|turn| field(turn, "start").map(Json::is_null).unwrap_or(true))
            .unwrap_or(false);
        if waiting {
            self.begin(game);
        }
    }

    /// The turn's start: the state the client first shows me, which is also what
    /// the first step of the turn is measured against.
    fn begin(&mut self, game: &Game) {
        let mut state = self.full_state(game);
        if let Some(object) = state.as_obj_mut() {
            put(object, "lobby", self.lobby_state(game));
        }
        if let Some(turn) = self.turn.as_mut() {
            set(turn, "start", state.clone());
        }
        self.last = Some(state);
        self.last_gold = gold(game);
        self.last_health = health(game);
    }

    fn send_option(&mut self, game: &Game, body: &str) {
        let Some((index, sub, target)) = taken(body) else {
            return;
        };
        self.join_turn(game);
        self.close_step(game);
        if self.turn.is_none() {
            self.unread.bump("a step outside a shop turn");
            return;
        }
        let waiting = self
            .turn
            .as_ref()
            .map(|turn| field(turn, "start").map(Json::is_null).unwrap_or(true))
            .unwrap_or(false);
        if waiting {
            self.begin(game);
        }
        if self.options.get(index).is_none() {
            self.unread.bump("a step the options dump does not list");
            self.open_step(
                game,
                vec![
                    ("step".to_string(), str_("other")),
                    (
                        "detail".to_string(),
                        str_("not in the options dump"),
                    ),
                ],
                None,
            );
            return;
        }
        let offer = self.options.get(index).unwrap();
        let entity = offer.entity.as_ref().map(|r| r.id);
        let at = self.target_of(game, offer, target);
        let mut step = self.classify(game, entity, &at, offer);
        put(&mut step, "option", Json::Int(index));
        if sub >= 0 {
            put(&mut step, "sub_option", Json::Int(sub));
        }
        self.open_step(game, step, entity);
    }

    fn target_of(&self, game: &Game, offer: &Offer, target: i64) -> Json {
        if target == 0 {
            return Json::Null;
        }
        let r = match offer.targets.get(&target) {
            Some(r) => r.clone(),
            None => Ref::only(target as i32),
        };
        self.where_(game, Some(&r))
    }

    /// An entity named by where it sits: my board, the Tavern, my hand or my
    /// hero. Read from the live table by id, since an options dump's zones and
    /// positions can lag a step behind.
    fn where_(&self, game: &Game, r: Option<&Ref>) -> Json {
        let Some(r) = r else {
            return Json::Null;
        };
        let e = if r.id != 0 {
            game.entities.get(&r.id)
        } else {
            None
        };
        let Some(e) = e else {
            let mut out = vec![
                ("id".to_string(), Json::Int(r.id as i64)),
                (
                    "card_id".to_string(),
                    if r.card_id.is_empty() {
                        Json::Null
                    } else {
                        str_(&r.card_id)
                    },
                ),
                (
                    "name".to_string(),
                    if r.card_id.is_empty() {
                        Json::Null
                    } else {
                        str_(self.cards.name(&r.card_id))
                    },
                ),
                ("kind".to_string(), str_(&lower(&r.zone, "unknown"))),
            ];
            if r.pos != 0 {
                put(&mut out, "position", Json::Int(r.pos));
            }
            return Json::Obj(out);
        };
        let mut out = vec![
            ("id".to_string(), Json::Int(e.id as i64)),
            (
                "card_id".to_string(),
                if e.card_id.is_empty() {
                    Json::Null
                } else {
                    str_(&e.card_id)
                },
            ),
            (
                "name".to_string(),
                if e.card_id.is_empty() {
                    Json::Null
                } else {
                    str_(self.cards.name(&e.card_id))
                },
            ),
        ];
        let zone = e.text("ZONE");
        let ct = e.text("CARDTYPE");
        let controller = e.int("CONTROLLER");
        let mine = me(game).map(|m| m as i64);
        let theirs = dummy(game).map(|d| d as i64);
        if zone == "PLAY" && Some(controller) == mine {
            if ct == "HERO" {
                put(&mut out, "kind", str_("hero"));
            } else if ct == "MINION" {
                put(&mut out, "kind", str_("board"));
                put(&mut out, "position", Json::Int(e.int("ZONE_POSITION")));
            } else {
                put(&mut out, "kind", str_(&lower(ct, "play")));
            }
        } else if zone == "PLAY" && Some(controller) == theirs {
            put(&mut out, "kind", str_("slot"));
            put(&mut out, "slot", Json::Int(e.int("ZONE_POSITION")));
        } else if zone == "HAND" && Some(controller) == mine {
            put(&mut out, "kind", str_("hand"));
            put(&mut out, "index", Json::Int(e.int("ZONE_POSITION")));
        } else {
            put(&mut out, "kind", str_(&lower(zone, "unknown")));
        }
        Json::Obj(out)
    }

    /// Name a step from the option I picked. The card is read from the live
    /// table: an option's own zone and position can lag one step behind.
    fn classify(&self, game: &Game, entity: Option<i32>, at: &Json, offer: &Offer) -> Obj {
        let e = entity.and_then(|id| game.entities.get(&id));
        let cid = e.map(|e| e.card_id.as_str()).unwrap_or("");
        let ct = e.map(|e| e.text("CARDTYPE")).unwrap_or("");
        let mine = me(game).map(|m| m as i64);
        let theirs = dummy(game).map(|d| d as i64);
        if offer.kind == "END_TURN" {
            return step_of("end_turn");
        }
        if DRAG_BUY.contains(&cid) {
            let mut step = vec![
                ("step".to_string(), str_("buy")),
                ("target".to_string(), at.clone()),
            ];
            if cid.ends_with("_Spell") {
                put(&mut step, "spell", Json::Bool(true));
            }
            return step;
        }
        if cid == SELL {
            return vec![
                ("step".to_string(), str_("sell")),
                ("target".to_string(), at.clone()),
            ];
        }
        if cid == ROLL {
            return step_of("roll");
        }
        if cid == FREEZE {
            return step_of("freeze");
        }
        if cid.starts_with(LEVEL) {
            return step_of("level");
        }
        if cid == DARK {
            return step_of("dark_discovery");
        }
        let Some(e) = e else {
            return vec![
                ("step".to_string(), str_("other")),
                ("detail".to_string(), str_("an option with no card")),
            ];
        };
        let zone = e.text("ZONE");
        let controller = e.int("CONTROLLER");
        if zone == "PLAY" && Some(controller) == theirs && SHOP_TYPES.contains(&ct) {
            // a click on the minion itself buys it, as dragging does
            let mut step = vec![
                ("step".to_string(), str_("buy")),
                ("target".to_string(), self.where_(game, Some(&Ref::only(e.id)))),
            ];
            if ct == "BATTLEGROUND_SPELL" {
                put(&mut step, "spell", Json::Bool(true));
            }
            return step;
        }
        if ct == "HERO_POWER" && Some(controller) == mine {
            let second = e.int("ADDITIONAL_HERO_POWER_INDEX") != 0;
            return vec![
                ("step".to_string(), str_("hero_power")),
                (
                    "which".to_string(),
                    str_(if second { "second" } else { "main" }),
                ),
                ("target".to_string(), at.clone()),
            ];
        }
        if zone == "HAND" && Some(controller) == mine {
            return vec![
                ("step".to_string(), str_("play")),
                ("target".to_string(), at.clone()),
            ];
        }
        if zone == "PLAY" && Some(controller) == mine {
            // a minion, a trinket or a reward of mine: Activate or drag, which
            // the block that follows says
            return vec![
                ("step".to_string(), str_("board")),
                ("target".to_string(), at.clone()),
            ];
        }
        let mut step = vec![
            ("step".to_string(), str_("other")),
            (
                "detail".to_string(),
                str_("an option the script does not name"),
            ),
        ];
        if !ct.is_empty() {
            put(&mut step, "card_type", str_(ct));
        } else {
            put(&mut step, "card_type", Json::Null);
        }
        put(&mut step, "target", at.clone());
        step
    }

    fn choice_line(&mut self, game: &Game, body: &str) {
        if let Some(start) = choice_start(body) {
            if !start.general {
                // the mulligan's hero picks and cards
                self.choice = None;
                return;
            }
            self.decision(game);
            self.choice = Some(Choice::default());
            return;
        }
        let Some(choice) = self.choice.as_mut() else {
            return;
        };
        choice.read(body);
    }

    fn send_choices(&mut self, game: &Game, body: &str) {
        if pick_start(body).is_some() {
            self.join_turn(game);
            self.close_step(game);
            if self.choice.is_none() || self.turn.is_none() {
                return;
            }
            let waiting = self
                .turn
                .as_ref()
                .map(|turn| field(turn, "start").map(Json::is_null).unwrap_or(true))
                .unwrap_or(false);
            if waiting {
                self.begin(game);
            }
            let step = self.choice_step(game);
            self.open_step(game, step, None);
            return;
        }
        if let Some(choice) = self.choice.as_mut() {
            choice.read_pick(body);
        }
    }

    /// The step a choice is written as. Which one it is comes from what the game
    /// says offered it: a trinket slot, the Dark Discovery, a triple's reward, or
    /// a card's own Discover or Choose One.
    fn choice_step(&self, game: &Game) -> Obj {
        let src = self.choice.as_ref().and_then(|c| c.source.clone());
        let cid = src.as_ref().map(|r| r.card_id.as_str()).unwrap_or("");
        let e = src
            .as_ref()
            .filter(|r| r.id != 0)
            .and_then(|r| game.entities.get(&r.id));
        let options: Vec<Json> = self
            .choice
            .as_ref()
            .map(|c| {
                c.entities
                    .values()
                    .map(|r| self.card_of(game, Some(r)))
                    .collect()
            })
            .unwrap_or_default();
        let mut step = vec![
            ("step".to_string(), str_("discover")),
            ("source".to_string(), self.card_of(game, src.as_ref())),
            ("options".to_string(), Json::Arr(options)),
        ];
        if let Some((_, slot)) = TRINKET_SLOTS.iter().find(|(card, _)| *card == cid) {
            set(&mut step, "step", str_("trinket_pick"));
            put(&mut step, "slot_kind", str_(slot));
        } else if cid == DARK_DISCOVERY {
            put(&mut step, "kind", str_("dark_gift"));
        } else if cid == TRIPLE {
            put(&mut step, "kind", str_("triple_reward"));
        } else if e.map(|e| e.int("CHOOSE_ONE") != 0).unwrap_or(false) {
            put(&mut step, "kind", str_("choose_one"));
        } else {
            put(&mut step, "kind", str_("discover"));
        }
        step
    }

    /// A card as a step names it: id, name, and what else the log shows of it
    /// (its Dark Gift, its stats, its price).
    fn card_of(&self, game: &Game, r: Option<&Ref>) -> Json {
        let Some(r) = r else {
            return Json::Null;
        };
        let e = if r.id != 0 {
            game.entities.get(&r.id)
        } else {
            None
        };
        let mut out = vec![
            ("id".to_string(), Json::Int(r.id as i64)),
            (
                "card_id".to_string(),
                if r.card_id.is_empty() {
                    Json::Null
                } else {
                    str_(&r.card_id)
                },
            ),
            (
                "name".to_string(),
                if r.card_id.is_empty() {
                    Json::Null
                } else {
                    str_(self.cards.name(&r.card_id))
                },
            ),
        ];
        let Some(e) = e else {
            return Json::Obj(out);
        };
        let mut own = gifts_by_minion(game).get(&e.id).cloned().unwrap_or_default();
        let pointed = e.int("DARK_GIFT_ENTITY");
        if pointed != 0
            && let Some(pointed) = game.entities.get(&(pointed as i32))
            && !pointed.card_id.is_empty()
            && !own.contains(&pointed.card_id)
        {
            own.insert(0, pointed.card_id.clone());
        }
        if !own.is_empty() {
            let gifts: Vec<Json> = own
                .iter()
                .map(|card| {
                    Json::Obj(vec![
                        ("card_id".to_string(), str_(card)),
                        ("name".to_string(), str_(self.cards.name(card))),
                    ])
                })
                .collect();
            put(&mut out, "dark_gifts", Json::Arr(gifts));
        }
        if e.text("CARDTYPE") == "MINION" {
            let m = plain_minion(game, &self.cards, e);
            put(&mut out, "attack", Json::Int(m.attack));
            put(&mut out, "health", Json::Int(m.health));
            put(&mut out, "golden", Json::Bool(m.golden));
            put(
                &mut out,
                "keywords",
                Json::Arr(m.keywords.iter().map(|k| str_(k)).collect()),
            );
            if let Some(sd) = minion_numbers(&m.tags) {
                put(&mut out, "numbers", numbers_json(&sd));
            }
        }
        if e.int("COST") != 0 {
            put(&mut out, "cost", Json::Int(e.int("COST")));
        }
        Json::Obj(out)
    }

    // -- steps --------------------------------------------------------------

    /// Open a step: the card it acts on, the Gold and Health it starts from, and
    /// the state it starts from.
    fn open_step(&mut self, game: &Game, mut step: Obj, entity: Option<i32>) {
        // a step is measured against the state the last decision saw, so that
        // nothing that happens between two decisions is lost
        let before = match self.last.clone() {
            Some(before) => before,
            None => Json::Obj(self.state(game)),
        };
        let mut ent = entity;
        if matches!(text_in(&step, "step"), "buy" | "sell")
            && let Some(target) = field(&step, "target")
            && let Some(object) = target.as_obj()
        {
            ent = field(object, "id").and_then(Json::as_int).map(|n| n as i32);
        }
        put(&mut step, "paid", Json::Null);
        let starts_health = if self.last.is_some() {
            self.last_health
        } else {
            health(game)
        };
        let starts_gold = if self.last.is_some() {
            self.last_gold
        } else {
            gold(game)
        };
        let hero = hero_entity(game);
        self.damage = hero.map(|hero| hero.int("DAMAGE"));
        self.armor_now = hero.map(|hero| hero.int("ARMOR"));
        self.gold_live = Some(gold(game));
        self.stats = Some(Stats {
            floor: starts_health,
            restored: 0,
            restore_by: None,
            gold_gain: 0,
            gold_by: None,
        });
        let step_name = text_in(&step, "step").to_string();
        if let Some(ent) = ent
            && matches!(
                step_name.as_str(),
                "buy" | "play" | "sell" | "activate" | "board"
            )
        {
            for (key, value) in self.place(ent, &before) {
                set(&mut step, &key, value);
            }
        }
        if COSTED.contains(&step_name.as_str())
            && let Some(ent) = ent.filter(|id| *id != 0)
            && let Some(e) = game.entities.get(&ent)
        {
            // a minion's COST tag is not what the Tavern charges for it (that is
            // 3 Gold by the game's rule), so only a spell's price is read here
            let spell = field(&step, "spell").is_some();
            if e.int("COST") != 0 && (step_name != "buy" || spell) {
                put(&mut step, "cost", Json::Int(e.int("COST")));
            }
            if step_name == "roll" && e.int("BACON_FREE_REFRESH_COUNT") != 0 {
                // a reroll from a pool of free ones: the button keeps its price
                // but takes nothing
                put(
                    &mut step,
                    "free",
                    Json::Int(e.int("BACON_FREE_REFRESH_COUNT")),
                );
            }
        }
        let pool = self.pool_cards();
        self.pending = Some(Pending {
            step,
            entity: ent,
            gold: starts_gold,
            health: starts_health,
            before,
            free: self.free,
            pool,
        });
    }

    /// Where the card a step acts on sits, from the state the step starts from: a
    /// Tavern slot for a buy, a hand index or a board position.
    fn place(&self, ent: i32, state: &Json) -> Vec<(String, Json)> {
        for (key, place) in [
            ("tavern", "slot"),
            ("hand", "index"),
            ("board", "position"),
        ] {
            for card in parts(state, key) {
                let Some(card) = card.as_obj() else {
                    continue;
                };
                if number_in(card, "id") != ent as i64 {
                    continue;
                }
                let mut out = vec![(place.to_string(), Json::Int(number_in(card, place)))];
                let mut brief = self.card_brief(card);
                if key == "tavern"
                    && let Some(cost) = field(card, "cost")
                {
                    put(&mut brief, "cost", cost.clone());
                }
                out.push(("card".to_string(), Json::Obj(brief)));
                return out;
            }
        }
        Vec::new()
    }

    /// Where a card sits in a state: its hand index, its board position, its
    /// Tavern slot, or off when it is in none of the three (sold, consumed, or a
    /// spell that is done).
    fn moved_to(&self, ent: i32, state: &Json) -> Obj {
        for (key, place) in [
            ("hand", "index"),
            ("board", "position"),
            ("tavern", "slot"),
        ] {
            for card in parts(state, key) {
                let Some(card) = card.as_obj() else {
                    continue;
                };
                if number_in(card, "id") == ent as i64 {
                    return vec![
                        ("where".to_string(), str_(key)),
                        (place.to_string(), Json::Int(number_in(card, place))),
                    ];
                }
            }
        }
        vec![("where".to_string(), str_("off"))]
    }

    /// Where a pick that took a slot went: the one card of that card id the step
    /// added to my hand, my board or my Tavern, with the entity the game made it
    /// as.
    fn landed(&self, card_id: &str, before: &Json, after: &Json) -> Obj {
        if card_id.is_empty() {
            return vec![("where".to_string(), str_("off"))];
        }
        for (key, place) in [
            ("hand", "index"),
            ("board", "position"),
            ("tavern", "slot"),
        ] {
            let had: HashSet<i64> = parts(before, key)
                .iter()
                .filter_map(|c| c.as_obj())
                .map(|c| number_in(c, "id"))
                .collect();
            for card in parts(after, key) {
                let Some(card) = card.as_obj() else {
                    continue;
                };
                if had.contains(&number_in(card, "id")) {
                    continue;
                }
                if field(card, "card_id").and_then(Json::as_str) != Some(card_id) {
                    continue;
                }
                return vec![
                    ("where".to_string(), str_(key)),
                    (place.to_string(), Json::Int(number_in(card, place))),
                    ("id".to_string(), Json::Int(number_in(card, "id"))),
                ];
            }
        }
        vec![("where".to_string(), str_("off"))]
    }

    fn card_brief(&self, card: &Obj) -> Obj {
        let mut out = Vec::with_capacity(8);
        for key in ["id", "card_id", "name"] {
            put(&mut out, key, field(card, key).cloned().unwrap_or(Json::Null));
        }
        for key in ["attack", "health", "golden", "dark_gifts", "numbers"] {
            if let Some(value) = field(card, key) {
                put(&mut out, key, value.clone());
            }
        }
        out
    }

    fn close_step(&mut self, game: &Game) {
        let Some(mut pending) = self.pending.take() else {
            return;
        };
        let Some(mut turn) = self.turn.take() else {
            return;
        };
        let mut after = self.full_state(game);
        // A Golden minion played from hand counts for "Golden minions you
        // played this game" (Hooktusk, Master Marauder; the log shows no
        // number for it). The state the step left carries the count, so the
        // increment goes in before the state is put on the step. A Magnetic
        // minion that merged into a host is a Magnetization, not a play, and
        // stands on no board afterwards.
        let golden_play = text_in(&pending.step, "step") == "play"
            && field(&pending.step, "card")
                .and_then(Json::as_obj)
                .and_then(|c| field(c, "golden"))
                == Some(&Json::Bool(true))
            && pending.entity.is_some_and(|id| {
                parts(&after, "board")
                    .iter()
                    .any(|m| m.as_obj().is_some_and(|m| number_in(m, "id") == id as i64))
            });
        if golden_play {
            self.goldens += 1;
            if let Some(counters) = after.as_obj_mut().and_then(|o| field_mut(o, "counters"))
                && let Some(counters) = counters.as_obj_mut()
            {
                set(counters, "golden_played", Json::Int(self.goldens));
            }
        }
        self.last = Some(after.clone());
        self.last_gold = gold(game);
        self.last_health = health(game);
        let mut step = mem::take(&mut pending.step);
        // the state the step left, which is what the replay loads before the
        // next one and compares this one's result with
        put(&mut step, "after", after.clone());
        put(
            &mut step,
            "paid",
            Json::Obj(vec![
                (
                    "gold".to_string(),
                    Json::Int(pending.gold - gold(game)),
                ),
                (
                    "health".to_string(),
                    Json::Int(pending.health - health(game)),
                ),
            ]),
        );
        self.finish_watch(&mut step, &pending);
        if text_in(&step, "step") == "other"
            && field(&step, "target").map(Json::is_null).unwrap_or(true)
        {
            drop_key(&mut step, "target");
        }
        if let Some(ent) = pending.entity {
            self.move_of(&mut step, ent, &pending.before, &after);
        }
        let kind = text_in(&step, "step").to_string();
        if kind == "discover" || kind == "trinket_pick" {
            let picked = self.choice.as_ref().and_then(|c| c.pick.clone());
            put(&mut step, "picked", self.card_of(game, picked.as_ref()));
            match picked {
                None => self.unread.bump("a pick the log does not show"),
                Some(picked) => {
                    // where the card I picked went: my hand, my board, or a
                    // trinket slot, which is neither. A pick that takes a slot
                    // is a new entity with the picked card's id, so it is found
                    // by what the step added, not by the offer's entity id.
                    let mut now = self.moved_to(picked.id, &after);
                    if where_of(&now) == "off" && kind == "discover" {
                        now = self.landed(&picked.card_id, &pending.before, &after);
                    }
                    let to = if where_of(&now) == "off" {
                        Json::Null
                    } else {
                        Json::Obj(now)
                    };
                    put(
                        &mut step,
                        "moved",
                        Json::Obj(vec![
                            ("id".to_string(), Json::Int(picked.id as i64)),
                            (
                                "from".to_string(),
                                Json::Obj(vec![("where".to_string(), str_("offer"))]),
                            ),
                            ("to".to_string(), to),
                        ]),
                    );
                }
            }
            self.choice = None;
        } else if kind == "freeze" {
            put(&mut step, "on", Json::Bool(self.frozen(game)));
        } else if kind == "roll" {
            let tavern = after
                .as_obj()
                .and_then(|o| field(o, "tavern"))
                .cloned()
                .unwrap_or(Json::Arr(Vec::new()));
            put(&mut step, "tavern", tavern);
        } else if kind == "level" {
            let tier = my_player(game).map(|p| p.int("PLAYER_TECH_LEVEL"));
            put(&mut step, "tier_after", Json::Int(tier.unwrap_or(0)));
        } else if kind == "move" {
            let moved: Vec<&Json> = parts(&after, "board")
                .iter()
                .filter(|c| {
                    c.as_obj()
                        .map(|c| number_in(c, "id"))
                        .unwrap_or(0)
                        == pending.entity.map(|id| id as i64).unwrap_or(-1)
                })
                .collect();
            let position = moved
                .first()
                .and_then(|c| c.as_obj())
                .map(|c| number_in(c, "position"));
            if moved.is_empty() {
                self.unread.bump("a move the log does not finish");
            } else if position == field(&step, "position").and_then(Json::as_int) {
                let canceled = number_in(&turn, "canceled") + 1;
                set(&mut turn, "canceled", Json::Int(canceled));
                self.turn = Some(turn);
                return;
            } else {
                put(&mut step, "to", Json::Int(position.unwrap_or(0)));
            }
        }
        let acting = self.acting(&pending, &step);
        self.diff(&mut step, &pending.before, &after, &acting);
        if text_in(&step, "step") == "board" && self.changed(&step) {
            self.unread
                .bump("a click on my own board that no block explains");
            set(&mut step, "step", str_("other"));
            set(
                &mut step,
                "detail",
                str_("a click on my own board that no block explains"),
            );
        }
        if text_in(&step, "step") == "board" || self.canceled(&step) {
            let canceled = number_in(&turn, "canceled") + 1;
            set(&mut turn, "canceled", Json::Int(canceled));
            self.turn = Some(turn);
            return;
        }
        if let Some(Json::Arr(actions)) = field_mut(&mut turn, "actions") {
            actions.push(Json::Obj(step));
        }
        self.turn = Some(turn);
    }

    /// Whether a step took or changed anything at all.
    fn changed(&self, step: &Obj) -> bool {
        let paid = field(step, "paid").and_then(Json::as_obj);
        let (gold, health) = match paid {
            Some(paid) => (number_in(paid, "gold"), number_in(paid, "health")),
            None => (0, 0),
        };
        if gold != 0 || health != 0 {
            return true;
        }
        [
            "entered",
            "left",
            "changed",
            "tavern",
            "hand_hidden",
            "picked",
            "moved",
        ]
        .iter()
        .any(|key| field(step, key).is_some())
    }

    /// A click that took nothing and changed nothing: picking a minion up and
    /// putting it back, inspecting one, or the like.
    fn canceled(&self, step: &Obj) -> bool {
        let kind = text_in(step, "step");
        if kind == "buy" {
            let to = field(step, "moved")
                .and_then(Json::as_obj)
                .and_then(|moved| field(moved, "to"))
                .filter(|to| !to.is_null());
            let where_ = to
                .and_then(Json::as_obj)
                .map(where_of)
                .unwrap_or("off");
            if where_ == "hand" || field(step, "entered").is_some() {
                return false; // it reached my hand, or a triple took it
            }
            let free = || {
                let paid = field(step, "paid").and_then(Json::as_obj);
                match paid {
                    Some(paid) => {
                        number_in(paid, "gold") == 0 && number_in(paid, "health") == 0
                    }
                    None => true,
                }
            };
            return free() && field(step, "gold_gain").is_none();
        }
        if !matches!(
            kind,
            "sell" | "play" | "activate" | "discover" | "trinket_pick" | "dark_discovery"
        ) {
            return false;
        }
        !self.changed(step)
    }

    /// Write what the step did besides take Gold and spend it: the Health it
    /// cost and what gave it back, the Gold it gained, the free Refreshes it
    /// used, each with the card the log names.
    fn finish_watch(&mut self, step: &mut Obj, pending: &Pending) {
        let stats = match self.stats.clone() {
            Some(stats) => stats,
            None => Stats {
                floor: pending.health,
                ..Stats::default()
            },
        };
        let net = field(step, "paid")
            .and_then(Json::as_obj)
            .map(|paid| number_in(paid, "health"))
            .unwrap_or(0);
        let peak = pending.health - stats.floor;
        let given_back = peak - net.max(0);
        if peak > 0 && given_back > 0 {
            // Health went down and came back: what it cost, and what gave it
            // back (Soul Rewinder, Ashen Corruptor)
            put(step, "health_cost", Json::Int(peak));
            put(
                step,
                "rewound",
                Json::Obj(vec![
                    ("health".to_string(), Json::Int(given_back)),
                    (
                        "by".to_string(),
                        stats.restore_by.clone().unwrap_or(Json::Null),
                    ),
                ]),
            );
            if stats.restore_by.is_none() {
                self.unread
                    .bump("Health given back by no card I can name");
            }
        }
        if stats.gold_gain != 0 && text_in(step, "step") != "sell" {
            put(
                step,
                "gold_gain",
                Json::Obj(vec![
                    ("gold".to_string(), Json::Int(stats.gold_gain)),
                    (
                        "by".to_string(),
                        stats.gold_by.clone().unwrap_or(Json::Null),
                    ),
                ]),
            );
        }
        if self.free != pending.free {
            put(
                step,
                "free_refreshes",
                Json::Obj(vec![
                    ("before".to_string(), Json::Int(pending.free)),
                    ("after".to_string(), Json::Int(self.free)),
                ]),
            );
            let free_roll = field(step, "paid")
                .and_then(Json::as_obj)
                .map(|paid| number_in(paid, "gold") == 0)
                .unwrap_or(false);
            if text_in(step, "step") == "roll" && free_roll && !pending.pool.is_empty() {
                put(step, "free_from", Json::Arr(pending.pool.clone()));
            }
        }
        self.stats = None;
    }

    /// The cards that put the free Refreshes I hold in the pool, in order.
    fn pool_cards(&self) -> Vec<Json> {
        let mut cards: Vec<Json> = Vec::new();
        for grant in &self.pool {
            if let Some(card) = &grant.card
                && !cards.contains(card)
            {
                cards.push(card.clone());
            }
        }
        cards
    }

    /// Where the card the step acted on went, when it went anywhere.
    fn move_of(&self, step: &mut Obj, ent: i32, before: &Json, after: &Json) {
        let was = self.moved_to(ent, before);
        let now = self.moved_to(ent, after);
        if Json::Obj(was.clone()) == Json::Obj(now.clone()) {
            return;
        }
        let to = if where_of(&now) == "off" {
            Json::Null
        } else {
            Json::Obj(now)
        };
        put(
            step,
            "moved",
            Json::Obj(vec![
                ("id".to_string(), Json::Int(ent as i64)),
                ("from".to_string(), Json::Obj(was)),
                ("to".to_string(), to),
            ]),
        );
    }

    /// The entities this step itself acted on, which the diff leaves out of what
    /// it reports: the card bought or sold, the card played, the card picked.
    fn acting(&self, pending: &Pending, step: &Obj) -> HashSet<i32> {
        let mut ids = HashSet::new();
        if let Some(entity) = pending.entity.filter(|id| *id != 0) {
            ids.insert(entity);
        }
        if let Some(picked) = field(step, "picked").and_then(Json::as_obj)
            && let Some(id) = field(picked, "id").and_then(Json::as_int)
        {
            ids.insert(id as i32);
        }
        ids
    }

    /// What changed between two states, written into the step.
    fn diff(&mut self, step: &mut Obj, before: &Json, after: &Json, acting: &HashSet<i32>) {
        let mut b_all: Vec<(i64, &Json)> = Vec::new();
        for part in ["board", "hand"] {
            for card in parts(before, part) {
                let id = card.as_obj().map(|c| number_in(c, "id")).unwrap_or(0);
                if !b_all.iter().any(|(have, _)| *have == id) {
                    b_all.push((id, card));
                }
            }
        }
        let mut a_all: Vec<(i64, &Json)> = Vec::new();
        for part in ["board", "hand"] {
            for card in parts(after, part) {
                let id = card.as_obj().map(|c| number_in(c, "id")).unwrap_or(0);
                if !a_all.iter().any(|(have, _)| *have == id) {
                    a_all.push((id, card));
                }
            }
        }
        let ids = |state: &Json, part: &str| -> HashSet<i64> {
            parts(state, part)
                .iter()
                .filter_map(|c| c.as_obj())
                .map(|c| number_in(c, "id"))
                .collect()
        };
        let b_hand = ids(before, "hand");
        let a_hand = ids(after, "hand");
        let b_ids: HashSet<i64> = b_all.iter().map(|(id, _)| *id).collect();
        let a_ids: HashSet<i64> = a_all.iter().map(|(id, _)| *id).collect();
        let mut entered: Vec<Json> = Vec::new();
        for (id, card) in &a_all {
            if b_ids.contains(id) || acting.contains(&(*id as i32)) {
                continue;
            }
            let c = card.as_obj().unwrap();
            let mut d = self.brief(c, if a_hand.contains(id) { "hand" } else { "board" });
            if field(&d, "from").is_none() {
                let slot = parts(before, "tavern")
                    .iter()
                    .filter_map(|t| t.as_obj())
                    .find(|t| number_in(t, "id") == *id)
                    .map(|t| number_in(t, "slot"));
                match slot {
                    Some(slot) => put(
                        &mut d,
                        "stolen",
                        Json::Obj(vec![
                            ("where".to_string(), str_("tavern")),
                            ("slot".to_string(), Json::Int(slot)),
                        ]),
                    ),
                    None => self.unread.bump("a card came in with no card behind it"),
                }
            }
            entered.push(Json::Obj(d));
        }
        let mut left: Vec<Json> = Vec::new();
        for (id, card) in &b_all {
            if a_ids.contains(id) || acting.contains(&(*id as i32)) {
                continue;
            }
            let c = card.as_obj().unwrap();
            left.push(Json::Obj(
                self.brief(c, if b_hand.contains(id) { "hand" } else { "board" }),
            ));
        }
        if !entered.is_empty() {
            put(step, "entered", Json::Arr(entered));
        }
        if !left.is_empty() {
            put(step, "left", Json::Arr(left));
        }
        let mut changed = self.moved(before, after, "board", "position");
        changed.extend(self.moved(before, after, "hand", "index"));
        if !changed.is_empty() {
            put(step, "changed", Json::Arr(changed));
        }
        let tavern = |state: &Json| -> Vec<Json> {
            parts(state, "tavern")
                .iter()
                .map(|card| Json::Obj(json_of(&self.key(card))))
                .collect()
        };
        if tavern(before) != tavern(after) {
            put(
                step,
                "tavern",
                field(after.as_obj().unwrap(), "tavern")
                    .cloned()
                    .unwrap_or(Json::Arr(Vec::new())),
            );
        }
        let hidden = |state: &Json| -> i64 {
            state
                .as_obj()
                .map(|o| number_in(o, "hand_hidden"))
                .unwrap_or(0)
        };
        if hidden(before) != hidden(after) {
            put(&mut *step, "hand_hidden", Json::Int(hidden(after)));
        }
    }

    fn brief(&self, card: &Obj, where_: &str) -> Obj {
        let mut out = vec![
            ("id".to_string(), Json::Int(number_in(card, "id"))),
            ("where".to_string(), str_(where_)),
            (
                "card_id".to_string(),
                field(card, "card_id").cloned().unwrap_or(Json::Null),
            ),
            (
                "name".to_string(),
                field(card, "name").cloned().unwrap_or(Json::Null),
            ),
        ];
        if where_ == "board" {
            put(
                &mut out,
                "position",
                Json::Int(number_in(card, "position")),
            );
        } else {
            put(&mut out, "index", Json::Int(number_in(card, "index")));
        }
        if let Some(from) = field(card, "from") {
            put(&mut out, "from", from.clone());
        }
        out
    }

    /// The cards that changed between two states of one part.
    fn moved(&self, before: &Json, after: &Json, part: &str, place: &str) -> Vec<Json> {
        let old: Vec<(i64, &Obj)> = parts(before, part)
            .iter()
            .filter_map(|c| c.as_obj())
            .map(|c| (number_in(c, "id"), c))
            .collect();
        let mut out = Vec::new();
        for card in parts(after, part) {
            let Some(card) = card.as_obj() else {
                continue;
            };
            let id = number_in(card, "id");
            let Some((_, was)) = old.iter().find(|(have, _)| *have == id) else {
                continue;
            };
            let pick = |object: &Obj| -> Vec<Json> {
                KEYS.iter()
                    .map(|key| field(object, key).cloned().unwrap_or(Json::Null))
                    .collect()
            };
            let (before_keys, after_keys) = (pick(was), pick(card));
            if before_keys == after_keys {
                continue;
            }
            out.push(Json::Obj(vec![
                (place.to_string(), Json::Int(number_in(card, place))),
                (
                    "card_id".to_string(),
                    field(card, "card_id").cloned().unwrap_or(Json::Null),
                ),
                (
                    "name".to_string(),
                    field(card, "name").cloned().unwrap_or(Json::Null),
                ),
                ("was".to_string(), Json::Obj(named(KEYS, before_keys))),
                ("now".to_string(), Json::Obj(named(KEYS, after_keys))),
            ]));
        }
        out
    }

    // -- the states ---------------------------------------------------------

    /// A turnover point of the shop phase as the replay loads it.
    fn full_state(&mut self, game: &Game) -> Json {
        let mut state = self.state(game);
        let player = my_player(game);
        put(&mut state, "gold", Json::Int(gold(game)));
        put(
            &mut state,
            "gold_max",
            player.map_or(Json::Null, |p| Json::Int(p.int("RESOURCES"))),
        );
        put(
            &mut state,
            "tier",
            player.map_or(Json::Null, |p| Json::Int(p.int("PLAYER_TECH_LEVEL"))),
        );
        put(&mut state, "upgrade_cost", upgrade_cost(game));
        put(&mut state, "health", Json::Int(hero_health(game)));
        put(&mut state, "armor", Json::Int(armor(game)));
        let powers = self.hero_powers(game);
        put(&mut state, "hero_powers", powers);
        let trinkets = self.trinket_list(game);
        put(&mut state, "trinkets", trinkets);
        put(&mut state, "free_refreshes", Json::Int(self.free));
        let fodder = self.fodder_refreshes(game);
        put(&mut state, "fodder_refreshes", fodder);
        let frozen = self.frozen(game);
        put(&mut state, "frozen", Json::Bool(frozen));
        let counters = self.counters(game);
        put(&mut state, "counters", counters);
        let secrets = self.secrets(game);
        put(&mut state, "secrets", secrets);
        let won = self.won_last_combat(game);
        put(&mut state, "won_last_combat", won);
        let theirs = last_opponent_board(&self.cards, self.tracker.fights());
        put(&mut state, "last_opponent_board", theirs);
        Json::Obj(state)
    }

    /// The other seven players at the turn's start, as the Power.log shows
    /// them (docs/real-boards.md "The action file"). The seats are in the log's
    /// order after mine, so `seat` 1 is the first opponent the observation
    /// writes. The board, Gem and Tavern spell numbers are the last fight I had
    /// with that player; the log has no leaderboard tribe line and no other
    /// player's fights, so those stay out.
    fn lobby_state(&self, game: &Game) -> Json {
        let Some(mine) = me(game) else {
            return Json::Arr(Vec::new());
        };
        let seats = self.tracker.seats();
        let ids: Vec<i32> = seats.keys().copied().collect();
        let Some(at) = ids.iter().position(|&p| p == mine) else {
            return Json::Arr(Vec::new());
        };
        let fights = self.tracker.fights();
        let last_done = fights.iter().rev().find(|f| !f.result.is_empty());
        let next = hero_entity(game)
            .map(|h| h.int("NEXT_OPPONENT_PLAYER_ID"))
            .filter(|n| *n > 0)
            .or_else(|| {
                game.player_entity(mine)
                    .map(|p| p.int("NEXT_OPPONENT_PLAYER_ID"))
                    .filter(|n| *n > 0)
            })
            .unwrap_or(0) as i32;
        let mut out = Vec::new();
        for k in 1..ids.len() {
            let pid = ids[(at + k) % ids.len()];
            let Some(seat) = seats.get(&pid) else {
                continue;
            };
            let hero = game
                .entities_in_order()
                .filter(|e| {
                    e.text("CARDTYPE") == "HERO"
                        && e.int("PLAYER_ID") == pid as i64
                        && matches!(e.text("ZONE"), "PLAY" | "SETASIDE")
                })
                .last();
            let hero_card = if seat.hero_card.is_empty() {
                hero.map(|h| h.card_id.clone()).unwrap_or_default()
            } else {
                seat.hero_card.clone()
            };
            let hero_base = hero
                .and_then(|h| {
                    let power_id = h.int("HERO_POWER_ENTITY");
                    let hero_id = h.id as i64;
                    let power = game
                        .entities
                        .get(&(power_id as i32))
                        .filter(|e| e.text("CARDTYPE") == "HERO_POWER")
                        .or_else(|| {
                            game.entities_in_order().find(|e| {
                                e.text("CARDTYPE") == "HERO_POWER"
                                    && e.int("CREATOR") == hero_id
                            })
                        });
                    power.and_then(|e| {
                        let dbf = e.int("BACON_HEROPOWER_BASE_HERO_ID");
                        (dbf > 0)
                            .then(|| self.dbf.get(&(dbf as u32)).cloned())
                            .flatten()
                    })
                })
                .unwrap_or_default();
            let alive = seat.health > 0;
            let mut o = vec![
                ("player_id".to_string(), Json::Int(pid as i64)),
                ("seat".to_string(), Json::Int(k as i64)),
                ("hero_card".to_string(), str_(&hero_card)),
                ("hero_base".to_string(), str_(&hero_base)),
                ("hero_name".to_string(), str_(&seat.hero)),
                ("health".to_string(), Json::Int(seat.health)),
                ("armor".to_string(), Json::Int(seat.armor)),
                ("tier".to_string(), Json::Int(seat.tier)),
                ("alive".to_string(), Json::Bool(alive)),
                (
                    "placement".to_string(),
                    Json::Int(if alive { 0 } else { seat.place }),
                ),
                ("next".to_string(), Json::Bool(next == pid)),
                (
                    "last".to_string(),
                    Json::Bool(last_done.is_some_and(|f| f.opponent == pid)),
                ),
            ];
            if let Some(f) = fights
                .iter()
                .rev()
                .find(|f| f.opponent == pid && !f.result.is_empty())
            {
                put(&mut o, "seen_turn", Json::Int(f.turn));
                put(&mut o, "seen_tier", Json::Int(seat.tier));
                if !f.theirs.is_empty() {
                    let marks = &f.reanimates.1;
                    let board: Vec<Json> = f
                        .theirs
                        .iter()
                        .enumerate()
                        .map(|(i, m)| {
                            Json::Obj(minion_json(
                                &self.cards,
                                i,
                                m,
                                marks.get(i).copied().unwrap_or(false),
                            ))
                        })
                        .collect();
                    put(&mut o, "last_board", Json::Arr(board));
                }
                if let Some(d) = &f.deities.1 {
                    put(
                        &mut o,
                        "last_deity",
                        Json::Obj(vec![
                            ("card_id".to_string(), str_(&d.card_id)),
                            ("name".to_string(), str_(&d.name)),
                            ("attack".to_string(), Json::Int(d.attack)),
                            ("health".to_string(), Json::Int(d.health)),
                            (
                                "golden".to_string(),
                                Json::Bool(d.card_id.contains("_G")),
                            ),
                        ]),
                    );
                }
                put(&mut o, "gem_attack", Json::Int(f.gems.1.0));
                put(&mut o, "gem_health", Json::Int(f.gems.1.1));
                put(
                    &mut o,
                    "spell_attack",
                    Json::Int(f.side_numbers.1.get("spell_attack").copied().unwrap_or(0)),
                );
                put(
                    &mut o,
                    "spell_health",
                    Json::Int(f.side_numbers.1.get("spell_health").copied().unwrap_or(0)),
                );
            }
            if let Some(hero) = hero {
                let mut trinkets = Vec::new();
                for (tag, slot) in [
                    ("BACON_FIRST_TRINKET_DATABASE_ID", 0),
                    ("BACON_SECOND_TRINKET_DATABASE_ID", 1),
                    ("BACON_HEROPOWER_TRINKET_DATABASE_ID", 2),
                ] {
                    let dbf = hero.int(tag);
                    if dbf <= 0 {
                        continue;
                    }
                    let Some(card_id) = self.dbf.get(&(dbf as u32)) else {
                        continue;
                    };
                    trinkets.push(Json::Obj(vec![
                        ("slot".to_string(), Json::Int(slot)),
                        ("card_id".to_string(), str_(card_id)),
                        ("name".to_string(), str_(self.cards.name(card_id))),
                    ]));
                }
                if !trinkets.is_empty() {
                    put(&mut o, "trinkets", Json::Arr(trinkets));
                }
            }
            out.push(Json::Obj(o));
        }
        Json::Arr(out)
    }

    /// My hand and board and the Tavern, each card as `boards/real/` writes it
    /// plus where it sits.
    fn state(&mut self, game: &Game) -> Obj {
        let mine = me(game).map(|m| m as i64);
        let theirs = dummy(game).map(|d| d as i64);
        let gifts = gifts_by_minion(game);
        let ench = hero_enchants_by_minion(game);
        let marks = self.reanimate_marks(game);
        let mut board = Vec::new();
        let mut hand = Vec::new();
        let mut tavern = Vec::new();
        let mut hidden = 0;
        let mut tribes: BTreeSet<String> = BTreeSet::new();
        for e in game.entities_in_order() {
            let ct = e.text("CARDTYPE");
            let zone = e.text("ZONE");
            let controller = e.int("CONTROLLER");
            if Some(controller) == mine && zone == "PLAY" && ct == "MINION" {
                let mut d = self.one(e, game, &gifts, &ench, "position");
                if marks.contains(&e.id) {
                    // marked by Rapid Reanimation, as the board files write it
                    put(&mut d, "reanimate", Json::Bool(true));
                }
                tribes.extend(card_tribes(&self.cards, &e.card_id));
                board.push(Json::Obj(d));
            } else if Some(controller) == theirs && zone == "PLAY" && SHOP_TYPES.contains(&ct) {
                let mut d = self.one(e, game, &gifts, &ench, "slot");
                put(
                    &mut d,
                    "kind",
                    str_(if ct == "BATTLEGROUND_SPELL" {
                        "spell"
                    } else {
                        "minion"
                    }),
                );
                if e.int("COST") != 0 {
                    // only a Tavern spell carries its Gold price in the log: a
                    // minion costs 3 by the game's rule, not by a tag
                    put(&mut d, "cost", Json::Int(e.int("COST")));
                }
                put(&mut d, "frozen", Json::Bool(e.int("FROZEN") != 0));
                if e.int("BACON_COSTS_HEALTH_TO_BUY") != 0 {
                    put(&mut d, "costs_health", Json::Bool(true));
                }
                // The price the game charges for this card when it is not the
                // rule's: the override that Chillmere Mosaic's Spellcraft puts
                // on every minion it deals (BACON_OVERRIDE_BG_COST, the whole
                // price) and the reduction a free buy leaves on the Tavern's
                // minions (BACON_REDUCE_BUY_COST, taken off the buy cost:
                // Aranna's Spectral Sight takes the whole 3 off).
                if e.int("BACON_OVERRIDE_BG_COST") != 0 {
                    put(&mut d, "cost_override", Json::Int(e.int("BACON_OVERRIDE_BG_COST")));
                }
                if e.int("BACON_REDUCE_BUY_COST") != 0 {
                    put(&mut d, "cost_reduce", Json::Int(e.int("BACON_REDUCE_BUY_COST")));
                }
                let shown = card_tribes(&self.cards, &e.card_id);
                // Era's lobby rule (docs/rules/design.md "The lobby"): a type is
                // in the lobby when a minion of only that type shows in the
                // Tavern. Two-tribe minions, "all types" minions (no tribe the
                // engine names) and everything outside the Tavern say nothing.
                if ct == "MINION"
                    && shown.len() == 1
                    && LOBBY_TRIBES.contains(&shown[0].as_str())
                {
                    self.lobby.entry(shown[0].clone()).or_default().insert(e.card_id.clone());
                }
                tribes.extend(shown);
                tavern.push(Json::Obj(d));
            } else if Some(controller) == mine
                && zone == "HAND"
                && !NOT_IN_HAND.contains(&ct)
            {
                // a hand card the log does not name is kept, so that it can be
                // followed to where it goes: its card id is null and it is
                // counted in hand_hidden
                let mut d = self.one(e, game, &gifts, &ench, "index");
                if e.card_id.is_empty() {
                    hidden += 1;
                }
                if SHOP_TYPES.contains(&ct) {
                    put(
                        &mut d,
                        "kind",
                        str_(if ct == "BATTLEGROUND_SPELL" {
                            "spell"
                        } else {
                            "minion"
                        }),
                    );
                } else if !ct.is_empty() {
                    put(&mut d, "kind", str_(&ct.to_lowercase()));
                }
                tribes.extend(card_tribes(&self.cards, &e.card_id));
                hand.push(Json::Obj(d));
            }
        }
        sort_by_place(&mut board, "position");
        sort_by_place(&mut hand, "index");
        sort_by_place(&mut tavern, "slot");
        self.tribes.extend(tribes);
        vec![
            ("board".to_string(), Json::Arr(board)),
            ("hand".to_string(), Json::Arr(hand)),
            ("tavern".to_string(), Json::Arr(tavern)),
            ("hand_hidden".to_string(), Json::Int(hidden)),
        ]
    }

    /// A Dark Gift on a minion: its card id, and the countdown its own
    /// enchantment shows.
    fn gift_of(&self, game: &Game, host: &Entity, card: &str) -> Obj {
        let mut out = vec![
            ("card_id".to_string(), str_(card)),
            ("name".to_string(), str_(self.cards.name(card))),
        ];
        for x in game.entities_in_order() {
            if x.card_id.is_empty()
                || !x.card_id.starts_with(card)
                || x.int("ATTACHED") != host.id as i64
            {
                continue;
            }
            if x.text("ZONE") != "PLAY" {
                continue;
            }
            if let Some(n) = x.get("TAG_SCRIPT_DATA_NUM_1") {
                put(&mut out, "state", Json::Int(n.as_int()));
            }
            break;
        }
        out
    }

    fn one(
        &self,
        e: &Entity,
        game: &Game,
        gifts: &HashMap<i32, Vec<String>>,
        ench: &HashMap<i32, Vec<String>>,
        place: &str,
    ) -> Obj {
        let mut d = vec![
            ("id".to_string(), Json::Int(e.id as i64)),
            (place.to_string(), Json::Int(e.int("ZONE_POSITION"))),
        ];
        if e.card_id.is_empty() {
            put(&mut d, "card_id", Json::Null);
            put(&mut d, "name", Json::Null);
            return d;
        }
        let m = minion(e, game, &self.cards, gifts, ench);
        put(&mut d, "card_id", str_(&m.card_id));
        put(&mut d, "name", str_(&m.name));
        put(&mut d, "attack", Json::Int(m.attack));
        put(&mut d, "health", Json::Int(m.health));
        put(&mut d, "golden", Json::Bool(m.golden));
        put(
            &mut d,
            "keywords",
            Json::Arr(m.keywords.iter().map(|k| str_(k)).collect()),
        );
        let creator = {
            let id = e.int("CREATOR");
            if id != 0 {
                game.entities.get(&(id as i32))
            } else {
                None
            }
        };
        if let Some(creator) = creator
            && !creator.card_id.is_empty()
            && !is_system(&creator.card_id)
        {
            put(
                &mut d,
                "from",
                Json::Obj(vec![
                    ("card_id".to_string(), str_(&creator.card_id)),
                    (
                        "name".to_string(),
                        str_(self.cards.name(&creator.card_id)),
                    ),
                ]),
            );
        }
        if !m.dark_gifts.is_empty() {
            let gifts: Vec<Json> = m
                .dark_gifts
                .iter()
                .map(|card| Json::Obj(self.gift_of(game, e, card)))
                .collect();
            put(&mut d, "dark_gifts", Json::Arr(gifts));
        }
        if let Some(sd) = minion_numbers(&m.tags) {
            put(&mut d, "numbers", numbers_json(&sd));
        }
        // What this minion's Blood Gems gave it: the replay seats it as
        // `Minion::gem_attack`/`gem_health`, which Gem Confiscation takes from
        // the neighbours (`gem_stats`).
        if let Some((attack, health)) = gem_stats(&m.tags) {
            put(&mut d, "gem_attack", Json::Int(attack));
            put(&mut d, "gem_health", Json::Int(health));
        }
        // The Magnetizations inside it, for Utility Drone's end of turn: each
        // Magnetic that merged left its own enchantment on the host
        // (`magnetization_count`).
        let magnetizations = magnetization_count(game, e);
        if magnetizations != 0 {
            put(&mut d, "magnetizations", Json::Int(magnetizations));
        }
        // A held Drone Duplicator Magnetization, for the play that spends it:
        // the replay seats it as `Minion::extra_magnetize`, the number of extra
        // times the next Magnetization to this minion happens
        // (`EXTRA_MAGNETIZATION_TAG`). Written only when one is held, so a file
        // from before the key existed reads as none held, which is what the
        // loader always seated.
        let extra = extra_magnetizations(e, m.golden);
        if extra != 0 {
            put(&mut d, "extra_magnetize", Json::Int(extra));
        }
        // Holy Vanguard's "+30/+30 if you have 15 or less Health" is part of
        // the card's stats: the log counts it in ATK/HEALTH and keeps it on
        // the minion's `Vanguard's Valor` enchantment. The file writes the
        // pair beside the stats, and the loader seats it as
        // `Minion::vanguard_aura`, which takes it off when the Health goes back
        // above 15 and which a triple leaves out of its sum (one copy's aura
        // per copy is not a buff). A file from before the key existed reads as
        // no aura, and its stats keep what the log counted in them.
        if let Some((attack, health)) = vanguard_aura_numbers(game, e) {
            put(
                &mut d,
                "vanguard_aura",
                Json::Arr(vec![Json::Int(attack), Json::Int(health)]),
            );
        }
        // A Golden minion gives a Triple Reward when it is played, unless its
        // own source says otherwise (`no_triple_reward`).
        if no_triple_reward(&m, creator.map(|c| c.card_id.as_str())) {
            put(&mut d, "no_reward", Json::Bool(true));
        }
        d
    }

    // -- the client's numbers -----------------------------------------------

    fn hero_powers(&self, game: &Game) -> Json {
        let mine = me(game).map(|m| m as i64);
        let mut xs: Vec<&Entity> = game
            .entities_in_order()
            .filter(|x| {
                Some(x.int("CONTROLLER")) == mine
                    && x.text("ZONE") == "PLAY"
                    && x.text("CARDTYPE") == "HERO_POWER"
            })
            .collect();
        xs.sort_by_key(|x| x.id);
        let mut out = Vec::new();
        for x in xs {
            let numbers: Vec<Json> = (1..=4)
                .map(|i| Json::Int(x.int(&format!("TAG_SCRIPT_DATA_NUM_{i}"))))
                .collect();
            let mut d = vec![
                ("card_id".to_string(), str_(&x.card_id)),
                ("name".to_string(), str_(self.cards.name(&x.card_id))),
                ("numbers".to_string(), Json::Arr(numbers)),
                ("cost".to_string(), Json::Int(x.int("COST"))),
                (
                    "used_this_turn".to_string(),
                    Json::Int(x.int("HEROPOWER_ACTIVATIONS_THIS_TURN")),
                ),
            ];
            if x.int("ADDITIONAL_HERO_POWER_INDEX") != 0 {
                put(&mut d, "second", Json::Bool(true));
            }
            // Lock and Load holds the Tavern minion it stole in the power's own
            // entity tag and summons a copy of it at the next Start of Combat.
            // Every other power owes nothing, and says so with a null (the
            // board files write the same key).
            let waiting = self.waiting_minion(game, x);
            put(&mut d, "waiting", waiting);
            out.push(Json::Obj(d));
        }
        Json::Arr(out)
    }

    /// The minion a power still owes the board, as a board minion, or `None`.
    fn waiting_minion(&self, game: &Game, power: &Entity) -> Json {
        let ent = power.int("TAG_SCRIPT_DATA_ENT_1");
        if ent == 0 {
            return Json::Null;
        }
        let waited = self.waited.get(&ent).cloned().unwrap_or(Json::Null);
        let Some(held) = game.entities.get(&(ent as i32)) else {
            return waited;
        };
        if held.text("CARDTYPE") != "MINION" || held.card_id.is_empty() {
            return waited;
        }
        Json::Obj(minion_brief(game, &self.cards, held))
    }

    fn trinket_list(&self, game: &Game) -> Json {
        let Some(mine) = me(game) else {
            return Json::Arr(Vec::new());
        };
        Json::Arr(
            trinkets(game, &self.cards, Some(mine))
                .iter()
                .map(|t| {
                    Json::Obj(vec![
                        ("card_id".to_string(), str_(&t.card_id)),
                        ("name".to_string(), str_(&t.name)),
                        ("greater".to_string(), Json::Bool(t.greater)),
                        ("numbers".to_string(), numbers_json(&t.numbers)),
                    ])
                })
                .collect(),
        )
    }

    /// My Secret zone: the Secret card ids in it, in the order I got them.
    fn secrets(&self, game: &Game) -> Json {
        let mine = me(game).map(|m| m as i64);
        let mut xs: Vec<&Entity> = game
            .entities_in_order()
            .filter(|x| {
                x.text("ZONE") == "SECRET"
                    && Some(x.int("CONTROLLER")) == mine
                    && SECRET_CARDS.contains(&x.card_id.as_str())
            })
            .collect();
        xs.sort_by_key(|x| x.id);
        Json::Arr(xs.iter().map(|x| str_(&x.card_id)).collect())
    }

    /// The entity ids of my minions Rapid Reanimation marked.
    fn reanimate_marks(&self, game: &Game) -> HashSet<i32> {
        let attached: HashSet<i64> = game
            .entities_in_order()
            .filter(|x| {
                x.card_id == REANIMATE_MARK && x.text("ZONE") == "PLAY" && x.int("ATTACHED") != 0
            })
            .map(|x| x.int("ATTACHED"))
            .collect();
        let mine = me(game).map(|m| m as i64);
        game.entities_in_order()
            .filter(|e| {
                e.text("CARDTYPE") == "MINION"
                    && e.text("ZONE") == "PLAY"
                    && Some(e.int("CONTROLLER")) == mine
                    && attached.contains(&(e.id as i64))
            })
            .map(|e| e.id)
            .collect()
    }

    fn won_last_combat(&self, game: &Game) -> Json {
        let Some(player) = my_player(game) else {
            return Json::Null;
        };
        match player.get("BACON_WON_LAST_COMBAT") {
            Some(value) => Json::Bool(value.as_int() != 0),
            None => Json::Null,
        }
    }

    /// The player-level numbers a shop step reads, by the board files' keys.
    fn counters(&self, game: &Game) -> Json {
        let Some(player) = my_player(game) else {
            return Json::Obj(Vec::new());
        };
        let mut out: Obj = Vec::new();
        // What one Blood Gem gives: the printed +1/+1 plus the bonus the two
        // BACON_BLOODGEMBUFF tags hold.
        let gem_attack = 1 + player.int("BACON_BLOODGEMBUFFATKVALUE").max(0);
        let gem_health = 1 + player.int("BACON_BLOODGEMBUFFHEALTHVALUE").max(0);
        if gem_attack != 1 || gem_health != 1 {
            put(&mut out, "gem_attack", Json::Int(gem_attack));
            put(&mut out, "gem_health", Json::Int(gem_health));
        }
        // The Tavern spell bonus, which every Tavern spell this player casts and
        // every spell on a minion gets.
        if player.int("TAVERN_SPELL_ATTACK_INCREASE") != 0 {
            put(
                &mut out,
                "spell_attack",
                Json::Int(player.int("TAVERN_SPELL_ATTACK_INCREASE")),
            );
        }
        if player.int("TAVERN_SPELL_HEALTH_INCREASE") != 0 {
            put(
                &mut out,
                "spell_health",
                Json::Int(player.int("TAVERN_SPELL_HEALTH_INCREASE")),
            );
        }
        for (card_id, (attack, health)) in SAME_NUMBER_ENCHANTS {
            let Some(sd) = enchant_numbers(game, player, card_id) else {
                continue;
            };
            if sd[0] == 0 {
                continue;
            }
            put(&mut out, attack, Json::Int(sd[0]));
            put(&mut out, health, Json::Int(sd[0]));
        }
        for (card_id, (attack, health)) in PLAYER_ENCHANTS {
            let Some(sd) = enchant_numbers(game, player, card_id) else {
                continue;
            };
            if !attack.is_empty() && sd[0] != 0 {
                put(&mut out, attack, Json::Int(sd[0]));
            }
            if let Some(health) = health
                && sd[1] != 0
            {
                put(&mut out, health, Json::Int(sd[1]));
            }
        }
        // The Health half of "your Undead have +A/+H this game, wherever they
        // are": the player enchant above carries the Attack alone, and every
        // Undead on my board carries the whole pair on its own `Undead Army`
        // enchant (docs/real-boards.md "counters").
        if let Some(sd) = undead_army_numbers(game)
            && sd[1] != 0
        {
            put(&mut out, "undead_health", Json::Int(sd[1]));
        }
        // "Your Volumizers have +A/+H this game (wherever they are)": every
        // Volumizer of mine reads it, and playing another one raises it
        // (`Player::volumizer_bonus`).
        if let Some([attack, health]) = volumizer_bonus_numbers(game)
            && (attack != 0 || health != 0)
        {
            put(&mut out, "volumizer_attack", Json::Int(attack));
            put(&mut out, "volumizer_health", Json::Int(health));
        }
        if player.int("NUM_SPELLS_PLAYED_THIS_GAME") != 0 {
            put(
                &mut out,
                "spells_cast",
                Json::Int(player.int("NUM_SPELLS_PLAYED_THIS_GAME")),
            );
        }
        if player.int("NUM_RESOURCES_SPENT_THIS_GAME") != 0 {
            put(
                &mut out,
                "gold_spent",
                Json::Int(player.int("NUM_RESOURCES_SPENT_THIS_GAME")),
            );
        }
        if self.upper != 0 {
            put(&mut out, "upper_hand", Json::Int(self.upper));
        }
        if self.goldens != 0 {
            put(&mut out, "golden_played", Json::Int(self.goldens));
        }
        Json::Obj(out)
    }

    /// The Demon Fodders queued for the coming Refreshes: the game keeps the
    /// queue on its own player enchant, one number per upcoming Refresh in
    /// the four script numbers. A Trapped Clapper's Deathrattle gives one
    /// Fodder to each of the next three Refreshes, which the log writes as
    /// `[1, 1, 1]`, and the Refresh that follows takes the first number only
    /// (the count the Fodders of Hearthstone_2026_10_27_04_30_04 give a
    /// single Refresh). The numbers are the engine's `fodder_refreshes`, so
    /// the file carries them as they stand, trailing empty Refreshes dropped
    /// (`[5, 5, 0]` is `[5, 5]`).
    fn fodder_refreshes(&self, game: &Game) -> Json {
        let mine = me(game).map(|m| m as i64);
        let Some(e) = game
            .entities_in_order()
            .find(|x| x.card_id == FODDER_ENCHANT && Some(x.int("CONTROLLER")) == mine)
        else {
            return Json::Arr(Vec::new());
        };
        let mut out: Vec<i64> = (1..=4)
            .map(|i| e.int(&format!("TAG_SCRIPT_DATA_NUM_{i}")))
            .collect();
        while out.last() == Some(&0) {
            out.pop();
        }
        Json::Arr(out.into_iter().map(Json::Int).collect())
    }

    /// The Tavern's lock: whether its offers are frozen.
    fn frozen(&self, game: &Game) -> bool {
        let theirs = dummy(game).map(|d| d as i64);
        let offers: Vec<&Entity> = game
            .entities_in_order()
            .filter(|x| {
                Some(x.int("CONTROLLER")) == theirs
                    && x.text("ZONE") == "PLAY"
                    && SHOP_TYPES.contains(&x.text("CARDTYPE"))
            })
            .collect();
        if !offers.is_empty() {
            return offers.iter().any(|x| x.int("FROZEN") != 0);
        }
        let mine = me(game).map(|m| m as i64);
        game.entities_in_order()
            .find(|x| {
                Some(x.int("CONTROLLER")) == mine
                    && x.text("ZONE") == "PLAY"
                    && x.card_id == FREEZE
            })
            .map(|x| x.int("TAG_SCRIPT_DATA_NUM_1") != 0)
            .unwrap_or(false)
    }
}

/// A minion as `waiting_minion` writes it.
fn minion_brief(game: &Game, cards: &Cards, e: &Entity) -> Obj {
    let m = plain_minion(game, cards, e);
    let mut d = vec![
        ("card_id".to_string(), str_(&m.card_id)),
        ("name".to_string(), str_(&m.name)),
        ("attack".to_string(), Json::Int(m.attack)),
        ("health".to_string(), Json::Int(m.health)),
        ("golden".to_string(), Json::Bool(m.golden)),
        (
            "keywords".to_string(),
            Json::Arr(m.keywords.iter().map(|k| str_(k)).collect()),
        ),
    ];
    if let Some(sd) = minion_numbers(&m.tags) {
        put(&mut d, "numbers", numbers_json(&sd));
    }
    d
}

/// The cards of a state, by the number that says where they sit.
fn sort_by_place(cards: &mut [Json], place: &str) {
    cards.sort_by_key(|card| {
        let object = card.as_obj().unwrap();
        (number_in(object, place), number_in(object, "id"))
    });
}

/// A list of keys and their values, as `was`/`now` hold them.
fn named(keys: [&str; 6], values: Vec<Json>) -> Obj {
    keys.iter()
        .zip(values)
        .map(|(key, value)| (key.to_string(), value))
        .collect()
}

/// A number card-key list as JSON, for comparing two states key by key.
fn json_of(key: &CardKey) -> Obj {
    vec![
        ("id".to_string(), Json::Int(key.id)),
        (
            "card_id".to_string(),
            key.card_id.clone().map_or(Json::Null, Json::Str),
        ),
        ("attack".to_string(), key.attack.map_or(Json::Null, Json::Int)),
        ("health".to_string(), key.health.map_or(Json::Null, Json::Int)),
        (
            "golden".to_string(),
            key.golden.map_or(Json::Null, Json::Bool),
        ),
        (
            "keywords".to_string(),
            key.keywords.clone().unwrap_or(Json::Null),
        ),
        (
            "dark_gifts".to_string(),
            key.dark_gifts.clone().unwrap_or(Json::Null),
        ),
        (
            "frozen".to_string(),
            key.frozen.map_or(Json::Null, Json::Bool),
        ),
    ]
}

/// One card's tribes, as the Python's `card_tribes` reads them.
fn card_tribes(cards: &Cards, card_id: &str) -> Vec<String> {
    cards.tribes(card_id).to_vec()
}

/// The dbf id to card id of the `--cards` file (HearthstoneJSON). A hero's
/// leaderboard tags name another player's trinkets by dbf id, and the card id
/// is what the engine's trinket table knows.
fn load_dbf(path: &str) -> Result<HashMap<u32, String>, String> {
    #[derive(serde::Deserialize)]
    struct RawCard {
        #[serde(alias = "card_id")]
        id: String,
        #[serde(default, rename = "dbfId", alias = "dbf_id")]
        dbf_id: u32,
    }
    let text = fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let raw: Vec<RawCard> = serde_json::from_str(&text).map_err(|e| format!("{path}: {e}"))?;
    Ok(raw
        .into_iter()
        .filter(|c| c.dbf_id != 0)
        .map(|c| (c.dbf_id, c.id))
        .collect())
}

/// Every Battlegrounds game of a log, in order, with `None` for a game that has
/// no shop turn: the Python's `read_log`. `label` is the log folder's name, for
/// the warning a game with more than five lobby types prints.
fn read_log(
    path: &Path,
    cards: &Cards,
    label: &str,
    dbf: &HashMap<u32, String>,
) -> Result<Vec<Option<Json>>, String> {
    let file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut reader = BufReader::with_capacity(1 << 20, file);
    read_log_reader(&mut reader, cards, label, dbf)
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// The same reader over any stream, so a test can feed a trimmed log.
fn read_log_reader(
    reader: &mut impl BufRead,
    cards: &Cards,
    label: &str,
    dbf: &HashMap<u32, String>,
) -> Result<Vec<Option<Json>>, String> {
    let mut collector = Collector::new_with_dbf(cards.clone(), label, dbf.clone());
    let mut bytes: Vec<u8> = Vec::new();
    loop {
        bytes.clear();
        let read = reader
            .read_until(b'\n', &mut bytes)
            .map_err(|e| format!("log: {e}"))?;
        if read == 0 {
            break;
        }
        // the log is text, but a truncated write can leave a byte that is not
        // UTF-8: the Python reads it with `errors="replace"` and so do we
        let line = String::from_utf8_lossy(&bytes);
        collector.feed(&line);
    }
    collector.finish_log();
    collector.last_game();
    Ok(collector.games)
}

impl Collector {
    fn my_place(&self) -> Option<i64> {
        let game = self.log.game()?;
        let p = my_player(game)?;
        if p.text("PLAYSTATE") == "WON" {
            return Some(1);
        }
        let place = p.int("PLAYER_LEADERBOARD_PLACE");
        (place > 0).then_some(place)
    }

    fn snapshot_live(&self) -> Option<(usize, u32, usize, bool, usize, Json)> {
        let game = self.log.game()?;
        if !game.game_type.starts_with("GT_BATTLEGROUNDS") || self.game_complete {
            return None;
        }
        let turn_obj = self.turn.as_ref()?;
        if self.ended || field(turn_obj, "start").map(Json::is_null).unwrap_or(true) {
            return None;
        }
        let mut live_turn = turn_obj.clone();
        set(&mut live_turn, "end", Json::Null);
        let choice_len = self.choice.as_ref().map_or(0, |c| c.entities.len());
        let has_choice = self.pending.is_none() && choice_len > 0;
        if has_choice {
            put(&mut live_turn, "pending_choice", Json::Obj(self.choice_step(game)));
        }
        let step_count = parts_of(turn_obj, "actions").len();
        let turn_num = field(turn_obj, "turn")
            .and_then(Json::as_int)
            .unwrap_or(0) as u32;
        let gi = self.games.len() + 1;

        let mut turns = match self.record.as_obj().and_then(|o| field(o, "turns")) {
            Some(Json::Arr(existing)) => existing.clone(),
            _ => Vec::new(),
        };
        turns.push(Json::Obj(live_turn));

        let mut fields: Obj = Vec::with_capacity(7);
        put(&mut fields, "source", str_(&self.label));
        put(&mut fields, "game", Json::Int(gi as i64));
        put(&mut fields, "turns", Json::Arr(turns));
        put(&mut fields, "hero", self.my_hero(game));
        put(&mut fields, "unread", self.unread.to_json());
        put(
            &mut fields,
            "tribes",
            Json::Arr(self.tribes.iter().map(|t| str_(t)).collect()),
        );
        put(
            &mut fields,
            "lobby",
            Json::Arr(self.lobby.keys().map(|t| str_(t)).collect()),
        );
        Some((gi, turn_num, step_count, has_choice, choice_len, Json::Obj(fields)))
    }
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn collect_live(
    out: &Path,
    cards_path: &str,
    path: &Path,
    label_arg: &str,
    follow: bool,
) -> Result<(), String> {
    let cards = Cards::load(cards_path)?;
    let dbf = load_dbf(cards_path)?;
    fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let label = if !label_arg.is_empty() {
        label_arg.to_string()
    } else {
        path.parent()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().into_owned())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "live".to_string())
    };
    let file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut reader = BufReader::with_capacity(1 << 20, file);
    let mut collector = Collector::new_with_dbf(cards.clone(), &label, dbf.clone());
    let mut bytes: Vec<u8> = Vec::new();
    let mut offset: u64 = 0;
    let mut saw_data = false;
    let mut last_decision: Option<(usize, u32, usize, bool, usize)> = None;
    let mut last_ended_game: Option<usize> = None;
    let mut stdout = io::stdout();

    loop {
        let prev_len = bytes.len();
        let read = reader
            .read_until(b'\n', &mut bytes)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if read > 0 {
            offset += read as u64;
            if !follow || bytes.ends_with(b"\n") {
                let line = String::from_utf8_lossy(&bytes);
                collector.feed(&line);
                bytes.clear();
                saw_data = true;
            }
            continue;
        }
        // Keep partial trailing line buffered until its newline is flushed.
        if bytes.len() == prev_len && !bytes.is_empty() && !follow {
            let line = String::from_utf8_lossy(&bytes);
            collector.feed(&line);
            bytes.clear();
            saw_data = true;
        }
        if collector.game_complete
            && collector
                .log
                .game()
                .is_some_and(|g| g.game_type.starts_with("GT_BATTLEGROUNDS"))
        {
            let gi = collector.games.len() + 1;
            if last_ended_game != Some(gi) {
                let game_id = format!("{label}-g{gi}");
                let place = collector
                    .my_place()
                    .map_or_else(|| "null".to_string(), |p| p.to_string());
                writeln!(
                    stdout,
                    "{{\"event\":\"game_end\",\"game\":{},\"place\":{place}}}",
                    json_escape(&game_id)
                )
                .map_err(|e| format!("stdout: {e}"))?;
                stdout.flush().map_err(|e| format!("stdout: {e}"))?;
                last_ended_game = Some(gi);
            }
        } else if saw_data {
            let waiting_choice_entities =
                collector.pending.is_none() && collector.choice.as_ref().is_some_and(|c| c.entities.is_empty());
            let settled = !follow || (collector.pending.is_none() && !waiting_choice_entities);
            if settled && let Some((gi, turn, step, has_choice, choice_len, doc)) = collector.snapshot_live() {
                let key = (gi, turn, step, has_choice, choice_len);
                if last_decision != Some(key) {
                    let game_id = format!("{label}-g{gi}");
                    let json_path = out.join(format!("{game_id}.json"));
                    let tmp_path = out.join(format!(".{game_id}.json.tmp"));
                    fs::write(&tmp_path, render(&doc))
                        .map_err(|e| format!("{}: {e}", tmp_path.display()))?;
                    fs::rename(&tmp_path, &json_path)
                        .map_err(|e| format!("{}: {e}", json_path.display()))?;
                    let file_str = json_path.to_string_lossy();
                    writeln!(
                        stdout,
                        "{{\"event\":\"decision\",\"game\":{},\"file\":{},\"turn\":{turn},\"step\":{step},\"pending_choice\":{has_choice}}}",
                        json_escape(&game_id),
                        json_escape(&file_str),
                    )
                    .map_err(|e| format!("stdout: {e}"))?;
                    stdout.flush().map_err(|e| format!("stdout: {e}"))?;
                    last_decision = Some(key);
                }
            }
        }
        if !follow {
            break;
        }
        saw_data = false;
        if let Ok(meta) = fs::metadata(path)
            && meta.len() < offset
            && let Ok(reopened) = File::open(path)
        {
            reader = BufReader::with_capacity(1 << 20, reopened);
            collector = Collector::new_with_dbf(cards.clone(), &label, dbf.clone());
            bytes.clear();
            offset = 0;
            last_decision = None;
            last_ended_game = None;
            continue;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tally_prints_as_the_python_counter_dict() {
        let mut tally = Tally::new();
        tally.bump("buy");
        tally.bump("other");
        tally.bump("buy");
        assert_eq!(tally.render(), "{'buy': 2, 'other': 1}");
        assert_eq!(Tally::new().render(), "{}");
    }

    /// The warband of the opponent I last fought goes into a state in the shape
    /// the state's own `board` has, from the fight that has a result: a fight
    /// the log has already staged for the round to come is not the one I last
    /// fought, and an empty warband is null, which leaves the powers that copy
    /// one unpressed (`burgle_has_board`, `IspyNextOpponent`).
    #[test]
    fn the_last_opponents_warband_is_written_in_the_board_shape() {
        let cards = Cards::from_pairs([
            ("BG31_149".to_string(), "Bubble Gunner".to_string()),
            ("BG31_803".to_string(), "Buzzing Vermin".to_string()),
            ("BG36_MidGameEffect_000t82".to_string(), "Affinity".to_string()),
        ]);
        let mut tags = HashMap::new();
        tags.insert("TAG_SCRIPT_DATA_NUM_1".to_string(), TagValue::Int(2));
        let mut fought = bglog::Fight::new(
            2,
            3,
            Vec::new(),
            vec![
                bglog::Minion {
                    card_id: "BG31_149".to_string(),
                    name: "Bubble Gunner".to_string(),
                    attack: 4,
                    health: 3,
                    keywords: vec!["VENOMOUS"],
                    ..Default::default()
                },
                bglog::Minion {
                    card_id: "BG31_803".to_string(),
                    name: "Buzzing Vermin".to_string(),
                    attack: 2,
                    health: 2,
                    golden: true,
                    dark_gifts: vec!["BG36_MidGameEffect_000t82".to_string()],
                    tags,
                    ..Default::default()
                },
            ],
        );
        fought.result = "win".to_string();
        fought.reanimates = (Vec::new(), vec![false, true]);
        // A fight for the round to come, staged but not played out.
        let staged = bglog::Fight::new(
            3,
            4,
            Vec::new(),
            vec![bglog::Minion {
                card_id: "BG31_149".to_string(),
                ..Default::default()
            }],
        );
        let json = last_opponent_board(&cards, &[fought.clone(), staged]);
        let got: serde_json::Value = serde_json::from_str(&render(&json)).unwrap();
        let want = serde_json::json!([
            {"position": 1, "card_id": "BG31_149", "name": "Bubble Gunner",
             "attack": 4, "health": 3, "golden": false, "keywords": ["VENOMOUS"]},
            {"position": 2, "card_id": "BG31_803", "name": "Buzzing Vermin",
             "attack": 2, "health": 2, "golden": true, "keywords": [],
             "dark_gifts": [{"card_id": "BG36_MidGameEffect_000t82", "name": "Affinity"}],
             "numbers": [2, 0, 0, 0], "reanimate": true}
        ]);
        assert_eq!(got, want, "the warband as the state's board holds it");
        // Nothing fought yet: the state says null, not an empty board.
        assert_eq!(last_opponent_board(&cards, &[]), Json::Null);
        // A fight with no warband on the other side (an empty board).
        let mut empty = bglog::Fight::new(1, 3, Vec::new(), Vec::new());
        empty.result = "tie".to_string();
        assert_eq!(last_opponent_board(&cards, &[empty]), Json::Null);
    }

    /// The `no_reward` mark: only the sources whose own card says a Golden
    /// gives no Triple Reward (docs/rules/design.md, THE GOLDEN RULE); every
    /// other Golden gives one, a Golden carrying another Dark Gift too.
    #[test]
    fn only_the_named_sources_give_no_triple_reward() {
        fn golden(card_id: &str, gifts: &[&str]) -> bglog::Minion {
            bglog::Minion {
                card_id: card_id.to_string(),
                golden: true,
                dark_gifts: gifts.iter().map(|gift| gift.to_string()).collect(),
                ..Default::default()
            }
        }
        // Silent Deliverer's Battlecry goldens.
        assert!(no_triple_reward(
            &golden("BG35_341_G", &[]),
            Some(SILENT_DELIVERER)
        ));
        // Sinister Invitation's golden: a Dark Gift offer, whatever gift it got.
        assert!(no_triple_reward(
            &golden("BG30_125_G", &["BG36_MidGameEffect_000t71"]),
            Some(DARK_DISCOVERY)
        ));
        // The Gilding Dark Gift, which is what made it Golden.
        assert!(no_triple_reward(&golden("BG32_340_G", &[GILDING]), None));
        // Aureate Laureate, always Golden, and its printed golden card.
        assert!(no_triple_reward(&golden("BG32_236", &[]), None));
        assert!(no_triple_reward(&golden("BG32_236_G", &[]), None));
        // A Golden that carries any other Dark Gift gives one: the Mummifier of
        // Hearthstone_2026_10_24_00_56_06-g1 turn 7 step 5, whose log hand
        // gains a Triple Reward.
        assert!(!no_triple_reward(
            &golden("BG28_309_G", &["BG36_MidGameEffect_000t52"]),
            None
        ));
        // A triple's Golden, and one a hero power handed out.
        assert!(!no_triple_reward(
            &golden("BG28_309_G", &[]),
            Some("TB_BaconShop_CheckTriples")
        ));
        assert!(!no_triple_reward(
            &golden("BG36_854_G", &[]),
            Some("TB_BaconShop_HP_080")
        ));
        // A minion that is not Golden is never marked: playing it gives no
        // Triple Reward in the first place.
        let normal = bglog::Minion {
            card_id: "BG32_236".to_string(),
            ..Default::default()
        };
        assert!(!no_triple_reward(&normal, None));
    }

    /// "Your Volumizers have +A/+H this game (wherever they are)" is on every
    /// Volumizer of mine as its `Volumized` enchantment, not on the player:
    /// any one of them answers for the pair, an opponent's is never read, and
    /// one that left play keeps its old pair without being current.
    #[test]
    fn the_volumized_enchant_carries_the_volumizer_bonus() {
        fn tag(value: &str) -> TagValue {
            TagValue::Text(value.to_string())
        }
        fn number(value: i64) -> TagValue {
            TagValue::Int(value)
        }
        let volumized = |controller: i64, attack: i64, health: i64, zone: &str| {
            vec![
                ("ZONE", tag(zone)),
                ("CONTROLLER", number(controller)),
                ("TAG_SCRIPT_DATA_NUM_1", number(attack)),
                ("TAG_SCRIPT_DATA_NUM_2", number(health)),
            ]
        };
        let mut g = bglog::Game::new();
        g.real_players = [1].into_iter().collect();
        let mut add = |id: i32, card_id: &str, tags: Vec<(&str, TagValue)>| {
            let e = g.entity(id);
            e.card_id = card_id.to_string();
            for (k, v) in tags {
                e.set(k, v);
            }
        };
        // Two of mine at the current pair, an old one of mine that left play
        // with a bigger pair, and the opponent's.
        add(300, VOLUMIZED, volumized(1, 24, 39, "PLAY"));
        add(301, VOLUMIZED, volumized(1, 24, 39, "PLAY"));
        add(302, VOLUMIZED, volumized(1, 90, 90, "REMOVEDFROMGAME"));
        add(303, VOLUMIZED, volumized(2, 99, 99, "PLAY"));
        assert_eq!(volumizer_bonus_numbers(&g), Some([24, 39]));
        // No Volumizer of mine: the log shows no pair at all.
        let mut g = bglog::Game::new();
        g.real_players = [1].into_iter().collect();
        let mut add = |id: i32, card_id: &str, tags: Vec<(&str, TagValue)>| {
            let e = g.entity(id);
            e.card_id = card_id.to_string();
            for (k, v) in tags {
                e.set(k, v);
            }
        };
        add(303, VOLUMIZED, volumized(2, 99, 99, "PLAY"));
        assert_eq!(volumizer_bonus_numbers(&g), None);
    }

    /// A minion's Magnetization count is the game's own (`3670`), not the
    /// `Magnetic` enchantments the game left on it: a Satellite's stats pile
    /// into one enchantment however many of them merged. A Magnetic minion's
    /// own one is not a merge. With no such tag in the log, the enchants are
    /// the count, as they always were.
    #[test]
    fn a_minions_magnetizations_are_the_games_own_count() {
        fn tag(value: &str) -> TagValue {
            TagValue::Text(value.to_string())
        }
        fn number(value: i64) -> TagValue {
            TagValue::Int(value)
        }
        let host = |controller: i64| {
            vec![
                ("CARDTYPE", tag("MINION")),
                ("ZONE", tag("PLAY")),
                ("CONTROLLER", number(controller)),
            ]
        };
        let attached = |to: i64, magnetic: i64| {
            vec![
                ("CARDTYPE", tag("ENCHANTMENT")),
                ("ZONE", tag("PLAY")),
                ("ATTACHED", number(to)),
                ("MAGNETIC", number(magnetic)),
            ]
        };
        let mut g = bglog::Game::new();
        let mut add = |id: i32, card_id: &str, tags: Vec<(&str, TagValue)>| {
            let e = g.entity(id);
            e.card_id = card_id.to_string();
            for (k, v) in tags {
                e.set(k, v);
            }
        };
        add(100, "BG36_506", host(1));
        add(101, "BG36_506", host(1));
        // The game's own count, and an enchantment the game kept for a
        // Magnetization that merged: the count is what the Drone reads, so it
        // wins over the enchantments (`2026_11_02_00_10_22-g2` turn 10: 94
        // Magnetizations under one `BG31_171te`).
        let mut counted = host(1);
        counted.push((MAGNETIZED_TAG, number(94)));
        add(102, "BG36_506", counted);
        // A Magnetic minion's own count includes itself; what merged into it
        // is what the engine seats.
        let mut magnetic = host(1);
        magnetic.push(("MAGNETIC", number(1)));
        magnetic.push((MAGNETIZED_TAG, number(3)));
        add(103, "BG31_171t", magnetic);
        // A log with no such tag at all: the enchantments, one per merge.
        // Two Magnetizations on the first host (a Volumizer and a Satellite),
        // the plain `Volumized` aura enchantment is not one, and one
        // Magnetization on the second.
        add(300, "BG34_170te", attached(100, 1));
        add(301, "BG31_171te", attached(100, 1));
        add(302, VOLUMIZED, attached(100, 0));
        add(303, "BG34_170te", attached(101, 1));
        // The Satellite enchantment the game accumulated: one enchantment
        // only, for however many Magnetizations.
        add(304, "BG31_171te", attached(102, 1));
        let hosts: Vec<i32> = g.entities_in_order().map(|e| e.id).collect();
        let count = |id: i32| {
            let e = &g.entities[&id];
            magnetization_count(&g, e)
        };
        assert!(hosts.contains(&100));
        assert_eq!(count(100), 2, "no tag: the enchantments");
        assert_eq!(count(101), 1, "no tag: the enchantments");
        assert_eq!(count(102), 94, "the game's own count, not its one enchantment");
        assert_eq!(count(103), 2, "a Magnetic minion's own one is not a merge");
    }

    /// A Drone Duplicator's held extra Magnetization is the game's own tag
    /// (`4945`), and a Golden one applies its 1 twice: the file writes the
    /// number of extra times the replay's `Minion.extra_magnetize` holds.
    #[test]
    fn a_held_extra_magnetization_is_the_games_own_tag() {
        fn number(value: i64) -> TagValue {
            TagValue::Int(value)
        }
        let mut g = bglog::Game::new();
        let mut add = |id: i32, tags: Vec<(&str, TagValue)>| {
            let e = g.entity(id);
            e.card_id = "BG36_506".to_string();
            for (k, v) in tags {
                e.set(k, v);
            }
        };
        add(100, vec![(EXTRA_MAGNETIZATION_TAG, number(1))]);
        add(101, vec![]);
        let extra = |g: &bglog::Game, id: i32, golden: bool| {
            extra_magnetizations(&g.entities[&id], golden)
        };
        assert_eq!(extra(&g, 100, false), 1, "the plain card's one extra time");
        assert_eq!(extra(&g, 100, true), 2, "a golden one applies its 1 twice");
        assert_eq!(extra(&g, 101, false), 0, "a Duplicator with nothing held");
        assert_eq!(extra(&g, 101, true), 0);
    }

    /// The Health half of "your Undead have +A/+H this game, wherever they
    /// are" is not on the player enchant (`BG25_011pe` has the Attack alone):
    /// it rides on every Undead minion's own `Undead Army` enchant, and any one
    /// of mine answers for the pair.
    #[test]
    fn the_undead_army_enchant_carries_the_undead_health_bonus() {
        fn tag(value: &str) -> TagValue {
            TagValue::Text(value.to_string())
        }
        fn number(value: i64) -> TagValue {
            TagValue::Int(value)
        }
        let minion = |controller: i64| {
            vec![
                ("CARDTYPE", tag("MINION")),
                ("ZONE", tag("PLAY")),
                ("CONTROLLER", number(controller)),
            ]
        };
        let army = |attack: i64, health: i64| {
            vec![
                ("ZONE", tag("PLAY")),
                ("TAG_SCRIPT_DATA_NUM_1", number(attack)),
                ("TAG_SCRIPT_DATA_NUM_2", number(health)),
            ]
        };
        let mut g = bglog::Game::new();
        g.real_players = [1].into_iter().collect();
        let mut add = |id: i32, card_id: &str, tags: Vec<(&str, TagValue)>| {
            let e = g.entity(id);
            e.card_id = card_id.to_string();
            for (k, v) in tags {
                e.set(k, v);
            }
        };
        // My two Undead and the opponent's one, which the log also enchants.
        add(100, "BG25_008", minion(1));
        add(101, "BG28_309", minion(1));
        add(200, "BG25_008", minion(2));
        // An older pair on one of mine, the current one on the other, and the
        // opponent's pair much bigger: the newest of mine wins and theirs is
        // never read.
        let mut older = army(12, 3);
        older.push(("ATTACHED", number(100)));
        add(300, UNDEAD_ARMY, older);
        let mut current = army(23, 6);
        current.push(("ATTACHED", number(101)));
        add(310, UNDEAD_ARMY, current);
        let mut theirs = army(99, 99);
        theirs.push(("ATTACHED", number(200)));
        add(320, UNDEAD_ARMY, theirs);
        // One that left play is not the current pair.
        let mut gone = army(70, 70);
        gone.push(("ATTACHED", number(101)));
        gone.push(("ZONE", tag("REMOVEDFROMGAME")));
        add(330, UNDEAD_ARMY, gone);
        assert_eq!(undead_army_numbers(&g), Some([23, 6, 0, 0]));

        // No Undead of mine on the board: the log shows no pair at all (`the
        // Attack comes from the player enchant`).
        let mut g = bglog::Game::new();
        g.real_players = [1].into_iter().collect();
        let mut add = |id: i32, card_id: &str, tags: Vec<(&str, TagValue)>| {
            let e = g.entity(id);
            e.card_id = card_id.to_string();
            for (k, v) in tags {
                e.set(k, v);
            }
        };
        add(200, "BG25_008", minion(2));
        let mut theirs = army(99, 99);
        theirs.push(("ATTACHED", number(200)));
        add(320, UNDEAD_ARMY, theirs);
        assert_eq!(undead_army_numbers(&g), None);
    }

    /// A minion's own Blood Gem stats are the log's `4270`/`4271` pair on the
    /// card, written as `gem_attack`/`gem_health` in every state: the replay
    /// seats them as `Minion::gem_attack`/`gem_health`, which is what Gem
    /// Confiscation takes from the target's neighbours (log
    /// `2026_10_26_00_48_15-g2` turn 7 step 24, the Sly Infiltrator at 4/4
    /// and then 0/0). A card with none keeps the keys out.
    #[test]
    fn a_minions_blood_gem_stats_are_written_with_its_card() {
        fn tag(value: &str) -> TagValue {
            TagValue::Text(value.to_string())
        }
        fn number(value: i64) -> TagValue {
            TagValue::Int(value)
        }
        let cards = Cards::from_pairs([("BG20_101".to_string(), "Roadboar".to_string())]);
        let mut g = bglog::Game::new();
        g.real_players = [1].into_iter().collect();
        let mut add = |id: i32, tags: Vec<(&str, TagValue)>| {
            let e = g.entity(id);
            e.card_id = "BG20_101".to_string();
            for (k, v) in tags {
                e.set(k, v);
            }
        };
        let board = |position: i64, attack: i64, health: i64| {
            vec![
                ("CARDTYPE", tag("MINION")),
                ("ZONE", tag("PLAY")),
                ("CONTROLLER", number(1)),
                ("ZONE_POSITION", number(position)),
                ("ATK", number(attack)),
                ("HEALTH", number(health)),
            ]
        };
        let mut keeps = board(1, 10, 12);
        keeps.push((GEM_TAGS[0], number(4)));
        keeps.push((GEM_TAGS[1], number(4)));
        add(100, keeps);
        let mut none = board(2, 6, 6);
        none.push((GEM_TAGS[0], number(0)));
        none.push((GEM_TAGS[1], number(0)));
        add(101, none);

        let mut collector = Collector::new(cards, "test");
        let state = collector.state(&g);
        let got: serde_json::Value = serde_json::from_str(&render(&Json::Obj(state))).unwrap();
        let board = got["board"].as_array().unwrap();
        assert_eq!(board[0]["gem_attack"], 4, "{got}");
        assert_eq!(board[0]["gem_health"], 4);
        assert!(
            board[1].get("gem_attack").is_none() && board[1].get("gem_health").is_none(),
            "a minion the log gives no Blood Gem stats has no key: {got}"
        );
        // The pair is read as one: a minion with only one half still says both.
        let mut only = HashMap::new();
        only.insert(GEM_TAGS[0].to_string(), TagValue::Int(0));
        only.insert(GEM_TAGS[1].to_string(), TagValue::Int(5));
        assert_eq!(gem_stats(&only), Some((0, 5)));
    }

    /// `crates/bglog/tests/data/fight.log`, a trimmed real log, with the turn
    /// moved into the full-state dump the way a reconnect prints it: no
    /// `TURN` tag change follows, so the collector has to open the turn from
    /// the dump. The options dump and the click are the real ones' shape.
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
            "D 00:58:25.1783940 GameState.DebugPrintOptions() - id=1\n\
             D 00:58:25.1783940 GameState.DebugPrintOptions() -   option 0 type=END_TURN mainEntity= error=INVALID errorParam=\n\
             D 00:58:25.1783940 GameState.SendOption() - selectedOption=0 selectedSubOption=-1 selectedTarget=0 selectedPosition=0\n",
            1,
        )
    }

    #[test]
    fn a_mid_turn_reconnect_records_its_actions() {
        let cards = Cards::from_pairs(Vec::<(String, String)>::new());
        let mut reader = std::io::Cursor::new(reconnect_excerpt());
        let games = read_log_reader(&mut reader, &cards, "test", &HashMap::new()).unwrap();
        assert_eq!(games.len(), 1, "one game");
        let game = games[0].as_ref().expect("the reconnected game has a file");
        let got: serde_json::Value = serde_json::from_str(&render(game)).unwrap();
        let turns = got["turns"].as_array().unwrap();
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0]["turn"], 2);
        assert_eq!(turns[0]["joined_mid_turn"], true);
        assert_eq!(turns[0]["actions"].as_array().unwrap().len(), 1);
        assert_eq!(turns[0]["actions"][0]["step"], "end_turn");
    }

    /// A whole trimmed real game between the spectator markers is another
    /// player's game: it writes no action file, and still takes its session
    /// number (a `None` record) so later games keep theirs.
    #[test]
    fn a_spectated_stretch_writes_no_action_file() {
        let text = format!(
            "D 00:00:00.0000000 ================== Begin Spectating 1st player ==================\n\
             {}D 00:00:00.0000000 ================== End Spectator Mode ==================\n",
            include_str!("../../bglog/tests/data/fight.log")
        );
        let cards = Cards::from_pairs(Vec::<(String, String)>::new());
        let mut reader = std::io::Cursor::new(text);
        let games = read_log_reader(&mut reader, &cards, "test", &HashMap::new()).unwrap();
        assert_eq!(games.len(), 1, "the spectated session keeps its number");
        assert!(games[0].is_none(), "and has no action file: {games:?}");
    }
}
