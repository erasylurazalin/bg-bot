//! Reads Hearthstone's `Power.log`: a game, its entities and their tags.
//!
//! A port of `~/projects/bg/bg/powerlog.py`, the tracker's log reader, and the
//! base the Battlegrounds views are built on. Feed it lines as they are
//! written and it keeps the entity table the game client holds: every
//! `FULL_ENTITY`, `SHOW_ENTITY`, `CHANGE_ENTITY`, `HIDE_ENTITY` and
//! `TAG_CHANGE`, and the indented `tag=` lines under an entity.
//!
//! ```
//! use bglog::PowerLog;
//!
//! let mut log = PowerLog::new();
//! let text = "\
//! D 00:00:00.0000000 GameState.DebugPrintPower() - CREATE_GAME
//! D 00:00:00.0000000 GameState.DebugPrintPower() - FULL_ENTITY - Creating ID=25 CardID=BG33_HERO_001
//! D 00:00:00.0000000 GameState.DebugPrintPower() -     tag=ZONE value=HAND
//! D 00:00:01.0000000 GameState.DebugPrintPower() - TAG_CHANGE Entity=25 tag=ZONE value=PLAY
//! ";
//! for line in text.lines() {
//!     log.feed(line, &mut ());
//! }
//! let game = log.game().unwrap();
//! assert_eq!(game.entities[&25].card_id, "BG33_HERO_001");
//! assert_eq!(game.entities[&25].text("ZONE"), "PLAY");
//! ```
//!
//! A game starts at `CREATE_GAME` and the next one replaces it, so the reader
//! holds the latest game only. A collector that wants every game copies it
//! when [`Handler::end_game`] fires, which hands over the finished game just
//! before the next one starts.
//!
//! Three rules from the Python that the rest of the log path relies on:
//!
//! - Only `GameState.` lines are read. The log prints every line a second time
//!   through `PowerTaskList`, as the client animates it, and the earlier copy
//!   is the one we want.
//! - Tags keep the spelling of the log: names stay names, and a tag the client
//!   has no name for keeps its number as the key. Values that are numbers are
//!   [`TagValue::Int`], enums like `PLAY` stay [`TagValue::Text`].
//! - An entity reference is an id (`Entity=25`), a bracketed description
//!   (`[entityName=... id=287 ...]`), or a name (`Entity=Bartender Bob`).
//!   Names resolve through the header's player names, and any other name goes
//!   to the player with no real account behind it (the stand-in opponent).

mod entity;
mod game;
mod parser;

pub use entity::{Entity, TagValue};
pub use game::Game;
pub use parser::{Handler, PowerLog};
