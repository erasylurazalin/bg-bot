//! One 8-player game: shop phase, pairing, fights, knockouts.
//!
//! The shop phase is simultaneous in the real game. Here the players act one
//! action at a time, in a random order that changes every turn, until each
//! has ended their turn. `current_player` says whose action is next.

mod discover;
mod effects;
mod heroes;
mod invariants;
mod play;
mod player;
mod shop;
mod spells;
mod targets;
mod triples;
mod turn;

use std::sync::Arc;

use arrayvec::ArrayVec;

use crate::action::{Action, N_ACTIONS};
use crate::cards::{
    N_COUNTERS, ChooseOption, ConsumePick, Effect, EventKind, SpellEffect, SpellTarget,
    Tribe, Tribes,
};
use crate::fight::{Counters, Deity, SeenCounters, SideOutcome};
use crate::heroes::PowerEffect;
use crate::minion::{CardId, Keywords, Minion};
use crate::real_boards::RealBoards;
use crate::rng::Rng;
use crate::{BOARD, DISCOVER, Data, HAND, MAX_TIER, PLAYERS, SHOP, TRIBES};

/// Upper bound on pool minions, for the fixed-size copy count in
/// `check_invariants`. The card parser refuses a card list past it.
pub const MAX_POOL_CARDS: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Opponent {
    Player(u8),
    /// The ghost of a knocked-out player.
    Ghost(u8),
}

/// What a player last saw of an opponent's board, in their last fight.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Seen {
    pub board: ArrayVec<Minion, BOARD>,
    pub tier: u8,
    /// The turn of that fight, 0 for never.
    pub turn: u32,
    /// Their Deity at the first attack of that fight, after Start of
    /// Combat effects.
    pub deity: Option<Deity>,
    /// Their Blood Gem value at that moment (what the client shows during
    /// combat, so it is "as last seen").
    pub gem: (i32, i32),
    /// Their Tavern spell bonus at that moment.
    pub spell: (i32, i32),
}

impl Seen {
    /// Both sides of a fight as the other player sees it: the board that went
    /// in, and the counters at the first attack, after Start of Combat.
    fn at_fight(board: ArrayVec<Minion, BOARD>, tier: u8, turn: u32, c: SeenCounters) -> Seen {
        Seen {
            board,
            tier,
            turn,
            deity: c.deity,
            gem: c.gem,
            spell: c.spell,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TargetKind {
    MinionBuff {
        tribe: Option<Tribe>,
        attack: i32,
        health: i32,
        keyword: u8,
        /// Also give a random Bonus Keyword the minion doesn't have.
        random_keyword: bool,
    },
    /// Pick the friendly minion that consumes `count` Tavern minions.
    /// `stat_mul` doubles the stats taken by a golden consumer.
    Consume {
        tribe: Option<Tribe>,
        pick: ConsumePick,
        keywords: bool,
        count: u8,
        stat_mul: u8,
    },
    Spell(SpellTarget),
    /// Trigger the Rally of the chosen minion in the shop (Sky-hatch
    /// Runaway), `runs` times.
    TriggerRally,
    /// Trigger the Battlecry of the chosen minion (Kelp Keeper), `runs`
    /// times.
    TriggerBattlecry,
    /// Buff the chosen minion on the board or in the Tavern (Suspicious
    /// Prisonguard).
    AnyBuff { attack: i32, health: i32 },
    /// Set the chosen minion's (board or Tavern) stats (Tyrael).
    AnySetStats { attack: i32, health: i32 },
    /// Make the chosen friendly minion (of a tribe, up to a tier) Golden on
    /// the board (Elite Navigator, Captain Sanders). The pending target's
    /// `runs` is how many are still to be made.
    MakeGolden { tribe: Option<Tribe>, max_tier: u8 },
    /// Destroy the chosen friendly minion of a tribe (not this one), then
    /// Discover `discover` minions of that tribe (Maw Caster).
    DestroyDiscover { tribe: Tribe, discover: u8 },
    /// Destroy the chosen friendly minion of a tribe (not this one), then
    /// get `copies` plain copies of it (Disguised Graverobber).
    DestroyCopy { tribe: Tribe, copies: u8 },
    /// Give the chosen friendly minion of a tribe, not a copy of this card,
    /// Reborn and destroy it; this minion gains the stats (Dead Bellringer).
    RebornDestroy { tribe: Tribe, attack: i32, health: i32 },
    /// Steal the chosen Tavern card into hand, any card (the spell slot
    /// included, a spell in a minion slot too), then the hero takes `damage`
    /// (Graveyard Shift).
    StealShopCard { damage: i32 },
}

impl TargetKind {
    /// Whether the player may pick a minion in the Tavern for this target.
    pub fn takes_shop(self) -> bool {
        match self {
            TargetKind::AnyBuff { .. } | TargetKind::AnySetStats { .. } => true,
            TargetKind::Spell(st) => st.can_target_shop(),
            TargetKind::StealShopCard { .. } => true,
            _ => false,
        }
    }
}

/// A targeted Battlecry, Choose One option, or Tavern spell waiting for the
/// player to pick its target on the board.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PendingTarget {
    /// Board slot of the minion whose effect it is, or None for a spell.
    pub source: Option<u8>,
    pub kind: TargetKind,
    pub runs: u8,
    /// The spell being cast is a Tavern spell: only those fire "Tavern spell"
    /// triggers and spell counters, and get spell power.
    pub tavern: bool,
    /// The spell being cast is a Blood Gem: its buff is the player's Blood
    /// Gem value, the stats it gives are Blood Gem stats, and each cast is a
    /// Gem played on the target (Geomagus Roogug reacts to those from hand).
    pub gem: bool,
}

/// A chosen consume waiting for the player to pick the Tavern minion.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PendingShopTarget {
    /// Board slot of the minion whose effect it is.
    pub source: u8,
    pub kind: ShopTargetKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShopTargetKind {
    Consume {
        keywords: bool,
        /// How many more Tavern minions it consumes.
        remaining: u8,
        /// Doubles the stats taken by a golden consumer.
        stat_mul: u8,
    },
    /// Replace the Tavern minion with this card (golden or not), then have the
    /// left-most Beast attack it.
    Fishbait { card: CardId, golden: bool },
}

/// A buff to Tavern minions that also reaches every minion that shows up in
/// the player's Tavern later.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TavernBuff {
    pub tribe: Option<Tribe>,
    pub attack: i32,
    pub health: i32,
    /// Ends when the player's turn does. Otherwise it lasts the game.
    pub this_turn: bool,
}

/// One buff per (tribe filter, duration): 10 tribes and no filter, times 2.
pub const MAX_TAVERN_BUFFS: usize = 22;

/// A Choose One minion or spell waiting for the player to pick one of its
/// two options.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PendingChooseOne {
    pub source: Option<u8>,
    pub golden: bool,
    pub options: [ChooseOption; 2],
    /// The spell being cast is a Tavern spell (see `PendingTarget::tavern`).
    pub tavern: bool,
}

/// One option of a "both effects combined" Choose One that still has to run,
/// after the option before it is done waiting on its own target.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct QueuedOption {
    pub source: Option<u8>,
    pub option: ChooseOption,
    /// Whether the Choose One spell was a Tavern spell.
    pub tavern: bool,
}

/// "Discard a card" (Mangled Bandit's Activate) waiting for the player to
/// pick the card in hand to throw away.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PendingDiscard {
    /// Board slot of the minion whose effect it is.
    pub source: u8,
    /// What the discard earns, run after the card is gone.
    pub reward: Effect,
    /// Whether the minion with the effect is golden (doubles the reward).
    pub golden: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DiscoverRequest {
    MinionTier(u8),
    /// A minion of exactly this tier, locked in hand for a turn (Search
    /// Through Time).
    SearchMinionTier(u8),
    SpellTier(u8),
    /// A Tavern spell of this tier or lower (Rodeo Performer).
    SpellMaxTier(u8),
    DeathrattleMinion(u8),
    BattlecryMinion(u8),
    TribeMinion(Tribe),
    /// A minion of the tribe, up to the tier, that dies if it is played in
    /// the turn it was Discovered (Tomb Turning).
    DoomedTribeMinion(Tribe),
    /// A Demon of Tier `max_tier` or lower; the pick damages the hero by its
    /// Tier, dealt by the friendly minion in board slot `source` (Imposing
    /// Percussionist).
    DemonMinion { max_tier: u8, source: u8 },
    /// A Choose One minion, any tribe, up to the tier; the pick has both
    /// effects combined (Fandral's Fortune).
    ChooseOneMinion,
}

/// Why the Tavern is filled. Only the player's own Refresh action (paid with
/// Gold, Health or free) is a Refresh for card purposes, so only it takes
/// queued Fodders. The automatic roll at the start of a turn and a card
/// effect that says "Refresh" (Snarky Shark) leave the queue alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RefreshKind {
    TurnStart,
    Explicit,
    Effect,
}

/// A player's hero power: which hero's power it is and everything the game
/// tracks about using it. Replacing the power (Unmasked Identity, Finley's
/// Discover) builds a fresh one: the old one's progress is lost.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PowerState {
    /// Index into `HeroDb` of the hero whose power this is (the player's own
    /// hero, or the one whose power was Discovered).
    pub of: u8,
    /// Button presses left this turn.
    pub uses_left: u8,
    /// Button presses this game.
    pub used_game: u32,
    /// Whether an unlock condition ("Unlocks at Tier 4") has been met.
    pub unlocked: bool,
    /// One generic progress counter, for later powers ("(N left!)").
    pub progress: u32,
}

impl PowerState {
    fn fresh(of: u8, power: &crate::heroes::HeroPower) -> PowerState {
        PowerState { of, uses_left: power.uses_per_turn, used_game: 0, unlocked: true, progress: 0 }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Player {
    pub health: i32,
    pub alive: bool,
    /// The hero this player picked (index into `HeroDb`), None in a game
    /// without heroes.
    pub hero: Option<u8>,
    /// Armor: every damage to the hero takes it first, and it never comes
    /// back unless a card says so.
    pub armor: i32,
    /// The hero power and its state. None without a hero; replaced whole by
    /// a Discovered power.
    pub power: Option<PowerState>,
    /// Heroes offered before turn 1 (indices into `HeroDb`), `u8::MAX` for an
    /// empty offer slot. While one is open the only legal action answers it.
    pub hero_offer: [u8; DISCOVER],
    /// Powers offered by a Hero Power Discover (indices into `HeroDb`),
    /// answered like a Discover.
    pub power_offer: ArrayVec<u8, DISCOVER>,
    /// Trinket slots (lesser, greater). Room for a later run, unused.
    pub trinkets: [u8; 2],
    /// Dark Gift uses. Room for a later run, unused.
    pub dark_gifts: u8,
    /// A real-board seat (docs/real-boards.md): no hero, no shopping. At the
    /// start of each of its turns it takes a random board of that turn from
    /// `Game.real_boards` and its turn is over.
    pub real: bool,
    /// 1 to 8 once decided, 0 while still playing.
    pub placement: u8,
    /// Turn the player reached each tier (1 to 6), 0 if never reached.
    pub tier_turns: [u8; MAX_TIER as usize],
    /// Counts of (Beast, Mech, neutral) minions on the player's final board.
    pub final_tribes: [u8; TRIBES],
    pub tavern_buffs: ArrayVec<TavernBuff, MAX_TAVERN_BUFFS>,
    /// Damage the hero took last, what "rewind it" gives back.
    pub last_damage: i32,
    /// How much of `last_damage` armor took: a rewind gives that much back as
    /// armor, the rest as Health.
    pub last_damage_armor: i32,
    pub gold: u8,
    pub gold_next_turn: u8,
    pub extra_max_gold: u8,
    pub free_rolls: u8,
    pub tier: u8,
    pub upgrade_cost: u8,
    pub frozen: bool,
    pub lost_last_combat: bool,
    pub won_last_combat: bool,
    pub overconfidence_win: u8,
    pub overconfidence_tie: u8,
    pub winners_bread_buff: (i32, i32),
    pub next_turn_board_buffs: ArrayVec<(i32, i32, u8), 8>,
    pub boon_of_beetles: u8,
    pub brood_of_nozdormu: u8,
    pub upper_hand: u8,
    pub sharing_is_caring: u8,
    pub volumizer_bonus: (i32, i32),
    pub beetle_bonus: (i32, i32),
    /// How many times each "improve your future ..." effect has triggered
    /// this game (Tasty Lobster).
    pub improve: [u16; N_COUNTERS],
    /// Beast bonus until next turn (Goldrinn triggered in the shop). Every
    /// Beast on the board carries it in its stats and it comes off at the
    /// start of the next turn.
    pub beast_bonus: (i32, i32),
    /// Deathrattles triggered this game, which Falling Sky Golem reads.
    pub deathrattles_triggered: u32,
    /// Golden minions played from hand to the board this game (Maritime
    /// Extortionist, Hooktusk). A minion made Golden on the board isn't one.
    pub golden_played: u32,
    /// Gold spent this turn (Lovesick Balladist). Resets every turn.
    pub gold_spent_turn: u16,
    /// Elementals played from hand this turn (Conflagration). Resets every
    /// turn.
    pub elementals_played_turn: u16,
    /// What "after the Tavern is Refreshed this game" gives: the sum of every
    /// Waveling, En-Djinn Blazer and Easterly Winds so far. Each explicit
    /// Refresh gives the whole amount to one random Tavern minion.
    pub refresh_buff: (i32, i32),
    pub spell_bonus: (i32, i32),
    pub spell_discount: u8,
    pub last_spell: Option<CardId>,
    pub shop_spell: Option<CardId>,
    pub shop: ArrayVec<Minion, SHOP>,
    pub hand: ArrayVec<Minion, HAND>,
    pub board: ArrayVec<Minion, BOARD>,
    pub pending_choose_one: Option<PendingChooseOne>,
    pub pending_target: Option<PendingTarget>,
    pub pending_shop_target: Option<PendingShopTarget>,
    /// "Discard a card" waiting for the player's pick.
    pub pending_discard: Option<PendingDiscard>,
    /// Choose One cards played whose choice is still open: Turbo Hogrider
    /// reacts once the card is fully resolved (`resolve_hogrider`).
    pub hogrider_pending: u8,
    /// Options of a "both effects combined" Choose One that still have to
    /// run, after the one before them is done waiting on its target.
    pub choose_queue: ArrayVec<QueuedOption, 2>,
    /// Discover options waiting for a choice. Minion cards here are out of the pool.
    pub discover: ArrayVec<CardId, DISCOVER>,
    /// Queued Discover requests to offer once pending choices finish.
    pub discover_queue: ArrayVec<DiscoverRequest, 8>,
    /// The board slot of the Demon whose Discover makes the hero take damage
    /// equal to the chosen card's Tier (Imposing Percussionist), if any.
    pub discover_damage_from: Option<u8>,
    /// Whether the Discover now waiting locks its minion in hand for a turn
    /// (Search Through Time).
    pub discover_locked: bool,
    /// Whether the Discover now waiting marks its minion to die when played
    /// this turn (Tomb Turning).
    pub discover_doomed: bool,
    /// Your Undead's Attack and Health "this game, wherever they are". Every
    /// Undead you own carries it in its stats (board, hand, Tavern), and
    /// every Undead made later gets it.
    pub undead_bonus: (i32, i32),
    /// Eternal Knights that died this game, which every Knight reads.
    pub knight_deaths: u32,
    /// Upcoming Refreshes and how many Fodders each one gets, oldest first
    /// (Laboratory Assistant, Trapped Clapper, Twisted Wrathguard).
    pub fodder_refreshes: ArrayVec<u8, 64>,
    /// What one of this player's Blood Gems gives, raised by "your Blood Gems
    /// give an extra +X/+Y this game". Starts at +1/+1.
    pub gem_attack: i32,
    pub gem_health: i32,
    /// Blood Gem Barrage casts: every Refresh gives this much, as Blood
    /// Gems, to every minion in the Tavern (the turn-start roll included
    /// when the Tavern changed).
    pub barrage: (i32, i32),
    /// Whether the Discover now waiting marks its Choose One pick as having
    /// both effects (Fandral's Fortune).
    pub discover_both: bool,
    pub deity: Option<Deity>,
    pub discarded: u32,
    pub spells_cast: u32,
    pub next_link: u16,
    pub actions: u16,
    pub done: bool,
    pub opponent: Option<Opponent>,
    pub last_opponent: Option<Opponent>,
    pub seen: [Seen; PLAYERS],
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Ghost {
    pub player: u8,
    pub board: ArrayVec<Minion, BOARD>,
    pub tier: u8,
    pub counters: Counters,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IllegalAction;

#[derive(Clone, Debug)]
pub struct Game {
    pub data: Arc<Data>,
    pub rng: Rng,
    /// The five tribes of this game's lobby (DESIGN.md "The lobby's
    /// tribes"): the pool and every draw are filtered to them.
    pub lobby: Tribes,
    pub turn: u32,
    pub players: [Player; PLAYERS],
    /// Copies left of each pool card, indexed by card `pool_slot`.
    pub pool: Vec<u8>,
    /// Pool copies permanently consumed into hosts via Magnetize, indexed by
    /// card `pool_slot`.
    pub consumed: Vec<u32>,
    pub ghost: Option<Ghost>,
    pub over: bool,
    /// Set only by `Game::new_sandbox`: one player, a rigged Tavern and a
    /// fight against a copy of the player's own board. None in a normal game.
    pub sandbox: Option<Sandbox>,
    /// The boards real seats (`Player.real`) draw from, set with them by
    /// `Game::new_random_heroes_real`. None in a game without real seats.
    pub real_boards: Option<Arc<RealBoards>>,
    order: ArrayVec<u8, PLAYERS>,
    cursor: usize,
}

/// Settings and results of a sandbox game (see `Game::new_sandbox`).
#[derive(Clone, Debug)]
pub struct Sandbox {
    /// Free Refreshes the player has at the start of every turn.
    pub free_rolls: u8,
    /// The latest mirror fight, for whoever records the game.
    pub last_fight: Option<SandboxFight>,
}

/// One mirror fight in a sandbox game: what fought, who won and what the
/// player's side carried over.
#[derive(Clone, Debug)]
pub struct SandboxFight {
    pub turn: u32,
    pub tier: u8,
    pub health: i32,
    /// The board and the hand as they went into the fight (after the
    /// end-of-turn effects).
    pub board: ArrayVec<Minion, BOARD>,
    pub hand: ArrayVec<Minion, HAND>,
    /// Some(0) the player won, Some(1) the mirror won, None a tie.
    pub winner: Option<usize>,
    /// What the player's side carried over, as the game applied it.
    pub side: SideOutcome,
}

impl Game {
    pub fn new(data: Arc<Data>, seed: u64) -> Game {
        let mut g = Game::unstarted(data, seed, None);
        g.start_turn();
        g
    }

    /// A game with heroes: before turn 1 every living player is offered
    /// `hero_choices` heroes and picks one with `Action::Choose`, then turn 1
    /// starts (DESIGN.md "The pick"). The lobby is rolled from the seed.
    pub fn new_heroes(data: Arc<Data>, seed: u64) -> Game {
        let mut g = Game::unstarted(data, seed, None);
        g.open_hero_picks();
        g
    }

    /// The hero game training and `bgbench` play: every player gets a random
    /// hero, no pick (Era, 2026-10-25). Finley's power Discover still waits
    /// for an answer before turn 1.
    pub fn new_random_heroes(data: Arc<Data>, seed: u64) -> Game {
        let mut g = Game::unstarted(data, seed, None);
        g.deal_random_heroes();
        g
    }

    /// The hero game with real-board seats (docs/real-boards.md): every
    /// player not in `real` gets a random hero as usual; a real seat gets
    /// none (and uses up no hero from the deck), plays with a flat 30 Health
    /// and 10 armor whatever its boards, and at the start of each of its
    /// turns takes a random board of that turn from `boards` instead of
    /// shopping.
    pub fn new_random_heroes_real(
        data: Arc<Data>,
        seed: u64,
        real: &[usize],
        boards: Arc<RealBoards>,
    ) -> Game {
        let mut g = Game::unstarted(data, seed, None);
        for &p in real {
            assert!(p < PLAYERS, "real seat {p} is not a player");
            let pl = &mut g.players[p];
            pl.real = true;
            pl.health = 30;
            pl.armor = 10;
        }
        g.real_boards = Some(boards);
        g.deal_random_heroes();
        g
    }

    /// The hero game with a fixed lobby, for tests and the sandbox.
    pub fn new_heroes_with_lobby(data: Arc<Data>, seed: u64, lobby: Tribes) -> Game {
        let mut g = Game::unstarted(data, seed, Some(lobby));
        g.open_hero_picks();
        g
    }

    /// A normal game with a fixed lobby: for tests and the sandbox, which
    /// need known tribes. Games that roll one pick exactly 5 of the 10
    /// (DESIGN.md); a test may pass more to set up its cards.
    pub fn new_with_lobby(data: Arc<Data>, seed: u64, lobby: Tribes) -> Game {
        let mut g = Game::unstarted(data, seed, Some(lobby));
        g.start_turn();
        g
    }

    /// A one-player practice game for the sandbox: only player 0 plays, no
    /// pairing and no opponents. With a `lobby`, the game's tribes are fixed
    /// to it (the sandbox's --tribe); None rolls the normal 5. The player
    /// gets `free_rolls` free Refreshes each turn, and each turn ends with a
    /// fight against a copy of the player's own board that never damages the
    /// hero. Shop-phase damage still counts and ends the game at 0 Health.
    /// Normal games never set any of this.
    pub fn new_sandbox(data: Arc<Data>, seed: u64, lobby: Option<Tribes>, free_rolls: u8) -> Game {
        let mut g = Game::unstarted(data, seed, lobby);
        for p in 1..PLAYERS {
            g.players[p].alive = false;
            g.players[p].placement = p as u8 + 1;
        }
        g.sandbox = Some(Sandbox { free_rolls, last_fight: None });
        g.start_turn();
        g
    }

    /// The sandbox with heroes: player 0 picks from an offer before turn 1,
    /// like any hero game, with the sandbox's one living player.
    pub fn new_sandbox_heroes(
        data: Arc<Data>,
        seed: u64,
        lobby: Option<Tribes>,
        free_rolls: u8,
    ) -> Game {
        let mut g = Game::unstarted(data, seed, lobby);
        for p in 1..PLAYERS {
            g.players[p].alive = false;
            g.players[p].placement = p as u8 + 1;
        }
        g.sandbox = Some(Sandbox { free_rolls, last_fight: None });
        g.open_hero_picks();
        g
    }

    /// The sandbox with one hero forced (the --hero option): the pick phase
    /// runs with that hero as the only offer, so Finley's power Discover and
    /// turn 1 line up exactly as they do with a picked hero.
    pub fn new_sandbox_with_hero(
        data: Arc<Data>,
        seed: u64,
        lobby: Option<Tribes>,
        free_rolls: u8,
        hero: u8,
    ) -> Game {
        let mut g = Game::unstarted(data, seed, lobby);
        for p in 1..PLAYERS {
            g.players[p].alive = false;
            g.players[p].placement = p as u8 + 1;
        }
        g.sandbox = Some(Sandbox { free_rolls, last_fight: None });
        g.open_hero_picks();
        g.players[0].hero_offer = [hero, u8::MAX, u8::MAX];
        g.step(crate::action::Action::Choose(0)).expect("the forced hero is offered");
        g
    }

    fn unstarted(data: Arc<Data>, seed: u64, fixed: Option<Tribes>) -> Game {
        let mut rng = Rng::new(seed);
        let lobby = fixed.unwrap_or_else(|| Tribes::random_lobby(5, &mut rng));
        let pool = (0..data.cards.pool_cards)
            .map(|c| {
                let card = &data.cards.cards[c];
                if card.pool_slot as usize == c && card.lobby_ok(lobby) {
                    data.rules.copies(card.tier)
                } else {
                    0
                }
            })
            .collect();
        let consumed = vec![0u32; data.cards.pool_cards];
        let mut players: [Player; PLAYERS] = std::array::from_fn(|_| Player::new(&data));
        let gods: ArrayVec<CardId, 2> = data.cards.cards.iter().enumerate()
            .filter(|(_, c)| c.deity.is_some())
            .map(|(id, _)| id as CardId)
            .collect();
        // The Deity only exists in lobbies with Aberrations (DESIGN.md).
        let god = (lobby.has(Tribe::Aberration) && !gods.is_empty())
            .then(|| gods[rng.below(gods.len() as u32) as usize]);
        if let Some(card) = god {
            for pl in &mut players {
                pl.deity = Some(Deity { card, attack: 1, health: 1, golden: false });
            }
        }
        Game {
            data,
            rng,
            lobby,
            turn: 0,
            players,
            pool,
            consumed,
            ghost: None,
            over: false,
            sandbox: None,
            real_boards: None,
            order: ArrayVec::new(),
            cursor: 0,
        }
    }

    /// Whose action is next, None once the game is over.
    pub fn current_player(&self) -> Option<usize> {
        if self.over { None } else { Some(self.order[self.cursor] as usize) }
    }

    pub fn alive(&self) -> usize {
        self.players.iter().filter(|p| p.alive).count()
    }

    pub fn placements(&self) -> [u8; PLAYERS] {
        std::array::from_fn(|i| self.players[i].placement)
    }

    /// Whether the current player may take `a`.
    pub fn is_legal(&self, a: Action) -> bool {
        let Some(p) = self.current_player() else { return false };
        let pl = &self.players[p];
        let r = &self.data.rules;
        // The hero pick (turn 0): only answering the offer is legal.
        if self.turn == 0 && pl.hero_offer.iter().any(|&h| h != u8::MAX) {
            return matches!(a, Action::Choose(k) if (k as usize) < DISCOVER
                && pl.hero_offer[k as usize] != u8::MAX);
        }
        if let Some(pt) = pl.pending_target {
            if pt.kind.takes_shop() {
                return match a {
                    Action::Target(s) => self.valid_target(p, &pt, s as usize),
                    Action::TargetShop(s) => self.valid_shop_target(p, &pt, s as usize),
                    _ => false,
                };
            }
            return matches!(a, Action::Target(s) if self.valid_target(p, &pt, s as usize));
        }
        if pl.pending_shop_target.is_some() {
            return matches!(a, Action::TargetShop(s) if (s as usize) < pl.shop.len());
        }
        if pl.pending_discard.is_some() {
            return matches!(a, Action::Discard(s) if self.valid_discard(p, s as usize));
        }
        if let Some(pco) = pl.pending_choose_one {
            return matches!(a, Action::Choose(k) if k < 2 && self.can_choose_option(p, pco.options[k as usize]));
        }
        if !pl.discover.is_empty() {
            return matches!(a, Action::Choose(k) if (k as usize) < pl.discover.len());
        }
        // A Hero Power Discover (Unmasked Identity, Finley): like a Discover.
        if !pl.power_offer.is_empty() {
            return matches!(a, Action::Choose(k) if (k as usize) < pl.power_offer.len());
        }
        match a {
            Action::Buy(s) => {
                let s = s as usize;
                if s >= pl.shop.len() || pl.hand.is_full() {
                    return false;
                }
                let card = self.data.cards.get(pl.shop[s].card);
                if card.is_spell() {
                    if card.pay_health {
                        pl.can_pay_health(card.cost as i32, &self.data.cards)
                    } else {
                        let cost = card.cost.saturating_sub(pl.spell_discount);
                        pl.gold >= cost
                    }
                } else {
                    pl.gold >= r.buy_cost
                }
            }
            Action::BuySpell => {
                let Some(id) = pl.shop_spell else { return false };
                let card = self.data.cards.get(id);
                if pl.hand.is_full() {
                    return false;
                }
                if card.pay_health {
                    pl.can_pay_health(card.cost as i32, &self.data.cards)
                } else {
                    pl.gold >= card.cost.saturating_sub(pl.spell_discount)
                }
            }
            Action::Sell(s) => (s as usize) < pl.board.len(),
            Action::Activate(s) => {
                (s as usize) < pl.board.len()
                    && !pl.board[s as usize].activated
                    && self.activate_of(p, s as usize).is_some_and(|(act, _)| {
                        pl.gold >= act.cost
                            && (!act.discard || self.has_discardable(p))
                            && (!act.effect.needs_shop_slot() || self.shop_has_minion(p))
                            && (!act.effect.needs_any_minion()
                                || pl.board.len() > 1
                                || self.shop_has_minion(p))
                            && (!act.effect.needs_shop_minion() || !pl.hand.is_full())
                            && act
                                .effect
                                .needs_different_of_tribe()
                                .is_none_or(|t| self.has_different_of_tribe(p, s as usize, t))
                            && (!act.effect.needs_rally_target()
                                || pl.board.iter().enumerate().any(|(i, m)| {
                                    i != s as usize
                                        && m.parts().any(|(c, _)| self.data.cards.get(c).rally.is_some())
                                }))
                            && (!act.effect.needs_battlecry_target()
                                || pl.board.iter().enumerate().any(|(i, m)| {
                                    i != s as usize
                                        && m.parts().any(|(c, _)| {
                                            let card = self.data.cards.get(c);
                                            card.battlecry.is_some() || card.choose_one.is_some()
                                        })
                                }))
                    })
            }
            Action::Play { hand, pos } => {
                let h = hand as usize;
                let pos = pos as usize;
                if h >= pl.hand.len() {
                    return false;
                }
                let card_id = pl.hand[h].card;
                if pl.hand[h].locked_turns > 0 {
                    false
                } else if self.data.cards.get(card_id).is_spell() {
                    pos == 0 && self.can_cast_spell(p, card_id)
                } else if pos > pl.board.len() {
                    false
                } else if pl.hand[h].keywords.has(Keywords::MAGNETIC)
                    && pos < pl.board.len()
                    && self.can_magnetize(card_id, pl.board[pos].card)
                {
                    true
                } else {
                    !pl.board.is_full()
                }
            }
            Action::Target(_) | Action::TargetShop(_) | Action::Choose(_) | Action::Discard(_) => {
                false
            }
            Action::Move { from, to } => {
                (from as usize) < pl.board.len() && (to as usize) < pl.board.len() && from != to
            }
            Action::Roll => {
                pl.free_rolls > 0
                    || pl.can_refresh_with_health(&self.data.cards, r)
                    || pl.gold >= r.roll_cost
            }
            Action::Freeze | Action::EndTurn => true,
            Action::Level => pl.tier < MAX_TIER && pl.gold >= pl.upgrade_cost,
            Action::HeroPower => self.power_usable(p),
        }
    }

    /// Fills `mask` with `is_legal` for every action. Only the index ranges
    /// that can be legal in the current state are asked (a slot past the end
    /// of the board, hand or shop never is), which gives the same mask as
    /// asking all of them.
    pub fn legal_mask(&self, mask: &mut [bool; N_ACTIONS]) {
        mask.fill(false);
        let Some(p) = self.current_player() else { return };
        let pl = &self.players[p];
        let mut ask = |a: Action| mask[a.index()] = self.is_legal(a);
        if self.turn == 0 && pl.hero_offer.iter().any(|&h| h != u8::MAX) {
            for k in 0..DISCOVER as u8 {
                ask(Action::Choose(k));
            }
        } else if let Some(pt) = pl.pending_target {
            for s in 0..BOARD as u8 {
                ask(Action::Target(s));
            }
            if pt.kind.takes_shop() {
                for s in 0..SHOP as u8 {
                    ask(Action::TargetShop(s));
                }
            }
        } else if pl.pending_shop_target.is_some() {
            for s in 0..pl.shop.len() as u8 {
                ask(Action::TargetShop(s));
            }
        } else if pl.pending_discard.is_some() {
            for s in 0..pl.hand.len() as u8 {
                ask(Action::Discard(s));
            }
        } else if pl.pending_choose_one.is_some() {
            for k in 0..2 {
                ask(Action::Choose(k));
            }
        } else if !pl.discover.is_empty() {
            for k in 0..pl.discover.len() as u8 {
                ask(Action::Choose(k));
            }
        } else if !pl.power_offer.is_empty() {
            for k in 0..pl.power_offer.len() as u8 {
                ask(Action::Choose(k));
            }
        } else {
            for s in 0..pl.shop.len() as u8 {
                ask(Action::Buy(s));
            }
            ask(Action::BuySpell);
            for s in 0..pl.board.len() as u8 {
                ask(Action::Sell(s));
                ask(Action::Activate(s));
            }
            for hand in 0..pl.hand.len() as u8 {
                for pos in 0..=BOARD as u8 {
                    ask(Action::Play { hand, pos });
                }
            }
            for from in 0..pl.board.len() as u8 {
                for to in 0..pl.board.len() as u8 {
                    ask(Action::Move { from, to });
                }
            }
            ask(Action::Roll);
            ask(Action::Freeze);
            ask(Action::Level);
            ask(Action::HeroPower);
            ask(Action::EndTurn);
        }
    }

    /// Whether the Magnetic card `hand_card` can merge into the board minion
    /// `board_card`: a Mech, or an Undead for a card that says so
    /// (Prosthetic Hand).
    fn can_magnetize(&self, hand_card: CardId, board_card: CardId) -> bool {
        let cards = &self.data.cards;
        let t = cards.get(board_card).tribes;
        t.has(Tribe::Mech) || (cards.get(hand_card).magnetize_to_undead && t.has(Tribe::Undead))
    }

    /// Whether the board minion in slot `s` has another friendly minion of
    /// `tribe` that is not a copy of it to point at.
    fn has_different_of_tribe(&self, p: usize, s: usize, tribe: Tribe) -> bool {
        let board = &self.players[p].board;
        let cards = &self.data.cards;
        board.iter().enumerate().any(|(i, m)| {
            i != s && m.card != board[s].card && cards.get(m.card).tribes.has(tribe)
        })
    }

    /// Whether the card in hand slot `s` may be discarded: any playable card
    /// (a Blood Gem included), never an unplayable one (the Lockbox) or a
    /// minion locked in hand.
    fn valid_discard(&self, p: usize, s: usize) -> bool {
        let Some(m) = self.players[p].hand.get(s) else { return false };
        if m.locked_turns > 0 {
            return false;
        }
        !matches!(self.data.cards.get(m.card).spell_effect, Some(SpellEffect::Lockbox(_)))
    }

    /// Whether the player holds any card a discard could take.
    fn has_discardable(&self, p: usize) -> bool {
        (0..self.players[p].hand.len()).any(|s| self.valid_discard(p, s))
    }

    /// The Activate ability of the board minion in slot `s` (the first part
    /// of it that has one) and whether that part is golden.
    fn activate_of(&self, p: usize, s: usize) -> Option<(crate::cards::Activate, bool)> {
        self.players[p].board[s]
            .parts()
            .find_map(|(card, golden)| self.data.cards.get(card).activate.map(|a| (a, golden)))
    }

    /// Whether the Tavern holds a minion (not a spell) for an effect that
    /// needs one.
    fn shop_has_minion(&self, p: usize) -> bool {
        self.players[p]
            .shop
            .iter()
            .any(|m| !self.data.cards.get(m.card).is_spell())
    }

    fn spell_target_ok(&self, st: SpellTarget, m: &Minion) -> bool {
        let c = self.data.cards.get(m.card);
        if c.is_spell() {
            return false;
        }
        match st {
            SpellTarget::Buff { .. }
            | SpellTarget::BuffPerBonusKeyword { .. }
            | SpellTarget::SetStats { .. }
            | SpellTarget::TrickyTrousers { .. }
            | SpellTarget::RobustEvolution => true,
            SpellTarget::BuffSameTribe { .. }
            | SpellTarget::SameTribeMinion
            | SpellTarget::TavernBuffType { .. }
            | SpellTarget::RefreshWithType => !c.tribes.is_neutral(),
            SpellTarget::Consume { tribe, .. } => c.is(tribe),
            SpellTarget::MakeGolden { max_tier } => !m.golden && c.tier <= max_tier,
            SpellTarget::Devourer => true,
            SpellTarget::HalfOfHighestTavern { tribe } => c.tribes.has(tribe),
            SpellTarget::BuffPerTribePlayed { .. } => true,
            SpellTarget::Butchering { .. } => c.tribes.has(Tribe::Undead),
            SpellTarget::Confiscation { .. } => true,
        }
    }

    fn valid_spell_target(&self, p: usize, st: SpellTarget, s: usize) -> bool {
        let b = &self.players[p].board;
        s < b.len() && self.spell_target_ok(st, &b[s])
    }

    /// Whether the pending target may be taken on the Tavern minion in
    /// shop slot `s`.
    fn valid_shop_target(&self, p: usize, pt: &PendingTarget, s: usize) -> bool {
        let pl = &self.players[p];
        // A card steal takes any Tavern card: a shop slot whatever is in it,
        // and one past the last, the spell slot.
        if let TargetKind::StealShopCard { .. } = pt.kind {
            return s < pl.shop.len()
                || (s == pl.shop.len() && pl.shop_spell.is_some() && s < SHOP);
        }
        if s >= pl.shop.len() || self.data.cards.get(pl.shop[s].card).is_spell() {
            return false;
        }
        match pt.kind {
            TargetKind::AnyBuff { .. } | TargetKind::AnySetStats { .. } => true,
            TargetKind::Spell(st) => st.can_target_shop() && self.spell_target_ok(st, &pl.shop[s]),
            _ => false,
        }
    }

    fn can_choose_option(&self, p: usize, opt: ChooseOption) -> bool {
        match opt {
            ChooseOption::SpellTarget(st) => {
                let pl = &self.players[p];
                (0..pl.board.len()).any(|s| self.valid_spell_target(p, st, s))
                    || (st.can_target_shop()
                        && (0..pl.shop.len())
                            .any(|s| self.spell_target_ok(st, &pl.shop[s])))
            }
            ChooseOption::MinionTarget { .. }
            | ChooseOption::SpellBonus { .. }
            | ChooseOption::SpellEffect(_) => true,
        }
    }

    pub fn can_cast_spell(&self, p: usize, card_id: CardId) -> bool {
        let c = self.data.cards.get(card_id);
        if let Some(st) = c.spell_target {
            let pl = &self.players[p];
            (0..pl.board.len()).any(|s| self.valid_spell_target(p, st, s))
                || (st.can_target_shop()
                    && (0..pl.shop.len()).any(|s| self.spell_target_ok(st, &pl.shop[s])))
        } else if let Some(opts) = c.choose_one {
            opts.iter().any(|&opt| self.can_choose_option(p, opt))
        } else {
            // Playing the spell frees its own hand slot, so these only need
            // something to steal; they are never cast from the Tavern.
            match c.spell_effect {
                Some(SpellEffect::StealRandomTavern) => self.shop_has_minion(p),
                // A Lockbox is unplayable.
                Some(SpellEffect::Lockbox(_)) => false,
                Some(_) => true,
                None => false,
            }
        }
    }

    fn valid_target(&self, p: usize, pt: &PendingTarget, s: usize) -> bool {
        let b = &self.players[p].board;
        if s >= b.len() {
            return false;
        }
        if let Some(src) = pt.source
            && s == src as usize
        {
            return false;
        }
        match pt.kind {
            TargetKind::MinionBuff { tribe, .. } | TargetKind::Consume { tribe, .. } => {
                self.data.cards.get(b[s].card).is(tribe)
            }
            TargetKind::AnyBuff { .. } | TargetKind::AnySetStats { .. } => true,
            // A card steal is answered on a Tavern slot, never a board one.
            TargetKind::StealShopCard { .. } => false,
            TargetKind::Spell(st) => self.valid_spell_target(p, st, s),
            TargetKind::MakeGolden { tribe, max_tier } => {
                let c = self.data.cards.get(b[s].card);
                !b[s].golden && c.tier <= max_tier && c.is(tribe)
            }
            TargetKind::DestroyDiscover { tribe, .. } | TargetKind::DestroyCopy { tribe, .. } => {
                self.data.cards.get(b[s].card).tribes.has(tribe)
            }
            TargetKind::RebornDestroy { tribe, .. } => {
                let host = pt.source.map(|src| b[src as usize].card);
                self.data.cards.get(b[s].card).tribes.has(tribe) && Some(b[s].card) != host
            }
            TargetKind::TriggerRally => {
                b[s].parts().any(|(c, _)| self.data.cards.get(c).rally.is_some())
            }
            TargetKind::TriggerBattlecry => b[s]
                .parts()
                .any(|(c, _)| {
                    let card = self.data.cards.get(c);
                    card.battlecry.is_some() || card.choose_one.is_some()
                }),
        }
    }

    /// Take an action for the current player.
    pub fn step(&mut self, a: Action) -> Result<(), IllegalAction> {
        if !self.is_legal(a) {
            return Err(IllegalAction);
        }
        let p = self.current_player().unwrap();
        self.apply(p, a);
        self.resolve_doomed(p);
        self.resolve_hogrider(p);
        // A hero knocked out mid-action may still collect cards from effects
        // that were running when it died.
        for q in 0..PLAYERS {
            self.settle_dead(q);
        }
        if self.over {
            #[cfg(debug_assertions)]
            self.check_invariants().unwrap();
            return Ok(());
        }
        // A hero knocked out by its own effect damage takes no more actions.
        if self.players[p].alive {
            self.feed_waiting_fodders(p);
            self.check_attack_thresholds(p);
            if self.turn == 0 {
                // The pick phase: a player who picked a hero and answered
                // everything it opened is done until turn 1.
                let pl = &self.players[p];
                let picked = pl.hero.is_some()
                    && pl.hero_offer.iter().all(|&h| h == u8::MAX)
                    && !Self::choice_open(pl);
                if picked {
                    self.players[p].done = true;
                }
            } else {
                let pl = &mut self.players[p];
                pl.actions += 1;
                if a == Action::EndTurn || pl.actions >= self.data.rules.max_actions_per_turn {
                    self.finish_shop(p);
                }
            }
        }
        self.advance();
        #[cfg(debug_assertions)]
        self.check_invariants().unwrap();
        Ok(())
    }

    fn apply(&mut self, p: usize, a: Action) {
        match a {
            Action::Buy(s) => {
                let slot = s as usize;
                let id = self.players[p].shop[slot].card;
                let c = self.data.cards.get(id);
                if c.is_spell() {
                    // A Saloon's Finest slot holds a spell: it is bought at
                    // its own cost, like the spell slot.
                    let (pay_health, cost) = (c.pay_health, c.cost);
                    let m = self.players[p].shop.remove(slot);
                    self.add_to_hand(p, m);
                    self.pay_for_spell(p, pay_health, cost);
                    self.magicfin_teach(p, id);
                    self.fire_event(p, EventKind::CardsBought, 1);
                } else {
                    let buy_cost = self.data.rules.buy_cost;
                    let pl = &mut self.players[p];
                    let m = pl.shop.remove(slot);
                    pl.gold -= buy_cost;
                    self.add_to_hand(p, m);
                    self.after_buy_effects(p);
                    self.check_triple(p, m.card);
                    self.fire_event(p, EventKind::GoldSpent, buy_cost as u32);
                    self.fire_event(p, EventKind::CardsBought, 1);
                    // "After you buy a Pirate, gain 1 Gold" (I'm the Cap'n Now).
                    if let Some(st) = self.players[p].power
                        && let PowerEffect::GoldAfterBuyTribe { tribe, gold } =
                            self.data.heroes.get(st.of).power.effect
                        && self.data.cards.get(m.card).tribes.has(tribe)
                    {
                        self.players[p].gold = self.players[p].gold.saturating_add(gold);
                    }
                }
            }
            Action::BuySpell => {
                let id = self.players[p].shop_spell.take().unwrap();
                let c = self.data.cards.get(id);
                let (pay_health, cost) = (c.pay_health, c.cost);
                let pl = &mut self.players[p];
                let m = pl.make_card(&self.data.cards, id, false, 0);
                self.add_to_hand(p, m);
                self.pay_for_spell(p, pay_health, cost);
                self.magicfin_teach(p, id);
                self.fire_event(p, EventKind::CardsBought, 1);
            }
            Action::Sell(s) => {
                self.sell_minion(p, s as usize);
            }
            Action::Activate(s) => {
                let (act, golden) = self.activate_of(p, s as usize).unwrap();
                self.players[p].gold -= act.cost;
                self.players[p].board[s as usize].activated = true;
                if act.discard {
                    // The reward runs once the player has picked the card.
                    self.players[p].pending_discard = Some(PendingDiscard {
                        source: s,
                        reward: act.effect,
                        golden,
                    });
                } else {
                    let mut pos = s as usize;
                    self.run_effect(p, &mut pos, act.effect, golden, 1);
                    self.maybe_start_discover(p);
                }
                self.fire_event(p, EventKind::GoldSpent, act.cost as u32);
            }
            Action::Play { hand, pos } => {
                let m = self.players[p].hand.remove(hand as usize);
                if m.linked != 0 {
                    let group = m.linked & 0x3fff;
                    if m.linked & 0xc000 == 0x8000 {
                        for other in &mut self.players[p].hand {
                            if other.linked & 0x3fff == group {
                                other.linked |= 0x4000;
                            }
                        }
                    } else {
                        while let Some(slot) = self.players[p].hand.iter()
                            .position(|other| other.linked & 0x3fff == group)
                        {
                            self.discard_card(p, slot);
                        }
                    }
                }
                let card = self.data.cards.get(m.card);
                if card.is_spell() {
                    let choose_one = card.choose_one.is_some();
                    self.cast_spell(p, m);
                    if choose_one {
                        // Turbo Hogrider reacts once the card is resolved.
                        self.players[p].hogrider_pending += 1;
                    }
                    return;
                }
                let pos = pos as usize;
                let is_merge = m.keywords.has(Keywords::MAGNETIC)
                    && pos < self.players[p].board.len()
                    && self.can_magnetize(m.card, self.players[p].board[pos].card);
                let mut choose_one_play = false;
                if is_merge {
                    self.magnetize_to(p, pos, m, true);
                    if m.golden && !m.no_reward {
                        self.give_triple_reward(p);
                    }
                } else {
                    let mut m = m;
                    if self.data.cards.get(m.card).tribes.has(Tribe::Beast) {
                        let (ba, bh) = self.players[p].beast_bonus;
                        m.attack += ba;
                        m.health += bh;
                    }
                    // A minion landing on the board starts with its limited
                    // uses for the turn (a Trailblazer's combines, a
                    // Mycologist's teachings, Malchezaar's Health Refreshes).
                    m.uses_left = m.turn_uses(&self.data.cards);
                    self.players[p].board.insert(pos, m);
                    let t = self.data.cards.get(m.card).tribes;
                    // The Triple Reward comes before anything the Battlecry
                    // adds to the hand (and is lost on a full hand).
                    if m.golden && !m.no_reward {
                        self.give_triple_reward(p);
                    }
                    choose_one_play = self.data.cards.get(m.card).choose_one.is_some();
                    if t.has(Tribe::Elemental) {
                        self.players[p].elementals_played_turn =
                            self.players[p].elementals_played_turn.saturating_add(1);
                        self.give_play_shield(p, pos);
                    }
                    self.fire_after_play_tribe(p, t, pos);
                    if t.has(Tribe::Elemental) {
                        self.fire_event_skipping(p, EventKind::ElementalsPlayed, 1, Some(pos));
                    }
                    if let Some((da, dh)) = self.data.cards.get(m.card).volumizer_buff {
                        let mul = if m.golden { 2 } else { 1 };
                        self.add_volumizer_bonus(p, da * mul, dh * mul);
                    }
                    if t.has(Tribe::Beast) {
                        self.trigger_summon_beast_in_shop(p, pos);
                    }
                    if t.has(Tribe::Mech) {
                        self.trigger_mech_played_or_magnetized(p, pos, true, true);
                    }
                    self.battlecry_or_choose_one(p, pos);
                    self.maybe_start_discover(p);
                    if t.has(Tribe::Murloc) {
                        self.trigger_in_hand_on_play(p, Tribe::Murloc);
                    }
                }
                if m.golden {
                    // A Golden minion played from hand counts for Maritime
                    // Extortionist and Hooktusk.
                    self.add_golden_played(p, 1);
                }
                if choose_one_play {
                    self.players[p].hogrider_pending += 1;
                }
            }
            Action::Target(s) => self.resolve_target(p, s as usize),
            Action::TargetShop(s) => self.resolve_shop_target(p, s as usize),
            Action::Choose(k) => {
                if self.turn == 0 && self.players[p].hero_offer.iter().any(|&h| h != u8::MAX) {
                    self.pick_hero(p, k as usize);
                } else {
                    self.resolve_choose(p, k as usize);
                }
            }
            Action::Discard(s) => self.resolve_discard(p, s as usize),
            Action::Move { from, to } => {
                let pl = &mut self.players[p];
                let m = pl.board.remove(from as usize);
                pl.board.insert(to as usize, m);
            }
            Action::Roll => {
                let Game { data, rng, players, pool, lobby, .. } = self;
                let (cards, rules) = (&data.cards, &data.rules);
                let pl = &mut players[p];
                let mut gold_spent = 0;
                let mut health_paid = 0;
                if pl.free_rolls > 0 {
                    pl.free_rolls -= 1;
                } else if pl.can_refresh_with_health(cards, rules) {
                    pl.spend_health_refresh(cards);
                    health_paid = rules.health_refresh_cost;
                } else {
                    pl.gold -= rules.roll_cost;
                    gold_spent = rules.roll_cost;
                }
                Self::refresh_shop(pl, pool, cards, rules, rng, *lobby, RefreshKind::Explicit);
                if health_paid > 0 {
                    // A Health cost is not a Demon dealing damage.
                    self.take_hero_damage(p, health_paid, None);
                }
                self.fire_event(p, EventKind::GoldSpent, gold_spent as u32);
            }
            Action::Freeze => self.players[p].frozen = !self.players[p].frozen,
            Action::Level => {
                let turn = self.turn as u8;
                let pl = &mut self.players[p];
                let cost = pl.upgrade_cost;
                pl.gold -= cost;
                pl.tier += 1;
                pl.upgrade_cost = self.data.rules.upgrade_cost(pl.tier);
                pl.tier_turns[pl.tier as usize - 1] = turn;
                // "After you upgrade the Tavern, gain 2 Gold" (Everbloom), and
                // an unlock condition may just be met (Queen of Dragons).
                let gain = self.players[p].power
                    .and_then(|st| match self.data.heroes.get(st.of).power.effect {
                        PowerEffect::GoldAfterUpgrade(g) => Some(g),
                        _ => None,
                    })
                    .unwrap_or(0);
                self.players[p].gold = self.players[p].gold.saturating_add(gain);
                self.refresh_power_unlock(p);
                self.fire_event(p, EventKind::GoldSpent, cost as u32);
            }
            Action::HeroPower => self.press_power(p),
            Action::EndTurn => {}
        }
    }

    fn give_generated(&mut self, p: usize, card: CardId) {
        let mut dummy = 0;
        self.give_generated_tracking(p, card, &mut dummy);
    }

    fn add_to_hand(&mut self, p: usize, m: Minion) -> bool {
        if self.players[p].hand.is_full() {
            return false;
        }
        self.players[p].hand.push(m);
        for i in 0..self.players[p].board.len() {
            let board = self.players[p].board[i];
            for (card, golden) in board.parts() {
                if let Some((attack, health)) = self.data.cards.get(card).on_card_added {
                    let mul = if golden { 2 } else { 1 };
                    if let Some(deity) = &mut self.players[p].deity {
                        deity.attack += attack * mul;
                        deity.health += health * mul;
                    }
                }
            }
        }
        true
    }

    fn give_linked(&mut self, p: usize, cards: &[CardId], golden: bool) {
        if cards.is_empty() {
            return;
        }
        let id = self.players[p].next_link & 0x3fff;
        self.players[p].next_link = if id == 0x3fff { 1 } else { id + 1 };
        let linked = id | if golden { 0x8000 } else { 0 };
        for &card in cards {
            let mut m = self.players[p].make_card(&self.data.cards, card, false, 0);
            m.linked = linked;
            if self.add_to_hand(p, m) && !self.data.cards.get(card).is_spell() {
                self.check_triple(p, card);
            }
        }
    }

    fn give_generated_tracking(&mut self, p: usize, card: CardId, tracked_pos: &mut usize) {
        let m = self.players[p].make_card(&self.data.cards, card, false, 0);
        if self.add_to_hand(p, m) && !self.data.cards.get(card).is_spell() {
            self.check_triple_tracking(p, card, tracked_pos);
        }
    }

    fn advance(&mut self) {
        if self.over {
            return;
        }
        let n = self.order.len();
        for k in 1..=n {
            let c = (self.cursor + k) % n;
            if !self.players[self.order[c] as usize].done {
                self.cursor = c;
                return;
            }
        }
        // Turn 0 is the hero pick: once everyone has picked, turn 1 starts
        // (no fights before it).
        if self.turn == 0 {
            self.start_turn();
            return;
        }
        self.combat();
        if !self.over {
            self.start_turn();
        }
    }

    /// Take a card straight into a player's hand. Pool minions come out of
    /// `self.pool`; spells and tokens are generated with `pool_copies = 0`.
    /// For tests and setting up positions, not part of the game.
    pub fn give(&mut self, p: usize, name: &str) -> Option<Minion> {
        let c = self.data.cards.by_name(name)?;
        if self.players[p].hand.is_full() {
            return None;
        }
        let card = self.data.cards.get(c);
        if card.is_spell() || card.token {
            let m = self.players[p].make_card(&self.data.cards, c, false, 0);
            self.add_to_hand(p, m);
            return Some(m);
        }
        let slot = card.pool_slot as usize;
        if self.pool.get(slot).copied().unwrap_or(0) == 0 {
            return None;
        }
        self.pool[slot] -= 1;
        let m = self.players[p].make_card(&self.data.cards, c, false, 1);
        self.add_to_hand(p, m);
        Some(m)
    }
}
