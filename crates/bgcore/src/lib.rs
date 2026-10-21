//! Rules of the v0.1 mini Battlegrounds: state, shop, fight. No I/O and no
//! Python here, see DESIGN.md.

// Card ids index the pool and the card list, so index loops read best there.
#![allow(clippy::needless_range_loop)]

pub mod action;
pub mod bots;
pub mod cards;
pub mod fight;
pub mod game;
pub mod minion;
pub mod obs;
pub mod rng;
pub mod rules;

pub use action::{Action, N_ACTIONS};
pub use cards::{Card, CardDb, Effect, Tribe};
pub use fight::{Outcome, fight};
pub use game::{Game, IllegalAction, Opponent, Player};
pub use minion::{CardId, Keywords, Minion};
pub use rng::Rng;
pub use rules::Rules;

pub const PLAYERS: usize = 8;
pub const BOARD: usize = 7;
pub const HAND: usize = 10;
pub const SHOP: usize = 6;
pub const DISCOVER: usize = 3;
pub const MAX_TIER: u8 = 6;

/// Cards and rules together, shared by every game.
#[derive(Debug, Clone)]
pub struct Data {
    pub cards: CardDb,
    pub rules: Rules,
}

impl Data {
    pub fn from_toml(cards: &str, rules: &str) -> Result<Data, String> {
        Ok(Data { cards: CardDb::from_toml(cards)?, rules: Rules::from_toml(rules)? })
    }

    /// The cards.toml and rules.toml at the repo root, built into the binary.
    pub fn bundled() -> Data {
        Data::from_toml(
            include_str!("../../../cards.toml"),
            include_str!("../../../rules.toml"),
        )
        .expect("bundled cards.toml or rules.toml is broken")
    }
}
