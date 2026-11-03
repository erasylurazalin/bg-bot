//! Reading `Power.log` lines into a [`Game`].
//!
//! A port of `PowerLog` in the older tracker's `powerlog.py`, line for line: the
//! same lines are read, in the same order, into the same entities and tags.
//!
//! Only the `GameState.` lines are read. The log prints every line a second
//! time through `PowerTaskList`, later, as the client animates it, and we want
//! the earliest copy.
//!
//! A [`Handler`] hears about the moments the Battlegrounds layer cares about (a
//! game ending and another starting, a tag change, a block opening and
//! closing), and about every other `GameState` call the client prints, which is
//! where the options it offered and the ones I took are written down. The
//! handler is passed to each call rather than held by the reader, so a layer
//! that owns the reader can keep the handler as a separate value: it gets the
//! block depth as an argument, so it never has to look back into the reader.

use std::io::{self, BufRead};

use crate::entity::TagValue;
use crate::game::Game;
use crate::options::Ref;

/// The moments the reader reports.
///
/// Every method has an empty default, so a handler only writes the ones it
/// needs.
#[allow(unused_variables)]
pub trait Handler {
    /// The game that a `CREATE_GAME` line just ended, as it stood when the
    /// line came. It fires before [`Handler::new_game`].
    ///
    /// The Python has no such call: its layer keeps every view of a game as
    /// the lines go by. A port that only wants the finished game cannot, since
    /// most of a game's tags arrive on `FULL_ENTITY` and tag lines that fire
    /// no callback at all.
    fn end_game(&mut self, game: &Game) {}

    /// `CREATE_GAME`: the reader has just started an empty game.
    fn new_game(&mut self, game: &mut Game) {}

    /// A `TAG_CHANGE` line set `tag` on the entity `entity`. The value is
    /// already in the entity's tags when this fires. `depth` counts the blocks
    /// open now, as in [`Handler::block_start`]: the Python's Battlegrounds
    /// layer reads the reader's depth from here to know where a fight was
    /// staged.
    fn tag_change(
        &mut self,
        game: &mut Game,
        depth: usize,
        entity: i32,
        tag: &str,
        value: &TagValue,
    ) {
    }

    /// A block opened. `depth` counts the blocks open now, this one included.
    fn block_start(&mut self, game: &mut Game, depth: usize, block: Block<'_>) {}

    /// A block closed. `depth` counts the blocks still open.
    fn block_end(&mut self, game: &mut Game, depth: usize) {}

    /// A `GameState` call that is neither the power log nor the game's header:
    /// the options the client offers, the option and the card I pick, the
    /// power lists it prints around them, and the rest.
    ///
    /// `body` is the line as printed, indentation and all, since the dumps are
    /// told apart by how far they are indented. `game` is `None` until the log
    /// has started a game, and then nothing is reported, as in the Python.
    fn line(&mut self, game: Option<&mut Game>, call: &str, body: &str) {}
}

/// Turns log lines into a live [`Game`]: the parser state and the latest game.
///
/// ```
/// use bglog::{Handler, PowerLog};
///
/// let mut log = PowerLog::new();
/// log.feed("D 00:00:00.0000000 GameState.DebugPrintPower() - CREATE_GAME", &mut ());
/// assert!(log.game().is_some());
/// ```
///
/// A no-op handler is `&mut ()`: the tuple implements [`Handler`] by doing
/// nothing.
#[derive(Clone, Debug, Default)]
pub struct PowerLog {
    game: Option<Game>,
    /// Blocks open right now.
    depth: usize,
    /// The entity the run of indented `tag=` lines belongs to.
    current: Option<i32>,
}

impl Handler for () {}

impl PowerLog {
    pub fn new() -> PowerLog {
        PowerLog::default()
    }

    /// Read one line. Lines that are not `GameState.` lines are ignored.
    pub fn feed(&mut self, line: &str, handler: &mut dyn Handler) {
        match split_line(line) {
            Some((Kind::Power, body)) => self.power(body, handler),
            Some((Kind::Header, body)) => self.header(body),
            Some((Kind::Call(call), body)) => {
                handler.line(self.game.as_mut(), call, body);
            }
            None => {}
        }
    }

    /// Read until the reader ends. Lines must be UTF-8, which the log is.
    pub fn feed_reader(
        &mut self,
        reader: &mut impl BufRead,
        handler: &mut dyn Handler,
    ) -> io::Result<()> {
        let mut line = String::new();
        loop {
            line.clear();
            if reader.read_line(&mut line)? == 0 {
                return Ok(());
            }
            self.feed(&line, handler);
        }
    }

    /// The game being read: the latest `CREATE_GAME`, until a newer one.
    pub fn game(&self) -> Option<&Game> {
        self.game.as_ref()
    }

    /// Blocks open right now.
    pub fn depth(&self) -> usize {
        self.depth
    }

    /// A `DebugPrintGame` line: the game's type, then one line per player.
    fn header(&mut self, body: &str) {
        let Some(game) = self.game.as_mut() else {
            return;
        };
        if let Some(rest) = body.strip_prefix("GameType=") {
            game.game_type = rest.to_string();
        } else if let Some((player_id, name)) = player_name(body) {
            game.set_player_name(player_id, name.to_string());
        }
    }

    /// A `DebugPrintPower` line, already stripped.
    fn power(&mut self, body: &str, handler: &mut dyn Handler) {
        if body == "CREATE_GAME" {
            if let Some(ended) = self.game.take() {
                handler.end_game(&ended);
            }
            self.game = Some(Game::new());
            self.current = None;
            self.depth = 0;
            if let Some(game) = self.game.as_mut() {
                handler.new_game(game);
            }
            return;
        }
        let Some(game) = self.game.as_mut() else {
            return;
        };

        if let Some((tag, value)) = tag_line(body) {
            if let Some(entity) = self.current {
                game.entity(entity).set(tag, value);
            }
            return;
        }
        // Any other line ends the run of tag= lines that belonged to an
        // entity.
        self.current = None;

        if body.starts_with("BLOCK_START") {
            // A block counts as open even when its line cannot be read: the
            // Battlegrounds layer keeps its own stack in step with this depth.
            self.depth += 1;
            let depth = self.depth;
            if let Some((kind, entity, target)) = block_line(body) {
                let entity = resolve(game, entity);
                handler.block_start(
                    game,
                    depth,
                    Block {
                        kind,
                        entity,
                        target,
                    },
                );
            }
        } else if body == "BLOCK_END" {
            self.depth = self.depth.saturating_sub(1);
            handler.block_end(game, self.depth);
        } else if let Some((entity, tag, value)) = tag_change(body) {
            if let Some(entity) = resolve(game, entity) {
                let value = TagValue::parse(value);
                game.entity(entity).set(tag, value.clone());
                handler.tag_change(game, self.depth, entity, tag, &value);
            }
        } else if let Some((entity, card_id)) = full_entity(body) {
            if let Some(entity) = resolve(game, entity) {
                set_card_id(game, entity, card_id);
                self.current = Some(entity);
            }
        } else if let Some((entity, card_id)) = show_or_change_entity(body) {
            if let Some(entity) = resolve(game, entity) {
                set_card_id(game, entity, card_id);
                self.current = Some(entity);
            }
        } else if let Some((entity, tag, value)) = hide_entity(body) {
            if let Some(entity) = resolve(game, entity) {
                game.entity(entity).set(tag, TagValue::parse(value));
            }
        } else if let Some(id) = game_entity(body) {
            game.entity(id);
            game.names.insert("GameEntity".to_string(), id);
            self.current = Some(id);
        } else if let Some((id, player_id, real)) = player(body) {
            game.players.insert(player_id, id);
            if real {
                game.real_players.insert(player_id);
            }
            game.entity(id);
            self.current = Some(id);
        }
    }
}

/// A full entity line with no card id leaves the card id as it was.
fn set_card_id(game: &mut Game, entity: i32, card_id: &str) {
    if !card_id.is_empty() {
        game.entity(entity).card_id = card_id.to_string();
    }
}

/// The entity a reference means, created if it is an id the log has not named
/// yet. A name resolves through the game's names, and a name never seen goes
/// to the player standing in for your opponent.
fn resolve(game: &mut Game, reference: &str) -> Option<i32> {
    if !reference.is_empty() && reference.bytes().all(|b| b.is_ascii_digit()) {
        let id: i32 = reference.parse().ok()?;
        game.entity(id);
        return Some(id);
    }
    if reference.starts_with('[') {
        let id = bracket_id(reference)?;
        game.entity(id);
        return Some(id);
    }
    let id = match game.names.get(reference) {
        Some(&id) => id,
        None => learn_name(game, reference)?,
    };
    if game.entities.contains_key(&id) {
        game.last_name.insert(id, reference.to_string());
        Some(id)
    } else {
        None
    }
}

/// Tie a name to a player entity.
///
/// The header names every player, but in Battlegrounds the second player
/// entity stands in for whoever you are fighting, and the log calls it by that
/// opponent's name. So a name from the header goes to its own player, and any
/// other name goes to the player with no real account behind it.
fn learn_name(game: &mut Game, name: &str) -> Option<i32> {
    let from_header = game
        .player_names
        .iter()
        .find(|(player_id, header_name)| {
            header_name == name && game.players.contains_key(player_id)
        })
        .and_then(|(player_id, _)| game.players.get(player_id).copied());
    let entity = match from_header {
        Some(entity) => entity,
        None => game.stand_in()?,
    };
    game.names.insert(name.to_string(), entity);
    Some(entity)
}

/// Which kind of `GameState.` line this is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind<'a> {
    /// `DebugPrintPower`: the game itself.
    Power,
    /// `DebugPrintGame`: the game's header.
    Header,
    /// Any other call the client prints, by name: the options it offers, the
    /// pick I make, and the power lists and debug dumps around them.
    Call(&'a str),
}

/// `D <timestamp> GameState.<call>() - <body>`: the kind of line and its body.
///
/// A power or header line is trimmed, since the client puts no indentation
/// under those. Any other call keeps its body as printed: the dumps are
/// indented, and that indentation is how their lines are told apart.
fn split_line(line: &str) -> Option<(Kind<'_>, &str)> {
    let line = line.trim_end_matches(['\r', '\n']);
    let rest = line.strip_prefix("D ")?;
    let timestamp = rest.find(' ')?;
    if timestamp == 0 {
        return None;
    }
    let rest = rest[timestamp + 1..].strip_prefix("GameState.")?;
    let (call, body) = rest.split_once("() - ")?;
    if call.is_empty()
        || !call
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        return None;
    }
    Some(match call {
        "DebugPrintPower" => (Kind::Power, body.trim()),
        "DebugPrintGame" => (Kind::Header, body.trim()),
        other => (Kind::Call(other), body),
    })
}

/// `PlayerID=<id>, PlayerName=<name>`: the header's line for one player.
fn player_name(body: &str) -> Option<(i32, &str)> {
    let rest = body.strip_prefix("PlayerID=")?;
    let digits = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
    let player_id = rest[..digits].parse().ok()?;
    let name = rest[digits..].strip_prefix(", PlayerName=")?;
    Some((player_id, name))
}

/// `tag=<tag> value=<value>`: one of the lines under an entity.
fn tag_line(body: &str) -> Option<(&str, TagValue)> {
    let rest = body.strip_prefix("tag=")?;
    let space = rest.find(' ')?;
    let tag = &rest[..space];
    if tag.is_empty() || tag.chars().any(char::is_whitespace) {
        return None;
    }
    let value = rest[space + 1..].strip_prefix("value=")?;
    if value.is_empty() || value.chars().any(char::is_whitespace) {
        return None;
    }
    Some((tag, TagValue::parse(value)))
}

/// A block the log has just opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block<'a> {
    /// `BlockType=`: `PLAY`, `MOVE_MINION`, `TRIGGER` and the rest.
    pub kind: &'a str,
    /// `Entity=`: the entity the block acts on, resolved to the number the
    /// reader keeps. It may name an entity no line has created yet, since the
    /// reader makes one for a bare number.
    pub entity: Option<i32>,
    /// `Target=`: what the block is aimed at, as the line spells it out, or
    /// `None` when the line names nothing or could not be read.
    ///
    /// The target is not added to the game, because the Python's Battlegrounds
    /// layer does not add it either: it is read only by a layer that works out
    /// what an action did, and that layer looks the entity up itself.
    pub target: Option<Ref>,
}

/// `BLOCK_START BlockType=<type> Entity=<ref> EffectCardId=<card>
/// EffectIndex=<n> Target=<ref> SubOption=<n>`: the block's type, the entity
/// reference and its target.
///
/// Both references run to a fixed word after them, so the log's greedy
/// `(.+?)` fields are cut at the first word that leaves the rest of the line
/// well formed, the way the Python's regex backtracks.
fn block_line(body: &str) -> Option<(&str, &str, Option<Ref>)> {
    let rest = body.strip_prefix("BLOCK_START BlockType=")?;
    let space = rest.find(' ')?;
    let block_type = &rest[..space];
    if block_type.is_empty() || block_type.chars().any(char::is_whitespace) {
        return None;
    }
    let rest = rest[space + 1..].strip_prefix("Entity=")?;
    let end = rest.find(" EffectCardId=")?;
    if end == 0 {
        return None;
    }
    let entity = &rest[..end];
    let rest = &rest[end + " EffectCardId=".len()..];
    let rest = rest.trim_start_matches(|c: char| !c.is_whitespace());
    let rest = rest.strip_prefix(" EffectIndex=")?;
    let rest = signed(rest)?;
    let rest = rest.strip_prefix(" Target=")?;
    // The target runs to the ` SubOption=` that leaves the rest of the line
    // well formed; a later one is tried when an earlier one does not.
    let mut from = 0;
    loop {
        let at = rest[from..].find(" SubOption=")? + from;
        if sub_option(&rest[at + " SubOption=".len()..]) {
            return Some((block_type, entity, Ref::parse(&rest[..at])));
        }
        from = at + 1;
    }
}

/// The tail of a block's line after `SubOption=`: the number, then the
/// keyword the card may have triggered, then the end of the line.
fn sub_option(rest: &str) -> bool {
    let Some(rest) = signed(rest) else {
        return false;
    };
    match rest.strip_prefix(" TriggerKeyword=") {
        Some(rest) => rest.trim_start_matches(|c: char| !c.is_whitespace()).is_empty(),
        None => rest.is_empty(),
    }
}

/// `-?<digits>` at the start of `rest`, and what follows.
fn signed(rest: &str) -> Option<&str> {
    let rest = rest.strip_prefix('-').unwrap_or(rest);
    let len = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
    if len == 0 {
        return None;
    }
    Some(&rest[len..])
}

/// `TAG_CHANGE Entity=<ref> tag=<tag> value=<value>`, with or without the
/// ` DEF CHANGE` marker the log puts on a change it decides itself.
fn tag_change(body: &str) -> Option<(&str, &str, &str)> {
    let rest = body.strip_prefix("TAG_CHANGE Entity=")?;
    split_at_tag(rest, true)
}

/// `HIDE_ENTITY - Entity=<ref> tag=<tag> value=<value>`.
fn hide_entity(body: &str) -> Option<(&str, &str, &str)> {
    let rest = body.strip_prefix("HIDE_ENTITY - Entity=")?;
    split_at_tag(rest, false)
}

/// Split `<ref> tag=<tag> value=<value>[ DEF CHANGE]` at the last ` tag=` that
/// leaves a well formed tail. The Python's `(.+)` is greedy, so the longest
/// reference wins; references never contain ` tag=` in practice.
fn split_at_tag(rest: &str, def_change: bool) -> Option<(&str, &str, &str)> {
    let mut end = rest.len();
    loop {
        let at = rest[..end].rfind(" tag=")?;
        if at > 0
            && let Some((tag, value)) = tag_and_value(&rest[at + 5..], def_change)
        {
            return Some((&rest[..at], tag, value));
        }
        if at == 0 {
            return None;
        }
        end = at;
    }
}

/// `<tag> value=<value>[ DEF CHANGE]` running to the end of the line.
fn tag_and_value(text: &str, def_change: bool) -> Option<(&str, &str)> {
    let space = text.find(' ')?;
    let tag = &text[..space];
    if tag.is_empty() || tag.chars().any(char::is_whitespace) {
        return None;
    }
    let rest = text[space + 1..].strip_prefix("value=")?;
    let value = match def_change {
        true => rest.strip_suffix(" DEF CHANGE").unwrap_or(rest),
        false => rest,
    };
    if value.is_empty() || value.chars().any(char::is_whitespace) {
        return None;
    }
    Some((tag, value))
}

/// `FULL_ENTITY - Creating|Updating ID=|Entity=<ref> CardID=<card id>`.
fn full_entity(body: &str) -> Option<(&str, &str)> {
    let rest = body.strip_prefix("FULL_ENTITY - ")?;
    let rest = rest
        .strip_prefix("Creating ")
        .or_else(|| rest.strip_prefix("Updating "))?;
    let rest = rest
        .strip_prefix("ID=")
        .or_else(|| rest.strip_prefix("Entity="))?;
    split_at_card_id(rest)
}

/// `SHOW_ENTITY - Updating Entity=<ref> CardID=<card id>`, and the same line
/// for a `CHANGE_ENTITY`.
fn show_or_change_entity(body: &str) -> Option<(&str, &str)> {
    let rest = body
        .strip_prefix("SHOW_ENTITY - Updating Entity=")
        .or_else(|| body.strip_prefix("CHANGE_ENTITY - Updating Entity="))?;
    split_at_card_id(rest)
}

/// `<ref> CardID=<card id>`: the reference up to the first ` CardID=`, the card
/// id (which may be empty) to the end.
fn split_at_card_id(rest: &str) -> Option<(&str, &str)> {
    let end = rest.find(" CardID=")?;
    if end == 0 {
        return None;
    }
    let card_id = &rest[end + " CardID=".len()..];
    if card_id.chars().any(char::is_whitespace) {
        return None;
    }
    Some((&rest[..end], card_id))
}

/// `GameEntity EntityID=<id>`.
fn game_entity(body: &str) -> Option<i32> {
    let rest = body.strip_prefix("GameEntity EntityID=")?;
    digits(rest)?.parse().ok()
}

/// `Player EntityID=<id> PlayerID=<id> GameAccountId=[hi=<hi> lo=<lo>]`: the
/// entity, the player, and whether a real account is behind it (`hi` is not 0).
fn player(body: &str) -> Option<(i32, i32, bool)> {
    let rest = body.strip_prefix("Player EntityID=")?;
    let digits_len = rest.find(|c: char| !c.is_ascii_digit())?;
    let entity: i32 = rest[..digits_len].parse().ok()?;
    let rest = rest[digits_len..].strip_prefix(" PlayerID=")?;
    let digits_len = rest.find(|c: char| !c.is_ascii_digit())?;
    let player_id: i32 = rest[..digits_len].parse().ok()?;
    let rest = rest[digits_len..].strip_prefix(" GameAccountId=[hi=")?;
    let digits_len = rest.find(|c: char| !c.is_ascii_digit())?;
    let hi = &rest[..digits_len];
    let rest = rest[digits_len..].strip_prefix(" lo=")?;
    digits(rest.strip_suffix(']')?)?;
    Some((entity, player_id, hi != "0"))
}

/// The whole text when it is one or more digits.
fn digits(text: &str) -> Option<&str> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(text)
}

/// The `id=` of an entity reference `[entityName=... id=287 zone=... ]`: the
/// first `\bid=` the Python's `BRACKET_ID` finds.
fn bracket_id(reference: &str) -> Option<i32> {
    let bytes = reference.as_bytes();
    let mut from = 0;
    while let Some(at) = reference[from..].find("id=").map(|k| from + k) {
        let starts_word = at == 0
            || !(bytes[at - 1].is_ascii_alphanumeric() || bytes[at - 1] == b'_');
        if starts_word {
            let digits_start = at + "id=".len();
            let mut end = digits_start;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
            if end > digits_start {
                return reference[digits_start..end].parse().ok();
            }
        }
        from = at + 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gamestate_lines_are_split_into_a_kind_and_a_body() {
        let (kind, body) = split_line(
            "D 00:57:53.6478530 GameState.DebugPrintPower() -     tag=ZONE value=PLAY\r\n",
        )
        .unwrap();
        assert!(matches!(kind, Kind::Power));
        assert_eq!(body, "tag=ZONE value=PLAY");
        let (kind, body) =
            split_line("D 00:57:53.6478530 GameState.DebugPrintGame() - GameType=GT_BATTLEGROUNDS")
                .unwrap();
        assert!(matches!(kind, Kind::Header));
        assert_eq!(body, "GameType=GT_BATTLEGROUNDS");
    }

    #[test]
    fn other_log_lines_are_not_read() {
        // The later copy of every line, and lines that are not `GameState.`.
        assert!(
            split_line("D 00:57:53.6478530 PowerTaskList.DebugPrintPower() - CREATE_GAME").is_none()
        );
        assert!(split_line("not a log line").is_none());
        assert!(split_line("D GameState.DebugPrintPower() - CREATE_GAME").is_none());
        assert!(split_line("").is_none());
    }

    #[test]
    fn the_other_gamestate_calls_keep_their_indentation() {
        let (kind, body) = split_line("D 00:57:53.6478530 GameState.DebugPrintOptions() - id=1")
            .unwrap();
        assert!(matches!(kind, Kind::Call("DebugPrintOptions")));
        assert_eq!(body, "id=1");
        let (kind, body) = split_line(
            "D 00:57:53.6478530 GameState.DebugPrintOptions() -   option 0 type=END_TURN mainEntity= error=INVALID errorParam=\r\n",
        )
        .unwrap();
        assert!(matches!(kind, Kind::Call("DebugPrintOptions")));
        assert_eq!(
            body,
            "  option 0 type=END_TURN mainEntity= error=INVALID errorParam="
        );
        assert!(matches!(
            split_line("D 00:57:53.6478530 GameState.DebugPrintPowerList() - Count=44")
                .unwrap()
                .0,
            Kind::Call("DebugPrintPowerList")
        ));
    }

    #[test]
    fn entity_references_are_numbers_brackets_or_names() {
        let mut game = Game::new();
        assert_eq!(resolve(&mut game, "25"), Some(25));
        assert!(game.entities.contains_key(&25));
        assert_eq!(
            resolve(
                &mut game,
                "[entityName=Lesser Trinket id=287 zone=PLAY zonePos=0 cardId=BG30_Trinket_1st player=1]"
            ),
            Some(287)
        );
        // No id in the bracket: nothing to make.
        assert_eq!(resolve(&mut game, "[entityName=UNKNOWN]"), None);
        // cardId= has no bare id=, so it is not mistaken for one.
        assert_eq!(resolve(&mut game, "[cardId=BG28_966]"), None);
    }

    #[test]
    fn a_block_with_an_unreadable_line_still_counts() {
        let mut log = PowerLog::new();
        log.feed("D 0 GameState.DebugPrintPower() - CREATE_GAME", &mut ());
        log.feed(
            "D 0 GameState.DebugPrintPower() - BLOCK_START BlockType=POWER Entity=1",
            &mut (),
        );
        assert_eq!(log.depth(), 1);
        log.feed("D 0 GameState.DebugPrintPower() - BLOCK_END", &mut ());
        assert_eq!(log.depth(), 0);
        // One too many ends stays at 0.
        log.feed("D 0 GameState.DebugPrintPower() - BLOCK_END", &mut ());
        assert_eq!(log.depth(), 0);
    }

    #[test]
    fn full_entity_lines_take_both_forms() {
        // What the GameState lines use.
        assert_eq!(
            full_entity("FULL_ENTITY - Creating ID=25 CardID=TB_BaconShop_HERO_PH"),
            Some(("25", "TB_BaconShop_HERO_PH"))
        );
        assert_eq!(full_entity("FULL_ENTITY - Creating ID=250 CardID="), Some(("250", "")));
        // The bracket form is what the PowerTaskList copy of the line looks
        // like, and those lines are never read; the Python does not read it
        // either, and neither do we.
        assert_eq!(
            full_entity(
                "FULL_ENTITY - Updating [entityName=BaconPHhero id=25 zone=PLAY zonePos=0 cardId=TB_BaconShop_HERO_PH player=1] CardID=TB_BaconShop_HERO_PH"
            ),
            None
        );
        // The GameState form of an update names the entity directly.
        assert_eq!(
            full_entity("FULL_ENTITY - Updating Entity=250 CardID=TB_BaconShop_HP_038e"),
            Some(("250", "TB_BaconShop_HP_038e"))
        );
        assert_eq!(
            show_or_change_entity("SHOW_ENTITY - Updating Entity=250 CardID=TB_BaconShop_HP_038e"),
            Some(("250", "TB_BaconShop_HP_038e"))
        );
        assert_eq!(
            show_or_change_entity(
                "CHANGE_ENTITY - Updating Entity=[entityName=Lesser Trinket id=287 zone=PLAY zonePos=0 cardId=BG30_Trinket_1st player=1] CardID=BG36_MagicItem_220"
            ),
            Some((
                "[entityName=Lesser Trinket id=287 zone=PLAY zonePos=0 cardId=BG30_Trinket_1st player=1]",
                "BG36_MagicItem_220"
            ))
        );
        assert_eq!(show_or_change_entity("SHOW_ENTITY - Updating Entity=250"), None);
    }

    #[test]
    fn a_tag_change_may_carry_the_def_change_marker() {
        let plain = tag_change("TAG_CHANGE Entity=25 tag=ZONE value=PLAY");
        let marked = tag_change("TAG_CHANGE Entity=25 tag=ZONE value=PLAY DEF CHANGE");
        assert_eq!(plain, Some(("25", "ZONE", "PLAY")));
        assert_eq!(marked, Some(("25", "ZONE", "PLAY")));
        // A hide line never carries the marker.
        assert_eq!(
            hide_entity("HIDE_ENTITY - Entity=25 tag=ZONE value=REMOVEDFROMGAME"),
            Some(("25", "ZONE", "REMOVEDFROMGAME"))
        );
        assert_eq!(
            hide_entity("HIDE_ENTITY - Entity=25 tag=ZONE value=GONE DEF CHANGE"),
            None
        );
    }
}
