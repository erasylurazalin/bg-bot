//! Reads Hearthstone's `Power.log`: a game, its entities and their tags, and
//! the Battlegrounds views built on them.
//!
//! A port of `powerlog.py` and `battlegrounds.py`, the log reader and the
//! views of an older private tracker. Feed it lines as they are
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
//!
//! # The Battlegrounds views
//!
//! [`Tracker`] follows the lines and keeps the lobby and the fights current:
//! a fight is snapped when it is staged (`MAIN_START`, or the setup block
//! closing if the step came first), snapped again at its first attack, and
//! finished when `BACON_CURRENT_COMBAT_PLAYER_ID` drops back to 0.
//!
//! ```
//! use bglog::{Cards, Tracker};
//!
//! let mut tracker = Tracker::new(Cards::default());
//! tracker.feed("D 00:00:00.0000000 GameState.DebugPrintPower() - CREATE_GAME");
//! assert!(tracker.game().is_some());
//! assert!(tracker.fights().is_empty());
//! ```
//!
//! Each view is also a plain function of one [`Game`] at one moment: [`board`]
//! and [`hand`] (their [`Minion`]s, with Dark Gifts and hero power
//! enchantments), [`hero_power`], [`trinkets`], [`rewards`], [`deity`],
//! [`counts`], [`gem`], [`waiting`], [`hidden_in_hand`] and [`turn_of`].
//! [`gifts_by_minion`] counts a Dark Gift once per copy: one entry per gift
//! card that gave it (the carriers of one gift card are one copy, unless the
//! game copied them onto a golden one by one), where the Python kept one per
//! gift id. A minion that kept the same gift twice carries two. The buffs a
//! gift leaves ("+0/+0." and the like) are not gifts.
//!
//! The options the client offered me, the ones I took, and the cards of a
//! Discover or a Choose One are not in the power log: [`Options`] and
//! [`Choice`] keep what the client prints about them, and [`Handler::line`]
//! hands every such line over. [`Block`] carries a block's target, which the
//! power log prints but nothing else reads.
//!
//! Card names come from [`Cards`], read from `ref/hsjson-cards.json`
//! (git-ignored); an id no data names prints as itself, as the Python's
//! `card_name` does.

mod cards;
mod entity;
mod game;
mod options;
mod parser;
mod tracker;
mod views;

pub use cards::Cards;
pub use entity::{Entity, TagValue};
pub use game::{Game, local_time};
pub use options::{Choice, Offer, Options, Ref, choice_start, pick_start, taken};
pub use parser::{Block, Handler, PowerLog};
pub use tracker::{Snapshot, Tracker};
pub use views::{
    Board, DEITY_MINIONS, Deity, Fight, GIFT_TAG, HERO_ENCHANTMENTS, HeroPower, KEYWORDS,
    LOCK_AND_LOAD, Minion, OWES_A_SUMMON, PLAYER_COUNTS, PLAYER_ENCHANTS, REANIMATE_MARK, Reward,
    SECRET_CARDS, Seat, Trinket, UPPER_HAND, VANGUARD_AURA, WINGMEN, board, counts, deity,
    enchant_numbers, gem, gifts_by_minion, hand, held_entity, hero_enchants_by_minion, hero_power,
    hidden_in_hand, minion, one_minion, reanimates, rewards, secrets, side_numbers, trinkets,
    turn_of, vanguard_aura, waiting,
};
