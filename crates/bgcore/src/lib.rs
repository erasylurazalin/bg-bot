//! The rules of Hearthstone Battlegrounds as this project models them:
//! card data, the shop phase and the fight. No I/O and no Python here,
//! see DESIGN.md.

// Card ids index the pool and the card list, so index loops read best there.
#![allow(clippy::needless_range_loop)]

pub mod action;
pub mod bots;
pub mod cards;
pub mod fight;
pub mod game;
pub mod minion;
pub mod obs;
pub mod record;
pub mod rng;
pub mod rules;

pub use action::{ACTION_KINDS, Action, N_ACTIONS, N_KINDS};
pub use cards::{Card, CardDb, Effect, Tribe, Tribes};
pub use fight::{Counters, Outcome, SideOutcome, fight, fight_with};
pub use game::{Game, IllegalAction, Opponent, Player, Sandbox, SandboxFight};
pub use minion::{Attached, CardId, Keywords, MAX_ATTACHED, Minion};
pub use rng::Rng;
pub use rules::Rules;

pub const PLAYERS: usize = 8;
pub const BOARD: usize = 7;
pub const HAND: usize = 10;
/// Tavern minion slots. A tavern has `shop_size(tier)` minion slots plus the
/// spell slot; a Fodder can take the spell slot too, so the shop can hold one
/// more minion than the tier's shop size.
pub const SHOP: usize = 7;
pub const DISCOVER: usize = 3;
/// The highest Tavern tier: the most a player can level to, and the most any
/// "up to your tier" draw can reach.
pub const MAX_TIER: u8 = 6;
/// The highest tier a card can have. Tier 7 minions are in the pool but the
/// Tavern never offers them: only an effect that names tier 7 produces one.
pub const CARD_MAX_TIER: u8 = 7;
/// Slots in `Player::final_tribes`: the ten tribes in `Tribe::EVERY` order,
/// then neutral. A multi-tribe minion counts in each of its tribes' slots.
pub const TRIBES: usize = 11;

/// Cards and rules together, shared by every game.
#[derive(Debug, Clone)]
pub struct Data {
    pub cards: CardDb,
    pub rules: Rules,
}

impl Data {
    pub fn from_toml(cards: &str, rules: &str) -> Result<Data, String> {
        let cards = CardDb::from_toml(cards)?;
        if cards.triple_reward.is_none() {
            return Err("cards.toml: no Triple Reward spell: a golden minion played from hand has no reward to give".into());
        }
        Ok(Data { cards, rules: Rules::from_toml(rules)? })
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
