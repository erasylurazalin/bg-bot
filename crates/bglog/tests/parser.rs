//! The reader on snippets cut from Era's logs.
//!
//! Every fixture in `tests/data/` is a few lines of a real `Power.log` (one or
//! two consecutive lines, or two short runs joined, from the games of
//! 2026-10-24 00:56 and 2026-10-24 17:59), with the player names replaced by
//! Alice, Eve and Carol and the account ids by fake numbers, so they carry
//! nothing personal. Each one holds the lines for one kind of handling; see
//! the test that reads it.
//!
//! Where each came from, `create_game.log` first:
//!
//! | fixture | `~/bg-logs/Hearthstone_...` |
//! |---|---|
//! | `create_game.log` | `2026_10_24_00_56_06`: 2-5, 48-50, 73-75, 94-101, 240-245 |
//! | `entities.log` | the same game: 2-3, 2026-2036, 4742-4743, 52632-52639, 16836 |
//! | `blocks.log` | the same game: 2-3, 208-216 |
//! | `names.log` | the same game: 2-3, 48-50, 73-75, 240-245, 616, 1962-1963 |
//! | `noise.log` | the same game: 2-3, 1, 457, 1743-1744, 2921-2922, 319, 340-341, 451-453 |
//! | `two_games.log` | `2026_10_24_17_59_26`: 2-3, 48-50, 94-96, 244-245, 725036-725037, 725082-725084, 725128-725130, 725278-725279 |

use std::collections::{HashMap, HashSet};
use std::fs;

use bglog::{Game, Handler, PowerLog, Ref, TagValue};

fn fixture(name: &str) -> String {
    let path = format!("{}/tests/data/{name}", env!("CARGO_MANIFEST_DIR"));
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn read(name: &str, handler: &mut dyn Handler) -> PowerLog {
    let text = fixture(name);
    let mut lines = text.as_bytes();
    let mut log = PowerLog::new();
    log.feed_reader(&mut lines, handler)
        .unwrap_or_else(|e| panic!("{name}: {e}"));
    log
}

/// A block opening or closing, in the order the log had them.
#[derive(Debug, PartialEq, Eq)]
enum BlockEvent {
    Start {
        depth: usize,
        block_type: String,
        entity: Option<i32>,
        target: Option<Ref>,
    },
    End {
        depth: usize,
    },
}

/// What a phase 2 layer will do: hear about the moments and keep a copy of
/// each game as it grows.
#[derive(Default)]
struct Recorder {
    new_games: usize,
    games: Vec<Game>,
    tags: Vec<(i32, String, TagValue)>,
    blocks: Vec<BlockEvent>,
}

impl Recorder {
    /// Keep the game as it is now, as the copy of the game being read.
    fn keep(&mut self, game: &Game) {
        match self.games.last_mut() {
            Some(last) => *last = game.clone(),
            None => self.games.push(game.clone()),
        }
    }
}

impl Handler for Recorder {
    fn end_game(&mut self, game: &Game) {
        // This is how the finished game reaches a collector.
        *self.games.last_mut().unwrap() = game.clone();
    }

    fn new_game(&mut self, game: &mut Game) {
        self.new_games += 1;
        self.games.push(game.clone());
    }

    fn tag_change(
        &mut self,
        game: &mut Game,
        _depth: usize,
        entity: i32,
        tag: &str,
        value: &TagValue,
    ) {
        self.keep(game);
        self.tags.push((entity, tag.to_string(), value.clone()));
    }

    fn block_start(&mut self, game: &mut Game, depth: usize, block: bglog::Block<'_>) {
        self.keep(game);
        self.blocks.push(BlockEvent::Start {
            depth,
            block_type: block.kind.to_string(),
            entity: block.entity,
            target: block.target,
        });
    }

    fn block_end(&mut self, game: &mut Game, depth: usize) {
        self.keep(game);
        self.blocks.push(BlockEvent::End { depth });
    }
}

#[test]
fn create_game_reads_the_header_the_players_and_the_entities() {
    let mut rec = Recorder::default();
    let log = read("create_game.log", &mut rec);
    assert_eq!(rec.new_games, 1);
    assert_eq!(log.depth(), 0);

    let game = log.game().unwrap();
    assert_eq!(game.game_type, "GT_BATTLEGROUNDS");
    assert_eq!(
        game.player_names,
        vec![(1, "Alice#1919".to_string()), (9, "Eve".to_string())]
    );
    assert_eq!(game.player_name(9), Some("Eve"));
    assert_eq!(game.players, HashMap::from([(1, 2), (9, 3)]));
    // The header says player 1 has an account (hi is not 0), player 9 has
    // not: that is the stand-in.
    assert_eq!(game.real_players, HashSet::from([1]));

    // The indented tag= lines belong to the GameEntity line above them.
    let game_entity = game.entities.get(&1).unwrap();
    assert_eq!(game_entity.text("CARDTYPE"), "GAME");
    assert_eq!(game_entity.text("ZONE"), "PLAY");
    assert_eq!(game_entity.id, 1);

    // A Player line reads both ids and hands the following tags to the player.
    let me = game.player_entity(1).unwrap();
    assert_eq!(me.id, 2);
    assert_eq!(me.text("CARDTYPE"), "PLAYER");
    assert_eq!(me.int("CONTROLLER"), 1);
    assert_eq!(game.player_entity(9).unwrap().id, 3);

    // A FULL_ENTITY makes the entity and gives its card id, its tags follow.
    let hero = game.entities.get(&25).unwrap();
    assert_eq!(hero.card_id, "TB_BaconShop_HERO_PH");
    assert_eq!(hero.text("CARDTYPE"), "HERO");
    assert_eq!(hero.get("HEALTH"), Some(&TagValue::Int(30)));
    assert_eq!(hero.text("CLASS"), "NEUTRAL");
    // The log spells the entity's own id as a tag too.
    assert_eq!(hero.get("ENTITY_ID"), Some(&TagValue::Int(25)));
}

#[test]
fn entities_get_their_card_id_and_tags() {
    let mut rec = Recorder::default();
    let log = read("entities.log", &mut rec);
    let game = log.game().unwrap();

    // Creating with an empty CardID leaves the card id empty, then a
    // SHOW_ENTITY fills it in and the tags under that line join the entity.
    let shown = game.entities.get(&250).unwrap();
    assert_eq!(shown.card_id, "TB_BaconShop_HP_038e");
    assert_eq!(shown.text("CARDTYPE"), "ENCHANTMENT");
    assert_eq!(shown.get("ATTACHED"), Some(&TagValue::Int(2)));
    assert_eq!(shown.text("ZONE"), "SETASIDE");

    // A bracketed reference names the entity by the id= inside it, and a tag
    // the client has no name for keeps its number as the name.
    let enchant = game.entities.get(&467).unwrap();
    assert_eq!(enchant.get("1234"), Some(&TagValue::Int(296)));
    // The HIDE_ENTITY below it sets ZONE, and sets it without a tag change
    // callback: only the TAG_CHANGE line above fires one.
    assert_eq!(enchant.text("ZONE"), "REMOVEDFROMGAME");
    assert_eq!(rec.tags.iter().filter(|(entity, _, _)| *entity == 467).count(), 1);
    assert!(
        rec.tags
            .iter()
            .all(|(entity, tag, _)| !(*entity == 467 && tag == "ZONE"))
    );

    // CHANGE_ENTITY updates the card id the same way SHOW_ENTITY does.
    let trinket = game.entities.get(&287).unwrap();
    assert_eq!(trinket.card_id, "BG36_MagicItem_220");
    assert_eq!(trinket.text("CARDTYPE"), "BATTLEGROUND_TRINKET");
    assert_eq!(trinket.get("1069"), Some(&TagValue::Int(0)));
    assert_eq!(trinket.get("TAG_NOT_SET"), Some(&TagValue::Int(0)));

    // The marker the log puts on a change it decides itself does not disturb
    // the tag or the value.
    let apples = game.entities.get(&1098).unwrap();
    assert_eq!(apples.text("CARDTYPE"), "SPELL");
    assert_eq!(apples.int("CONTROLLER"), 0);
}

#[test]
fn blocks_report_their_depth() {
    let mut rec = Recorder::default();
    let log = read("blocks.log", &mut rec);
    let trigger = |depth| BlockEvent::Start {
        depth,
        block_type: "TRIGGER".to_string(),
        entity: Some(1),
        target: None,
    };
    assert_eq!(
        rec.blocks,
        vec![
            trigger(1),
            BlockEvent::End { depth: 0 },
            trigger(1),
            BlockEvent::End { depth: 0 },
            trigger(1),
            BlockEvent::End { depth: 0 },
            trigger(1),
        ]
    );
    // The snippet stops inside the last block, which is still open.
    assert_eq!(log.depth(), 1);
    // Tags inside a block belong to their entity as usual.
    let game = log.game().unwrap();
    assert_eq!(game.entities[&3].get("CURRENT_PLAYER"), Some(&TagValue::Int(1)));
    assert_eq!(game.entities[&3].get("FIRST_PLAYER"), Some(&TagValue::Int(1)));
    assert_eq!(
        rec.tags,
        vec![
            (3, "CURRENT_PLAYER".to_string(), TagValue::Int(1)),
            (3, "FIRST_PLAYER".to_string(), TagValue::Int(1)),
        ]
    );
}

#[test]
fn a_name_finds_its_player_and_an_unknown_name_the_stand_in() {
    let mut rec = Recorder::default();
    let log = read("names.log", &mut rec);
    let game = log.game().unwrap();

    // The header's own name goes to that player, the tavern keeper's name
    // (which no header names) to the only player without an account.
    assert_eq!(
        game.names,
        HashMap::from([
            ("GameEntity".to_string(), 1),
            ("Alice#1919".to_string(), 2),
            ("Bartender Bob".to_string(), 3),
        ])
    );
    assert_eq!(game.last_name.get(&2), Some(&"Alice#1919".to_string()));
    assert_eq!(game.last_name.get(&3), Some(&"Bartender Bob".to_string()));

    assert_eq!(game.entities[&2].get("NUM_TURNS_IN_PLAY"), Some(&TagValue::Int(1)));
    assert_eq!(game.entities[&2].text("MULLIGAN_STATE"), "INPUT");
    assert_eq!(game.entities[&3].get("NUM_TURNS_IN_PLAY"), Some(&TagValue::Int(1)));
}

#[test]
fn lines_outside_gamestate_change_nothing() {
    let mut rec = Recorder::default();
    let log = read("noise.log", &mut rec);
    assert_eq!(rec.new_games, 1);
    assert_eq!(log.depth(), 0);

    let game = log.game().unwrap();
    // Only the game entity, and only the one TAG_CHANGE that came from a
    // GameState line: the PowerTaskList copies, the options, the choices and
    // the PowerList count line are all skipped.
    assert_eq!(game.entities.keys().copied().collect::<Vec<_>>(), vec![1]);
    assert_eq!(
        game.entities[&1].tags,
        HashMap::from([("STEP".to_string(), TagValue::from("BEGIN_MULLIGAN"))])
    );
    assert_eq!(game.game_type, "");
    assert!(game.player_names.is_empty());
    assert!(game.players.is_empty());
    assert_eq!(rec.tags.len(), 1);
    assert!(rec.blocks.is_empty());
}

#[test]
fn the_next_game_starts_from_nothing() {
    let mut rec = Recorder::default();
    let log = read("two_games.log", &mut rec);
    assert_eq!(rec.new_games, 2);
    assert_eq!(rec.games.len(), 2);

    // The game the reader holds is the second one, with none of the first
    // game's entities.
    let game = log.game().unwrap();
    let mut ids: Vec<i32> = game.entities.keys().copied().collect();
    ids.sort();
    assert_eq!(ids, vec![1, 2, 25]);
    assert_eq!(game.names.get("GameEntity"), Some(&1));
    assert_eq!(
        game.player_names,
        vec![(1, "Alice#1919".to_string()), (9, "Carol".to_string())]
    );

    // The first game as it stood: it had its own game entity (19, not 1) and
    // its own player ids, and the second game did not touch it.
    let first = &rec.games[0];
    assert_eq!(first.names.get("GameEntity"), Some(&19));
    assert_eq!(first.players, HashMap::from([(7, 20)]));
    assert_eq!(
        first.player_names,
        vec![(7, "Alice#1919".to_string()), (15, "Eve".to_string())]
    );
    assert_eq!(first.entities[&37].card_id, "TB_BaconShop_HERO_PH");
    assert!(!first.entities.contains_key(&25));
}

#[test]
fn carriage_returns_are_trimmed() {
    let mut log = PowerLog::new();
    let lines = concat!(
        "D 00:00:00.0000000 GameState.DebugPrintPower() - CREATE_GAME\r\n",
        "D 00:00:00.0000000 GameState.DebugPrintGame() - GameType=GT_BATTLEGROUNDS\r\n",
        "D 00:00:00.0000000 GameState.DebugPrintPower() - FULL_ENTITY - Creating ID=7 CardID=BG_1\r\n",
        "D 00:00:00.0000000 GameState.DebugPrintPower() -     tag=ZONE value=PLAY\r\n",
    );
    let mut reader = lines.as_bytes();
    log.feed_reader(&mut reader, &mut ()).unwrap();
    let game = log.game().unwrap();
    assert_eq!(game.game_type, "GT_BATTLEGROUNDS");
    assert_eq!(game.entities[&7].card_id, "BG_1");
    assert_eq!(game.entities[&7].text("ZONE"), "PLAY");
}
