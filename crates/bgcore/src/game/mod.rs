//! One 8-player game: shop phase, pairing, fights, knockouts.
//!
//! The shop phase is simultaneous in the real game. Here the players act one
//! action at a time, in a random order that changes every turn, until each
//! has ended their turn. `current_player` says whose action is next.

pub mod darkgifts;
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
mod trinkets;
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
use crate::darkgifts::DarkGiftPlan;
use crate::minion::{CardId, GiftOn, Keywords, Minion};
use crate::real_boards::RealBoards;
use crate::trinkets::{CardMod, Event};
use self::trinkets::MinionPrice;
use crate::rng::Rng;
use crate::{BOARD, CHOICES, DISCOVER, Data, HAND, MAX_TIER, PLAYERS, SHOP, TRIBES};

/// Upper bound on pool minions, for the fixed-size copy count in
/// `check_invariants`. The card parser refuses a card list past it.
pub const MAX_POOL_CARDS: usize = 256;
/// Card rewrites one player can hold at once: the six portraits and the
/// Trailblazer Sticker fit with room to spare.
pub const CARD_MODS: usize = 8;
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
        /// The pick may be a Tavern minion too (docs/rules/design.md "Targets in the
        /// shop": a text that says "another Murloc" or "a Beast", not "a
        /// friendly minion").
        shop: bool,
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
    /// Become an exact copy of the chosen minion, on the board or in the
    /// Tavern (Faceless Manipulator).
    BecomeCopy,
    /// Throw the chosen minion, on the board or in the Tavern, in the pot: it
    /// is gone for good, and `count` throws make the Discover (Stir the Pot).
    ThrowInPot { count: u8 },
    /// Remove the chosen friendly minion and Discover one from the Tier below
    /// it (Trash for Treasure).
    RemoveTierBelow,
    /// Swap the chosen friendly non-Golden minion with a random Tavern
    /// minion (Swap, Lock, & Shop It).
    SwapWithShop,
    // ---- hero batch 2 (2026-11-01) ----
    /// Give the chosen minion, the Tavern's included, Reborn until the next
    /// turn (Reborn Rites).
    TempReborn,
    /// Replace the chosen Tavern minion with a Discover from exactly one Tier
    /// higher (Galakrond's Greed); the old card goes back to the pool.
    ReplaceHigher,
    /// Replace the chosen card, on the board or in the Tavern, with a random
    /// one of the same Tier (Arcane Alteration).
    ReplaceSameTier,
    /// Steal the chosen Tavern card into hand as these stats (See the Light).
    StealShopAsStats { attack: i32, health: i32 },
    /// Take the chosen Tavern card into hand, locked for this many turns
    /// (Imprison).
    ImprisonShop { turns: u8 },
    /// Spirit Swap, the first of the two minions (any minion).
    SpiritSwapFirst,
    /// Spirit Swap, the second one: it swaps Attack with the one the first
    /// pick remembered.
    SpiritSwapSecond,
    /// Devour, the friendly minion to sell.
    DevourFirst,
    /// Devour, the friendly minion that takes its stats.
    DevourSecond,
    /// Destroy the chosen friendly minion of a tribe (a real death in the
    /// shop) and get a random one of that tribe (Rune of Damnation).
    DestroyGetTribe { tribe: Tribe },
    /// Summon an exact copy of the chosen friendly minion (Cloning Gallery).
    SummonExactCopy,
    // ---- hero batch 3 (2026-11-01) ----
    /// Lock the chosen Tavern minion in hand for 1 turn and summon a copy of
    /// it in the next combat (Lock and Load).
    LockAndLoad,
    /// Mark the chosen friendly minion to be destroyed at Start of Combat and
    /// resummoned as an exact copy when there is space (Rapid Reanimation).
    RapidReanimation,
}

impl TargetKind {
    /// Whether the player may pick a minion in the Tavern for this target.
    pub fn takes_shop(self) -> bool {
        match self {
            TargetKind::AnyBuff { .. }
            | TargetKind::AnySetStats { .. }
            | TargetKind::BecomeCopy => true,
            // "another Murloc", "a Pirate", "a Beast": the Tavern is included
            // when the card's words do not say "friendly".
            TargetKind::MinionBuff { shop, .. } => shop,
            TargetKind::Spell(st) => st.can_target_shop(),
            TargetKind::StealShopCard { .. } | TargetKind::ThrowInPot { .. } => true,
            // Hero batch 2: a Tavern minion, or any Tavern card.
            TargetKind::TempReborn
            | TargetKind::ReplaceHigher
            | TargetKind::ReplaceSameTier
            | TargetKind::StealShopAsStats { .. }
            | TargetKind::ImprisonShop { .. }
            | TargetKind::SpiritSwapFirst
            | TargetKind::SpiritSwapSecond
            | TargetKind::LockAndLoad => true,
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
    /// Gem played on the target (a Geomagus Roogug there reacts to it).
    pub gem: bool,
    /// The spell is one a minion's own effect casts for the player (Gatekeeper
    /// Amalgam's Misplaced Tea Set, a Magicfin token's taught spell). It still
    /// happens and buffs as printed, but it is not a spell the player cast
    /// (Era 2026-11-02), so no "spells you cast" trigger and no count sees it.
    pub by_minion: bool,
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

impl ShopTargetKind {
    /// Whether the pick may name the Tavern's spell slot, one past the last
    /// minion. A Fishbait takes a Tavern spell too: "choose a card in the
    /// Tavern" is any card there (Era, 2026-11-03).
    pub fn takes_spell_slot(self) -> bool {
        matches!(self, ShopTargetKind::Fishbait { .. })
    }
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
    /// With `tribe: None`, only minions that have a type: "minions of its
    /// type" on an all-type minion (Eonar's Favor on an Amalgam) means every
    /// Tavern minion with a type, not a random tribe and not the tribeless
    /// ones.
    pub typed_only: bool,
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
    /// How many times the card is cast by this one choice: the player picks the
    /// option and its target once, and every cast after the first uses the
    /// same option on the same target (docs/rules/design.md "Set versus add",
    /// Era 2026-11-04). The runs still owed are the `Player.repeats` queue.
    pub repeats: u8,
}

/// A repeat a Battlecry trigger or a spell cast still owes after its first run
/// asked for a target or a Choose One option: it lands on the target that was
/// picked there, and on the Choose One option the player picked there
/// (docs/rules/design.md "Set versus add", Era 2026-11-04). Only a spell a
/// minion casts gets no pick of its own, so its repeats take a new random
/// option each. No heap: the queue is a fixed array on `Player`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct QueuedRepeat {
    pub what: RepeatWhat,
    /// Runs still owed. The first run of an entry is not one of them: it is
    /// the one that asked.
    pub runs: u8,
    /// Whether the first run has happened. Until it has, the player answers
    /// what that run asks; after it, every prompt is answered from `at`.
    pub started: bool,
    /// The board or Tavern slot the first run picked, when it picked one:
    /// every repeat lands on it. A repeat whose own Battlecry asks for a
    /// target it was never given picks a random one.
    pub at: Option<u8>,
    /// The Choose One option (0 or 1) the player picked for the first run, if
    /// it asked: every repeat takes the same one. `None` when the first run
    /// had no pick of its own (a spell a minion casts picks at random), and
    /// then every run picks its own option at random.
    pub option: Option<u8>,
    /// Whether a Discover a run opens is answered with a random card of its
    /// pool instead of being offered, and does not count as a Discover for
    /// "after you Discover" cards: a Battlecry an Activate triggers does that
    /// (Kelp Keeper; docs/rules/design.md "Shop replay answers", Era
    /// 2026-11-03).
    pub random_discover: bool,
}

/// What a `QueuedRepeat` repeats.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RepeatWhat {
    /// The Battlecry of the minion in this board slot, triggered again (a
    /// golden Kelp Keeper, Drakkari on Young Murk-Eye's neighbours).
    Battlecry { slot: u8 },
    /// A Choose One cast again, on the option (and target) the player picked,
    /// or both running again when the card covers both (Trailblazer
    /// Sticker). `by_minion` marks a spell a minion cast, whose casts get no
    /// choice: each picks a random option of its own.
    ChooseOne {
        source: Option<u8>,
        options: [ChooseOption; 2],
        both: bool,
        tavern: bool,
        by_minion: bool,
    },
}

impl QueuedRepeat {
    /// The board slot whose effect this repeat is: the minion whose Battlecry
    /// it triggers, or the minion that cast the spell. A target prompt from
    /// that source is the one the first run picked, so its answer is what
    /// every repeat lands on.
    pub fn source(&self) -> Option<u8> {
        match self.what {
            RepeatWhat::Battlecry { slot } => Some(slot),
            RepeatWhat::ChooseOne { source, .. } => source,
        }
    }
}

/// One option of a "both effects combined" Choose One that still has to run,
/// after the option before it is done waiting on its own target.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct QueuedOption {
    pub source: Option<u8>,
    pub option: ChooseOption,
    /// Whether the Choose One spell was a Tavern spell.
    pub tavern: bool,
    /// Whether a minion's effect cast the spell (see `PendingChooseOne`).
    pub by_minion: bool,
}

/// A Choose One spell's own "after you cast a spell" triggers, waiting for
/// the option and its target to be picked (docs/rules/design.md "Targets in the shop":
/// cast, pick the option, pick the target, then the cast triggers). The
/// counters of the cast are taken at the play; these are what "after" means.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CastAfterChoice {
    /// The spell cast, for the trinkets and Dark Gifts that read it.
    pub card: CardId,
    /// How many "spells cast" events it gives: a Tavern spell gives `casts`
    /// of each (Balinda's extra cast), any other spell one.
    pub casts: u8,
    pub tavern: bool,
    /// The "whenever you cast a spell" triggers and the trinket events
    /// already fired at the play, so only the "after you cast a spell" ones
    /// are owed (a targeting spell). A Choose One spell owes both.
    pub at_play: bool,
}

/// "Discard a card" (Mangled Bandit's Activate) waiting for the player to
/// pick the card in hand to throw away.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PendingDiscard {
    /// Board slot of the minion whose effect it is, None for a hero power
    /// (Incubate), which has no minion to run the reward for.
    pub source: Option<u8>,
    /// What the discard earns, run after the card is gone.
    pub reward: Effect,
    /// Whether the minion with the effect is golden (doubles the reward).
    pub golden: bool,
    /// Times the reward runs (Dark Gifts Efficiency doubles an Activate).
    pub runs: u8,
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
    /// A minion of exactly this tier that has a type, so no neutral
    /// (Portable Factory).
    TypedMinionTier(u8),
    /// A minion of exactly this tier, locked in hand until the Tavern reaches
    /// that tier (Procrastinate, Expedition Plans).
    TierLockedMinion(u8),
    /// A Dark Gift offer: one minion per slot with a gift each (the Dark
    /// Discovery button, Ominous Stone, Wax Lance, Sinister Invitation,
    /// Xavius, Void Power).
    DarkGift(DarkGiftPlan),
    /// A Dark Gift offer of plain copies of the player's own gifted minions
    /// (Maldraxxus Dagger).
    DarkGiftCopies,
    /// A Magnetic Mech up to the player's tier, from the pool (Tinker).
    MagneticMinion,
    /// One minion per thrown type, in order (Stir the Pot): the thrown
    /// minions' tribes, up to `count` of them, 0 for an empty slot.
    PotTypes([u8; 3]),
    /// A minion from the warband of the `i`th hero that died in the round
    /// (`Game.kitty_warbands`), exactly as it stood there (Kel'Thuzad's
    /// Kitty).
    DeadWarband(u8),
    /// A minion that died in the last fight, either side, from the pool
    /// (Reclaimed Souls).
    DeadLastFight,
    /// A Tier 7 minion, locked in hand until this much Gold has been spent
    /// this game (Choose Your Champion).
    Champion(u16),
    /// A plain copy of a minion from the next opponent's warband as it stood
    /// at the start of the round (I Spy).
    IspyNextOpponent,
    // ---- block 5 ----
    /// Three different Darkmoon Prizes of one Prize tier (the Triple Prize,
    /// Tickatus Sticker, Big Winner!, Tickatus' Prize Wall). Prizes are never
    /// in the pool, so no pool copy moves for any of the three.
    Prize(u8),
    /// A plain copy of a minion from the last opponent's warband as it was
    /// last seen (Time Thief).
    LastOpponentWarband,
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

/// A player's hero power: which power it is (`HeroDb::powers`, not a hero) and
/// everything the game tracks about using it. Replacing the power (Unmasked
/// Identity, Finley's Discover, a power that turns into another) builds a
/// fresh one: the old one's progress is lost.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PowerState {
    /// Index into `HeroDb::powers` (the player's hero's starting power, or one
    /// Discovered or turned into since).
    pub of: u8,
    /// Button presses left this turn.
    pub uses_left: u8,
    /// Button presses this game.
    pub used_game: u32,
    /// Whether an unlock condition ("Unlocks at Tier 4") has been met.
    pub unlocked: bool,
    /// One generic progress counter: the cards bought towards a milestone
    /// (BUY, INSECT! counts to 12), or the turns a chosen power has counted
    /// down (a flightpath's "In 2 turns").
    pub progress: u32,
    /// The power this one completed into its chooser, plus one: the chooser
    /// never offers that one straight after (Galewing's flightpaths). 0 when
    /// there is none.
    pub last_pick: u8,
    /// A change to this power's press price that lasts: +1 for every Lead
    /// Explorer press, -1 for each Pirate bought (Pirate Parrrrty!) or for
    /// each turn waited (The Perfect Crime, The Galaxy's Lens). The price
    /// never drops below 0.
    pub cost_delta: i8,
    /// Whether this power was pressed this turn and its effect fires at the
    /// end of it (Saturday C'Thuns!). Cleared at the start of every turn.
    pub armed: bool,
}

impl PowerState {
    fn fresh(of: u8, power: &crate::heroes::HeroPower) -> PowerState {
        PowerState {
            of,
            uses_left: power.uses_per_turn,
            used_game: 0,
            unlocked: true,
            progress: 0,
            last_pick: 0,
            cost_delta: 0,
            armed: false,
        }
    }
}

/// The standing effects of a player's trinkets (see `Act` in trinkets.rs for
/// which card sets each field). Reset nowhere: a trinket is never removed,
/// except by a re-offer, which replaces it wholesale.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TrinketPowers {
    /// Minions that cost Health instead of Gold: a tribe (None for any) and
    /// how many this turn.
    pub health_price_tribe: Option<Tribe>,
    pub health_price_left: u8,
    /// A Tavern spell that costs Health this turn (Bazaar Sticker).
    pub spell_health_left: u8,
    /// The highest-Tier minion in the Tavern costs Health (Demonic Tapestry).
    pub highest_tier_health: bool,
    /// Every Nth card bought costs Health; the count so far.
    pub fourth_buy: u8,
    pub fourth_buy_every: u8,
    /// Free minion buys this turn (Warcry Totem) and a free tribe (Grifter).
    pub free_buys: u8,
    pub free_tribe: Option<Tribe>,
    pub free_tribe_left: u8,
    /// Battlecries the player has triggered this game (Murky Sticker's
    /// improve step).
    pub battlecries: u16,
    /// On reaching Tier 6, gain this much Gold once (Stuffed Coin Purse).
    pub gold_on_tier6: u8,
    /// Gold per distinct friendly minion type at the start of the turn
    /// (Glowing Crystal), per friendly Golden minion (Rascal Sticker).
    pub gold_per_type: u8,
    pub gold_per_golden: u8,
    /// "When you have 15 Gold": get a random Golden Tier 5 minion, once
    /// (Splinter of Aurum). The tier is the draw; 0 when not held.
    pub golden_when_rich: u8,
    /// The next trinket offer is a lesser one (Mysterious Orb).
    pub next_lesser: bool,
    /// The turn the greater slot is offered instead of turn 9 (Ornate Clock).
    pub greater_offer_turn: u32,
    /// The turn a greater offer goes into the lesser slot (Trip Vouchers).
    pub greater_into_lesser_turn: u32,
    /// Minion buys still to be copied into hand, bought ones only
    /// (Transcribing Typewriter).
    pub copy_buys: u8,
    /// Discovers left this turn that also give a copy of the pick (Sinstone
    /// Sticker), and how many each turn starts with.
    pub discover_copies: u8,
    pub discover_copies_base: u8,
    /// The slot (plus one) of a trinket waiting for the card a Discover
    /// picked (Portable Factory); 0 when none.
    pub remember_in: u8,
    /// The next Refresh holds plain copies of the warband (Warband Whistle).
    pub whistle: bool,
    /// Two copies of a Pirate make it Golden (Designer Eyepatch).
    pub pirate_triple: bool,
    /// Turns sooner every Lockbox opens (Escapee Portrait).
    pub lockbox_sooner: u8,
    /// A Mystery Cube slot's turn-start offer had to wait for another offer
    /// to be answered; the slot (plus one), 0 when nothing waits.
    pub cube_pending: u8,
    /// Times the hero power's effect runs per press and per passive tick
    /// (Ancient Wishbone: twice). 0 means once.
    pub power_triggers: u8,
    /// Extra Hero Power uses each turn (Sous Chef Sticker).
    pub power_extra_uses: u8,
    /// Gold each Hero Power press pays back (Sous Chef Sticker).
    pub power_gold: u8,
    /// The Tavern always holds six minions and one spell, and refills a
    /// bought, consumed or stolen card at once (Glowing Gauntlet).
    pub gauntlet: bool,
    /// One extra Tavern minion of the tier above yours (Innkeeper's Stein).
    pub extra_high_tier: bool,
    /// One extra Magnetic Mech in the Tavern (Electrode Attractor).
    pub extra_magnetic: bool,
    /// What a Magnetic Mech costs, 0 for its own price (Electrode Attractor).
    pub magnetic_cost: u8,
    /// Tier 6 Refreshes left this turn that draw only Tier 6 minions, and
    /// how many each turn starts with (Guiding Candle).
    pub refresh_tier6: u8,
    pub refresh_tier6_base: u8,
    /// The lowest Tavern tier on offer, 0 for tier 1 (Bob-blehead).
    pub min_tier: u8,
    /// Makeshift Master: the stats the watched minion (`Minion.watched`) had
    /// when the watching started, so whatever it gains from then on goes to
    /// the Deity too. It follows the card: the Tavern minion chosen, the hand
    /// after a buy, the board after it is played. `deity_watching` says whether
    /// anything is watched at all, so the pass after every action costs one
    /// read when no Makeshift Master is out.
    pub deity_watching: bool,
    pub deity_watch_stats: (i32, i32),
    /// The price of the minions a Refresh put in the Tavern (Chillmere
    /// Mosaic: they cost 1), 0 for their own price. It ends with the next
    /// fill of the Tavern.
    pub tavern_price: u8,
    /// Token of the Old Gods: the board slot (plus one) a Discover's pick
    /// transforms, 0 when none.
    pub transform_slot: u8,
    /// The stats a Discover's pick is set to instead of the card's own
    /// (Innkeeper's Hearth), and how many picks are left to set.
    pub discover_stats: Option<(i32, i32, u8)>,
    /// Minions still to be played this turn that gain Divine Shield
    /// (Defensive Shell).
    pub play_shields: u8,
    /// The player's end-of-turn effects run one more time (Ghastly Sticker).
    pub end_of_turn_ghastly: bool,
    /// Tavern spells this game whose cast is doubled by a trinket, and how
    /// many each turn starts with (Replica Cathedral).
    pub first_spell_double: u8,
    pub first_spell_double_base: u8,
    // ---- block 4 ----
    /// Extra times the first Battlecry each turn triggers (War Drum).
    pub war_drum: u8,
    /// Extra times your Dragons' Battlecries trigger (Dragon's Eye).
    pub dragon_battlecry: u8,
    /// After a spell on a friendly minion, cast it on another (Lovely
    /// Locket).
    pub locket: bool,
    /// Set while the Locket's own copy is being cast, so it never chains.
    pub locket_copy: bool,
    /// A rewrite a portrait gives every copy of a card the player has or
    /// will have (docs/rules/trinkets.md "Portraits that rewrite a card").
    pub card_mods: [Option<(CardId, CardMod)>; CARD_MODS],
    /// Your Choose One cards always have both effects (Trailblazer
    /// Sticker).
    pub choose_one_all: bool,
    /// Your minions have the base, plus the per-card step for every card
    /// discarded this game (Hammer of Twilight).
    pub hammer_base: (i32, i32),
    pub hammer_per: (i32, i32),
    /// Your Tavern spells give this much more for each different friendly
    /// minion type (Multilayered Shield).
    pub spell_bonus_per_type: (i32, i32),
    /// Your Tavern spells give this much more for this turn only (Honeycomb
    /// Ring).
    pub spell_bonus_turn: (i32, i32),
    /// A friendly Demon's consume also takes Bonus Keywords and this much
    /// (Consuming Claw).
    pub consume_claw: (i32, i32),
    /// The slot (plus one) of a Souvenir Stand that becomes a copy of the
    /// next Greater trinket picked; 0 when none.
    pub souvenir: u8,
    /// The Tavern slot (plus one) a spell was just cast on, for a trinket
    /// that consumes it (Felsteel Cleaver).
    pub shop_hit: u8,
    /// The minion just bought: its stats (Reusable Batteries' Satellite) and
    /// its card (the Gold-plated Compass's Golden buy); for a Tavern spell,
    /// its card (Magicfin Sticker).
    pub bought_stats: (i32, i32),
    pub bought_card: Option<CardId>,
    pub bought_spell: Option<CardId>,
    // ---- block 5 ----
    /// Every Triple Reward becomes this card (Corrupted Tome's Triple
    /// Prize), and one went to hand at the pick.
    pub triple_prize: Option<CardId>,
    /// The Surprise Portrait's chance out of 100 of a surprise on each buy,
    /// sell and Refresh, 0 for none, and whether the last one was a surprise
    /// (never twice in a row).
    pub surprise: u8,
    pub surprise_last: bool,
}

/// The standing rules a Darkmoon Prize leaves behind. A Prize is a spell, so
/// what outlasts the cast needs somewhere to live: one small block, like
/// `TrinketPowers`, so a shop path reads a field and never the card. The
/// per-turn fields are cleared in `start_turn`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PrizePowers {
    /// The Tavern holds one more minion for good (New Recruit). The +2/+2 it
    /// comes with is not its own: it goes into `Player.refresh_buff`, the same
    /// "after the Tavern is Refreshed this game" amount Waveling and Easterly
    /// Winds give (Era: they share one buff, so they stack).
    pub extra_shop: bool,
    /// Free Refreshes gained at the start of every turn (Rocking and
    /// Rolling, Open Bar).
    pub free_rolls_per_turn: u8,
    /// Tavern spells cost this much less this turn (Reserve Prices).
    pub spell_discount_turn: u8,
    /// Minions in the Tavern cost this much for the rest of the game
    /// (Friends and Family Discount), 0 for their own price.
    pub minion_price: u8,
    /// Your Battlecries trigger this many extra times this turn (Big Brann
    /// Play).
    pub battlecry_extra_turn: u8,
    /// The card that comes back to hand at the end of every turn (The
    /// Unlimited Coin).
    pub returns_each_turn: Option<CardId>,
}

/// One held trinket: which one, its "every N" counter, the uses its hooks
/// have left this turn, and whether a `once` rule has fired.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TrinketState {
    /// Index into `TrinketDb` plus one; 0 for an empty slot.
    pub of: u8,
    pub count: u16,
    /// A number the trinket improves as it plays ("improve this": Shark
    /// Cannon, Panpipes).
    pub value: u8,
    pub uses_left: u8,
    pub done: bool,
    /// The card this trinket remembers from a Discover, so it can hand out
    /// another copy each turn (Portable Factory).
    pub card: Option<CardId>,
    /// The minion type this trinket locked in at the pick, which it keeps for
    /// the rest of the game whatever the board becomes: the Colorful
    /// Compasses hand out minions of it, the Gold-plated Compass makes the
    /// next minion of it bought Golden (and spends it). None when the trinket
    /// locks no type, or the board had none at the pick (docs/rules/trinkets.md,
    /// "Block 6 rules").
    pub tribe: Option<Tribe>,
    /// The Mystery Cube's effect belongs to the slot and not to the trinket
    /// in it: while this is set, the start of every turn offers new free
    /// Lesser trinkets to replace whatever the slot holds (docs/rules/design.md).
    pub cube: bool,
    /// How many options a cube slot offers ("choose from 2"), kept on the
    /// slot with the flag.
    pub cube_choices: u8,
}

/// The counters hero powers keep: one block, so a power that counts
/// something adds a field here instead of a power's own. All of them last the
/// whole game; the per-turn counts live in the power's `progress`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct HeroCounts {
    /// Cards bought, Tavern spells and free buys included (Sharpen Blades).
    pub cards_bought: u32,
    /// Minions bought (For the Horde!, Verdant Spheres, Glaive Ricochet).
    pub minions_bought: u32,
    /// Tiers' worth of cards bought: a card's Tier, a spell by its Tier
    /// (Natural Balance).
    pub tiers_bought: u32,
    /// Minions sold, by an effect too (Gone Fishing).
    pub minions_sold: u32,
    /// Friendly minions that attacked in a fight, once per swing (Heroic
    /// Inspiration).
    pub attacks: u32,
    /// Gold spent this game; a Health price is not Gold (Choose Your
    /// Champion).
    pub gold_spent: u32,
    /// Tavern spells bought (Reliquary Research).
    pub tavern_spells_bought: u32,
    /// Battlecry minions bought (Battle Brand).
    pub battlecry_bought: u32,
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
    /// The MAIN hero power and its state. None without a hero; replaced whole
    /// by a Discovered power (Unmasked Identity changes only this one).
    pub power: Option<PowerState>,
    /// The SECOND hero power, from Genn's King of Duality or the
    /// Reinvigorating Light trinket. Nothing changes or disables it, and it
    /// has its own uses, cost and state (docs/rules/heroes.md "Two hero powers").
    pub power2: Option<PowerState>,
    /// Heroes offered before turn 1 (indices into `HeroDb`), `u8::MAX` for an
    /// empty offer slot. While one is open the only legal action answers it.
    pub hero_offer: [u8; DISCOVER],
    /// Powers offered by a Hero Power Discover or a chooser (indices into
    /// `HeroDb::powers`), answered like a Discover.
    pub power_offer: ArrayVec<u8, CHOICES>,
    /// Whether the open power offer answers into the SECOND slot (Genn's
    /// first pick, the Reinvigorating Light trinket) instead of the main one.
    pub power_offer_second: bool,
    /// Power Discovers this press still owes after the open one: King of
    /// Duality deals two, so the power is not spent until both are answered.
    pub power_offer_left: u8,
    /// Whether this turn is skipped (Procrastinate, Expedition Plans): only
    /// Freeze and End Turn are legal. Set at the start of each of the
    /// player's turns from their powers, so the legality check is one read.
    pub skipped: bool,
    /// The heroes a power offer never names, by hero index (a card's
    /// `exclude_heroes`, or a hero's own list), kept while the offer and any
    /// pick after it are dealt.
    pub power_excludes: ArrayVec<u8, 8>,
    /// Trinket slots: lesser, greater, and the trinket a hero power became
    /// (Marin the Manager, Buttons). See `TrinketState`.
    pub trinkets: [TrinketState; crate::TRINKET_SLOTS],
    /// The offer the player has to answer before doing anything else: the
    /// four options with the price each is sold at, and which slot they fill
    /// (`trinket_offer_slot` is the slot plus one, 0 for none).
    pub trinket_offer: ArrayVec<(u8, u8), { crate::TRINKET_CHOICES }>,
    pub trinket_offer_slot: u8,
    /// The events this player's trinkets listen for (`Trinket::event_mask`
    /// ORed over the slots, `crate::trinkets::HAS_TRINKET` when any is held).
    /// A hook reads this one `u32` and leaves the shared trinket table alone
    /// unless the bit matches.
    pub trinket_events: u32,
    /// What this player's trinkets change for them: prices, the Tavern's
    /// shape, and the small standing rules. One block, so a trinket effect is
    /// a field write and the card paths read it without a table lookup.
    pub tp: TrinketPowers,
    /// What the Darkmoon Prizes this player cast left behind (New Recruit,
    /// Rocking and Rolling, Friends and Family Discount).
    pub prize: PrizePowers,
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
    /// The counters the hero powers keep (see `HeroCounts`).
    pub counts: HeroCounts,
    /// The standing rules a hero power sets (hero batch 2), kept in sync by
    /// `Game::sync_power_flags` whenever a power arrives and at the start of
    /// every turn, so a shop path reads one field and never the power table.
    /// Stay Frosty: one fewer Tavern minion, the Tavern frozen at every end of
    /// turn; Dream Portal: one more Tavern minion, a Dragon; Double Time: two
    /// copies make a minion Golden.
    pub shop_fewer: u8,
    pub freeze_end: bool,
    pub extra_dragon: bool,
    /// Double Time: the Tavern Coin a Golden minion gives instead of a Triple
    /// Reward, None without the power.
    pub double_time: Option<CardId>,
    /// Hat Trick: what each hat gives, (0, 0) for no hats.
    pub hat: (i32, i32),
    /// Enhancification: random Bonus Keywords a Refresh gives, 0 for none.
    pub enhance_times: u8,
    /// Twice as Nice: copy the Tavern's highest-Tier minion on every Refresh.
    pub twice_nice: bool,
    /// Come One, Come All!: the chance a Tavern card carries a Darkmoon
    /// Ticket, and the Tickets taken so far.
    pub ticket_chance: u8,
    pub tickets: u8,
    /// The spell slot's Darkmoon Ticket: 1 when it carries one, 2 when it was
    /// rolled for and carries none, 0 for a slot never marked.
    pub shop_spell_ticket: u8,
    /// I Spy: the warband of the opponent this player is paired against, as it
    /// stood at the start of the round (`Game::pair` copies it).
    pub next_board: ArrayVec<Minion, BOARD>,
    /// A two-step pick's first answer: the slot and whether it was in the
    /// Tavern (Spirit Swap's two minions; Devour's minion before the sale is
    /// gone by the second pick, so only Spirit Swap keeps it).
    pub two_step: Option<(u8, bool)>,
    /// Mishmash's snapshot of every board slot (card, Attack, Health) as of
    /// the end of the last action, so the stats a friendly Amalgam gained
    /// can be copied onto it. All zeros when no Mishmash is held.
    pub amalgam_seen: [(CardId, i32, i32); BOARD],
    /// The Secret zone (docs/rules/heroes.md "Secrets"): the combat Secrets this
    /// player holds in the order they were got, and the Ice Block a Safety
    /// Patch gives, unspent (the next fatal damage to the hero is prevented
    /// and the hero is Immune for the rest of the turn, docs/rules/trinkets.md).
    pub secrets: crate::secrets::Secrets,
    /// The Secrets an open Prestidigitation offers, as kinds (`secrets`
    /// module indexes), answered with `Action::Choose` like a Hero Power
    /// Discover. Empty when no offer is open.
    pub secret_offer: ArrayVec<u8, CHOICES>,
    /// The hero is Immune this turn: no damage at all. Cleared at the start
    /// of the next turn.
    pub immune: bool,
    /// The stats Devour sells, for the second pick to give.
    pub devour_stats: (i32, i32),
    /// A Discover whose pick lands in this Tavern slot (plus one) instead of
    /// the hand (Galakrond's Greed); 0 for a pick that goes to hand.
    pub discover_into_shop: u8,
    /// The tribes of the minions thrown into Stir the Pot, and the minion
    /// cards bought this turn that Glaive Ricochet may copy. Both are hero
    /// power odds and ends that fit no counter.
    pub pot: ArrayVec<u8, 3>,
    pub recent_buys: ArrayVec<CardId, 3>,
    /// Minions that died in the last fight, either side, for Reclaimed Souls.
    /// Tokens are in it like any other minion.
    pub dead_last_fight: ArrayVec<CardId, 16>,
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
    /// Tavern spells cast this turn (Inductive Gyroblade). Resets every turn.
    pub spells_cast_turn: u16,
    /// Elementals played from hand this game (Flourishing Frostling's aura:
    /// "wherever this is").
    pub elementals_played: u32,
    /// Times this player has Magnetized a card this game (Dr. Boom's
    /// Monster's aura).
    pub magnetizations: u32,
    /// What "after the Tavern is Refreshed this game" gives: the sum of every
    /// Waveling, En-Djinn Blazer and Easterly Winds so far. Each explicit
    /// Refresh gives the whole amount to one random Tavern minion.
    pub refresh_buff: (i32, i32),
    pub spell_bonus: (i32, i32),
    pub spell_discount: u8,
    /// The last Tavern spell this player cast themselves: what Cataclysmic
    /// Harbinger, the Galaxy's Lens and Lens Case copy. A spell a minion cast
    /// for them, a Blood Gem or any other non-Tavern spell does not replace
    /// it.
    pub last_spell: Option<CardId>,
    pub shop_spell: Option<CardId>,
    pub shop: ArrayVec<Minion, SHOP>,
    pub hand: ArrayVec<Minion, HAND>,
    /// Cards that arrived at the start of the turn with a full hand: the game
    /// queues them and they join the hand as soon as there is room, while a
    /// card got any other time is lost (docs/rules/design.md "A full hand").
    pub hand_queue: ArrayVec<Minion, HAND>,
    /// Whether a card got now is queued rather than lost: on through the
    /// start of the turn's own effects, off after them.
    pub hand_queue_on: bool,
    /// Cards a countdown payout (Replication) handed over at the end of the
    /// last turn: its copy arrives when the turn is over and nothing triples
    /// during the fights, so its triple check waits for the start of this
    /// turn (`dark_gift_start_of_turn` runs it; Era 2026-11-03).
    pub triples_next_turn: ArrayVec<CardId, BOARD>,
    pub board: ArrayVec<Minion, BOARD>,
    pub pending_choose_one: Option<PendingChooseOne>,
    pub pending_target: Option<PendingTarget>,
    pub pending_shop_target: Option<PendingShopTarget>,
    /// "Discard a card" waiting for the player's pick.
    pub pending_discard: Option<PendingDiscard>,
    /// A Choose One spell cast whose "after you cast a spell" triggers wait
    /// for the option and its target (see `CastAfterChoice`).
    pub cast_after_choice: Option<CastAfterChoice>,
    /// Battlecry triggers a Battlecry that opened a choice still owes the
    /// "for each Battlecry you've triggered" count: they are added once
    /// nothing is left to answer (docs/rules/design.md "Counting Battlecries": a
    /// Discover Battlecry counts at the pick, not on the play).
    pub pending_battlecries: u8,
    /// Choose One cards played whose choice is still open: Turbo Hogrider
    /// reacts once the card is fully resolved (`resolve_hogrider`).
    pub hogrider_pending: u8,
    /// The tribes of a just-played minion whose "after you play an X"
    /// triggers are still owed: its Battlecry, and whatever that asks (a
    /// Choose One, a target, a Discover), resolves first, then they fire
    /// (`resolve_after_play`). One at a time: a play never happens while a
    /// choice is open, so an earlier play's triggers are always done.
    pub pending_after_play: Option<crate::cards::Tribes>,
    /// Options of a "both effects combined" Choose One that still have to
    /// run, after the one before them is done waiting on its target.
    pub choose_queue: ArrayVec<QueuedOption, 2>,
    /// Repeats a Battlecry trigger or a spell cast still owes after the first
    /// run asked for a target or a Choose One option (see `QueuedRepeat`).
    pub repeats: ArrayVec<QueuedRepeat, 8>,
    /// Whether `run_queued_repeats` is on the stack: a prompt it answers must
    /// not re-enter it.
    pub repeating: bool,
    /// Discover options waiting for a choice. Minion cards here are out of the pool.
    pub discover: ArrayVec<CardId, DISCOVER>,
    /// Queued Discover requests to offer once pending choices finish.
    pub discover_queue: ArrayVec<DiscoverRequest, 8>,
    /// How many of the queued Discover requests a Battlecry an Activate (or a
    /// hero power) triggered opened. Those are answered with a random card of
    /// the Discover's pool instead of being offered (docs/rules/design.md
    /// "Shop replay answers": a Kelp Keeper's Activate).
    pub repeat_discover: u8,
    /// Whether the Discover offer now being answered is one of those: its
    /// pick is not a Discover for `after_discover` cards (Hooktusk; Era
    /// 2026-11-04, docs/rules/design.md "Set versus add").
    pub discover_silent: bool,
    /// The board slot of the Demon whose Discover makes the hero take damage
    /// equal to the chosen card's Tier (Imposing Percussionist), if any.
    pub discover_damage_from: Option<u8>,
    /// Whether the Discover now waiting locks its minion in hand for a turn
    /// (Search Through Time).
    pub discover_locked: bool,
    /// A Tavern tier a Discovered minion stays locked in hand until (Faelin's
    /// Expedition Plans), 0 for none. A locked minion can be buffed in hand
    /// but not played, discarded or pulled into a fight from the hand.
    pub discover_locked_tier: u8,
    /// Whether the Discover now waiting marks its minion to die when played
    /// this turn (Tomb Turning).
    pub discover_doomed: bool,
    /// The Dark Gift each offer of the open Discover carries (index
    /// `discover`), the pool copies each offer holds, and whether the open
    /// Discover is a Dark Gift offer at all.
    pub discover_gifts: ArrayVec<GiftOn, DISCOVER>,
    pub discover_copies: ArrayVec<u8, DISCOVER>,
    /// The minions an open Discover offers as they stand, one per `discover`
    /// slot, when its options are ready-made rather than pool cards
    /// (Kel'Thuzad's Kitty). Empty for every other Discover.
    pub discover_minions: ArrayVec<Minion, DISCOVER>,
    pub dark_offer: bool,
    /// The turn the open offer's gifts were picked as.
    pub dark_offer_turn: u32,
    /// Every offer of the open Discover is Golden (Sinister Invitation).
    pub dark_offer_golden: bool,
    /// Turns the open offer's pick stays locked in hand (Void Power).
    pub dark_offer_locked: u8,
    /// The Dark Discovery button: uses left this game, and whether it was
    /// pressed this turn (docs/rules/darkgifts.md: once a turn, 3 uses a game).
    pub dark_gift_uses: u8,
    pub dark_gift_pressed: bool,
    /// Battlecries this player has triggered this game, which Dark Gifts
    /// Battle Scars reads.
    pub battlecries_triggered: u32,
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
    /// What a Blood Gem played from the hand gives on top of that (Surveyor
    /// Portrait: "Blood Gems played from your hand give an extra +6/+6"). A
    /// Gem a minion plays does not get it.
    pub gem_hand_attack: i32,
    pub gem_hand_health: i32,
    /// Blood Gem Barrage casts: every Refresh gives this much, as Blood
    /// Gems, to every minion in the Tavern (the turn-start roll included
    /// when the Tavern changed).
    pub barrage: (i32, i32),
    /// Whether the Discover now waiting marks its Choose One pick as having
    /// both effects (Fandral's Fortune).
    pub discover_both: bool,
    /// Gold the open Discover's pick stays locked in hand for (Choose Your
    /// Champion): 0 for none.
    pub discover_locked_gold: u16,
    /// Spectral Sight: free minion buys left this turn (1 at turn start when
    /// the power is held).
    pub free_minion_left: u8,
    /// Power of the Storm (Master Nguyen): whether each hero-power slot (0 =
    /// main, 1 = second) came from Power of the Storm and swaps at the start
    /// of every turn.
    pub storm_power: [bool; 2],
    /// Conviction (Cariel Roame): `(count, attack, health)` accumulated over
    /// the game by choosing Offensive / Defensive / United Stance. Starts at
    /// `(2, 1, 1)` once the hero is picked; `count` caps at 7.
    pub conviction: (u8, i32, i32),
    /// Lock and Load (Saurfang): the exact copy of the Tavern minion locked in
    /// hand this turn to summon at the next Start of Combat when space opens.
    pub locked_and_loaded: Option<Minion>,
    /// Friendly Wager (Lord Barov): the two living players `(a, b)` offered by
    /// the open Choose One, and the active wagers `(guessed_player, opponent)`
    /// waiting for `finish_fights`.
    pub wager_offer: Option<(u8, u8)>,
    pub wagers: ArrayVec<(u8, u8), 2>,
    /// Detective for Hire (Murloc Holmes): the two candidate minion `CardId`s
    /// and the correct answer index (`0` or `1`) while the Choose One is open.
    pub holmes_offer: Option<([CardId; 2], u8)>,
    pub deity: Option<Deity>,
    pub discarded: u32,
    /// The last card this player discarded, for a reward that copies it
    /// (Writhing Tentacles). One minion, so it costs nothing per discard.
    pub last_discarded: Option<Minion>,
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
    /// The five tribes of this game's lobby (docs/rules/design.md "The lobby's
    /// tribes"): the pool and every draw are filtered to them.
    pub lobby: Tribes,
    pub turn: u32,
    pub players: [Player; PLAYERS],
    /// Copies left of each pool card, indexed by card `pool_slot`.
    pub pool: Vec<u8>,
    /// The Dark Paradox version in this game's pool: one per game, the same
    /// for every player, None for a card set without the card.
    pub paradox: Option<CardId>,
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
    /// The warband of every hero that died in a round, as it stood at its last
    /// moment, with the round it died in: Kel'Thuzad's Kitty offers one
    /// Discover per warband, back to back, at the start of the holder's next
    /// turn, so the warbands stay for the round after the deaths. Dropped once
    /// that round has started for everyone.
    pub kitty_warbands: ArrayVec<(u32, ArrayVec<Minion, BOARD>), { PLAYERS - 1 }>,
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
    /// starts (docs/rules/design.md "The pick"). The lobby is rolled from the seed.
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
    /// (docs/rules/design.md); a test may pass more to set up its cards.
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
        let mut pool: Vec<u8> = (0..data.cards.pool_cards)
            .map(|c| {
                let card = &data.cards.cards[c];
                if card.pool_slot as usize == c && card.lobby_ok(lobby) {
                    data.rules.copies(card.tier)
                } else {
                    0
                }
            })
            .collect();
        // Dark Paradox: one of its five versions is in this game's pool, the
        // same for every player, and the other four are not (docs/rules/darkgifts.md
        // "Dark Paradox"). The pick has an RNG of its own, seeded from the
        // game seed, so every draw the rest of the game makes stays what it
        // was before the card existed.
        let paradox = Self::paradox_versions(&data, lobby);
        let paradox = if paradox.is_empty() {
            None
        } else {
            let mut pick = Rng::new(seed ^ 0x5041_5241_444f_5800);
            Some(paradox[pick.below(paradox.len() as u32) as usize])
        };
        for c in 0..data.cards.pool_cards {
            if !data.cards.cards[c].paradox {
                continue;
            }
            pool[c] = if Some(c as CardId) == paradox {
                data.rules.copies(data.cards.cards[c].tier)
            } else {
                0
            };
        }
        let consumed = vec![0u32; data.cards.pool_cards];
        let mut players: [Player; PLAYERS] = std::array::from_fn(|_| Player::new(&data));
        let gods: ArrayVec<CardId, 2> = data.cards.cards.iter().enumerate()
            .filter(|(_, c)| c.deity.is_some())
            .map(|(id, _)| id as CardId)
            .collect();
        // The Deity only exists in lobbies with Aberrations (docs/rules/design.md).
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
            paradox,
            ghost: None,
            over: false,
            sandbox: None,
            real_boards: None,
            kitty_warbands: ArrayVec::new(),
            order: ArrayVec::new(),
            cursor: 0,
        }
    }

    /// The Dark Paradox versions that could be in this lobby's pool (they all
    /// have every minion type, so an all-type card is in every lobby). Empty
    /// for a card set without them.
    fn paradox_versions(data: &Data, lobby: Tribes) -> ArrayVec<CardId, 8> {
        (0..data.cards.pool_cards as CardId)
            .filter(|&c| {
                let card = data.cards.get(c);
                card.paradox && card.pool_slot == c && card.lobby_ok(lobby)
            })
            .collect()
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
        // A skipped turn (Procrastinate, Expedition Plans) can always freeze
        // or end, even with a choice open: End Turn answers it (Faelin's
        // Discovers open on turn 1, which is the turn he skips).
        if pl.skipped && matches!(a, Action::Freeze | Action::EndTurn) {
            return true;
        }
        // The hero pick (turn 0): only answering the offer is legal.
        if self.turn == 0 && pl.hero_offer.iter().any(|&h| h != u8::MAX) {
            return matches!(a, Action::Choose(k) if (k as usize) < DISCOVER
                && pl.hero_offer[k as usize] != u8::MAX);
        }
        // A trinket offer blocks the shopping actions until it is answered,
        // like a Discover. Passing is not a shopping action: Freeze and End
        // Turn stay legal, and ending the turn with the offer open buys the
        // cheapest option (the offer can't be declined). A choice its own
        // start-of-turn effect opened is answered first.
        if !pl.trinket_offer.is_empty() && !Self::other_choice_open(pl) {
            return match a {
                Action::ChooseTrinket(k) => (k as usize) < pl.trinket_offer.len(),
                Action::Freeze | Action::EndTurn => true,
                _ => false,
            };
        }
        // A choice that nothing can answer is no choice: `step` fizzles it,
        // so the normal actions are legal as if it had never opened (see
        // `choice_stuck`). The test only runs while a choice is open.
        if let Some(pt) = pl.pending_target
            && !self.choice_stuck(p)
        {
            if pt.kind.takes_shop() {
                return match a {
                    Action::Target(s) => self.valid_target(p, &pt, s as usize),
                    Action::TargetShop(s) => self.valid_shop_target(p, &pt, s as usize),
                    _ => false,
                };
            }
            return matches!(a, Action::Target(s) if self.valid_target(p, &pt, s as usize));
        }
        if let Some(pst) = pl.pending_shop_target
            && !self.choice_stuck(p)
        {
            return matches!(a, Action::TargetShop(s) if self.valid_tavern_pick(p, &pst, s as usize));
        }
        if pl.pending_discard.is_some() && !self.choice_stuck(p) {
            return matches!(a, Action::Discard(s) if self.valid_discard(p, s as usize));
        }
        if let Some(pco) = pl.pending_choose_one
            && !self.choice_stuck(p)
        {
            return matches!(a, Action::Choose(k) if k < 2 && self.can_choose_option(p, pco.options[k as usize]));
        }
        if !pl.discover.is_empty() {
            return matches!(a, Action::Choose(k) if (k as usize) < pl.discover.len());
        }
        // A Hero Power Discover (Unmasked Identity, Finley) or a chooser's
        // list (Bru'kan's Elements, Galewing's flightpaths): like a Discover.
        if !pl.power_offer.is_empty() {
            return matches!(a, Action::Choose(k) if (k as usize) < pl.power_offer.len());
        }
        // Prestidigitation's offer of Secrets: answered the same way.
        if !pl.secret_offer.is_empty() {
            return matches!(a, Action::Choose(k) if (k as usize) < pl.secret_offer.len());
        }
        if pl.wager_offer.is_some() || pl.holmes_offer.is_some() {
            return matches!(a, Action::Choose(k) if k < 2);
        }
        // A skipped turn (Procrastinate, Expedition Plans): Freeze and End
        // Turn (already answered above) and the answers to whatever choice the
        // turn opened, nothing else; the fight still happens.
        if pl.skipped {
            return false;
        }
        match a {
            Action::Buy(s) => {
                let s = s as usize;
                if s >= pl.shop.len() || pl.hand.is_full() {
                    return false;
                }
                let card_id = pl.shop[s].card;
                let card = self.data.cards.get(card_id);
                if card.is_spell() {
                    if card.pay_health || pl.tp.spell_health_left > 0 {
                        pl.can_pay_health(card.cost as i32, &self.data.cards)
                    } else {
                        let cost = self.spell_price(p, card_id).saturating_sub(pl.spell_discount);
                        pl.gold >= cost
                    }
                } else {
                    match self.minion_price(p, s) {
                        MinionPrice::Gold(g) => pl.gold >= g,
                        MinionPrice::Health(h) => {
                            pl.can_pay_health(h as i32, &self.data.cards)
                        }
                    }
                }
            }
            Action::BuySpell => {
                let Some(id) = pl.shop_spell else { return false };
                let card = self.data.cards.get(id);
                if pl.hand.is_full() {
                    return false;
                }
                if card.pay_health || pl.tp.spell_health_left > 0 {
                    pl.can_pay_health(card.cost as i32, &self.data.cards)
                } else {
                    pl.gold >= self.spell_price(p, id).saturating_sub(pl.spell_discount)
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
                                || self.has_battlecry_target(p, Some(s as usize)))
                    })
            }
            Action::Play { hand, pos } => {
                let h = hand as usize;
                let pos = pos as usize;
                if h >= pl.hand.len() {
                    return false;
                }
                let card_id = pl.hand[h].card;
                if pl.hand[h].locked_in_hand(pl.tier, pl.counts.gold_spent) {
                    false
                } else if self.data.cards.get(card_id).is_spell() {
                    pos == 0 && self.can_cast_spell(p, card_id)
                } else if pos > pl.board.len() {
                    false
                } else if pl.hand[h].keywords.has(Keywords::MAGNETIC)
                    && pos < pl.board.len()
                    && self.can_magnetize(card_id, &pl.board[pos])
                {
                    true
                } else {
                    !pl.board.is_full()
                }
            }
            Action::Target(_)
            | Action::TargetShop(_)
            | Action::Choose(_)
            | Action::Discard(_)
            | Action::ChooseTrinket(_) => false,
            Action::Move { from, to } => {
                (from as usize) < pl.board.len() && (to as usize) < pl.board.len() && from != to
            }
            Action::Roll => {
                pl.free_rolls > 0
                    || pl.can_refresh_with_health(&self.data.cards, r)
                    || pl.gold >= self.refresh_cost(p)
            }
            Action::Freeze | Action::EndTurn => true,
            Action::Level => pl.tier < MAX_TIER && pl.gold >= self.upgrade_price(p),
            Action::HeroPower => self.power_usable(p),
            Action::HeroPower2 => self.power_usable2(p),
            Action::DarkGift => self.dark_gift_usable(p),
        }
    }

    /// Whether a pending choice is open with nothing legal to answer it.
    /// A Discover and a Hero Power offer always have an option, and a target
    /// prompt is only opened with a legal target (see `offer_target`), but a
    /// prompt can lose its last answer afterwards, and a Choose One can have
    /// no option that can be chosen. No card reaches any of these today; a
    /// card that did would strand the player, so `step` fizzles the choice
    /// instead (`fizzle_stuck_choice`).
    fn choice_stuck(&self, p: usize) -> bool {
        let pl = &self.players[p];
        if let Some(pt) = pl.pending_target {
            let board = (0..pl.board.len()).any(|s| self.valid_target(p, &pt, s));
            let shop = pt.kind.takes_shop()
                && (0..pl.shop.len()).any(|s| self.valid_shop_target(p, &pt, s));
            return !board && !shop;
        }
        if let Some(pst) = pl.pending_shop_target {
            // The pick points at a Tavern slot that no longer exists.
            return pl.shop.is_empty()
                && !(pst.kind.takes_spell_slot() && pl.shop_spell.is_some());
        }
        if pl.pending_discard.is_some() {
            return !(0..pl.hand.len()).any(|s| self.valid_discard(p, s));
        }
        if let Some(pco) = pl.pending_choose_one {
            return !(0..2).any(|k| self.can_choose_option(p, pco.options[k]));
        }
        false
    }

    /// Drop a pending choice that no answer can satisfy (see `choice_stuck`):
    /// the effect is lost and the player's normal actions become legal again.
    /// Prints a warning because no current card can put a player here.
    fn fizzle_stuck_choice(&mut self, p: usize) {
        let pl = &mut self.players[p];
        let which = if pl.pending_target.is_some() {
            "target"
        } else if pl.pending_shop_target.is_some() {
            "Tavern pick"
        } else if pl.pending_discard.is_some() {
            "discard"
        } else {
            "Choose One"
        };
        eprintln!(
            "warning: no legal answer to a pending {which} at turn {} for player {p}: \
             the choice fizzles",
            self.turn
        );
        pl.pending_target = None;
        pl.pending_shop_target = None;
        pl.pending_discard = None;
        pl.pending_choose_one = None;
        // The repeats a fizzled choice owed have no target left to land on.
        pl.repeats.clear();
        // A Choose One spell whose choice files away has no "after" left to
        // fire: the cast counted itself already.
        pl.cast_after_choice = None;
        // A Battlecry's trigger did happen, only its choice had no answer, so
        // the count it owes lands now.
        self.pay_pending_battlecries(p);
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
        // A choice that nothing can answer is fizzled by `step`, so the
        // normal actions are the legal ones then, as in `is_legal`.
        let pending = pl.pending_target.is_some()
            || pl.pending_shop_target.is_some()
            || pl.pending_discard.is_some()
            || pl.pending_choose_one.is_some();
        let open = !pending || !self.choice_stuck(p);
        if open && self.turn == 0 && pl.hero_offer.iter().any(|&h| h != u8::MAX) {
            for k in 0..DISCOVER as u8 {
                ask(Action::Choose(k));
            }
        } else if open && !pl.trinket_offer.is_empty() && !Self::other_choice_open(pl) {
            // A trinket offer: the answers, and passing (see `is_legal`).
            for k in 0..pl.trinket_offer.len() as u8 {
                ask(Action::ChooseTrinket(k));
            }
            ask(Action::Freeze);
            ask(Action::EndTurn);
        } else if open && let Some(pt) = pl.pending_target {
            for s in 0..BOARD as u8 {
                ask(Action::Target(s));
            }
            if pt.kind.takes_shop() {
                for s in 0..SHOP as u8 {
                    ask(Action::TargetShop(s));
                }
            }
        } else if open && let Some(pst) = pl.pending_shop_target {
            let mut last = pl.shop.len();
            if pst.kind.takes_spell_slot() && pl.shop_spell.is_some() && last < SHOP {
                last += 1;
            }
            for s in 0..last as u8 {
                ask(Action::TargetShop(s));
            }
        } else if open && pl.pending_discard.is_some() {
            for s in 0..pl.hand.len() as u8 {
                ask(Action::Discard(s));
            }
        } else if open && pl.pending_choose_one.is_some() {
            for k in 0..2 {
                ask(Action::Choose(k));
            }
        } else if open && !pl.discover.is_empty() {
            for k in 0..pl.discover.len() as u8 {
                ask(Action::Choose(k));
            }
        } else if open && !pl.power_offer.is_empty() {
            for k in 0..pl.power_offer.len() as u8 {
                ask(Action::Choose(k));
            }
        } else if open && !pl.secret_offer.is_empty() {
            for k in 0..pl.secret_offer.len() as u8 {
                ask(Action::Choose(k));
            }
        } else if open && (pl.wager_offer.is_some() || pl.holmes_offer.is_some()) {
            for k in 0..2 {
                ask(Action::Choose(k));
            }
        } else if open && self.turn_skipped(p) {
            // A skipped turn: Freeze and End Turn only.
            ask(Action::Freeze);
            ask(Action::EndTurn);
        } else {
            for s in 0..pl.shop.len() as u8 {
                ask(Action::Buy(s));
            }
            ask(Action::BuySpell);
            for s in 0..pl.board.len() as u8 {
                ask(Action::Sell(s));
                ask(Action::Activate(s));
            }
            // A minion goes in a slot from 0 to the board's length, and a
            // spell only at 0: no position past the board can be legal.
            for hand in 0..pl.hand.len() as u8 {
                for pos in 0..=pl.board.len() as u8 {
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
            ask(Action::HeroPower2);
            ask(Action::DarkGift);
            ask(Action::EndTurn);
        }
        // A skipped turn can freeze or end whatever else is open (the End
        // Turn answers it), which the branches above do not all ask for.
        if pl.skipped {
            ask(Action::Freeze);
            ask(Action::EndTurn);
        }
    }

    /// Whether the Magnetic card `hand_card` can merge into the board minion
    /// `board_minion`: a Mech, or an Undead for a card that says so
    /// (Prosthetic Hand).
    fn can_magnetize(&self, hand_card: CardId, board_minion: &Minion) -> bool {
        let cards = &self.data.cards;
        // The tribes a card Magnetizes onto are data on the card: Mech for
        // all but Prosthetic Hand (Mech and Undead) and the Mecha-Demons
        // (Mech and Demon). The minion it lands on is a minion, so its tribes
        // are its own: an Amalgamation is a Mech too.
        let t = cards.tribes_of(board_minion);
        cards.get(hand_card).magnetize_tribes.overlaps(t)
    }

    /// Whether the board minion in slot `s` has another friendly minion of
    /// `tribe` that is not a copy of it to point at.
    fn has_different_of_tribe(&self, p: usize, s: usize, tribe: Tribe) -> bool {
        let board = &self.players[p].board;
        let cards = &self.data.cards;
        board.iter().enumerate().any(|(i, m)| {
            i != s && m.card != board[s].card && cards.tribes_of(m).has(tribe)
        })
    }

    /// Whether the card in hand slot `s` may be discarded: any playable card
    /// (a Blood Gem included), never an unplayable one (the Lockbox) or a
    /// minion locked in hand.
    fn valid_discard(&self, p: usize, s: usize) -> bool {
        let pl = &self.players[p];
        let Some(m) = pl.hand.get(s) else { return false };
        // A locked card is a real card in hand, but it can't be thrown away
        // (docs/rules/design.md "Cards locked in hand").
        if m.locked_in_hand(pl.tier, pl.counts.gold_spent) {
            return false;
        }
        !matches!(self.data.cards.get(m.card).spell_effect, Some(SpellEffect::Lockbox(_)))
    }

    /// Whether the player holds any card a discard could take.
    fn has_discardable(&self, p: usize) -> bool {
        (0..self.players[p].hand.len()).any(|s| self.valid_discard(p, s))
    }

    /// Whether a friendly minion's Battlecry can be triggered, skipping the
    /// minion in slot `except` (an Activate's own host). A Choose One counts:
    /// it runs through the same `battlecry_or_choose_one` path.
    fn has_battlecry_target(&self, p: usize, except: Option<usize>) -> bool {
        self.players[p].board.iter().enumerate().any(|(i, m)| {
            Some(i) != except
                && m.parts().any(|(c, _)| {
                    let card = self.data.cards.get(c);
                    card.battlecry.is_some() || card.choose_one.is_some()
                })
        })
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
    /// Whether the player has a minion Stir the Pot could throw: a typed
    /// minion of the board or the Tavern (a neutral one cannot be thrown).
    fn has_throwable(&self, p: usize) -> bool {
        let typed = |m: &Minion| !self.data.cards.tribes_of(m).is_neutral();
        let pl = &self.players[p];
        pl.board.iter().any(typed)
            || pl.shop.iter().any(|m| !self.data.cards.get(m.card).is_spell() && typed(m))
    }

    fn shop_has_minion(&self, p: usize) -> bool {
        self.players[p]
            .shop
            .iter()
            .any(|m| !self.data.cards.get(m.card).is_spell())
    }

    fn spell_target_ok(&self, p: usize, st: SpellTarget, m: &Minion) -> bool {
        let c = self.data.cards.get(m.card);
        if c.is_spell() {
            return false;
        }
        // A target is a minion, so a tribe test on it is its tribes now, an
        // Amalgamation and a portrait's extra tribes included.
        let tribes = self.data.cards.tribes_of(m);
        match st {
            SpellTarget::Buff { tribe, .. } => tribes.is(tribe),
            SpellTarget::BuffPerBonusKeyword { .. }
            | SpellTarget::SetStats { .. }
            | SpellTarget::TrickyTrousers { .. }
            | SpellTarget::RobustEvolution => true,
            SpellTarget::BuffSameTribe { .. }
            | SpellTarget::SameTribeMinion
            | SpellTarget::TavernBuffType { .. }
            | SpellTarget::RefreshWithType => !tribes.is_neutral(),
            SpellTarget::Consume { tribe, .. } => tribes.is(tribe),
            SpellTarget::MakeGolden { max_tier } => !m.golden && c.tier <= max_tier,
            SpellTarget::Devourer => true,
            SpellTarget::HalfOfHighestTavern { tribe } => tribes.has(tribe),
            SpellTarget::BuffPerTribePlayed { .. } => true,
            SpellTarget::Butchering { .. } => tribes.has(Tribe::Undead),
            SpellTarget::Confiscation { .. } => true,
            SpellTarget::PlainCopy { max_tier } => c.tier <= max_tier,
            SpellTarget::DestroyGetRandom { tribe, .. } => tribes.has(tribe),
            // Token of the Old Gods: the first pick takes any minion, and
            // the Tier-higher pick is the second one, so a top-Tier minion
            // is chosen and nothing follows (the log,
            // Hearthstone_2026_10_28_22_11_50-g3 turn 8: a Tier 6 pick and
            // the board unchanged).
            SpellTarget::TransformHigher => true,
            SpellTarget::WatchStats => true,
            SpellTarget::GemKeyword { .. } | SpellTarget::TriggerDeathrattle => true,
            // A "then double it" buff takes any minion.
            SpellTarget::BuffThenDouble { .. } => true,
            // Returning a minion to hand needs a non-Golden one and room in
            // hand (Repeat Customer, Raise the Stakes).
            SpellTarget::ReturnToHand { .. } => !m.golden && !self.players[p].hand.is_full(),
            SpellTarget::MakeGoldenReturn => !self.players[p].hand.is_full(),
            SpellTarget::GrantEndOfTurn { .. } => true,
        }
    }

    fn valid_spell_target(&self, p: usize, st: SpellTarget, s: usize) -> bool {
        let b = &self.players[p].board;
        s < b.len() && self.spell_target_ok(p, st, &b[s])
    }

    /// Whether the Tavern pick `s` can be answered: a minion slot, or the
    /// spell slot when the effect takes any Tavern card.
    fn valid_tavern_pick(&self, p: usize, pst: &PendingShopTarget, s: usize) -> bool {
        let pl = &self.players[p];
        s < pl.shop.len()
            || (pst.kind.takes_spell_slot()
                && s == pl.shop.len()
                && pl.shop_spell.is_some()
                && s < SHOP)
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
        // Arcane Alteration and Imprison take any Tavern card, a spell in a
        // minion slot and the spell slot included.
        if matches!(pt.kind, TargetKind::ReplaceSameTier | TargetKind::ImprisonShop { .. }) {
            return s < pl.shop.len()
                || (s == pl.shop.len() && pl.shop_spell.is_some() && s < SHOP);
        }
        if s >= pl.shop.len() || self.data.cards.get(pl.shop[s].card).is_spell() {
            return false;
        }
        match pt.kind {
            TargetKind::AnyBuff { .. } | TargetKind::AnySetStats { .. } | TargetKind::BecomeCopy => {
                true
            }
            TargetKind::Spell(st) => {
                st.can_target_shop() && self.spell_target_ok(p, st, &pl.shop[s])
            }
            // A Tavern minion may go in the pot when it has a type (Stir the
            // Pot).
            TargetKind::ThrowInPot { .. } => {
                !self.data.cards.tribes_of(&pl.shop[s]).is_neutral()
            }
            // Reborn Rites buffs any minion; Galakrond's Greed needs a Tier
            // below the top so there is one above it; See the Light takes a
            // minion; Spirit Swap takes any minion twice; Lock and Load takes
            // any Tavern minion.
            TargetKind::TempReborn
            | TargetKind::StealShopAsStats { .. }
            | TargetKind::LockAndLoad => true,
            // "another Murloc", "a Pirate": a tribe in either zone, when the
            // card's words do not say "friendly" (`takes_shop` said it may).
            TargetKind::MinionBuff { tribe, .. } => {
                tribe.is_none_or(|t| self.data.cards.tribes_of(&pl.shop[s]).has(t))
            }
            TargetKind::ReplaceHigher => self.data.cards.get(pl.shop[s].card).tier < crate::MAX_TIER,
            TargetKind::SpiritSwapFirst => true,
            TargetKind::SpiritSwapSecond => {
                !matches!(pl.two_step, Some((f, true)) if f as usize == s)
            }
            _ => false,
        }
    }

    /// Whether the pending target may be answered on the Tavern minion in
    /// slot `s` when a Battlecry picks its target at random (docs/rules/trinkets.md:
    /// a Tavern minion is allowed). Only a plain "a minion" buff reaches the
    /// Tavern this way; a Battlecry about your own minions does not.
    fn valid_battlecry_shop_target(&self, p: usize, pt: &PendingTarget, s: usize) -> bool {
        let pl = &self.players[p];
        if s >= pl.shop.len() {
            return false;
        }
        let c = self.data.cards.get(pl.shop[s].card);
        if c.is_spell() || c.fixed_stats {
            return false;
        }
        match pt.kind {
            TargetKind::MinionBuff { tribe, .. } => self.data.cards.tribes_of(&pl.shop[s]).is(tribe),
            TargetKind::AnyBuff { .. } | TargetKind::AnySetStats { .. } => true,
            TargetKind::Spell(st) => {
                st.can_target_shop() && self.spell_target_ok(p, st, &pl.shop[s])
            }
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
                            .any(|s| self.spell_target_ok(p, st, &pl.shop[s])))
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
                    && (0..pl.shop.len()).any(|s| self.spell_target_ok(p, st, &pl.shop[s])))
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
                self.data.cards.tribes_of(&b[s]).is(tribe)
            }
            TargetKind::AnyBuff { .. } | TargetKind::AnySetStats { .. } => true,
            // A card steal is answered on a Tavern slot, never a board one.
            TargetKind::StealShopCard { .. } => false,
            TargetKind::Spell(st) => self.valid_spell_target(p, st, s),
            TargetKind::MakeGolden { tribe, max_tier } => {
                let c = self.data.cards.get(b[s].card);
                !b[s].golden && c.tier <= max_tier && self.data.cards.tribes_of(&b[s]).is(tribe)
            }
            TargetKind::DestroyDiscover { tribe, .. } | TargetKind::DestroyCopy { tribe, .. } => {
                self.data.cards.tribes_of(&b[s]).has(tribe)
            }
            TargetKind::RebornDestroy { tribe, .. } => {
                let host = pt.source.map(|src| b[src as usize].card);
                self.data.cards.tribes_of(&b[s]).has(tribe) && Some(b[s].card) != host
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
            // Any minion will do: the effect becomes a copy of it.
            TargetKind::BecomeCopy => true,
            // Any friendly minion may be removed for the Discover from the
            // Tier below it; only a typed one may go in the pot (a neutral
            // minion cannot be thrown, docs/rules/heroes.md).
            TargetKind::RemoveTierBelow => true,
            TargetKind::ThrowInPot { .. } => !self.data.cards.tribes_of(&b[s]).is_neutral(),
            // Only a non-Golden minion may be swapped out.
            TargetKind::SwapWithShop => !b[s].golden,
            // Reborn Rites (any minion), Arcane Alteration (the board too),
            // Devour (the minion to sell and the one that takes its stats)
            // and Cloning Gallery (an exact copy) all take a board minion.
            TargetKind::TempReborn
            | TargetKind::ReplaceSameTier
            | TargetKind::DevourFirst
            | TargetKind::SpiritSwapFirst => true,
            // Spirit Swap's second pick is a minion different from the first
            // one, on the board or in the Tavern (checked with the remembered
            // pick).
            TargetKind::SpiritSwapSecond => {
                !matches!(self.players[p].two_step, Some((f, false)) if f as usize == s)
            }
            // A board minion only: the minion to sell into (Devour, after the
            // sale), the copy taken (Cloning Gallery) and the minion destroyed
            // at the start of combat (Rapid Reanimation).
            TargetKind::DevourSecond
            | TargetKind::SummonExactCopy
            | TargetKind::RapidReanimation => true,
            // A Tavern card only, and a Tavern minion only.
            TargetKind::ReplaceHigher
            | TargetKind::StealShopAsStats { .. }
            | TargetKind::ImprisonShop { .. }
            | TargetKind::LockAndLoad => false,
            TargetKind::DestroyGetTribe { tribe } => {
                self.data.cards.tribes_of(&b[s]).has(tribe)
            }
        }
    }

    /// Take an action for the current player.
    pub fn step(&mut self, a: Action) -> Result<(), IllegalAction> {
        // A choice nothing can answer fizzles here, before it is answered:
        // `is_legal` already let the normal actions through on that basis.
        if let Some(p) = self.current_player()
            && self.choice_stuck(p)
        {
            self.fizzle_stuck_choice(p);
        }
        if !self.is_legal(a) {
            return Err(IllegalAction);
        }
        let p = self.current_player().unwrap();
        self.apply(p, a);
        self.resolve_doomed(p);
        self.resolve_hogrider(p);
        // A play whose Battlecry asked for an answer fires its "after you
        // play an X" triggers once the last one is in.
        self.resolve_after_play(p);
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
            // A card the start of the turn queued with a full hand joins as
            // soon as this action freed a slot (docs/rules/design.md "A full hand").
            self.drain_hand_queue(p);
            // Makeshift Master: whatever the watched minion gained while the
            // action ran, the Deity gains too.
            self.watch_deity_gain(p);
            // Mishmash: whatever stats a friendly Amalgam gained, it gains
            // again (twice when golden).
            self.note_amalgam_gains(p);
            // Glowing Gauntlet: a Tavern card that just left is replaced at
            // once, a minion by a minion and the spell by a spell.
            self.refill_tavern(p);
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
        // A fight or a hook in `advance` can still hand a knocked-out player
        // something: one more sweep, so a dead hero never holds cards.
        for q in 0..PLAYERS {
            self.settle_dead(q);
        }
        #[cfg(debug_assertions)]
        self.check_invariants().unwrap();
        Ok(())
    }

    /// Recompute the standing rules a hero power or a trinket sets, the way a
    /// turn start does: the shop replay seats a logged power and logged
    /// trinkets on a player it built itself (`crate::shop_replay`).
    pub fn sync_seat_rules(&mut self, p: usize) {
        self.sync_power_flags(p);
    }

    /// Run one player's end of turn effects without the fight that follows
    /// them, which is the state a logged turn ends in: the shop replay
    /// (`bgbench shopreplay`) compares them. `Action::EndTurn` reaches the
    /// same code through `step`, and then fights.
    pub fn shop_end_of_turn(&mut self, p: usize) {
        self.finish_shop(p);
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
                    // Bazaar Sticker: one Tavern spell a turn costs Health.
                    let trinket_health = self.players[p].tp.spell_health_left > 0;
                    let pay_health = c.pay_health || trinket_health;
                    let cost = if pay_health { c.cost } else { self.spell_price(p, id) };
                    if trinket_health {
                        self.players[p].tp.spell_health_left -= 1;
                    }
                    let m = self.players[p].shop.remove(slot);
                    self.add_to_hand(p, m);
                    if m.tickets == 1 {
                        self.take_ticket(p);
                    }
                    self.pay_for_spell(p, pay_health, cost);
                    self.magicfin_teach(p, id);
                    self.fire_event(p, EventKind::CardsBought, 1);
                    // A bought card counts towards BUY, INSECT!, the Tavern
                    // spells included.
                    self.power_bought_card(p, &m);
                    self.players[p].tp.bought_spell = Some(id);
                    self.trinket_event(p, Event::BuySpell, Some(id), 1);
                } else {
                    let price = self.minion_price(p, slot);
                    let buy_cost = match price {
                        MinionPrice::Gold(g) => g,
                        MinionPrice::Health(_) => 0,
                    };
                    self.note_minion_bought(p, price);
                    let m = self.players[p].shop.remove(slot);
                    self.players[p].gold -= buy_cost;
                    self.add_to_hand(p, m);
                    // Come One, Come All!: the buy takes the card's Darkmoon
                    // Ticket with it. The card is in hand first, so a Ticket
                    // that pays out a reward never costs the buy its slot.
                    if m.tickets == 1 {
                        self.take_ticket(p);
                    }
                    // A Health price is self damage, not Gold spent
                    // (docs/rules/design.md). Paid once the card is in hand, so a buy
                    // that kills the hero still settles cleanly.
                    if let MinionPrice::Health(h) = price {
                        self.take_shop_damage(p, h as i32, None);
                    }
                    self.after_buy_effects(p);
                    // Transcribing Typewriter: an exact copy of the minion
                    // bought (buffs included), so the bought one keeps the
                    // buffs it was bought with and gains any "after you buy"
                    // ones before the copy is taken.
                    if self.players[p].tp.copy_buys > 0
                        && let Some(&bought) = self.players[p].hand.last()
                        && bought.card == m.card
                        && !self.players[p].hand.is_full()
                    {
                        self.players[p].tp.copy_buys -= 1;
                        let mut copy = bought;
                        copy.pool_copies = 0;
                        self.add_to_hand(p, copy);
                    }
                    self.check_triple(p, m.card);
                    self.fire_event(p, EventKind::GoldSpent, buy_cost as u32);
                    self.fire_event(p, EventKind::CardsBought, 1);
                    self.power_bought_card(p, &m);
                    // A trinket that reads the minion just bought: its stats
                    // as they stand now (Reusable Batteries) or its card (the
                    // Gold-plated Compass) or the Tavern spell (Magicfin
                    // Sticker).
                    let bought = self.players[p]
                        .hand
                        .iter()
                        .rev()
                        .find(|h| h.card == m.card)
                        .copied();
                    self.players[p].tp.bought_card = Some(m.card);
                    if let Some(b) = bought {
                        self.players[p].tp.bought_stats = (b.attack, b.health);
                    }
                    self.trinket_event_minion(p, Event::Buy, &m, 1);
                }
            }
            Action::BuySpell => {
                let id = self.players[p].shop_spell.take().unwrap();
                let ticket = std::mem::replace(&mut self.players[p].shop_spell_ticket, 0) == 1;
                let c = self.data.cards.get(id);
                let trinket_health = self.players[p].tp.spell_health_left > 0;
                let pay_health = c.pay_health || trinket_health;
                let cost = if pay_health { c.cost } else { self.spell_price(p, id) };
                if trinket_health {
                    self.players[p].tp.spell_health_left -= 1;
                }
                let pl = &mut self.players[p];
                let m = pl.make_card(&self.data.cards, id, false, 0);
                self.add_to_hand(p, m);
                // Come One, Come All!: the spell slot's Ticket, taken with the
                // spell.
                if ticket {
                    self.take_ticket(p);
                }
                self.pay_for_spell(p, pay_health, cost);
                self.magicfin_teach(p, id);
                self.fire_event(p, EventKind::CardsBought, 1);
                // A bought card counts towards BUY, INSECT!, the Tavern
                // spells included.
                self.power_bought_card(p, &m);
                self.players[p].tp.bought_spell = Some(id);
                self.trinket_event(p, Event::BuySpell, Some(id), 1);
            }
            Action::Sell(s) => {
                self.sell_minion(p, s as usize);
            }
            Action::Activate(s) => {
                let (act, golden) = self.activate_of(p, s as usize).unwrap();
                // Dark Gifts Efficiency: the Activate triggers twice.
                let runs = if self.players[p].board[s as usize]
                    .has_gift(&self.data.cards, |c| c.activate_twice)
                {
                    2
                } else {
                    1
                };
                self.players[p].gold -= act.cost;
                self.players[p].board[s as usize].activated = true;
                if act.discard {
                    // The reward runs once the player has picked the card.
                    self.players[p].pending_discard = Some(PendingDiscard {
                        source: Some(s),
                        reward: act.effect,
                        golden,
                        runs,
                    });
                } else {
                    let mut pos = s as usize;
                    self.run_effect(p, &mut pos, act.effect, golden, runs);
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
                    // A card played from the hand, of any kind: Blood Gems
                    // and Bananas too (Dragonwing Glider).
                    self.trinket_event(p, Event::CardPlayed, Some(m.card), 1);
                    // Dark Gifts "whenever you play a card" (Dexterity,
                    // Sharpened Sword, Toughened Shield).
                    self.dark_gift_card_played(p);
                    self.cast_spell(p, m);
                    if choose_one {
                        // Turbo Hogrider reacts once the card is resolved.
                        self.players[p].hogrider_pending += 1;
                    }
                    return;
                }
                let pos = pos as usize;
                // Dark Gifts "whenever you play a card".
                self.dark_gift_card_played(p);
                let magnetic = m.keywords.has(Keywords::MAGNETIC);
                let is_merge = magnetic
                    && pos < self.players[p].board.len()
                    && self.can_magnetize(m.card, &self.players[p].board[pos]);
                let mut choose_one_play = false;
                self.trinket_event(p, Event::CardPlayed, Some(m.card), 1);
                // A Magnetic minion played on its own is still a Magnetic
                // minion played (Spell-powered Wrench); a merge fires it too,
                // inside `magnetize_to`.
                if magnetic && !is_merge {
                    self.trinket_event(p, Event::Magnetize, Some(m.card), 1);
                }
                if is_merge {
                    self.magnetize_to(p, pos, m, true);
                    if m.golden && !m.no_reward {
                        self.give_play_reward(p);
                    }
                } else {
                    let mut m = m;
                    if self.data.cards.tribes_of(&m).has(Tribe::Beast) {
                        let (ba, bh) = self.players[p].beast_bonus;
                        m.attack += ba;
                        m.health += bh;
                    }
                    // A minion landing on the board starts with its limited
                    // uses for the turn (a Trailblazer's combines, a
                    // Mycologist's teachings, Malchezaar's Health Refreshes).
                    m.uses_left = m.turn_uses(&self.data.cards);
                    self.players[p].board.insert(pos, m);
                    // Hat Trick: the minion comes with a +1/+1 hat, already in
                    // its stats; hats stack (docs/rules/heroes.md).
                    let hat = self.players[p].hat;
                    if hat != (0, 0) {
                        let b = &mut self.players[p].board[pos];
                        b.attack += hat.0;
                        b.health += hat.1;
                        b.hats = b.hats.saturating_add(1);
                    }
                    // Defensive Shell: the first minion played each turn
                    // gains Divine Shield.
                    if self.players[p].tp.play_shields > 0 {
                        self.players[p].tp.play_shields -= 1;
                        let b = &mut self.players[p].board[pos];
                        b.keywords.set(Keywords::DIVINE_SHIELD);
                    }
                    let t = self.data.cards.tribes_of(&m);
                    // The Triple Reward comes before anything the Battlecry
                    // adds to the hand (and is lost on a full hand).
                    if m.golden && !m.no_reward {
                        self.give_play_reward(p);
                    }
                    choose_one_play = self.data.cards.get(m.card).choose_one.is_some();
                    if t.has(Tribe::Elemental) {
                        let pl = &mut self.players[p];
                        pl.elementals_played_turn = pl.elementals_played_turn.saturating_add(1);
                        pl.elementals_played = pl.elementals_played.saturating_add(1);
                        self.give_play_shield(p, pos);
                        // Avalanche: 3 off the upgrade every third Elemental.
                        self.power_elemental_played(p);
                    }
                    // "After you play an X" triggers (Wrath Weaver, Insatiable
                    // Ur'zul, Ultraviolet Ascendant) wait for the Battlecry to
                    // resolve: the raw log puts them after it, and after
                    // whatever it asks (2026-11-01_17_59_13-g1 t8s26,
                    // 2026-10-24_15_25_26's Imposing Percussionist Discover).
                    // `just_played` keeps the played minion out of its own
                    // trigger, wherever its Battlecry left it on the board.
                    self.players[p].board[pos].just_played = true;
                    self.players[p].pending_after_play = Some(t);
                    // A trinket that watches every minion played.
                    self.trinket_event_minion(p, Event::Play, &m, 1);
                    // Hammer of Twilight: "your minions have +X" reaches the
                    // minion that just landed.
                    self.hammer_on_play(p, pos);
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
                    // "After you play 7 Battlecry minions" (Chromatic Tear's
                    // greater form). It comes last: its reward can triple the
                    // board, which moves the slot the minion was played into
                    // (2026-10-31: a stale slot panicked here).
                    if self.data.cards.get(m.card).battlecry.is_some() {
                        self.trinket_event(p, Event::PlayBattlecry, Some(m.card), 1);
                    }
                    // The play is over: its "after you play an X" triggers
                    // fire now, or, if it left a prompt open, when the last
                    // answer comes (`step`, `finish_shop`).
                    self.resolve_after_play(p);
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
                let refresh_cost = self.refresh_cost(p);
                let Game { data, rng, players, pool, lobby, .. } = self;
                let (cards, rules) = (&data.cards, &data.rules);
                let pl = &mut players[p];
                let mut gold_spent = 0;
                let mut health_paid = 0;
                if pl.free_rolls > 0 {
                    pl.free_rolls -= 1;
                    // Malchezaar, Prince of Dance's bug, which we copy
                    // (docs/rules/design.md "Malchezaar and free Refreshes"): the last
                    // free Refresh of the turn also uses up one of his two
                    // Health Refreshes, so a turn with any free Refresh at all
                    // has one Health Refresh left, not two.
                    if pl.free_rolls == 0 {
                        pl.spend_health_refresh(cards);
                    }
                } else if pl.can_refresh_with_health(cards, rules) {
                    pl.spend_health_refresh(cards);
                    health_paid = rules.health_refresh_cost;
                } else {
                    pl.gold -= refresh_cost;
                    gold_spent = refresh_cost;
                }
                Self::refresh_shop(pl, pool, cards, rules, rng, *lobby, RefreshKind::Explicit);
                // "After the Tavern is Refreshed": the trinket hook.
                self.trinket_event(p, Event::Refresh, None, 1);
                if health_paid > 0 {
                    // A Health cost is not a Demon dealing damage.
                    self.take_hero_damage(p, health_paid, None);
                }
                self.fire_event(p, EventKind::GoldSpent, gold_spent as u32);
            }
            Action::Freeze => self.players[p].frozen = !self.players[p].frozen,
            Action::Level => {
                let turn = self.turn as u8;
                let cost = self.upgrade_price(p);
                let pl = &mut self.players[p];
                pl.gold -= cost;
                pl.tier += 1;
                pl.upgrade_cost = self.data.rules.upgrade_cost(pl.tier);
                pl.tier_turns[pl.tier as usize - 1] = turn;
                // "After you upgrade the Tavern, gain 2 Gold" (Everbloom), and
                // an unlock condition may just be met (Queen of Dragons).
                let runs = self.players[p].tp.power_triggers.max(1);
                let gain = [self.players[p].power, self.players[p].power2]
                    .iter()
                    .flatten()
                    .filter_map(|st| match self.data.heroes.power(st.of).effect {
                        PowerEffect::GoldAfterUpgrade(g) => Some(g),
                        _ => None,
                    })
                    .max()
                    .unwrap_or(0)
                    .saturating_mul(runs);
                self.players[p].gold = self.players[p].gold.saturating_add(gain);
                // "When you reach Tier 6, gain 12 Gold" (Stuffed Coin Purse).
                if self.players[p].tier == crate::MAX_TIER && self.players[p].tp.gold_on_tier6 > 0 {
                    let g = self.players[p].tp.gold_on_tier6;
                    self.players[p].tp.gold_on_tier6 = 0;
                    self.players[p].gold = self.players[p].gold.saturating_add(g);
                }
                self.refresh_power_unlock(p);
                self.fire_event(p, EventKind::GoldSpent, cost as u32);
            }
            Action::HeroPower => self.press_power(p, false),
            Action::HeroPower2 => self.press_power(p, true),
            Action::ChooseTrinket(k) => self.pick_trinket(p, k as usize),
            Action::DarkGift => self.press_dark_gift(p),
            Action::EndTurn => {}
        }
    }

    fn give_generated(&mut self, p: usize, card: CardId) {
        let mut dummy = 0;
        self.give_generated_tracking(p, card, &mut dummy);
    }

    fn add_to_hand(&mut self, p: usize, mut m: Minion) -> bool {
        if self.players[p].hand.is_full() {
            // A card that arrives at the start of the turn (a compass
            // trinket's tribe card) waits for room; a card got at any other
            // time is lost (docs/rules/design.md "A full hand"). A queued card is not in
            // the hand yet, so nothing that reads a card entering the hand
            // fires until it joins (`drain_hand_queue`).
            let pl = &mut self.players[p];
            if pl.hand_queue_on && !pl.hand_queue.is_full() {
                // A Darkmoon Ticket and a per-card Freeze stay in the Tavern.
                m.tickets = 0;
                m.frozen = false;
                pl.hand_queue.push(m);
            }
            return false;
        }
        // A Darkmoon Ticket and a per-card Freeze belong to the Tavern, not to
        // the card: a card that leaves it carries neither.
        m.tickets = 0;
        m.frozen = false;
        self.players[p].hand.push(m);
        // "Whenever you get a Pirate" (Doubloon Grifter's event trigger, and a
        // trinket's `on_get`): a card of that tribe entered the hand, from
        // anywhere (bought, generated, Discovered, copied).
        let tribes = self.data.cards.tribes_of(&m);
        if !tribes.is_empty() {
            for t in Tribe::EVERY {
                if !tribes.has(t) {
                    continue;
                }
                self.fire_event(p, EventKind::CardGot(t), 1);
                self.trinket_event(p, crate::trinkets::Event::GetTribe(t), Some(m.card), 1);
            }
        }
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
        // Dark Gifts Double Vision: the minion brings a plain copy of itself
        // along. The copy carries no gift, so this never chains.
        self.dark_gift_on_acquire(p, &m);
        true
    }

    /// The cards the start of the turn queued join the hand, first in first
    /// out, for as long as there is room (docs/rules/design.md "A full hand"). A card
    /// that joins can make a triple, and a triple's golden card takes a slot
    /// of its own, so the hand can fill again here.
    fn drain_hand_queue(&mut self, p: usize) {
        while !self.players[p].hand.is_full() && !self.players[p].hand_queue.is_empty() {
            let m = self.players[p].hand_queue.remove(0);
            let card = m.card;
            if self.add_to_hand(p, m) && !self.data.cards.get(card).is_spell() {
                self.check_triple(p, card);
            }
        }
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
        // A power chosen for the next Start of Combat has fired: the chooser
        // it came from is back (Bru'kan's Elements).
        for p in 0..PLAYERS {
            if self.players[p].alive {
                self.power_after_fight(p);
            }
        }
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
