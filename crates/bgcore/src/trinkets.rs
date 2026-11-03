//! Trinkets: what the lesser and greater slots do, parsed from
//! trinkets.toml. Effects are data here as they are in cards.toml, and the
//! rules code never matches a trinket by name. See docs/rules/design.md "Trinkets and
//! Dark Gifts (2026-10-30)" and docs/engine.md "Trinkets".
//!
//! A trinket has no host minion, so it needs its own place in the trigger
//! dispatch: the lists below are the hook points `Game::trinket_event` and
//! the turn flow call. The keys inside each list mirror the effect keys of
//! cards.toml wherever one exists (`get`, `get_gems`, `cast`, ...), and
//! `Game::run_trinket_act` calls the same code the card path calls. The few
//! keys with no card behind them are listed in `Act` with their card.

use std::collections::HashMap;

use serde::Deserialize;

use crate::cards::{CardDb, Tribe, Tribes};
use crate::minion::{CardId, Keywords};

/// Lesser slot, greater slot, and the trinket a hero power can become
/// (Marin the Manager, Buttons).
pub const TRINKET_SLOTS: usize = 3;
/// Options in one offer (Era's logs: always exactly 4).
pub const TRINKET_CHOICES: usize = 4;
/// Options the Mystery Cube offers each turn ("choose from 2 new Lesser
/// Trinkets").
pub const CUBE_CHOICES: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Slot {
    Lesser,
    Greater,
}

/// What a trinket leans toward, for the offer's mechanic lean. Only the
/// offer uses it; it is not a rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mechanic {
    Battlecry,
    Deathrattle,
    Spells,
}

/// The events a trinket's "every N" counter can count.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Event {
    /// Your turns, the pick turn counting as 1 (fired at the end of the
    /// turn the count reaches N).
    Turns,
    /// A minion bought (and played, for `Play`), a Tavern spell bought.
    Buy,
    BuySpell,
    Sell,
    Play,
    /// A minion of this tribe played.
    PlayTribe(Tribe),
    /// The player's own Refresh, or one an effect did (the same places
    /// "after the Tavern is Refreshed this game" fires).
    Refresh,
    /// Any spell cast at all, wherever the cast comes from: a Tavern spell,
    /// a Blood Gem, Gem Day, a Banana, a Spellcraft spell. "Whenever you
    /// cast a spell" / "after you cast N spells" (docs/rules/design.md "Read the
    /// card's exact words, every time"): Bewitched Ribbon, Bloodbound
    /// Earrings, Charming Panpipes.
    SpellCast,
    /// A Tavern spell cast (`tavern_spell = true` in cards.toml, a
    /// BATTLEGROUND_SPELL in the game data). "Cast a Tavern spell" is the
    /// narrower one: Comfy Coffin, Bluegill Flippers, Flighty Portrait.
    CastSpell,
    /// A spell cast on a friendly board minion, of any kind.
    CastSpellOnMinion,
    /// Gold spent, counted by the amount (a 3 Gold buy adds 3).
    GoldSpent,
    /// A card discarded.
    Discard,
    /// A friendly minion died, in the shop or in a fight (docs/rules/design.md: shop
    /// deaths count for death counters).
    Death,
    /// A friendly minion of this tribe died.
    DeathTribe(Tribe),
    /// A minion was summoned, which only happens inside a fight today.
    Summon,
    /// A minion of this tribe was summoned.
    SummonTribe(Tribe),
    /// A card of this tribe entered the player's hand, from anywhere:
    /// bought, generated, Discovered or copied (Trusty Crowbar).
    GetTribe(Tribe),
    // ---- block 4 ----
    /// A card played from the hand of any kind: a minion, a Tavern spell, a
    /// Blood Gem, a Banana (Dragonwing Glider).
    CardPlayed,
    /// A Battlecry triggered, once per trigger, so a Brann double counts
    /// twice (Dragon Skull).
    Battlecry,
    /// A minion with a Battlecry played from hand (Chromatic Tear's greater
    /// form).
    PlayBattlecry,
    /// A Blood Gem cast from hand (Ruby Tusk).
    Gem,
    /// A friendly minion attacked in a fight, once per swing, so a Windfury
    /// minion's two attacks count 2 (Dragonhead Staff).
    Attack,
    /// You destroyed a minion outside combat (Maw Caster Portrait,
    /// Plaguerunner Portrait).
    Destroy,
    /// A Magnetic minion was played, as a Magnetization or on its own
    /// (Spell-powered Wrench).
    Magnetize,
    /// A spell was cast on a Tavern minion (Felsteel Cleaver).
    CastSpellOnShop,
    /// A friendly minion consumed a Tavern minion (Consuming Claw).
    Consume,
    /// Your hero took damage outside combat (Nether Pendant).
    HeroDamage,
}

/// The player holds a trinket with an effect at all (any hook or rule).
pub const HAS_TRINKET: u32 = 1 << 31;
/// A trinket wants the start of the turn (a `start_of_turn` list or a
/// "every N turns" rule, which counts there).
pub const STARTS_TURN: u32 = 1 << 30;
/// A trinket wants the end of the turn (an `end_of_turn` list or a rule).
pub const ENDS_TURN: u32 = 1 << 29;

impl Event {
    /// The bit a player's trinket mask uses for this event.
    pub fn bit(self) -> u32 {
        match self {
            // A rule that counts a tribe's plays is still a Play event.
            Event::PlayTribe(_) => 1 << 4,
            Event::Turns => 1 << 0,
            Event::Buy => 1 << 1,
            Event::BuySpell => 1 << 2,
            Event::Sell => 1 << 3,
            Event::Play => 1 << 4,
            Event::Refresh => 1 << 5,
            Event::CastSpell => 1 << 6,
            Event::SpellCast => 1 << 23,
            Event::CastSpellOnMinion => 1 << 7,
            Event::GoldSpent => 1 << 8,
            Event::Discard => 1 << 9,
            // A rule that counts a tribe's deaths or summons is still a
            // Death or a Summon event.
            Event::Death | Event::DeathTribe(_) => 1 << 10,
            Event::Summon | Event::SummonTribe(_) => 1 << 11,
            Event::GetTribe(_) => 1 << 12,
            Event::CardPlayed => 1 << 13,
            Event::Battlecry => 1 << 14,
            Event::PlayBattlecry => 1 << 15,
            Event::Gem => 1 << 16,
            Event::Attack => 1 << 17,
            Event::Destroy => 1 << 18,
            Event::Magnetize => 1 << 19,
            Event::CastSpellOnShop => 1 << 20,
            Event::Consume => 1 << 21,
            Event::HeroDamage => 1 << 22,
        }
    }
}

/// One thing a trinket does. The doc names the card a key was written for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Act {    /// Get `count` generated copies of a card (`get`).
    Get { card: CardId, count: u8 },
    /// Gain Gold now (`gold`).
    Gold(u8),
    /// Raise the maximum Gold for good (`max_gold`).
    MaxGold(u8),
    /// Reduce the price of the next upgrade (`upgrade_discount`).
    UpgradeDiscount(u8),
    /// Get Blood Gems (`get_gems`).
    Gems(u8),
    /// Your Blood Gems give more for good (`gem_bonus`).
    GemBonus { attack: i32, health: i32 },
    /// Your Tavern spells give more for good (`spell_bonus`).
    SpellBonus { attack: i32, health: i32 },
    /// Discover this many Tavern spells (`discover_spell`).
    DiscoverSpell(u8),
    /// Discover this many minions with a Deathrattle (`discover_deathrattle`).
    DiscoverDeathrattle(u8),
    /// Discover a SECOND Hero Power, into the player's second slot
    /// (`second_power`): Reinvigorating Light.
    SecondPower,
    RandomChromadrake(u8),
    RandomVolumizer(u8),
    RandomBounty(u8),
    RandomChooseOne(u8),
    RandomTierMinion { tier: u8, count: u8 },
    /// Get `count` random minions of a tribe up to your tier (`random_minion`).
    RandomMinion { tribe: Tribe, count: u8 },
    /// Get this many random Tavern spells up to your tier (`random_spell`).
    RandomSpell(u8),
    /// Cast a named card (`cast`) this many times.
    Cast { card: CardId, times: u8 },
    /// Buff the Tavern (`tavern_buff`), now and, unless `this_turn`, for the
    /// rest of the game.
    TavernBuff { tribe: Option<Tribe>, attack: i32, health: i32, this_turn: bool },
    /// Buff the minions in the Tavern right now, this turn only, with no
    /// memory: Cursed Crystal's every Refresh must not stack (a remembered
    /// this-turn Tavern buff would be applied twice on the next one).
    BuffTavernThisTurn { attack: i32, health: i32 },
    /// Buff the left-most card in hand (`buff_leftmost_hand`).
    BuffLeftmostHand { attack: i32, health: i32 },
    /// Buff every friendly minion on the board (`buff_all`).
    BuffAll { tribe: Option<Tribe>, attack: i32, health: i32 },
    /// Your Undead have more, wherever they are (`undead_bonus`).
    UndeadBonus { attack: i32, health: i32 },
    /// Give your Deity stats (`buff_deity`).
    BuffDeity { attack: i32, health: i32 },
    FreeRefreshes(u8),
    /// Every friendly Demon consumes this many Tavern minions (`demons_consume`).
    DemonsConsume(u8),
    /// Add Fodders to the next Refresh (`add_fodder_refreshes`).
    AddFodderRefreshes(u8),
    /// Get a Lockbox, or open the held one sooner (`get_lockbox`).
    GetLockbox(u8),
    /// Get linked Tavern spells (`linked_spells`).
    LinkedSpells(u8),
    /// Make a random friendly minion of this tier or below Golden: Gold
    /// Pendant. `Effect::MakeGoldenChosen` asks the player instead.
    MakeGoldenRandom(u8),
    /// Get this many copies of the last Tavern spell you cast: Sphere of
    /// Memory.
    LastSpellCopies(u8),
    /// Play this many Blood Gems on every friendly minion: Bloodbound
    /// Earrings, Great Boar Sticker's counterpart (`gems_on_board`).
    GemsOnBoard(u8),
    // ---- prices, Gold and the trinket slots (docs/rules/design.md, second round) ----
    /// Minion buys that cost Health instead of Gold: this many this turn, of
    /// a tribe or any (Pilgrimp Sticker).
    HealthPriceMinions { tribe: Option<Tribe>, count: u8 },
    /// The highest-Tier minion in the Tavern costs Health (Demonic Tapestry).
    HealthPriceHighestTier,
    /// Every Nth card bought costs Health (The Eye of Sargeras).
    EveryNthBuyHealth(u8),
    /// A Tavern spell costs Health this turn (Bazaar Sticker).
    HealthPriceSpell(u8),
    /// This many minion buys are free this turn (Warcry Totem).
    FreeBuys(u8),
    /// The first buys of this tribe each turn are free (Doubloon Grifter).
    FreeTribeBuys { tribe: Tribe, count: u8 },
    /// Reaching Tier 6 gives this much Gold once (Stuffed Coin Purse).
    GoldOnTier6(u8),
    /// Gold for each different friendly minion type at the start of the turn
    /// (Glowing Crystal).
    GoldPerType(u8),
    /// Gold for each friendly Golden minion at the start of the turn (Rascal
    /// Sticker).
    GoldPerGolden(u8),
    /// With 15 Gold or more, a random Golden minion of this tier, once
    /// (Splinter of Aurum).
    GoldenWhenRich(u8),
    /// The next trinket offer is a Lesser one (Mysterious Orb).
    NextTrinketLesser,
    /// The Greater offer comes at the start of this turn instead of turn 9
    /// (Ornate Clock: the turn after the pick).
    GreaterOfferOnTurn(u32),
    /// A Greater offer into the lesser slot in this many turns, replacing
    /// this trinket (Trip Vouchers).
    GreaterOfferIntoLesser(u8),
    /// Replace this trinket with a random trinket of a slot (and tribe):
    /// Quartermaster's Hook, Dragonhead Staff, Ruby Tusk, Soul Battery.
    ReplaceWithRandom { slot: Slot, tribe: Option<Tribe> },
    /// Raise this trinket's own counter ("improve this"): Shark Cannon,
    /// Panpipes.
    GrowValue(u8),
    /// Buff every friendly minion of a tribe by the base plus the trinket's
    /// own counter (Shark Cannon's improving Pirate buff).
    BuffAllGrowing { tribe: Option<Tribe>, attack: i32, health: i32 },
    /// Buff the left-most friendly minion by the base plus the trinket's own
    /// counter (Charming Panpipes).
    BuffLeftmostGrowing { attack: i32, health: i32 },
    /// Buff every friendly minion by the base, once more for each Golden
    /// minion the player has played this game (Gold Mallet).
    BuffAllPerGolden { attack: i32, health: i32 },
    /// Buff the two left-most friendly minions by the base plus this many per
    /// Battlecry the player has triggered this game (Murky Sticker).
    BuffLeftmostTwoPerBattlecry { attack: i32, health: i32 },
    /// This many random minions of a minion type: the one this trinket locked
    /// in with `lock_type`, else the player's most common type now (Charisma,
    /// both Compasses).
    RandomMostCommonType(u8),
    /// Give this many random friendly minions of a tribe stats (Booty Bay
    /// Brew).
    BuffRandomOther { tribe: Option<Tribe>, attack: i32, health: i32, count: u8 },
    /// The Mystery Cube: deal this many free Lesser trinkets to replace the
    /// one in this slot, and mark the slot as a cube slot, so the offer comes
    /// back at the start of every turn for the rest of the game.
    OfferReplace(u8),
    // ---- getting cards and Discover (block 2) ----
    /// Discover a minion of exactly this tier, of any type or, when `typed`,
    /// of one with a type, and remember the pick in this trinket's slot
    /// (Portable Factory).
    DiscoverRemember { tier: u8, typed: bool },
    /// Discover a minion of exactly this tier (Azeroth Model Globe).
    DiscoverTier(u8),
    /// The next `n` minions bought also come as an exact copy (Transcribing
    /// Typewriter).
    CopyBuys(u8),
    /// Get this many copies of the card this trinket remembers (Portable
    /// Factory's every turn).
    GetRemembered(u8),
    /// Get this many random minions with a Battlecry (Rockin' Music Box).
    RandomBattlecry(u8),
    /// A Dark Gift offer (Ominous Stone, Wax Lance, Sinister Invitation): a
    /// fixed tier or, with `use_table`, the Dark Discovery button's tiers for
    /// this turn; `yours` wants minions of the player's most common type and
    /// `golden` makes every offer Golden.
    DarkGift { tier: Option<u8>, yours: bool, golden: bool, use_table: bool },
    /// A Dark Gift offer of plain copies of three of the player's own
    /// minions, each with a fresh gift picked for it (Maldraxxus Dagger).
    DarkGiftCopies,
    /// Get a plain copy of a random friendly minion (Reflective Pendant).
    PlainCopyRandomFriendly(u8),
    /// Get a copy of the last card discarded, its stats doubled (Writhing
    /// Tentacles).
    CopyLastDiscarded,
    /// Play this many Blood Gems on one friendly minion of each type on the
    /// board (Aggem Sticker).
    GemsEachType(u8),
    /// The first `n` Discovers each turn also give a copy of the pick
    /// (Sinstone Sticker).
    DiscoverCopyBonus(u8),
    /// Buff the left-most friendly minion (Trusty Crowbar).
    BuffLeftmostBoard { attack: i32, health: i32 },
    /// Get a Golden copy of a named card (Escapee Portrait).
    GetGolden { card: CardId },
    /// Your Lockboxes open this many turns sooner (Escapee Portrait).
    LockboxSooner(u8),
    /// Magnetize a copy of a named card to the left- and right-most minions
    /// of a tribe (Accord-o-Tron Portrait).
    MagnetizeEnds { card: CardId, tribe: Option<Tribe> },
    /// Get one of two named cards, at random (the lesser Baller Portrait).
    OneOf { a: CardId, b: CardId },
    /// Gain a free Refresh that holds plain copies of the warband (Warband
    /// Whistle).
    WarbandWhistle,
    /// Two copies of a Pirate make it Golden (Designer Eyepatch).
    TripleNeedTwoPirate,
    // ---- the hero power (Ancient Wishbone, Sous Chef Sticker) ----
    /// The Hero Power's effect runs this many times per press and per
    /// passive tick (Ancient Wishbone: twice). It still costs once.
    PowerTriggers(u8),
    /// Extra Hero Power uses each turn (Sous Chef Sticker).
    PowerExtraUses(u8),
    /// Gold after each Hero Power press (Sous Chef Sticker).
    PowerGold(u8),
    // ---- the Tavern's shape (block 3) ----
    /// The Tavern always holds six minions and one spell, and refills a
    /// bought or consumed card at once (Glowing Gauntlet).
    Gauntlet,
    /// The Tavern offers one extra minion, of the tier above yours, whenever
    /// it is Refreshed (Innkeeper's Stein).
    ExtraHighTier,
    /// The Tavern offers one extra Magnetic Mech whenever it is Refreshed
    /// (Electrode Attractor).
    ExtraMagnetic,
    /// Magnetic Mechs cost this much (Electrode Attractor: 2).
    MagneticCost(u8),
    /// Each turn, the first this many Refreshes hold only Tier 6 minions
    /// (Guiding Candle).
    Tier6Refreshes(u8),
    /// The Tavern no longer offers cards below this tier (Bob-blehead: 3).
    MinTier(u8),
    // ---- the leftovers (block 3) ----
    /// Make the player's Deity Golden for the rest of the game (Mask of the
    /// Ancient Ones).
    GoldenDeity,
    /// Discover `count` minions of exactly `tier` and set the picks' stats to
    /// this much (Innkeeper's Hearth: two Tier 6 minions at 30/30).
    DiscoverTierStats { tier: u8, count: u8, attack: i32, health: i32 },
    // ---- the cheap leftovers ----
    /// Get `count` random Magnetic minions (Scraper Sticker).
    RandomMagnetic(u8),
    /// The first minion played each turn gains Divine Shield (Defensive
    /// Shell): how many such shields this turn.
    FirstPlayShield(u8),
    /// Double the stats of the highest-Health minion in the Tavern (Upstart
    /// Embers).
    DoubleHighestTavern,
    /// Steal the highest-Tier card from the Tavern, ties at random (Rendle
    /// Sticker).
    StealHighestTierTavern,
    /// Your end-of-turn effects trigger this many times in all (Ghastly
    /// Sticker: twice).
    EndOfTurnTwice,
    /// Your first Tavern spell each turn casts this many times in all
    /// (Replica Cathedral: twice), and how many turns start with it.
    FirstSpellTwice(u8),
    // ---- block 4: player-level hooks (docs/rules/trinkets.md, 2026-10-31) ----
    /// The first Battlecry each turn triggers this many extra times
    /// (War Drum); the per-turn use is the trinket's own.
    WarDrum(u8),
    /// Your Dragons' Battlecries trigger this many extra times
    /// (Dragon's Eye).
    DragonBattlecry(u8),
    /// After you cast a spell on a friendly minion, the trinket casts the
    /// same spell on another friendly minion (Lovely Locket).
    Locket,
    /// Trigger the Battlecries of your left- and right-most minions
    /// (Young Murk-Eye Sticker).
    TriggerBattlecriesEnds,
    /// Trigger the Battlecries of your minions of a named card (Hackerfin
    /// Portrait).
    TriggerBattlecriesOf(CardId),
    /// Trigger every friendly Rally (Insurrectionist's Blade).
    TriggerAllRallies,
    /// Give your left- and right-most minions stats (Dragon Skull).
    BuffEnds { attack: i32, health: i32 },
    /// Trigger the Deathrattles of the `n` left-most Deathrattle minions
    /// (Wolfhead Flail).
    TriggerDeathrattlesLeftmost(u8),
    /// Swap the minion buy / Tavern spell buy alternation (Trickster's
    /// Sleeve).
    FlipAlternate,
    /// Cast a named card on the left-most friendly minion of a tribe
    /// (Emergency Gearblade: a Mech).
    CastOnLeftmost { card: CardId, tribe: Tribe },
    /// Cast a named card on a random friendly minion of a tribe
    /// (Spell-powered Wrench: a Mech).
    CastOnRandom { card: CardId, tribe: Tribe },
    /// A Magnetic Satellite with the Attack and Health of the minion just
    /// bought (Reusable Batteries).
    SatelliteOfBought(CardId),
    /// Get a token with these stats, plus this much per Tavern spell cast
    /// this turn (Inductive Gyroblade).
    GetWithStats { card: CardId, attack: i32, health: i32, per_spell: (i32, i32) },
    /// Get `n` 1/1 Murlocs taught the Tavern spell just bought (Magicfin
    /// Sticker).
    TeachBoughtSpell(u8),
    /// A random friendly minion consumes the Tavern minion a spell was just
    /// cast on (Felsteel Cleaver).
    ConsumeShopHit,
    /// A friendly Demon's consume also takes Bonus Keywords and this much
    /// (Consuming Claw).
    ConsumeClaw { attack: i32, health: i32 },
    // ---- block 4: numbers that grow ----
    /// Double the trinket's own number (Weighted Gauntlet).
    DoubleValue,
    /// Your Tavern spells give this much more for each different friendly
    /// minion type on the board (Multilayered Shield).
    SpellBonusPerType { attack: i32, health: i32 },
    /// Your Tavern spells give this much more for this turn only (Honeycomb
    /// Ring).
    SpellBonusThisTurn { attack: i32, health: i32 },
    /// Give the copies of a named card in your hand and on your board stats
    /// (Flighty Portrait).
    BuffCardHandAndBoard { card: CardId, attack: i32, health: i32 },
    /// Your minions have the base, plus this much per card you have
    /// discarded this game (Hammer of Twilight).
    Hammer { base: (i32, i32), per: (i32, i32) },
    /// One more card was discarded: the board gains the Hammer's step
    /// (Hammer of Twilight).
    HammerDiscarded,
    // ---- block 4: the portraits that rewrite a card ----
    /// Every copy of `card` the player has or will have gets this rewrite.
    CardMod { card: CardId, kind: CardMod },
    /// Your Choose One cards always have both effects (Trailblazer
    /// Sticker).
    ChooseOneAll,
    /// This trinket becomes a copy of the next Greater trinket you pick
    /// (Souvenir Stand).
    Souvenir,
    /// Get one of three named cards, at random (Jewelry Box).
    OneOf3 { a: CardId, b: CardId, c: CardId },
    // ---- block 5: the Darkmoon Prizes and the last trinkets ----
    /// Every Triple Reward becomes this card instead, and one is put in hand
    /// now (Corrupted Tome's Triple Prize).
    TriplePrize(CardId),
    /// Discover a Darkmoon Prize of this Prize tier (the Triple Prize itself,
    /// Tickatus Sticker).
    DiscoverPrize(u8),
    /// Get this many random Darkmoon Prizes, any tier (Hand of Fate, a Wheel
    /// result).
    RandomPrize(u8),
    /// A Safety Patch's Ice Block: the next fatal damage to the hero is
    /// prevented and the hero is Immune for the rest of the turn, once
    /// (docs/rules/trinkets.md).
    IceBlock,
    /// The hidden part of the Surprise Portrait: this chance out of 100 of a
    /// surprise on every buy, sell and Refresh, never twice in a row. On a
    /// buy or a sell an Elemental of Surprise arrives; on a Refresh one joins
    /// the Tavern, or every Tavern slot fills with them.
    Surprise(u8),
    /// Spin the Wheel of Yogg-Saron: one of the trinket's `wheel` acts, each
    /// equally likely (Yogg-Tastic Pastry).
    YoggWheel,
    /// A Wheel result, Curse of Flesh: a random friendly minion gains the
    /// stats of another friendly minion.
    CurseOfFlesh,
    /// A Wheel result, Devouring Hunger: consume every minion in the Tavern,
    /// give their stats to your minions, then Refresh.
    DevouringHunger,
    /// A Wheel result, Rod of Roasting: cast Pyrobuff at random, which gives
    /// a minion this much, until one lands on the bartender or the hero.
    RodOfRoasting { attack: i32, health: i32 },
    /// A Wheel result, Mysterybox: a random minion in the Tavern turns
    /// Golden.
    GoldenTavern,
    /// A Wheel result, Mindflayer Goggles: cast this many random Tavern
    /// spells (Puzzle Box's own random cast).
    CastRandomSpells(u8),
    /// Get this many random Magnetic Mecha-Demons (Mecha-Jaraxxus Sticker):
    /// the tokens `CardDb.mecha_demons` holds, generated.
    MechaDemon(u8),
    // ---- block 6: the compasses (docs/rules/trinkets.md, 2026-11-01) ----
    /// Read the player's most common minion type now and keep it on this
    /// trinket's slot for the rest of the game: "the type is fixed at the
    /// pick" (Colorful Compass, Gold-plated Compass). A `RandomMostCommonType`
    /// act hands out minions of the type it locked in.
    LockType,
    /// The next minion of this trinket's locked type that the player buys
    /// becomes Golden, and it is not a triple: playing it gives no Triple
    /// Reward (docs/rules/design.md). The type is spent by the buy (Gold-plated Compass).
    GoldenBoughtType,
}

/// A rewrite a portrait applies to every copy of a card the player has or
/// will have (docs/rules/trinkets.md "Portraits that rewrite a card"). The card's
/// own effect reads it where that effect runs, so the rules never name a
/// card: the trinket's data says which card it is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CardMod {
    /// The card's "after your hero takes damage" reward also gains this much
    /// Attack (Rewinder Portrait: Soul Rewinder).
    AlsoAttack(i32),
    /// A buff the card gives also lands on its two neighbours (Mindslasher
    /// Portrait, Flaming Portrait).
    AlsoNeighbors,
    /// A buff the card gives also gives Health equal to its Attack (Czarina
    /// Portrait: Charging Czarina).
    AlsoHealth,
    /// The card's Tavern buff also lands on your board's minions of this
    /// tribe (Azerite Portrait: Living Azerite).
    AlsoBoard(Tribe),
    /// Every copy of the card counts as these tribes too, wherever a tribe
    /// is read (Bronzebeard Portrait: your Branns are Murlocs and Dragons).
    /// `CardDb::tribes_of` adds it, which reaches every tribe check.
    AlsoTribes(Tribes),
}

/// "Every N events" / "after N events" / "once N events": one per trinket.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    pub event: Event,
    pub every: u8,
    /// Fire once and stop (Bubble Crown's threshold; nothing built yet).
    pub once: bool,
    pub acts: Vec<Act>,
}

// ---- a trinket's effects inside a fight ----

/// The fight effects one side can hold at once: three trinket slots with at
/// most two acts each. `check_invariants` catches a table that outgrows it.
pub const FIGHT_ACTS: usize = 6;
/// A `uses` that is not a cap.
pub const NO_CAP: u8 = u8::MAX;

/// When one of a trinket's fight effects fires. A trinket has no host
/// minion, so these are the side's own hook points in `fight.rs`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum When {
    /// Start of Combat, in the trinket step (the lesser trinket, then the
    /// greater one), before any minion's own.
    Soc,
    /// Avenge (`every`): after every `every` friendly deaths this fight.
    Avenge,
    /// Whenever you summon a minion.
    Summon,
    /// Whenever a friendly minion attacks, once per swing (a Windfury swing
    /// counts twice).
    Attack,
    /// Whenever a friendly minion dies.
    Death,
    /// After a friendly minion is Reborn.
    Reborn,
    /// The side cast a Tavern spell inside the fight (Bewitched Ribbon's
    /// in-combat numbers).
    CastSpell,
    /// After your Deity awakens.
    Awaken,
}

/// What the minion a trigger is about must be (the summoned one, the
/// attacker, the one that died or was Reborn).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Trigger {
    Any,
    Tribe(Tribe),
    /// A minion with no type at all.
    Neutral,
    /// A minion with a Rally.
    Rally,
}

impl Trigger {
    pub fn fits(self, cards: &CardDb, m: &crate::minion::Minion) -> bool {
        match self {
            Trigger::Any => true,
            Trigger::Tribe(t) => cards.tribes_of(m).has(t),
            Trigger::Neutral => cards.tribes_of(m).is_empty(),
            Trigger::Rally => m.parts().any(|(c, _)| cards.get(c).rally.is_some()),
        }
    }
}

/// Which friendly minions a fight effect reaches. Start of Combat and Avenge
/// acts pick here; a trigger act reaches the minion the trigger is about
/// (`Itself`) unless it names its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Pick {
    /// The minion the trigger is about.
    Itself,
    All,
    Tribe(Tribe),
    /// Minions with no type at all.
    Neutral,
    /// The left-most and the right-most of a tribe (of every minion when
    /// `None`); a lone minion gets it once, not twice.
    Ends { tribe: Option<Tribe> },
    /// `count` different random minions of a tribe (of any when `None`).
    Random { tribe: Option<Tribe>, count: u8 },
    /// The `count` minions with the lowest Attack; ties at random.
    LowestAttack(u8),
    /// Minions that have a Rally.
    Rally,
    /// Minions whose card, or a Magnetized part of it, is this one.
    Card(CardId),
}

/// How a fight effect multiplies the stats of the minions it picks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Scale {
    /// Attack and Health, as they stand.
    Double,
    Triple,
    /// Health only.
    Health,
    /// Attack only.
    Attack,
}

/// A Deathrattle a trinket gives minions for the fight (Hoggy Bank, Jarred
/// Frostling, Sky Golem Portrait). It fires when they die, once per Deathrattle
/// like any other.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GrantedDr {
    /// Get `n` Blood Gems in hand.
    Gems(u8),
    /// Summon `count` plain copies of a token where the minion stood.
    Summon { card: CardId, count: u8 },
    /// Give every friendly minion stats they keep after the fight.
    BuffAll { attack: i32, health: i32 },
    /// Give every other friendly minion this minion's Attack and maximum
    /// Health for the fight (Fragrant Phylactery).
    Phylactery,
}

/// One thing a trinket does inside a fight. Every variant with a `Pick` runs
/// on the side it belongs to. `grow` adds the trinket's own "improve this"
/// number to each number the act gives that is not 0, so a +3/+0 stays +0
/// Health however often it improves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FightAct {
    /// Give the picked minions keywords.
    Keywords { pick: Pick, keywords: u8 },
    /// Give the picked minions stats for the fight.
    Stats { pick: Pick, attack: i32, health: i32, grow: i32 },
    /// Multiply the picked minions' stats.
    Scale { pick: Pick, kind: Scale },
    /// Give each picked minion Health equal to half its Attack (rounded up).
    HalfAttackToHealth { pick: Pick },
    /// Fire the Deathrattles of the picked minions, in board order.
    TriggerDeathrattles { pick: Pick },
    /// Give the picked minions a Deathrattle for this fight.
    GrantDr { pick: Pick, dr: GrantedDr },
    /// Give the picked minions the Attack of the highest-Attack minion in
    /// hand.
    HighestHandAttack { pick: Pick },
    /// Play Blood Gems on the picked minions.
    Gems { pick: Pick, count: u8 },
    /// Summon `count` copies of a card at the right end, with keywords.
    Summon { card: CardId, count: u8, keywords: u8 },
    /// Raise your Blood Gem and Tavern spell bonuses for the rest of the
    /// game.
    Bonus { gem: (i32, i32), spell: (i32, i32) },
    /// Get `n` random Tavern spells.
    RandomSpell(u8),
    /// Get `n` random Magnetic minions.
    RandomMagnetic(u8),
    /// Gain `n` free Refreshes.
    FreeRefreshes(u8),
    /// Raise the trinket's own "improve this" number for good.
    Improve(u8),
    /// Get a plain copy (base stats) of the picked minions into hand.
    CopyToHand { pick: Pick },
    /// Give the picked minions stats they keep after the fight (Bewitched
    /// Ribbon: its in-combat numbers are permanent too). A minion summoned
    /// during the fight keeps nothing, like any other fight gain.
    PermanentStats { pick: Pick, attack: i32, health: i32 },
    // ---- block 4 ----
    /// Start of Combat: destroy the `count` left-most minions and bring them
    /// back exactly as they were once this side's last minion dies (Soul
    /// Fermenter).
    Fermenter { count: u8 },
    /// Summon a Golem with the Blood Gem stats of the minion that just died
    /// (Blood Golem Sticker).
    GemGolem { card: CardId },
}

/// One fight effect: when it fires, how often, and what it does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FightEntry {
    pub when: When,
    /// Fire on every Nth event (Avenge N, "after 2 attacks"); 0 and 1 mean
    /// every event.
    pub every: u8,
    /// Fire once per fight and stop.
    pub once: bool,
    /// Uses per fight, `NO_CAP` when the card sets no limit.
    pub uses: u8,
    /// What the minion the trigger is about must be.
    pub trigger: Trigger,
    pub act: FightAct,
}

impl FightEntry {
    /// A plain entry: every event, no limit, any minion.
    pub fn new(when: When, act: FightAct) -> FightEntry {
        FightEntry { when, every: 1, once: false, uses: NO_CAP, trigger: Trigger::Any, act }
    }
}

/// One trinket: its price, its tags and the hooks it listens on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trinket {
    pub name: String,
    pub card_id: String,
    /// Gold paid at the pick; 0 when the game data has no cost (free).
    pub cost: u8,
    pub slot: Slot,
    /// The lobby tribes it belongs to (`lobby_subsets`); empty is neutral.
    pub tribes: Tribes,
    pub mechanic: Option<Mechanic>,
    /// Caps the hook lists per turn ("Twice per turn").
    pub uses_per_turn: Option<u8>,
    /// The start-of-turn list fires only every this many turns ("At the start
    /// of every two turns": Azeroth Model Globe, Lens Case).
    pub start_every: Option<u8>,
    pub on_pick: Vec<Act>,
    pub start_of_turn: Vec<Act>,
    pub end_of_turn: Vec<Act>,
    pub on_buy: Vec<Act>,
    pub buy_tribe: Option<Tribe>,
    pub on_buy_spell: Vec<Act>,
    pub on_play: Vec<Act>,
    pub play_tribe: Option<Tribe>,
    pub on_sell: Vec<Act>,
    pub on_refresh: Vec<Act>,
    /// A Tavern spell cast (Comfy Coffin, Bluegill Flippers).
    pub on_cast_spell: Vec<Act>,
    /// Any spell cast (Bewitched Ribbon, Charming Panpipes).
    pub on_any_spell: Vec<Act>,
    pub on_cast_spell_on_minion: Vec<Act>,
    pub on_discard: Vec<Act>,
    /// Whenever a card of `get_tribe` enters the hand (Trusty Crowbar).
    pub on_get: Vec<Act>,
    pub get_tribe: Option<Tribe>,
    /// Whenever the player spends Gold, once per spending whatever the
    /// amount (docs/rules/design.md, second round): Booty Bay Brew.
    pub on_gold_spent: Vec<Act>,
    // ---- block 4: the events the block added ----
    /// A card of any kind played from the hand (Dragonwing Glider).
    pub on_card_play: Vec<Act>,
    /// A Battlecry triggered, once per trigger (Dragon Skull).
    pub on_battlecry: Vec<Act>,
    /// A Blood Gem cast from hand (Ruby Tusk).
    pub on_gem: Vec<Act>,
    /// A friendly minion attacked in a fight (Dragonhead Staff).
    pub on_attack: Vec<Act>,
    /// A minion destroyed outside combat (Maw Caster Portrait).
    pub on_destroy: Vec<Act>,
    /// A Magnetic minion played (Spell-powered Wrench).
    pub on_magnetize: Vec<Act>,
    /// A spell cast on a Tavern minion (Felsteel Cleaver).
    pub on_cast_spell_on_shop: Vec<Act>,
    /// A friendly minion consumed a Tavern minion (Consuming Claw's
    /// counterpart is a field, not a list).
    pub on_consume: Vec<Act>,
    /// The hero took damage outside combat (Nether Pendant counts it).
    pub on_hero_damage: Vec<Act>,
    /// A Death event only counts when the dead minion's card is this one
    /// (Snarky Portrait: a Fishbait).
    pub death_card: Option<CardId>,
    /// A Destroy event only counts when the destroyed minion's card is this
    /// one (Plaguerunner Portrait).
    pub destroy_card: Option<CardId>,
    /// The two buy lists alternate for the rest of the game: a minion buy,
    /// then a Tavern spell buy, then a minion (Trickster's Sleeve).
    pub alternate: bool,
    /// The end-of-turn list runs once per Drakkari trigger, on top of its
    /// own count (Young Murk-Eye Sticker, Insurrectionist's Blade).
    pub drakkari_end: bool,
    /// Only discard events of a spell (Shath'Yar Shrine).
    pub discard_spells_only: bool,
    /// Only discard events of a minion (Writhing Tentacles).
    pub discard_minions_only: bool,
    pub rule: Option<Rule>,
    /// The Wheel of Yogg-Saron's results, one picked at random by a
    /// `YoggWheel` act (Yogg-Tastic Pastry).
    pub wheel: Vec<Act>,
    /// A trinket that is a Secret (Safety Patch's Ice Block), tagged for
    /// Akazamzarak later. Not hidden: everyone sees everyone's trinkets live
    /// (docs/rules/trinkets.md). Nothing reads it yet.
    pub secret: bool,
    /// The effects it has inside a fight, in the order they run.
    pub fight: Vec<FightEntry>,
    /// "Your first Deathrattle each combat triggers an extra time"
    /// (Deathly Phylactery).
    pub first_deathrattle: bool,
    /// Whether the Mystery Cube may offer it. `cube = false` in
    /// trinkets.toml keeps out a one-time effect (whose whole point is spent
    /// when it is picked) and the trinkets that replace themselves on the
    /// pick (Orb of the Unknown, Quartermaster's Hook).
    pub cube: bool,
}

impl Trinket {
    /// The tier this trinket pays out at 15 Gold, if it is that card
    /// (Splinter of Aurum).
    pub fn golden_when_rich(&self) -> Option<u8> {
        self.on_pick
            .iter()
            .chain(self.start_of_turn.iter())
            .find_map(|a| match a {
                Act::GoldenWhenRich(t) => Some(*t),
                _ => None,
            })
    }

    /// The events this trinket listens for, as a bit mask: the player ORs
    /// its trinkets' masks together, so a hook can answer "nothing here"
    /// from one `u32` on the player and never read this table (shared by
    /// every game on every thread) until something matches.
    pub fn event_mask(&self) -> u32 {
        let mut m = 0;
        let mut any = false;
        let list = |on: bool, e: Event, m: &mut u32, any: &mut bool| {
            if on {
                *m |= e.bit();
                *any = true;
            }
        };
        list(!self.on_buy.is_empty(), Event::Buy, &mut m, &mut any);
        list(!self.on_buy_spell.is_empty(), Event::BuySpell, &mut m, &mut any);
        list(!self.on_play.is_empty(), Event::Play, &mut m, &mut any);
        list(!self.on_sell.is_empty(), Event::Sell, &mut m, &mut any);
        list(!self.on_refresh.is_empty(), Event::Refresh, &mut m, &mut any);
        list(!self.on_cast_spell.is_empty(), Event::CastSpell, &mut m, &mut any);
        list(!self.on_any_spell.is_empty(), Event::SpellCast, &mut m, &mut any);
        list(!self.on_cast_spell_on_minion.is_empty(), Event::CastSpellOnMinion, &mut m, &mut any);
        list(!self.on_discard.is_empty(), Event::Discard, &mut m, &mut any);
        list(!self.on_get.is_empty(), Event::GetTribe(Tribe::Beast), &mut m, &mut any);
        list(!self.on_gold_spent.is_empty(), Event::GoldSpent, &mut m, &mut any);
        list(!self.on_card_play.is_empty(), Event::CardPlayed, &mut m, &mut any);
        list(!self.on_battlecry.is_empty(), Event::Battlecry, &mut m, &mut any);
        list(!self.on_gem.is_empty(), Event::Gem, &mut m, &mut any);
        list(!self.on_attack.is_empty(), Event::Attack, &mut m, &mut any);
        list(!self.on_destroy.is_empty(), Event::Destroy, &mut m, &mut any);
        list(!self.on_magnetize.is_empty(), Event::Magnetize, &mut m, &mut any);
        list(!self.on_cast_spell_on_shop.is_empty(), Event::CastSpellOnShop, &mut m, &mut any);
        list(!self.on_consume.is_empty(), Event::Consume, &mut m, &mut any);
        list(!self.on_hero_damage.is_empty(), Event::HeroDamage, &mut m, &mut any);
        // An alternating trinket listens for both buys, whichever step it is
        // on (Trickster's Sleeve).
        if self.alternate {
            m |= Event::Buy.bit() | Event::BuySpell.bit();
            any = true;
        }
        list(!self.start_of_turn.is_empty() || self.rule.as_ref().is_some_and(|r| r.event == Event::Turns), Event::Turns, &mut m, &mut any);
        if let Some(r) = &self.rule {
            list(true, r.event, &mut m, &mut any);
        }
        // A capped hook needs the start of the turn for its uses back, even
        // with no start-of-turn list of its own (Water Wheel).
        if !self.start_of_turn.is_empty() || self.uses_per_turn.is_some() {
            m |= STARTS_TURN;
            any = true;
        }
        if !self.end_of_turn.is_empty() { m |= ENDS_TURN; any = true; }
        if let Some(r) = &self.rule {
            if r.event == Event::Turns { m |= STARTS_TURN | ENDS_TURN; }
            if !r.acts.is_empty() { m |= ENDS_TURN; }
        }
        if !self.on_pick.is_empty() { any = true; }
        if !self.fight.is_empty() || self.first_deathrattle { any = true; }
        if any { m | HAS_TRINKET } else { m }
    }

    /// Whether any of the lists above can fire, so the game knows whether to
    /// ask the trinket at all.
    pub fn has_event(&self, event: Event) -> bool {
        match event {
            Event::Buy => !self.on_buy.is_empty(),
            Event::BuySpell => !self.on_buy_spell.is_empty(),
            Event::Sell => !self.on_sell.is_empty(),
            Event::Play | Event::PlayTribe(_) => !self.on_play.is_empty(),
            Event::Refresh => !self.on_refresh.is_empty(),
            Event::CastSpell => !self.on_cast_spell.is_empty(),
            Event::SpellCast => !self.on_any_spell.is_empty(),
            Event::CastSpellOnMinion => !self.on_cast_spell_on_minion.is_empty(),
            Event::Discard => !self.on_discard.is_empty(),
            Event::GoldSpent => !self.on_gold_spent.is_empty(),
            Event::Turns => !self.end_of_turn.is_empty() || self.rule.is_some(),
            // The shop-side rules count these; the fight reports them home.
            Event::Death | Event::DeathTribe(_) => {
                self.rule.as_ref().is_some_and(|r| matches!(r.event, Event::Death | Event::DeathTribe(_)))
            }
            Event::Summon | Event::SummonTribe(_) => {
                self.rule.as_ref().is_some_and(|r| matches!(r.event, Event::Summon | Event::SummonTribe(_)))
            }
            Event::GetTribe(_) => !self.on_get.is_empty(),
            Event::CardPlayed => !self.on_card_play.is_empty(),
            Event::Battlecry => !self.on_battlecry.is_empty(),
            Event::PlayBattlecry => {
                self.rule.as_ref().is_some_and(|r| r.event == Event::PlayBattlecry)
            }
            Event::Gem => !self.on_gem.is_empty(),
            Event::Attack => {
                !self.on_attack.is_empty()
                    || self.rule.as_ref().is_some_and(|r| r.event == Event::Attack)
            }
            Event::Destroy => !self.on_destroy.is_empty(),
            Event::Magnetize => !self.on_magnetize.is_empty(),
            Event::CastSpellOnShop => !self.on_cast_spell_on_shop.is_empty(),
            Event::Consume => !self.on_consume.is_empty(),
            Event::HeroDamage => {
                !self.on_hero_damage.is_empty()
                    || self.rule.as_ref().is_some_and(|r| r.event == Event::HeroDamage)
            }
        }
    }
}

/// Every built trinket. Player.trinkets and the offers hold indices into
/// this table plus one (0 is an empty slot).
#[derive(Clone, Debug, Default)]
pub struct TrinketDb {
    pub trinkets: Vec<Trinket>,
    by_card_id: HashMap<String, u8>,
}

impl TrinketDb {
    pub fn len(&self) -> usize {
        self.trinkets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.trinkets.is_empty()
    }

    pub fn get(&self, i: u8) -> &Trinket {
        &self.trinkets[i as usize]
    }

    /// The trinket with this card_id, if it is built.
    pub fn by_card_id(&self, card_id: &str) -> Option<u8> {
        self.by_card_id.get(card_id).copied()
    }

    /// The trinket with this name (tests and the sandbox; the engine matches
    /// by index, never by name).
    pub fn by_name(&self, name: &str) -> Option<u8> {
        self.trinkets.iter().position(|t| t.name == name).map(|i| i as u8)
    }
}

#[derive(Deserialize)]
struct RawFile {
    #[serde(default)]
    trinket: Vec<RawTrinket>,
}

/// A trinket's Dark Gift offer (Ominous Stone, Wax Lance, Sinister
/// Invitation): `tier` left out means the button's tiers for the turn.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDarkGift {
    tier: Option<u8>,
    #[serde(default)]
    yours: bool,
    #[serde(default)]
    golden: bool,
}

#[derive(Deserialize, Default)]
struct RawAct {
    get: Option<String>,
    count: Option<u8>,
    gold: Option<u8>,
    max_gold: Option<u8>,
    upgrade_discount: Option<u8>,
    get_gems: Option<u8>,
    gem_bonus: Option<[i32; 2]>,
    spell_bonus: Option<[i32; 2]>,
    discover_spell: Option<u8>,
    discover_deathrattle: Option<u8>,
    random_chromadrake: Option<u8>,
    random_volumizer: Option<u8>,
    random_bounty: Option<u8>,
    random_choose_one: Option<u8>,
    random_tier_minion: Option<u8>,
    random_minion: Option<String>,
    random_spell: Option<u8>,
    cast: Option<String>,
    times: Option<u8>,
    tavern_buff: Option<[i32; 2]>,
    tribe: Option<String>,
    this_turn: Option<bool>,
    buff_tavern_this_turn: Option<[i32; 2]>,
    buff_leftmost_hand: Option<[i32; 2]>,
    buff_all: Option<[i32; 2]>,
    undead_bonus: Option<[i32; 2]>,
    buff_deity: Option<[i32; 2]>,
    free_refreshes: Option<u8>,
    demons_consume: Option<u8>,
    add_fodder_refreshes: Option<u8>,
    get_lockbox: Option<u8>,
    linked_spells: Option<u8>,
    make_golden_random: Option<u8>,
    last_spell_copies: Option<u8>,
    gems_on_board: Option<u8>,
    health_price_minions: Option<[String; 2]>,
    #[serde(default)]
    health_price_highest_tier: bool,
    every_nth_buy_health: Option<u8>,
    health_price_spell: Option<u8>,
    free_buys: Option<u8>,
    free_tribe_buys: Option<[String; 2]>,
    gold_on_tier6: Option<u8>,
    gold_per_type: Option<u8>,
    gold_per_golden: Option<u8>,
    golden_when_rich: Option<u8>,
    #[serde(default)]
    next_trinket_lesser: bool,
    greater_offer_in: Option<u8>,
    greater_offer_into_lesser: Option<u8>,
    replace_with_random: Option<[String; 2]>,
    grow_value: Option<u8>,
    buff_all_growing: Option<[i32; 2]>,
    buff_leftmost_growing: Option<[i32; 2]>,
    buff_all_per_golden: Option<[i32; 2]>,
    buff_leftmost_two_per_battlecry: Option<[i32; 2]>,
    random_most_common_type: Option<u8>,
    buff_random_other: Option<[i32; 2]>,
    offer_replace: Option<u8>,
    discover_remember: Option<u8>,
    #[serde(default)]
    discover_remember_typed: bool,
    discover_tier: Option<u8>,
    copy_buys: Option<u8>,
    get_remembered: Option<u8>,
    random_battlecry: Option<u8>,
    dark_gift: Option<RawDarkGift>,
    #[serde(default)]
    dark_gift_copies: bool,
    plain_copy_random_friendly: Option<u8>,
    #[serde(default)]
    copy_last_discarded: bool,
    gems_each_type: Option<u8>,
    discover_copy_bonus: Option<u8>,
    buff_leftmost: Option<[i32; 2]>,
    get_golden: Option<String>,
    lockbox_sooner: Option<u8>,
    magnetize_ends: Option<String>,
    one_of: Option<[String; 2]>,
    #[serde(default)]
    warband_whistle: bool,
    #[serde(default)]
    triple_need_two_pirate: bool,
    power_triggers: Option<u8>,
    power_extra_uses: Option<u8>,
    power_gold: Option<u8>,
    #[serde(default)]
    second_power: bool,
    #[serde(default)]
    gauntlet: bool,
    #[serde(default)]
    extra_high_tier: bool,
    #[serde(default)]
    extra_magnetic: bool,
    magnetic_cost: Option<u8>,
    tier6_refreshes: Option<u8>,
    min_tier: Option<u8>,
    #[serde(default)]
    golden_deity: bool,
    discover_tier_stats: Option<[i32; 4]>,
    random_magnetic: Option<u8>,
    first_play_shield: Option<u8>,
    #[serde(default)]
    double_highest_tavern: bool,
    #[serde(default)]
    steal_highest_tier_tavern: bool,
    #[serde(default)]
    end_of_turn_twice: bool,
    first_spell_twice: Option<u8>,
    // ---- block 4 ----
    war_drum: Option<u8>,
    dragon_battlecry: Option<u8>,
    #[serde(default)]
    locket: bool,
    #[serde(default)]
    trigger_battlecries_ends: bool,
    trigger_battlecries_of: Option<String>,
    #[serde(default)]
    trigger_all_rallies: bool,
    buff_ends: Option<[i32; 2]>,
    trigger_deathrattles_leftmost: Option<u8>,
    #[serde(default)]
    flip_alternate: bool,
    #[serde(default)]
    cast_on_leftmost: bool,
    #[serde(default)]
    cast_on_random: bool,
    satellite_of_bought: Option<String>,
    get_with_stats: Option<String>,
    stats: Option<[i32; 2]>,
    per_spell: Option<[i32; 2]>,
    teach_bought_spell: Option<u8>,
    #[serde(default)]
    consume_shop_hit: bool,
    consume_claw: Option<[i32; 2]>,
    #[serde(default)]
    double_value: bool,
    spell_bonus_per_type: Option<[i32; 2]>,
    spell_bonus_this_turn: Option<[i32; 2]>,
    /// A card the act is about (a Battlecry's card, a buff's card, a cast).
    card: Option<String>,
    buff_card_hand_and_board: Option<[i32; 2]>,
    hammer: Option<[i32; 4]>,
    #[serde(default)]
    hammer_discarded: bool,
    /// The card a portrait rewrites, and which rewrite it gets.
    card_of_mod: Option<String>,
    also_attack: Option<i32>,
    #[serde(default)]
    also_neighbors: bool,
    #[serde(default)]
    also_health: bool,
    also_board: Option<String>,
    #[serde(default)]
    choose_one_all: bool,
    #[serde(default)]
    souvenir: bool,
    one_of3: Option<[String; 3]>,
    // ---- block 5 ----
    triple_prize: Option<String>,
    discover_prize: Option<u8>,
    random_prize: Option<u8>,
    #[serde(default)]
    ice_block: bool,
    surprise: Option<u8>,
    #[serde(default)]
    yogg_wheel: bool,
    #[serde(default)]
    curse_of_flesh: bool,
    #[serde(default)]
    devouring_hunger: bool,
    rod_of_roasting: Option<[i32; 2]>,
    #[serde(default)]
    golden_tavern: bool,
    cast_random_spells: Option<u8>,
    mecha_demon: Option<u8>,
    /// The tribes a portrait adds to a card (Bronzebeard Portrait).
    #[serde(default)]
    also_tribes: Vec<String>,
    // ---- block 6 ----
    /// Lock the player's most common type in on this trinket's slot.
    #[serde(default)]
    lock_type: bool,
    /// The next minion of the locked type you buy becomes Golden.
    #[serde(default)]
    golden_bought: bool,
}

#[derive(Deserialize)]
struct RawRule {
    event: String,
    every: u8,
    #[serde(default)]
    once: bool,
    #[serde(default)]
    acts: Vec<RawAct>,
}


#[derive(Deserialize)]
struct RawTrinket {
    name: String,
    card_id: String,
    slot: String,
    #[serde(default)]
    cost: u8,
    #[serde(default)]
    tribes: Vec<String>,
    #[serde(default)]
    mechanic: Option<String>,
    #[serde(default)]
    uses_per_turn: Option<u8>,
    #[serde(default)]
    start_every: Option<u8>,
    #[serde(default)]
    on_pick: Vec<RawAct>,
    #[serde(default)]
    start_of_turn: Vec<RawAct>,
    #[serde(default)]
    end_of_turn: Vec<RawAct>,
    #[serde(default)]
    on_buy: Vec<RawAct>,
    #[serde(default)]
    buy_tribe: Option<String>,
    #[serde(default)]
    on_buy_spell: Vec<RawAct>,
    #[serde(default)]
    on_play: Vec<RawAct>,
    #[serde(default)]
    play_tribe: Option<String>,
    #[serde(default)]
    on_sell: Vec<RawAct>,
    #[serde(default)]
    on_refresh: Vec<RawAct>,
    #[serde(default)]
    on_cast_spell: Vec<RawAct>,
    #[serde(default)]
    on_any_spell: Vec<RawAct>,
    #[serde(default)]
    on_cast_spell_on_minion: Vec<RawAct>,
    #[serde(default)]
    on_discard: Vec<RawAct>,
    #[serde(default)]
    on_get: Vec<RawAct>,
    #[serde(default)]
    get_tribe: Option<String>,
    #[serde(default)]
    on_gold_spent: Vec<RawAct>,
    // ---- block 4 ----
    #[serde(default)]
    on_card_play: Vec<RawAct>,
    #[serde(default)]
    on_battlecry: Vec<RawAct>,
    #[serde(default)]
    on_gem: Vec<RawAct>,
    #[serde(default)]
    on_attack: Vec<RawAct>,
    #[serde(default)]
    on_destroy: Vec<RawAct>,
    #[serde(default)]
    on_magnetize: Vec<RawAct>,
    #[serde(default)]
    on_cast_spell_on_shop: Vec<RawAct>,
    #[serde(default)]
    on_consume: Vec<RawAct>,
    #[serde(default)]
    on_hero_damage: Vec<RawAct>,
    #[serde(default)]
    death_card: Option<String>,
    #[serde(default)]
    destroy_card: Option<String>,
    #[serde(default)]
    alternate: bool,
    #[serde(default)]
    drakkari_end: bool,
    #[serde(default)]
    discard_spells_only: bool,
    #[serde(default)]
    discard_minions_only: bool,
    #[serde(default)]
    rule: Option<RawRule>,
    /// The Wheel of Yogg-Saron's six results, in data order (Yogg-Tastic
    /// Pastry): one picked at random by a `yogg_wheel` act.
    #[serde(default)]
    wheel: Vec<RawAct>,
    /// A trinket that is a Secret (Safety Patch), tagged for Akazamzarak
    /// later.
    #[serde(default)]
    secret: bool,
    #[serde(default)]
    fight: Vec<RawFightAct>,
    #[serde(default)]
    first_deathrattle: bool,
    /// `cube = false` keeps this trinket out of the Mystery Cube's offers.
    #[serde(default = "yes")]
    cube: bool,
}

/// The default of a `cube` flag: every trinket is offered unless it says not.
fn yes() -> bool {
    true
}

/// One `fight = [{ ... }]` table. Which key carries the payload decides the
/// `FightAct`; `when` says where it fires.
#[derive(Deserialize, Default)]
struct RawFightAct {
    when: String,
    #[serde(default)]
    every: Option<u8>,
    #[serde(default)]
    once: bool,
    #[serde(default)]
    cap: Option<u8>,
    /// The trigger's filter (a trigger act) or the pick's tribe (Start of
    /// Combat and Avenge).
    #[serde(default)]
    tribe: Option<String>,
    #[serde(default)]
    neutral: bool,
    #[serde(default)]
    rally: bool,
    /// A pick of one named card ("your Eternal Knights").
    #[serde(default)]
    card: Option<String>,
    /// "all" (the default), "ends", "random" or "lowest_attack".
    #[serde(default)]
    pick: Option<String>,
    #[serde(default)]
    count: Option<u8>,
    #[serde(default)]
    keywords: Vec<String>,
    #[serde(default)]
    buff: Option<[i32; 2]>,
    #[serde(default)]
    grow: Option<i32>,
    #[serde(default)]
    scale: Option<String>,
    #[serde(default)]
    half_attack_health: bool,
    #[serde(default)]
    trigger_deathrattles: bool,
    #[serde(default)]
    dr: Option<RawDr>,
    #[serde(default)]
    hand_attack: bool,
    #[serde(default)]
    gems: Option<RawGems>,
    #[serde(default)]
    summon: Option<String>,
    #[serde(default)]
    gem_bonus: Option<[i32; 2]>,
    #[serde(default)]
    spell_bonus: Option<[i32; 2]>,
    #[serde(default)]
    random_spell: Option<u8>,
    #[serde(default)]
    random_magnetic: Option<u8>,
    #[serde(default)]
    free_refreshes: Option<u8>,
    #[serde(default)]
    improve: Option<u8>,
    #[serde(default)]
    copy: bool,
    // ---- block 4 ----
    /// A `buff` the minions keep after the fight (Bewitched Ribbon's
    /// in-combat numbers).
    #[serde(default)]
    permanent: bool,
    fermenter: Option<u8>,
    gem_golem: Option<String>,
}

/// The Deathrattle a `dr = { ... }` table gives.
#[derive(Deserialize, Default)]
struct RawDr {
    #[serde(default)]
    gems: Option<u8>,
    #[serde(default)]
    summon: Option<String>,
    #[serde(default)]
    count: Option<u8>,
    #[serde(default)]
    buff_all: Option<[i32; 2]>,
}

#[derive(Deserialize, Default)]
struct RawGems {
    #[serde(default)]
    count: u8,
    #[serde(default)]
    tribe: Option<String>,
}

fn tribe(name: &str) -> Result<Tribe, String> {
    crate::cards::tribe(name.trim().to_lowercase().as_str())
}

fn keywords(names: &[String]) -> Result<u8, String> {
    let mut mask = 0u8;
    for n in names {
        let bit = Keywords::ALL
            .iter()
            .find(|(_, k)| k == n)
            .map(|(b, _)| *b)
            .ok_or_else(|| format!("unknown keyword {n:?}"))?;
        mask |= bit;
    }
    Ok(mask)
}

impl TrinketDb {
    /// Parse trinkets.toml. `cards` resolves the `get` and `cast` names, so a
    /// misspelt card is a load error, not a game that silently does nothing.
    pub fn from_toml(s: &str, cards: &CardDb) -> Result<TrinketDb, String> {
        let raw: RawFile = toml::from_str(s).map_err(|e| format!("trinkets.toml: {e}"))?;
        let mut db = TrinketDb::default();
        for t in raw.trinket {
            let name = t.name.clone();
            let err = |e: String| format!("trinkets.toml {name}: {e}");
            let card = |n: &str| -> Result<CardId, String> {
                cards.by_name(n).ok_or_else(|| err(format!("unknown card {n:?}")))
            };
            let tribe_ = |n: &str| tribe(n);
            let act = |a: &RawAct| -> Result<Act, String> {
                Ok(if let Some(n) = &a.get {
                    Act::Get { card: card(n)?, count: a.count.unwrap_or(1) }
                } else if let Some(g) = a.gold {
                    Act::Gold(g)
                } else if let Some(g) = a.max_gold {
                    Act::MaxGold(g)
                } else if let Some(d) = a.upgrade_discount {
                    Act::UpgradeDiscount(d)
                } else if let Some(n) = a.get_gems {
                    Act::Gems(n)
                } else if let Some([attack, health]) = a.gem_bonus {
                    Act::GemBonus { attack, health }
                } else if let Some([attack, health]) = a.spell_bonus {
                    Act::SpellBonus { attack, health }
                } else if let Some(n) = a.discover_spell {
                    Act::DiscoverSpell(n)
                } else if let Some(n) = a.discover_deathrattle {
                    Act::DiscoverDeathrattle(n)
                } else if let Some(n) = a.random_chromadrake {
                    Act::RandomChromadrake(n)
                } else if let Some(n) = a.random_volumizer {
                    Act::RandomVolumizer(n)
                } else if let Some(n) = a.random_bounty {
                    Act::RandomBounty(n)
                } else if let Some(n) = a.random_choose_one {
                    Act::RandomChooseOne(n)
                } else if let Some(tier) = a.random_tier_minion {
                    Act::RandomTierMinion { tier, count: a.count.unwrap_or(1) }
                } else if let Some(t) = &a.random_minion {
                    Act::RandomMinion { tribe: tribe(t).map_err(err)?, count: a.count.unwrap_or(1) }
                } else if let Some(n) = a.random_spell {
                    Act::RandomSpell(n)
                } else if let Some(n) = &a.cast {
                    Act::Cast { card: card(n)?, times: a.times.unwrap_or(1) }
                } else if let Some([attack, health]) = a.tavern_buff {
                    Act::TavernBuff {
                        tribe: a.tribe.as_deref().map(tribe).transpose().map_err(err)?,
                        attack,
                        health,
                        this_turn: a.this_turn.unwrap_or(false),
                    }
                } else if let Some([attack, health]) = a.buff_tavern_this_turn {
                    Act::BuffTavernThisTurn { attack, health }
                } else if let Some([attack, health]) = a.buff_leftmost_hand {
                    Act::BuffLeftmostHand { attack, health }
                } else if let Some([attack, health]) = a.buff_all {
                    Act::BuffAll {
                        tribe: a.tribe.as_deref().map(tribe).transpose().map_err(err)?,
                        attack,
                        health,
                    }
                } else if let Some([attack, health]) = a.undead_bonus {
                    Act::UndeadBonus { attack, health }
                } else if let Some([attack, health]) = a.buff_deity {
                    Act::BuffDeity { attack, health }
                } else if let Some(n) = a.free_refreshes {
                    Act::FreeRefreshes(n)
                } else if let Some(n) = a.demons_consume {
                    Act::DemonsConsume(n)
                } else if let Some(n) = a.add_fodder_refreshes {
                    Act::AddFodderRefreshes(n)
                } else if let Some(n) = a.get_lockbox {
                    Act::GetLockbox(n)
                } else if let Some(n) = a.linked_spells {
                    Act::LinkedSpells(n)
                } else if let Some(tier) = a.make_golden_random {
                    Act::MakeGoldenRandom(tier)
                } else if let Some(n) = a.last_spell_copies {
                    Act::LastSpellCopies(n)
                } else if let Some(n) = a.gems_on_board {
                    Act::GemsOnBoard(n)
                } else if let Some([tribe, count]) = &a.health_price_minions {
                    let count = count.parse().map_err(|_| err("health_price_minions count".into()))?;
                    let tribe = if tribe.is_empty() { None } else { Some(tribe_(tribe).map_err(err)?) };
                    Act::HealthPriceMinions { tribe, count }
                } else if a.health_price_highest_tier {
                    Act::HealthPriceHighestTier
                } else if let Some(n) = a.every_nth_buy_health {
                    Act::EveryNthBuyHealth(n)
                } else if let Some(n) = a.health_price_spell {
                    Act::HealthPriceSpell(n)
                } else if let Some(n) = a.free_buys {
                    Act::FreeBuys(n)
                } else if let Some([tribe, count]) = &a.free_tribe_buys {
                    let count = count.parse().map_err(|_| err("free_tribe_buys count".into()))?;
                    Act::FreeTribeBuys { tribe: tribe_(tribe).map_err(err)?, count }
                } else if let Some(n) = a.gold_on_tier6 {
                    Act::GoldOnTier6(n)
                } else if let Some(n) = a.gold_per_type {
                    Act::GoldPerType(n)
                } else if let Some(n) = a.gold_per_golden {
                    Act::GoldPerGolden(n)
                } else if let Some(tier) = a.golden_when_rich {
                    Act::GoldenWhenRich(tier)
                } else if a.next_trinket_lesser {
                    Act::NextTrinketLesser
                } else if let Some(n) = a.greater_offer_in {
                    Act::GreaterOfferOnTurn(n as u32)
                } else if let Some(n) = a.greater_offer_into_lesser {
                    Act::GreaterOfferIntoLesser(n)
                } else if let Some(n) = a.grow_value {
                    Act::GrowValue(n)
                } else if let Some([attack, health]) = a.buff_all_growing {
                    Act::BuffAllGrowing { tribe: None, attack, health }
                } else if let Some([attack, health]) = a.buff_leftmost_growing {
                    Act::BuffLeftmostGrowing { attack, health }
                } else if let Some([attack, health]) = a.buff_all_per_golden {
                    Act::BuffAllPerGolden { attack, health }
                } else if let Some([attack, health]) = a.buff_leftmost_two_per_battlecry {
                    Act::BuffLeftmostTwoPerBattlecry { attack, health }
                } else if let Some([attack, health]) = a.buff_random_other {
                    Act::BuffRandomOther {
                        tribe: a.tribe.as_deref().map(tribe).transpose().map_err(err)?,
                        attack,
                        health,
                        count: a.count.unwrap_or(1),
                    }
                } else if let Some(n) = a.offer_replace {
                    Act::OfferReplace(n)
                } else if let Some(n) = a.random_most_common_type {
                    Act::RandomMostCommonType(n)
                } else if let Some(tier) = a.discover_remember {
                    Act::DiscoverRemember { tier, typed: a.discover_remember_typed }
                } else if let Some(tier) = a.discover_tier {
                    Act::DiscoverTier(tier)
                } else if let Some(n) = a.copy_buys {
                    Act::CopyBuys(n)
                } else if let Some(n) = a.get_remembered {
                    Act::GetRemembered(n)
                } else if let Some(g) = &a.dark_gift {
                    Act::DarkGift {
                        tier: g.tier,
                        yours: g.yours,
                        golden: g.golden,
                        use_table: g.tier.is_none(),
                    }
                } else if a.dark_gift_copies {
                    Act::DarkGiftCopies
                } else if let Some(n) = a.random_battlecry {
                    Act::RandomBattlecry(n)
                } else if let Some(n) = a.plain_copy_random_friendly {
                    Act::PlainCopyRandomFriendly(n)
                } else if a.copy_last_discarded {
                    Act::CopyLastDiscarded
                } else if let Some(n) = a.gems_each_type {
                    Act::GemsEachType(n)
                } else if let Some(n) = a.discover_copy_bonus {
                    Act::DiscoverCopyBonus(n)
                } else if let Some([attack, health]) = a.buff_leftmost {
                    Act::BuffLeftmostBoard { attack, health }
                } else if let Some(n) = &a.get_golden {
                    Act::GetGolden { card: card(n)? }
                } else if let Some(n) = a.lockbox_sooner {
                    Act::LockboxSooner(n)
                } else if let Some(n) = &a.magnetize_ends {
                    Act::MagnetizeEnds {
                        card: card(n)?,
                        tribe: a.tribe.as_deref().map(tribe).transpose().map_err(err)?,
                    }
                } else if let Some([a1, b1]) = &a.one_of {
                    Act::OneOf { a: card(a1)?, b: card(b1)? }
                } else if a.warband_whistle {
                    Act::WarbandWhistle
                } else if let Some(n) = a.power_triggers {
                    Act::PowerTriggers(n)
                } else if let Some(n) = a.power_extra_uses {
                    Act::PowerExtraUses(n)
                } else if let Some(n) = a.power_gold {
                    Act::PowerGold(n)
                } else if a.second_power {
                    Act::SecondPower
                } else if a.gauntlet {
                    Act::Gauntlet
                } else if a.extra_high_tier {
                    Act::ExtraHighTier
                } else if a.extra_magnetic {
                    Act::ExtraMagnetic
                } else if let Some(n) = a.magnetic_cost {
                    Act::MagneticCost(n)
                } else if let Some(n) = a.tier6_refreshes {
                    Act::Tier6Refreshes(n)
                } else if let Some(n) = a.min_tier {
                    Act::MinTier(n)
                } else if a.golden_deity {
                    Act::GoldenDeity
                } else if let Some([tier, count, attack, health]) = a.discover_tier_stats {
                    Act::DiscoverTierStats {
                        tier: tier.clamp(0, 255) as u8,
                        count: count.clamp(0, 255) as u8,
                        attack,
                        health,
                    }
                } else if let Some(n) = a.random_magnetic {
                    Act::RandomMagnetic(n)
                } else if let Some(n) = a.first_play_shield {
                    Act::FirstPlayShield(n)
                } else if a.double_highest_tavern {
                    Act::DoubleHighestTavern
                } else if a.steal_highest_tier_tavern {
                    Act::StealHighestTierTavern
                } else if a.end_of_turn_twice {
                    Act::EndOfTurnTwice
                } else if let Some(n) = a.first_spell_twice {
                    Act::FirstSpellTwice(n)
                } else if let Some(n) = a.war_drum {
                    Act::WarDrum(n)
                } else if let Some(n) = a.dragon_battlecry {
                    Act::DragonBattlecry(n)
                } else if a.locket {
                    Act::Locket
                } else if a.trigger_battlecries_ends {
                    Act::TriggerBattlecriesEnds
                } else if let Some(n) = &a.trigger_battlecries_of {
                    Act::TriggerBattlecriesOf(card(n)?)
                } else if a.trigger_all_rallies {
                    Act::TriggerAllRallies
                } else if let Some([attack, health]) = a.buff_ends {
                    Act::BuffEnds { attack, health }
                } else if let Some(n) = a.trigger_deathrattles_leftmost {
                    Act::TriggerDeathrattlesLeftmost(n)
                } else if a.flip_alternate {
                    Act::FlipAlternate
                } else if a.cast_on_leftmost || a.cast_on_random {
                    let name = a.card.as_deref().ok_or_else(|| err("a cast needs a card".into()))?;
                    let t = a.tribe.as_deref().ok_or_else(|| err("a cast needs a tribe".into()))?;
                    let card = card(name)?;
                    let tribe = tribe_(t).map_err(err)?;
                    if a.cast_on_leftmost {
                        Act::CastOnLeftmost { card, tribe }
                    } else {
                        Act::CastOnRandom { card, tribe }
                    }
                } else if let Some(n) = &a.satellite_of_bought {
                    Act::SatelliteOfBought(card(n)?)
                } else if let Some(n) = &a.get_with_stats {
                    let [attack, health] = a.stats.unwrap_or([0, 0]);
                    Act::GetWithStats {
                        card: card(n)?,
                        attack,
                        health,
                        per_spell: a.per_spell.unwrap_or([0, 0]).into(),
                    }
                } else if let Some(n) = a.teach_bought_spell {
                    Act::TeachBoughtSpell(n)
                } else if a.consume_shop_hit {
                    Act::ConsumeShopHit
                } else if let Some([attack, health]) = a.consume_claw {
                    Act::ConsumeClaw { attack, health }
                } else if a.double_value {
                    Act::DoubleValue
                } else if let Some([attack, health]) = a.spell_bonus_per_type {
                    Act::SpellBonusPerType { attack, health }
                } else if let Some([attack, health]) = a.spell_bonus_this_turn {
                    Act::SpellBonusThisTurn { attack, health }
                } else if let Some([attack, health]) = a.buff_card_hand_and_board {
                    let name = a.card.as_deref().ok_or_else(|| err("a card buff needs a card".into()))?;
                    Act::BuffCardHandAndBoard { card: card(name)?, attack, health }
                } else if let Some([ba, bh, pa, ph]) = a.hammer {
                    Act::Hammer { base: (ba, bh), per: (pa, ph) }
                } else if a.hammer_discarded {
                    Act::HammerDiscarded
                } else if let Some(name) = &a.card_of_mod {
                    let card = card(name)?;
                    let kind = if let Some(n) = a.also_attack {
                        CardMod::AlsoAttack(n)
                    } else if a.also_neighbors {
                        CardMod::AlsoNeighbors
                    } else if a.also_health {
                        CardMod::AlsoHealth
                    } else if let Some(t) = &a.also_board {
                        CardMod::AlsoBoard(tribe_(t).map_err(err)?)
                    } else if !a.also_tribes.is_empty() {
                        let mut ts = Tribes::NONE;
                        for n in &a.also_tribes {
                            ts = ts.with(tribe_(n).map_err(err)?);
                        }
                        CardMod::AlsoTribes(ts)
                    } else {
                        return Err(err("a card mod with no rewrite".into()));
                    };
                    Act::CardMod { card, kind }
                } else if a.choose_one_all {
                    Act::ChooseOneAll
                } else if a.souvenir {
                    Act::Souvenir
                } else if let Some([x, y, z]) = &a.one_of3 {
                    Act::OneOf3 { a: card(x)?, b: card(y)?, c: card(z)? }
                } else if let Some(n) = &a.triple_prize {
                    Act::TriplePrize(card(n)?)
                } else if let Some(tier) = a.discover_prize {
                    Act::DiscoverPrize(tier)
                } else if let Some(n) = a.random_prize {
                    Act::RandomPrize(n)
                } else if a.ice_block {
                    Act::IceBlock
                } else if let Some(pct) = a.surprise {
                    Act::Surprise(pct)
                } else if a.yogg_wheel {
                    Act::YoggWheel
                } else if a.curse_of_flesh {
                    Act::CurseOfFlesh
                } else if a.devouring_hunger {
                    Act::DevouringHunger
                } else if let Some([attack, health]) = a.rod_of_roasting {
                    Act::RodOfRoasting { attack, health }
                } else if a.golden_tavern {
                    Act::GoldenTavern
                } else if let Some(n) = a.cast_random_spells {
                    Act::CastRandomSpells(n)
                } else if let Some(n) = a.mecha_demon {
                    Act::MechaDemon(n)
                } else if a.lock_type {
                    Act::LockType
                } else if a.golden_bought {
                    Act::GoldenBoughtType
                } else if a.triple_need_two_pirate {
                    Act::TripleNeedTwoPirate
                } else if let Some([slot, tribe]) = &a.replace_with_random {
                    let slot = match slot.as_str() {
                        "lesser" => Slot::Lesser,
                        "greater" => Slot::Greater,
                        other => return Err(err(format!("unknown slot {other:?}"))),
                    };
                    let tribe = if tribe.is_empty() { None } else { Some(tribe_(tribe).map_err(err)?) };
                    Act::ReplaceWithRandom { slot, tribe }
                } else {
                    return Err(err(
                        "an act table with no known key (see trinkets.rs `Act`)".into(),
                    ));
                })
            };
            let acts = |v: &[RawAct]| -> Result<Vec<Act>, String> {
                v.iter().map(act).collect()
            };
            // One `fight = [{ ... }]` table: `when` plus whichever payload key
            // it carries. A trigger act reaches the minion the trigger is
            // about unless its payload names its own pick.
            let fight_entry = |a: &RawFightAct| -> Result<FightEntry, String> {
                let when = match a.when.as_str() {
                    "soc" => When::Soc,
                    "avenge" => When::Avenge,
                    "summon" => When::Summon,
                    "attack" => When::Attack,
                    "death" => When::Death,
                    "reborn" => When::Reborn,
                    "awaken" => When::Awaken,
                    "cast" => When::CastSpell,
                    other => return Err(err(format!("unknown fight when {other:?}"))),
                };
                let tribe_ = a.tribe.as_deref().map(tribe).transpose().map_err(err)?;
                // Start of Combat and Avenge pick minions; every other
                // trigger reaches the one it is about.
                let pick = match when {
                    When::Soc | When::Avenge | When::CastSpell => match a.pick.as_deref() {
                        // "your Eternal Knights": one named card.
                        None if a.card.is_some() => Pick::Card(card(a.card.as_deref().unwrap())?),
                        None | Some("all") => match (tribe_, a.neutral) {
                            (Some(t), _) => Pick::Tribe(t),
                            (None, true) => Pick::Neutral,
                            (None, false) => Pick::All,
                        },
                        Some("ends") => Pick::Ends { tribe: tribe_ },
                        Some("random") => Pick::Random { tribe: tribe_, count: a.count.unwrap_or(1) },
                        Some("lowest_attack") => Pick::LowestAttack(a.count.unwrap_or(1)),
                        Some("rally") => Pick::Rally,
                        Some("card") => match &a.card {
                            Some(n) => Pick::Card(card(n)?),
                            None => return Err(err("a card pick needs a card".into())),
                        },
                        Some(other) => return Err(err(format!("unknown pick {other:?}"))),
                    },
                    _ => match &a.card {
                        Some(n) => Pick::Card(card(n)?),
                        None => Pick::Itself,
                    },
                };
                let trigger = match when {
                    When::Soc | When::Avenge | When::CastSpell => Trigger::Any,
                    _ => match (tribe_, a.neutral, a.rally) {
                        (Some(t), _, _) => Trigger::Tribe(t),
                        (None, true, _) => Trigger::Neutral,
                        (None, false, true) => Trigger::Rally,
                        (None, false, false) => Trigger::Any,
                    },
                };
                let act = if let Some(n) = &a.summon {
                    FightAct::Summon {
                        card: card(n)?,
                        count: a.count.unwrap_or(1),
                        keywords: keywords(&a.keywords).map_err(err)?,
                    }
                } else if !a.keywords.is_empty() {
                    FightAct::Keywords { pick, keywords: keywords(&a.keywords).map_err(err)? }
                } else if let Some([attack, health]) = a.buff {
                    if a.permanent {
                        FightAct::PermanentStats { pick, attack, health }
                    } else {
                        FightAct::Stats { pick, attack, health, grow: a.grow.unwrap_or(0) }
                    }
                } else if let Some(kind) = &a.scale {
                    let kind = match kind.as_str() {
                        "double" => Scale::Double,
                        "triple" => Scale::Triple,
                        "health" => Scale::Health,
                        "attack" => Scale::Attack,
                        other => return Err(err(format!("unknown scale {other:?}"))),
                    };
                    FightAct::Scale { pick, kind }
                } else if a.half_attack_health {
                    FightAct::HalfAttackToHealth { pick }
                } else if a.trigger_deathrattles {
                    FightAct::TriggerDeathrattles { pick }
                } else if let Some(dr) = &a.dr {
                    let dr = if let Some(n) = dr.gems {
                        GrantedDr::Gems(n)
                    } else if let Some(n) = &dr.summon {
                        GrantedDr::Summon { card: card(n)?, count: dr.count.unwrap_or(1) }
                    } else if let Some([attack, health]) = dr.buff_all {
                        GrantedDr::BuffAll { attack, health }
                    } else {
                        return Err(err("a dr table with no known key".into()));
                    };
                    FightAct::GrantDr { pick, dr }
                } else if a.hand_attack {
                    FightAct::HighestHandAttack { pick }
                } else if let Some(g) = &a.gems {
                    let tribe_ = g.tribe.as_deref().map(tribe).transpose().map_err(err)?;
                    let pick = match tribe_ {
                        Some(t) => Pick::Tribe(t),
                        None => pick,
                    };
                    FightAct::Gems { pick, count: g.count }
                } else if a.gem_bonus.is_some() || a.spell_bonus.is_some() {
                    FightAct::Bonus {
                        gem: a.gem_bonus.unwrap_or([0, 0]).into(),
                        spell: a.spell_bonus.unwrap_or([0, 0]).into(),
                    }
                } else if let Some(n) = a.random_spell {
                    FightAct::RandomSpell(n)
                } else if let Some(n) = a.random_magnetic {
                    FightAct::RandomMagnetic(n)
                } else if let Some(n) = a.free_refreshes {
                    FightAct::FreeRefreshes(n)
                } else if let Some(n) = a.improve {
                    FightAct::Improve(n)
                } else if a.copy {
                    FightAct::CopyToHand { pick }
                } else if let Some(n) = a.fermenter {
                    FightAct::Fermenter { count: n }
                } else if let Some(n) = &a.gem_golem {
                    FightAct::GemGolem { card: card(n)? }
                } else {
                    return Err(err("a fight table with no known key".into()));
                };
                let every = match when {
                    When::Avenge => match a.every {
                        Some(0) | None => return Err(err("Avenge needs every >= 1".into())),
                        Some(n) => n,
                    },
                    _ => a.every.unwrap_or(1),
                };
                Ok(FightEntry {
                    when,
                    every,
                    once: a.once,
                    uses: a.cap.unwrap_or(NO_CAP),
                    trigger,
                    act,
                })
            };
            let slot = match t.slot.as_str() {
                "lesser" => Slot::Lesser,
                "greater" => Slot::Greater,
                other => return Err(err(format!("unknown slot {other:?}"))),
            };
            let mechanic = match t.mechanic.as_deref() {
                None => None,
                Some("battlecry") => Some(Mechanic::Battlecry),
                Some("deathrattle") => Some(Mechanic::Deathrattle),
                Some("spells") => Some(Mechanic::Spells),
                Some(other) => return Err(err(format!("unknown mechanic {other:?}"))),
            };
            let mut tribes = Tribes::NONE;
            for n in &t.tribes {
                // Naga left the game (docs/rules/design.md): a Naga tag is no tag, so a
                // trinket tagged Naga and something else stays offerable with
                // that something else.
                if n.trim().eq_ignore_ascii_case("naga") {
                    continue;
                }
                tribes = tribes.with(tribe(n).map_err(err)?);
            }
            let rule = match &t.rule {
                None => None,
                Some(r) => {
                    let event = match r.event.as_str() {
                        "turns" => Event::Turns,
                        "buy" => Event::Buy,
                        "buy_spell" => Event::BuySpell,
                        "sell" => Event::Sell,
                        "play" => Event::Play,
                        "refresh" => Event::Refresh,
                        "cast_spell" => Event::CastSpell,
                        "spell_cast" => Event::SpellCast,
                        "cast_spell_on_minion" => Event::CastSpellOnMinion,
                        "gold_spent" => Event::GoldSpent,
                        "discard" => Event::Discard,
                        "death" => Event::Death,
                        "summon" => Event::Summon,
                        "play_battlecry" => Event::PlayBattlecry,
                        "gem" => Event::Gem,
                        "attack" => Event::Attack,
                        "destroy" => Event::Destroy,
                        "magnetize" => Event::Magnetize,
                        "cast_spell_on_shop" => Event::CastSpellOnShop,
                        "consume" => Event::Consume,
                        "hero_damage" => Event::HeroDamage,
                        other => {
                            let prefixed = ["play_", "death_", "summon_"]
                                .into_iter()
                                .find_map(|p| other.strip_prefix(p).map(|t| (p, t)));
                            match prefixed {
                                Some(("play_", t)) => Event::PlayTribe(tribe(t).map_err(err)?),
                                Some(("death_", t)) => {
                                    Event::DeathTribe(tribe(t).map_err(err)?)
                                }
                                Some(("summon_", t)) => {
                                    Event::SummonTribe(tribe(t).map_err(err)?)
                                }
                                _ => return Err(err(format!("unknown event {other:?}"))),
                            }
                        }
                    };
                    if r.every == 0 {
                        return Err(err("a rule needs every >= 1".into()));
                    }
                    Some(Rule { event, every: r.every, once: r.once, acts: acts(&r.acts)? })
                }
            };
            let fight = t.fight.iter().map(fight_entry).collect::<Result<Vec<_>, String>>()?;
            if fight.len() > FIGHT_ACTS {
                return Err(err(format!("more than {FIGHT_ACTS} fight effects")));
            }
            let trinket = Trinket {
                name: t.name,
                card_id: t.card_id.clone(),
                cost: t.cost,
                slot,
                tribes,
                mechanic,
                uses_per_turn: t.uses_per_turn,
                start_every: t.start_every,
                on_pick: acts(&t.on_pick)?,
                start_of_turn: acts(&t.start_of_turn)?,
                end_of_turn: acts(&t.end_of_turn)?,
                on_buy: acts(&t.on_buy)?,
                buy_tribe: t.buy_tribe.as_deref().map(tribe).transpose().map_err(err)?,
                on_buy_spell: acts(&t.on_buy_spell)?,
                on_play: acts(&t.on_play)?,
                play_tribe: t.play_tribe.as_deref().map(tribe).transpose().map_err(err)?,
                on_sell: acts(&t.on_sell)?,
                on_refresh: acts(&t.on_refresh)?,
                on_cast_spell: acts(&t.on_cast_spell)?,
                on_any_spell: acts(&t.on_any_spell)?,
                on_cast_spell_on_minion: acts(&t.on_cast_spell_on_minion)?,
                on_discard: acts(&t.on_discard)?,
                on_get: acts(&t.on_get)?,
                get_tribe: t.get_tribe.as_deref().map(tribe).transpose().map_err(err)?,
                on_gold_spent: acts(&t.on_gold_spent)?,
                on_card_play: acts(&t.on_card_play)?,
                on_battlecry: acts(&t.on_battlecry)?,
                on_gem: acts(&t.on_gem)?,
                on_attack: acts(&t.on_attack)?,
                on_destroy: acts(&t.on_destroy)?,
                on_magnetize: acts(&t.on_magnetize)?,
                on_cast_spell_on_shop: acts(&t.on_cast_spell_on_shop)?,
                on_consume: acts(&t.on_consume)?,
                on_hero_damage: acts(&t.on_hero_damage)?,
                death_card: t.death_card.as_deref().map(card).transpose()?,
                destroy_card: t.destroy_card.as_deref().map(card).transpose()?,
                alternate: t.alternate,
                drakkari_end: t.drakkari_end,
                discard_spells_only: t.discard_spells_only,
                discard_minions_only: t.discard_minions_only,
                rule,
                wheel: acts(&t.wheel)?,
                secret: t.secret,
                fight,
                first_deathrattle: t.first_deathrattle,
                cube: t.cube,
            };
            if db.by_card_id.insert(t.card_id, db.trinkets.len() as u8).is_some() {
                return Err(format!("trinkets.toml: {} twice", trinket.name));
            }
            db.trinkets.push(trinket);
        }
        Ok(db)
    }
}
