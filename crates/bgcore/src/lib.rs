//! The rules of Hearthstone Battlegrounds as this project models them:
//! card data, the shop phase and the fight. No I/O and no Python here,
//! see docs/rules/design.md.

// Card ids index the pool and the card list, so index loops read best there.
#![allow(clippy::needless_range_loop)]

pub mod action;
pub mod bots;
pub mod cards;
pub mod comps;
pub mod darkgifts;
pub mod era_start;
pub mod fight;
pub mod fight_check;
pub mod fight_export;
pub mod game;
pub mod heroes;
pub mod lobster;
pub mod minion;
pub mod obs;
pub mod overlay;
pub mod planner;
pub mod real_boards;
pub mod record;
pub mod rng;
pub mod rules;
pub mod scenario;
pub mod screen_replay;
pub mod search;
pub mod secrets;
pub mod shop_replay;
pub mod trinkets;

pub use action::{ACTION_KINDS, Action, N_ACTIONS, N_KINDS};
pub use cards::{Card, CardDb, Effect, Timewarp, Tribe, Tribes};
pub use comps::{Comp, Comps};
pub use overlay::{Overlay, OverlayState, ShellLine};
pub use planner::Planner;
pub use darkgifts::{Gift, GiftCond, GiftDb};
pub use era_start::{EraPosition, EraSplit, EraStartConfig, EraStartPlan, EraStarts};
pub use fight::{Counters, Outcome, SideOutcome, fight, fight_with};
pub use game::{
    FightResult, FightSideSnapshot, FightSnapshot, Game, IllegalAction, Leaderboard, Opponent,
    Player, PowerState, Sandbox, SandboxFight, ShopStateCapture, TrinketState,
};
pub use heroes::{Hero, HeroDb, HeroPower, PowerEffect};
pub use lobster::LobsterMode;
pub use minion::{Attached, CardId, GiftOn, Keywords, MAX_ATTACHED, MAX_GIFTS, Minion};
pub use real_boards::{
    CardIdMap, LATE_TURN, RealBoard, RealBoardSpec, RealBoards, RealFightSpec, RealMinionSpec,
    RealPowerSpec, RealResult, RealRewardSpec, RealSide, RealTrinketSpec, pair_fights,
    parse_board_files,
};
pub use rng::Rng;
pub use rules::Rules;
pub use scenario::{ScenarioConfig, ScenarioPlan, ScenarioSeat, deadline_tier};
pub use secrets::Secrets;
pub use trinkets::{Trinket, TrinketDb, TRINKET_CHOICES, TRINKET_SLOTS};

pub const PLAYERS: usize = 8;
pub const BOARD: usize = 7;
pub const HAND: usize = 10;
/// Tavern minion slots. A tavern has `shop_size(tier)` minion slots plus the
/// spell slot; a Fodder can take the spell slot too, so the shop can hold one
/// more minion than the tier's shop size.
pub const SHOP: usize = 7;
pub const DISCOVER: usize = 3;
/// Answer slots of one `Action::Choose`: a Discover offers DISCOVER options,
/// a chooser a fixed list (Bru'kan's four Elements is the longest).
pub const CHOICES: usize = 4;
/// The highest Tavern tier: the most a player can level to, and the most any
/// "up to your tier" draw can reach.
pub const MAX_TIER: u8 = 6;
/// The highest tier a card can have. Tier 7 minions are in the pool but the
/// Tavern never offers them: only an effect that names tier 7 produces one.
pub const CARD_MAX_TIER: u8 = 7;
/// Slots in `Player::final_tribes`: the ten tribes in `Tribe::EVERY` order,
/// then neutral. A multi-tribe minion counts in each of its tribes' slots.
pub const TRIBES: usize = 11;
/// Fights the leaderboard shows for a player: the last three, most recent
/// first (`Player::fights`).
pub const FIGHTS_SHOWN: usize = 3;

/// Cards and rules together, shared by every game.
#[derive(Debug, Clone)]
pub struct Data {
    pub cards: CardDb,
    pub rules: Rules,
    /// The built heroes. Empty in a `from_toml` card set (no hero game).
    pub heroes: HeroDb,
    /// The built trinkets. Empty in a `from_toml` card set (no trinket).
    pub trinkets: TrinketDb,
    /// The built Dark Gifts. Empty in a `from_toml` card set (no gift).
    pub gifts: GiftDb,
}

impl Data {
    /// A card set with no heroes: games built from it have no pick, no armor
    /// and no powers (the parser tests use this).
    pub fn from_toml(cards: &str, rules: &str) -> Result<Data, String> {
        Data::from_toml_with_heroes(cards, rules, "")
    }

    pub fn from_toml_with_heroes(cards: &str, rules: &str, heroes: &str) -> Result<Data, String> {
        Data::from_toml_all(cards, rules, heroes, "")
    }

    /// Cards, rules, heroes and trinkets. The trinket names resolve against
    /// the card set, so a misspelt `get` is a load error. No Dark Gifts: a
    /// test that wants them passes darkgifts.toml to `from_toml_gifts`.
    pub fn from_toml_all(
        cards: &str,
        rules: &str,
        heroes: &str,
        trinkets: &str,
    ) -> Result<Data, String> {
        Data::from_toml_gifts(cards, rules, heroes, trinkets, "")
    }

    /// As `from_toml_all`, with the Dark Gift table too. Every gift names a
    /// card in the card set, so a misspelt gift card is a load error.
    pub fn from_toml_gifts(
        cards: &str,
        rules: &str,
        heroes: &str,
        trinkets: &str,
        gifts: &str,
    ) -> Result<Data, String> {
        let cards = CardDb::from_toml(cards)?;
        if cards.triple_reward.is_none() {
            return Err("cards.toml: no Triple Reward spell: a golden minion played from hand has no reward to give".into());
        }
        let trinkets = crate::trinkets::TrinketDb::from_toml(trinkets, &cards)?;
        let heroes = HeroDb::from_toml(heroes, &cards)?;
        let gifts = GiftDb::from_toml(gifts, &cards)?;
        Ok(Data { trinkets, cards, rules: Rules::from_toml(rules)?, heroes, gifts })
    }

    /// The cards.toml, rules.toml, heroes.toml, trinkets.toml and
    /// darkgifts.toml at the repo root, built into the binary.
    pub fn bundled() -> Data {
        Data::from_toml_gifts(
            include_str!("../../../cards.toml"),
            include_str!("../../../rules.toml"),
            include_str!("../../../heroes.toml"),
            include_str!("../../../trinkets.toml"),
            include_str!("../../../darkgifts.toml"),
        )
        .expect("bundled cards.toml, rules.toml, heroes.toml, trinkets.toml or darkgifts.toml is broken")
    }
}
