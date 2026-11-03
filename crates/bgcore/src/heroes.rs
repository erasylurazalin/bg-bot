//! The hero table: heroes, their armor and their powers, parsed from
//! heroes.toml. Effects are data here as they are in cards.toml; the rules
//! code never matches a hero or power by name. See docs/rules/design.md "Heroes".
//!
//! A power is its own entry, so `PowerState` names a power and not a hero: a
//! power can turn into another (Sulfuras, the four Invocations, Galewing's
//! flightpaths, the King of X family), and a hero only points at the power it
//! starts with. `[[hero]]` entries carry their starting power inline;
//! `[[power]]` entries are the ones a hero only reaches later.

use std::collections::HashMap;

use arrayvec::ArrayVec;

use serde::Deserialize;

use crate::cards::{CardDb, Tribe, Tribes};
use crate::minion::{CardId, Keywords};
use crate::trinkets::Slot;

/// What a hero power does. One variant per shape of effect; the numbers live
/// in heroes.toml.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PowerEffect {
    /// Nothing runs anywhere (All Patched Up: its extra Health lands at the
    /// pick, so a power Discovered mid-game does nothing).
    None,
    /// At the start of each of your turns, gain this many free Refreshes
    /// (Clairvoyance).
    FreeRefreshPerTurn(u8),
    /// After you upgrade the Tavern, gain this much Gold (Everbloom).
    GoldAfterUpgrade(u8),
    /// After you buy a minion of `tribe`, gain this much Gold (I'm the Cap'n
    /// Now).
    GoldAfterBuyTribe { tribe: Tribe, gold: u8 },
    /// After you sell a minion, gain this much Gold next turn (Smart
    /// Savings).
    GoldNextTurnAfterSell(u8),
    /// Start of Combat: your left-most minion gains these keywords (Swatting
    /// Insects).
    SocLeftmost { keywords: u8 },
    /// Each press raises your maximum Gold by this much, this turn's Gold
    /// unchanged (Wisdom of Ancients).
    ExtraMaxGold(u8),
    /// Give a friendly minion you pick Divine Shield (Boon of Light).
    GiveDivineShield,
    /// Steal any card in the Tavern (the spell slot included), then your hero
    /// takes this much damage (Graveyard Shift).
    StealCardDamage(i32),
    /// Steal a random minion in the Tavern and double its Health (Brick by
    /// Brick).
    StealRandomDoubleHealth,
    /// Make a friendly minion you pick Golden, where it stands (Gonna Be
    /// Rich!).
    MakeGolden,
    /// Get this many Blood Gems (Bloodbound).
    GetGems(u8),
    /// Discover a minion of `tribe`, up to your tier (Queen of Dragons).
    DiscoverTribe(Tribe),
    /// At the start of the game, Discover a new Hero Power that replaces this
    /// one (Adventure!).
    DiscoverPower,
    /// The next Tavern spell you buy costs this much less (Arcane Knowledge).
    /// The same player field a card uses (`Effect::SpellDiscount`): the
    /// discount is used up by the next spell bought.
    SpellDiscount(u8),
    /// Get a random Tavern spell, up to your tier (Blessing of the Nine
    /// Frogs; `Effect::GetRandomSpellUpTo` with no cost limit).
    GetRandomSpell,
    /// Trigger a friendly minion's Battlecry, chosen by the player
    /// (Snicker-snack; the `TargetKind::TriggerBattlecry` path Kelp Keeper
    /// uses).
    TriggerBattlecry,
    /// Discard a card to get a random minion of `tribe` (Incubate). Opens the
    /// same pending discard as an Activate with `discard = true`.
    DiscardGetRandomMinion { tribe: Tribe },
    /// Get this many random minions from the pool. Playing one discards the
    /// rest of the group (Dark Ritual; the linked cards of Wandering
    /// Willbreaker).
    GetLinkedMinions(u8),
    /// Refresh the Tavern with Tavern spells (Mana Per Minute; the same fill
    /// as Saloon's Finest).
    RefreshWithSpells,
    /// At the start of this power's unlock turn, offer a trinket of this slot;
    /// the pick turns the power into the trinket (Fantastic Treasure, Growing
    /// Collection).
    TrinketOffer(Slot),
    // ---- hero frame 2 (2026-10-31) ----
    /// A press offers this many Hero Power Discovers (King of Duality): the
    /// first pick becomes the SECOND power, the second pick the MAIN one.
    DiscoverPowers(u8),
    /// After this many cards are bought, the power becomes the one named by
    /// `becomes` (BUY, INSECT! becomes Sulfuras). Every bought card counts,
    /// a Tavern spell and a free buy included; a stolen card does not.
    BuyMilestone { count: u8, becomes: u8 },
    /// At the end of your turn, give your left-most and your right-most
    /// minion this; with one minion it gets both (Sulfuras).
    EndOfTurnEnds { attack: i32, health: i32 },
    /// A press offers the powers in this power's `options`: a fixed choice,
    /// not a Discover (Embrace the Elements, Dungar's Gryphon). Never the one
    /// that just completed (Galewing).
    ChooseSubPowers,
    /// Gain this much Gold when the power completes (Ironforge).
    Gold(u8),
    /// Get a random Tavern spell costing this much, when the power completes
    /// (Westfall).
    GetRandomSpellOfCost(u8),
    /// Discover a minion of your Tier when the power completes (Eastern
    /// Plaguelands).
    DiscoverMyTier,
    /// Start of Combat: give this many random friendly minions "Deathrattle:
    /// summon this card" for the fight (Earth Invocation's 1/1 Elemental).
    SocGrantDeathrattle { count: u8, card: CardId },
    /// Start of Combat: multiply your left-most minion's Attack by this many
    /// plus one, buffs included (Fire Invocation: 1 doubles it).
    SocDoubleLeftAttack(u8),
    /// Start of Combat: your right-most minion gains these keywords (Water
    /// Invocation).
    SocRightmost { keywords: u8 },
    /// Start of Combat: deal `damage` to this many random enemy minions, no
    /// one twice unless there are fewer of them (Lightning Invocation).
    SocDamageEnemies { hits: u8, damage: i32 },
    /// At the start of this turn, Discover a minion of each listed Tier, each
    /// locked in hand until the Tavern reaches it (Procrastinate, Expedition
    /// Plans).
    TurnDiscovers { turn: u32, tiers: [u8; 3] },
    /// At the start of your turn, get this many Bananas and give every other
    /// player `others` of them; each is the plain one or, `big_percent` of the
    /// time, the big one (Bananarama).
    Bananarama { mine: u8, others: u8, big_percent: u8, banana: CardId, big: CardId },
    /// At the start of the game, Discover a Tier 5 minion with a Dark Gift,
    /// locked in hand until turn 7 (Void Power). The gifts are picked as if
    /// it were turn 7 (docs/rules/darkgifts.md).
    DarkGiftStart,
    /// Every `turns` turns, Discover a minion with a Dark Gift (Feel
    /// Devastation). The count lives in the power's `progress`.
    DarkGiftEvery { turns: u8 },
    // ---- hero batch 1 (2026-11-01) ----
    /// Give a minion +`attack`/+`health`, and +1 more of each for every
    /// `every` cards bought this game (Sharpen Blades).
    BuffPerBuys { attack: i32, health: i32, every: u8 },
    /// Discover a Magnetic Mech up to your tier (Tinker).
    DiscoverMagnetic,
    /// Get a random minion of `tribe`. Every buy of that tribe takes `gold`
    /// off the next press, and a press uses the discount up (Pirate
    /// Parrrrty!).
    GetTribeCutCost { tribe: Tribe, gold: u8 },
    /// Discover a minion of your own tier; each press costs `more` more for
    /// the rest of the game (Lead Explorer).
    DiscoverMyTierCostsMore { more: u8 },
    /// At the start of your turn, cast a random Tavern spell at a random
    /// target (Puzzle Box).
    CastRandomSpell,
    /// The Tavern's prices: a minion and a Refresh cost this, and upgrading
    /// costs `upgrade_extra` more at every tier (Manastorm).
    Prices { minion: u8, refresh: u8, upgrade_extra: u8 },
    /// After every `every` minions sold, get a random minion of `tribe`
    /// (Gone Fishing).
    RandomTribeAfterSells { every: u8, tribe: Tribe },
    /// After every `every` buys of the kind `of`, get this card (Verdant
    /// Spheres' Tavern Coin, Battle Brand's Brann).
    CardAfterBuys { every: u8, of: BuyKind, card: CardId },
    /// Every press is one dig, and the `digs`th gives a Golden minion up to
    /// your tier (Buried Treasure).
    DigForGolden { digs: u8 },
    /// Remove a friendly minion and Discover one from the Tier below it
    /// (Trash for Treasure).
    DiscoverBelow,
    /// Gain Gold that has grown by 1 for every turn waited (Piggy Bank);
    /// `base` on the first turn.
    GrowingGold { base: u8 },
    /// After another hero dies, Discover a minion from their last warband
    /// (Kel'Thuzad's Kitty).
    KittyDiscover,
    /// Swap a friendly non-Golden minion with a random one in the Tavern
    /// (Swap, Lock, & Shop It).
    SwapWithShop,
    /// After every `every` Elementals played, the upgrade costs `discount`
    /// less, down to 0 (Avalanche).
    UpgradeDiscountAfterElementals { every: u8, discount: u8 },
    /// Find the third copy of a pair of minions, three times a game (Three
    /// Wishes).
    ThreeWishes,
    /// After `every` Tiers' worth of cards bought, get a Triple Reward
    /// (Natural Balance).
    TripleRewardAfterTiers { every: u32 },
    /// Minions in the Tavern have +`attack`/+`health`, one more of each after
    /// every `every` minions bought (For the Horde!).
    TavernBuffAfterBuys { attack: i32, health: i32, every: u8 },
    /// Once a turn, after every `every` minions bought, get a plain copy of
    /// a random one of them (Glaive Ricochet).
    CopyBuyAfterMinions { every: u8 },
    /// Throw a minion in the pot; at `count` thrown, Discover one minion per
    /// thrown type (Stir the Pot).
    StirThePot { count: u8 },
    /// Steal every card in the Tavern, left to right (The Perfect Crime).
    StealAllTavern,
    /// Discover a plain copy of a minion that died last combat (Reclaimed
    /// Souls).
    DiscoverDeadMinion,
    /// At the end of every `turns` turns, get a plain copy of the left-most
    /// card in hand (Upbeat Harmony).
    HandCopyEvery { turns: u8 },
    /// At the start of the game, Discover a Tier 7 minion to get once `gold`
    /// Gold has been spent (Choose Your Champion).
    Champion { gold: u32 },
    /// Roll a six-sided die and gain that much Gold; the power is then locked
    /// for that many turns (Lucky Roll).
    LuckyRoll,
    /// Every third Tavern spell bought costs 0 (Reliquary Research).
    FreeThirdSpell,
    /// Get a copy of the last Tavern spell cast (The Galaxy's Lens).
    CopyLastSpell,
    /// After every `every` friendly attacks, get a Triple Reward (Heroic
    /// Inspiration).
    TripleRewardAfterAttacks { every: u32 },
    // ---- hero batch 2 (2026-11-01) ----
    /// Replace a Tavern minion with a Discover from exactly one Tier higher
    /// (Galakrond's Greed). The old card goes back to the pool and the
    /// Tavern's buffs are lost.
    ReplaceShopHigher,
    /// Give a minion Reborn until the next turn, a Tavern minion included
    /// (Reborn Rites).
    TempReborn,
    /// Minions cost `price`, the Tavern offers one fewer minion, and it
    /// Freezes at the end of every turn (Stay Frosty).
    StayFrosty { price: u8 },
    /// A Refresh that also holds `count` minions from a Tier above yours
    /// (Temporal Tavern).
    RefreshHighTier { count: u8 },
    /// Pressed, at the end of the turn give a friendly minion
    /// +`attack`/+`health`, one more repeat every turn, all game (Saturday
    /// C'Thuns!).
    SaturdayCthun { attack: i32, health: i32 },
    /// Every minion you play gets a +`attack`/+`health` hat that moves to a
    /// friendly minion when it is sold (Hat Trick).
    HatTrick { attack: i32, health: i32 },
    /// Refresh the Tavern with plain copies of the warband you last fought
    /// (Bob's Burgles).
    BurgleLastOpponent,
    /// The Tavern offers one more minion, a Dragon, on every Refresh (Dream
    /// Portal).
    ExtraDragon,
    /// Replace a card, the Tavern's or your own, with a random one of the
    /// same Tier; a Tavern card keeps its buffs (Arcane Alteration).
    ReplaceSameTier,
    /// Lock a Tavern card in hand for `turns` turns, never bought (Imprison).
    Imprison { turns: u8 },
    /// Tavern spells give an extra +`attack`/+`health`, one more of each every
    /// three turns (Tavern Lighting).
    TavernLighting { attack: i32, health: i32 },
    /// Darkmoon Tickets sit on Tavern cards, `percent` of a chance each card;
    /// `count` of them Discover a minion of your Tier (Come One, Come All!).
    DarkmoonTickets { percent: u8, count: u8 },
    /// Steal a Tavern minion into hand as `attack`/`health` (See the Light).
    StealShopAsStats { attack: i32, health: i32 },
    /// Two minions swap Attack until the next turn (Spirit Swap).
    SpiritSwap,
    /// Sell a friendly minion and give its stats to another (Devour).
    Devour,
    /// Discover a plain copy of a minion from the next opponent's warband as
    /// it was at the start of the turn (I Spy).
    IspyNextOpponent,
    /// After a Refresh, copy the Tavern's highest-Tier minion and Freeze both
    /// cards (Twice as Nice).
    TwiceAsNice,
    /// After a Refresh, give a random Tavern minion a random Bonus Keyword,
    /// `times` times (Enhancification).
    Enhancification { times: u8 },
    /// Destroy a friendly minion of `tribe` and get a random one of it (Rune
    /// of Damnation).
    DestroyTribeGetRandom { tribe: Tribe },
    /// Give a minion Attack (or Health) equal to your Tier, twice a turn; the
    /// power is the other one of the two next turn (Major and Minor Hymn).
    Hymn { attack: bool },
    /// Summon an exact copy of a friendly minion (Cloning Gallery).
    CloningGallery,
    /// Two copies of a minion make it Golden, and they give `coin` (a Tavern
    /// Coin) instead of a Triple Reward (Double Time).
    DoubleTime { coin: CardId },
    // ---- hero batch 3 (2026-11-01) ----
    /// Start of Combat: left- and right-most minions gain +2/+1 and attack
    /// immediately (Wingmen).
    Wingmen,
    /// Start of Combat: give a friendly minion of each type +1/+1, +1/+1 more
    /// per 10 Gold spent this game (Wax Warband).
    WaxWarband,
    /// Start the game with a token minion on the board (Menagerist, Avatar of
    /// N'Zoth, Pilot the Shredder).
    StartMinion { card: CardId },
    /// After friendly minions deal `damage` damage, becomes `becomes`, index
    /// into `HeroDb::powers` (Bloodfury -> Nether Portal).
    DamageMilestone { damage: u32, becomes: u8 },
    /// At the start of each turn, get `count` random minions of `tribe`
    /// (Nether Portal).
    RandomMinionsEachTurn { tribe: Tribe, count: u8 },
    /// Next combat, get a plain copy of the first enemy minion you kill (I'll
    /// Take That!).
    TakeFirstKilled,
    /// Start of Combat: give ALL minions +2 Attack permanently (ALL Will
    /// Burn!).
    AllWillBurn,
    /// After `count` friendly minions attack, becomes `becomes`, index into
    /// `HeroDb::powers` (Demon Hunter Training -> Spectral Sight).
    AttackMilestone { count: u8, becomes: u8 },
    /// The first minion you buy each turn is free (Spectral Sight).
    FreeFirstMinion,
    /// Guess which player will win their next combat: 3 Tavern Coins (`coin`)
    /// on a win, 1 Gold next turn on a tie (Friendly Wager).
    FriendlyWager { coin: CardId },
    /// Start of Combat: summon a random minion of your Tier for this combat
    /// and add a copy of it to your hand (Embrace Your Rage).
    EmbraceYourRage,
    /// Give minions you summon during combat +1/+2 and Taunt (Sprout It Out!).
    SproutItOut,
    /// After a friendly minion kills an enemy by attacking, give it +1 Attack
    /// permanently (Glory of Combat).
    GloryOfCombat,
    /// At the start of every turn, choose from 2 new Hero Powers (Power of the
    /// Storm).
    PowerOfTheStorm,
    /// Give random friendly minions stats, upgraded at the start of each turn
    /// by choosing a stance (Conviction).
    Conviction,
    /// Upgrade Conviction's target count, Attack or Health (Offensive,
    /// Defensive and United Stance).
    Stance { count: u8, attack: i32, health: i32 },
    /// Start of Combat: give your lowest-Attack minion a Deathrattle that
    /// gives your other minions its stats (Fragrant Phylactery).
    FragrantPhylactery,
    /// Steal a minion from the Tavern; when you have space next combat, summon
    /// it and it attacks immediately (Lock and Load).
    LockAndLoad,
    /// When you have space in combat, summon an exact copy of your
    /// highest-Attack (or highest-Health) minion (Frostwolf Fervor, Stormpike
    /// Strength).
    CopyHighestWhenSpace { by_health: bool },
    /// After `every` friendly minions die, get a random Mech (MechGyver).
    MechGyver { every: u8 },
    /// Avenge (4): summon `card` (Onyxian Whelp) that attacks immediately, and
    /// improve its stats by +1/+1 permanently (Broodmother).
    Broodmother { card: CardId },
    /// Look at 2 minions; guess which one your next opponent had last combat
    /// for `coin` (Detective for Hire).
    DetectiveForHire { coin: CardId },
    /// When you have space in combat, summon `card` (Ozumat's Tentacle) with
    /// +1/+1 per minion sold this game (Tentacular).
    Tentacular { card: CardId },
    /// Choose a friendly minion; Start of Combat: destroy it, and once you
    /// have space resummon an exact copy (Rapid Reanimation).
    RapidReanimation,
    // ---- trinkets block 5 (2026-11-01) ----
    /// Prize Wall: on turn `every` (4), twice that, and so on, Discover a
    /// Darkmoon Prize of the tier the turn names: turn 4 is Tier 1, turn 8
    /// Tier 2, turn 12 Tier 3, turn 16 Tier 4, and Tier 4 every four turns
    /// after (a guess past 16, docs/rules/trinkets.md).
    PrizeWall { every: u8 },
    // ---- secrets (2026-11-01) ----
    /// Prestidigitation: offer every combat Secret the player does not hold
    /// (one to four cards) and put the pick in the Secret zone. With all four
    /// up the power cannot be pressed (docs/rules/heroes.md "Secrets").
    DiscoverSecret,
}

/// What a power's buy counter counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BuyKind {
    /// Any card, a Tavern spell included.
    Cards,
    /// Minions only.
    Minions,
    /// Minions with a Battlecry.
    Battlecry,
}

impl PowerEffect {
    /// Whether the power's fight effect only runs when the power was pressed
    /// in the same turn (the fight reads the press as `PowerState.armed`):
    /// I'll Take That!, Embrace Your Rage and Rapid Reanimation.
    pub fn fires_only_when_pressed(self) -> bool {
        matches!(
            self,
            PowerEffect::TakeFirstKilled
                | PowerEffect::EmbraceYourRage
                | PowerEffect::RapidReanimation
        )
    }
}

/// A hero power: its button's limits and its effect.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct HeroPower {
    pub name: String,
    pub card_id: String,
    /// Gold a press costs.
    pub cost: u8,
    /// No button: the effect hooks into the game instead.
    pub passive: bool,
    /// Presses per turn ("Twice per turn").
    pub uses_per_turn: u8,
    /// Presses a game: 1 for "Once per game", 3 for Three Wishes, 0 for no
    /// limit. The old `once_per_game` key in heroes.toml sets it to 1.
    pub uses_per_game: u8,
    /// Unlocks at this Tavern tier, 0 for no lock.
    pub unlock_tier: u8,
    /// Unlocks at this turn, 0 for no lock.
    pub unlock_turn: u32,
    pub effect: PowerEffect,
    /// The power this one goes back to when it completes, index into
    /// `HeroDb::powers` (Bru'kan's Invocations and Galewing's flightpaths go
    /// back to their chooser). None for a power that stays.
    pub parent: Option<u8>,
    /// Turns this power takes to complete once it is chosen ("In 2 turns,
    /// gain 2 Gold"): 0 means it completes at the end of the next fight.
    pub complete_turns: u8,
    /// Each turn's end takes 1 off the next press's cost, down to 0 (The
    /// Perfect Crime, The Galaxy's Lens).
    pub cheaper_each_turn: bool,
    /// Powers that rotate into each other (the King of X family), grouped
    /// under one number; 0 for a power outside a family.
    pub family: u8,
    /// The first turns of the game this power leaves only Freeze and End Turn
    /// (Procrastinate skips two, Expedition Plans one).
    pub skip_turns: u8,
    /// The powers a press may turn this one into (Embrace the Elements,
    /// Dungar's Gryphon), indices into `HeroDb::powers`.
    pub options: Vec<u8>,
    /// The power this one becomes at the start of every turn (Major Hymn and
    /// Minor Hymn swap): index into `HeroDb::powers`, None for a power that
    /// stays.
    pub swaps_to: Option<u8>,
}

impl HeroPower {
    /// The tribe a "Discover a minion of a tribe" power takes, which is the
    /// tribe the King of X family rotates.
    pub fn tribe_of(&self) -> Option<Tribe> {
        match self.effect {
            PowerEffect::DiscoverTribe(t) => Some(t),
            _ => None,
        }
    }
}

/// One hero. `hero_choices` offers are drawn from the heroes the lobby allows
/// (`tribes`: at least one of them must be in the lobby; a hero with no tie
/// is always allowed).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Hero {
    pub name: String,
    pub card_id: String,
    pub armor: u8,
    /// Health on top of the rules' starting Health (Patchwerk).
    pub extra_health: i32,
    pub tribes: Tribes,
    /// The power this hero starts with, index into `HeroDb::powers`.
    pub power: u8,
    /// Heroes whose powers this hero's power Discover never offers, by hero
    /// card_id (Sir Finley's own list: himself).
    pub power_excludes: Vec<String>,
}

/// Every built hero and every built power. `Player.power`, `Player.power2`
/// and `PowerState.of` index into `powers`; `Hero.power` is the starting
/// power a hero picks up.
#[derive(Clone, Debug, Default)]
pub struct HeroDb {
    pub heroes: Vec<Hero>,
    /// The powers: one starting power per hero, in hero order, then the
    /// `[[power]]` entries a hero only reaches later.
    pub powers: Vec<HeroPower>,
    by_card_id: HashMap<String, u8>,
    power_by_card_id: HashMap<String, u8>,
}

impl HeroDb {
    /// The hero with this card_id, if it is built.
    pub fn by_card_id(&self, card_id: &str) -> Option<u8> {
        self.by_card_id.get(card_id).copied()
    }

    /// The hero with this name, if it is built (for the sandbox's --hero and
    /// tests; the engine matches by index, never by name).
    pub fn by_name(&self, name: &str) -> Option<u8> {
        self.heroes.iter().position(|h| h.name == name).map(|i| i as u8)
    }

    /// The power with this card_id, if it is built (the data's
    /// `hero_powers` ids).
    pub fn power_by_card_id(&self, card_id: &str) -> Option<u8> {
        self.power_by_card_id.get(card_id).copied()
    }

    pub fn get(&self, i: u8) -> &Hero {
        &self.heroes[i as usize]
    }

    pub fn power(&self, i: u8) -> &HeroPower {
        &self.powers[i as usize]
    }

    /// The power hero `h` starts with.
    pub fn hero_power(&self, h: u8) -> &HeroPower {
        self.power(self.get(h).power)
    }

    /// The heroes a lobby may offer: those with no tribe tie, or one a tribe
    /// of the lobby matches.
    pub fn lobby_allowed(&self, lobby: Tribes) -> impl Iterator<Item = u8> + '_ {
        self.heroes
            .iter()
            .enumerate()
            .filter(move |(_, h)| h.tribes.is_neutral() || h.tribes.overlaps(lobby))
            .map(|(i, _)| i as u8)
    }

    /// The hero indices whose card_id is in `card_ids` (the exclusion list of
    /// a Hero Power Discover).
    pub fn hero_indices(&self, card_ids: &[String]) -> ArrayVec<u8, 8> {
        let mut out = ArrayVec::new();
        for h in self.lobby_allowed(Tribes::ALL) {
            if card_ids.contains(&self.get(h).card_id) {
                let _ = out.try_push(h);
            }
        }
        out
    }

    /// The powers in `family` whose tribe is in `lobby` and is not `except`,
    /// which is what a rotating power (A Tale of Kings) may turn into.
    pub fn family_options(&self, family: u8, lobby: Tribes, except: Option<Tribe>) -> ArrayVec<u8, 12> {
        let mut out = ArrayVec::new();
        for (i, p) in self.powers.iter().enumerate() {
            if p.family != family {
                continue;
            }
            if let Some(t) = p.tribe_of()
                && lobby.has(t)
                && except != Some(t)
            {
                let _ = out.try_push(i as u8);
            }
        }
        out
    }
}

#[derive(Deserialize)]
struct RawHero {
    name: String,
    card_id: String,
    #[serde(default)]
    armor: u8,
    #[serde(default)]
    extra_health: i32,
    #[serde(default)]
    tribes: Vec<String>,
    #[serde(default)]
    power_excludes: Vec<String>,
    power: RawPower,
}

/// A `[[power]]` entry: a power no hero starts with, reached later.
#[derive(Deserialize)]
struct RawExtraPower {
    name: String,
    card_id: String,
    #[serde(default)]
    cost: u8,
    #[serde(default)]
    passive: bool,
    #[serde(default = "one")]
    uses_per_turn: u8,
    #[serde(default)]
    once_per_game: bool,
    #[serde(default)]
    uses_per_game: u8,
    #[serde(default)]
    cheaper_each_turn: bool,
    #[serde(default)]
    unlock_tier: u8,
    #[serde(default)]
    unlock_turn: u32,
    /// The card_id of the power this one goes back to.
    #[serde(default)]
    returns_to: Option<String>,
    #[serde(default)]
    complete_turns: u8,
    #[serde(default)]
    family: Option<String>,
    #[serde(default)]
    skip_turns: u8,
    #[serde(default)]
    choose: Vec<String>,
    /// The card_id of the power this one becomes at the start of every turn.
    #[serde(default)]
    swaps_each_turn: Option<String>,
    #[serde(default)]
    effect: RawPowerEffect,
}

/// The shape a hero's own power has in the file: `[[hero]] power = { .. }`.
#[derive(Deserialize)]
struct RawPower {
    name: String,
    card_id: String,
    #[serde(default)]
    cost: u8,
    #[serde(default)]
    passive: bool,
    #[serde(default = "one")]
    uses_per_turn: u8,
    #[serde(default)]
    once_per_game: bool,
    #[serde(default)]
    uses_per_game: u8,
    #[serde(default)]
    cheaper_each_turn: bool,
    #[serde(default)]
    unlock_tier: u8,
    #[serde(default)]
    unlock_turn: u32,
    #[serde(default)]
    family: Option<String>,
    #[serde(default)]
    skip_turns: u8,
    #[serde(default)]
    choose: Vec<String>,
    /// The card_id of the power this one becomes at the start of every turn.
    #[serde(default)]
    swaps_each_turn: Option<String>,
    #[serde(default)]
    effect: RawPowerEffect,
}

fn one() -> u8 {
    1
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTurnDiscovers {
    turn: u32,
    tiers: Vec<u8>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBananas {
    mine: u8,
    others: u8,
    big_percent: u8,
    banana: String,
    big: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawHits {
    hits: u8,
    damage: i32,
}

#[derive(Deserialize, Default)]
struct RawPowerEffect {
    #[serde(default)]
    free_refresh_per_turn: Option<u8>,
    #[serde(default)]
    gold_after_upgrade: Option<u8>,
    #[serde(default)]
    gold_after_buy_tribe: Option<u8>,
    #[serde(default)]
    tribe: Option<String>,
    #[serde(default)]
    gold_next_turn_after_sell: Option<u8>,
    #[serde(default)]
    soc_leftmost: Option<Vec<String>>,
    #[serde(default)]
    extra_max_gold: Option<u8>,
    #[serde(default)]
    give_divine_shield: bool,
    #[serde(default)]
    steal_card_damage: Option<i32>,
    #[serde(default)]
    steal_random_double_health: bool,
    #[serde(default)]
    make_golden: bool,
    #[serde(default)]
    get_gems: Option<u8>,
    #[serde(default)]
    discover_tribe: Option<String>,
    #[serde(default)]
    discover_power: bool,
    #[serde(default)]
    spell_discount: Option<u8>,
    #[serde(default)]
    get_random_spell: bool,
    #[serde(default)]
    trigger_battlecry: bool,
    #[serde(default)]
    discard_get_random_minion: Option<String>,
    #[serde(default)]
    get_linked_minions: Option<u8>,
    #[serde(default)]
    refresh_with_spells: bool,
    #[serde(default)]
    trinket_offer: Option<String>,
    // ---- hero frame 2 ----
    #[serde(default)]
    discover_powers: Option<u8>,
    #[serde(default)]
    buy_milestone: Option<u8>,
    #[serde(default)]
    becomes: Option<String>,
    #[serde(default)]
    end_of_turn_ends: Option<[i32; 2]>,
    #[serde(default)]
    choose: Option<bool>,
    #[serde(default)]
    gold: Option<u8>,
    #[serde(default)]
    get_random_spell_cost: Option<u8>,
    #[serde(default)]
    discover_my_tier: bool,
    #[serde(default)]
    soc_grant_deathrattle: Option<u8>,
    #[serde(default)]
    card: Option<String>,
    #[serde(default)]
    soc_double_left_attack: Option<u8>,
    #[serde(default)]
    soc_rightmost: Option<Vec<String>>,
    #[serde(default)]
    soc_damage_enemies: Option<RawHits>,
    #[serde(default)]
    turn_discovers: Option<RawTurnDiscovers>,
    #[serde(default)]
    bananas: Option<RawBananas>,
    #[serde(default)]
    dark_gift_start: bool,
    #[serde(default)]
    dark_gift_every: Option<u8>,
    // ---- hero batch 1 (2026-11-01) ----
    /// `[attack, health, every]`: the buff and its improve step (Sharpen
    /// Blades).
    #[serde(default)]
    buff_per_buys: Option<[i32; 3]>,
    #[serde(default)]
    discover_magnetic: bool,
    #[serde(default)]
    get_tribe_cut_cost: Option<RawTribeCut>,
    /// The extra cost of each press (Lead Explorer).
    #[serde(default)]
    discover_my_tier_costs_more: Option<u8>,
    #[serde(default)]
    cast_random_spell: bool,
    #[serde(default)]
    prices: Option<RawPrices>,
    #[serde(default)]
    random_tribe_after_sells: Option<RawEveryTribe>,
    #[serde(default)]
    card_after_buys: Option<RawCardAfterBuys>,
    #[serde(default)]
    dig_for_golden: Option<u8>,
    #[serde(default)]
    discover_below: bool,
    #[serde(default)]
    growing_gold: Option<u8>,
    #[serde(default)]
    kitty_discover: bool,
    #[serde(default)]
    swap_with_shop: bool,
    /// `[every, discount]` (Avalanche).
    #[serde(default)]
    upgrade_discount_elements: Option<[u8; 2]>,
    #[serde(default)]
    three_wishes: bool,
    #[serde(default)]
    triple_reward_after_tiers: Option<u32>,
    /// `[attack, health, every]` (For the Horde!).
    #[serde(default)]
    tavern_buff_after_buys: Option<[i32; 3]>,
    #[serde(default)]
    copy_buy_after_minions: Option<u8>,
    #[serde(default)]
    stir_the_pot: Option<u8>,
    #[serde(default)]
    steal_all_tavern: bool,
    #[serde(default)]
    discover_dead_minion: bool,
    #[serde(default)]
    hand_copy_every: Option<u8>,
    /// The Gold to spend before the champion arrives (Choose Your Champion).
    #[serde(default)]
    champion: Option<u32>,
    #[serde(default)]
    lucky_roll: bool,
    #[serde(default)]
    free_third_spell: bool,
    #[serde(default)]
    copy_last_spell: bool,
    #[serde(default)]
    triple_reward_after_attacks: Option<u32>,
    // ---- hero batch 2 (2026-11-01) ----
    #[serde(default)]
    replace_shop_higher: bool,
    #[serde(default)]
    temp_reborn: bool,
    /// What minions cost (Stay Frosty).
    #[serde(default)]
    stay_frosty: Option<u8>,
    #[serde(default)]
    refresh_high_tier: Option<u8>,
    /// `[attack, health]` (Saturday C'Thuns!).
    #[serde(default)]
    saturday_cthun: Option<[i32; 2]>,
    /// `[attack, health]` (Hat Trick).
    #[serde(default)]
    hat_trick: Option<[i32; 2]>,
    #[serde(default)]
    burgle_last_opponent: bool,
    #[serde(default)]
    extra_dragon: bool,
    #[serde(default)]
    replace_same_tier: bool,
    #[serde(default)]
    imprison: Option<u8>,
    /// `[attack, health, every]` (Tavern Lighting).
    #[serde(default)]
    tavern_lighting: Option<[i32; 2]>,
    /// `[percent, count]` (Come One, Come All!).
    #[serde(default)]
    darkmoon_tickets: Option<[u8; 2]>,
    /// `[attack, health]` (See the Light).
    #[serde(default)]
    steal_shop_as_stats: Option<[i32; 2]>,
    #[serde(default)]
    spirit_swap: bool,
    #[serde(default)]
    devour: bool,
    #[serde(default)]
    ispy_next_opponent: bool,
    #[serde(default)]
    twice_as_nice: bool,
    #[serde(default)]
    enhancification: Option<u8>,
    /// The tribe whose minion is destroyed for a random one (Rune of
    /// Damnation).
    #[serde(default)]
    destroy_tribe_get_random: bool,
    /// "attack" or "health" (Major and Minor Hymn).
    #[serde(default)]
    hymn: Option<String>,
    #[serde(default)]
    cloning_gallery: bool,
    #[serde(default)]
    double_time: bool,
    // ---- hero batch 3 (2026-11-01) ----
    #[serde(default)]
    wingmen: bool,
    #[serde(default)]
    wax_warband: bool,
    #[serde(default)]
    start_minion: Option<String>,
    #[serde(default)]
    damage_milestone: Option<u32>,
    #[serde(default)]
    random_minions_each_turn: Option<u8>,
    #[serde(default)]
    take_first_killed: bool,
    #[serde(default)]
    all_will_burn: bool,
    #[serde(default)]
    attack_milestone: Option<u8>,
    #[serde(default)]
    free_first_minion: bool,
    #[serde(default)]
    friendly_wager: bool,
    #[serde(default)]
    embrace_your_rage: bool,
    #[serde(default)]
    sprout_it_out: bool,
    #[serde(default)]
    glory_of_combat: bool,
    #[serde(default)]
    power_of_the_storm: bool,
    #[serde(default)]
    conviction: bool,
    /// `[count, attack, health]` (Offensive, Defensive and United Stance).
    #[serde(default)]
    stance: Option<[i32; 3]>,
    #[serde(default)]
    fragrant_phylactery: bool,
    #[serde(default)]
    lock_and_load: bool,
    /// "attack" or "health" (Frostwolf Fervor, Stormpike Strength).
    #[serde(default)]
    copy_highest_when_space: Option<String>,
    #[serde(default)]
    mech_gyver: Option<u8>,
    #[serde(default)]
    broodmother: bool,
    #[serde(default)]
    detective_for_hire: bool,
    #[serde(default)]
    tentacular: bool,
    #[serde(default)]
    rapid_reanimation: bool,
    // ---- trinkets block 5 (2026-11-01) ----
    /// The turns between Prizes (Prize Wall: 4).
    prize_wall: Option<u8>,
    // ---- secrets ----
    /// Choose one of the combat Secrets still missing from the zone
    /// (Prestidigitation).
    #[serde(default)]
    discover_secret: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTribeCut {
    tribe: String,
    gold: u8,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPrices {
    minion: u8,
    refresh: u8,
    #[serde(default)]
    upgrade_extra: u8,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEveryTribe {
    every: u8,
    tribe: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCardAfterBuys {
    every: u8,
    /// "cards" (the default), "minions" or "battlecry".
    #[serde(default)]
    of: Option<String>,
    card: String,
}

/// Parse one power effect. The two effects whose card names need the card
/// set keep index 0 here; `resolve_effect` fills them in.
fn parse_effect(e: &RawPowerEffect) -> Result<PowerEffect, String> {
    let set = |names: &[String]| -> Result<u8, String> {
        let mut kw = 0u8;
        for n in names {
            let bit = Keywords::ALL
                .iter()
                .find(|(_, k)| k == n)
                .map(|(b, _)| *b)
                .ok_or_else(|| format!("unknown keyword {n:?}"))?;
            kw |= bit;
        }
        Ok(kw)
    };
    Ok(if let Some(n) = e.free_refresh_per_turn {
        PowerEffect::FreeRefreshPerTurn(n)
    } else if let Some(g) = e.gold_after_upgrade {
        PowerEffect::GoldAfterUpgrade(g)
    } else if let Some(g) = e.gold_after_buy_tribe {
        let t = e.tribe.as_deref().ok_or("gold_after_buy_tribe needs a tribe")?;
        PowerEffect::GoldAfterBuyTribe { tribe: tribe(t)?, gold: g }
    } else if let Some(g) = e.gold_next_turn_after_sell {
        PowerEffect::GoldNextTurnAfterSell(g)
    } else if let Some(names) = &e.soc_leftmost {
        PowerEffect::SocLeftmost { keywords: set(names)? }
    } else if let Some(n) = e.extra_max_gold {
        PowerEffect::ExtraMaxGold(n)
    } else if e.give_divine_shield {
        PowerEffect::GiveDivineShield
    } else if let Some(d) = e.steal_card_damage {
        PowerEffect::StealCardDamage(d)
    } else if e.steal_random_double_health {
        PowerEffect::StealRandomDoubleHealth
    } else if e.make_golden {
        PowerEffect::MakeGolden
    } else if let Some(n) = e.get_gems {
        PowerEffect::GetGems(n)
    } else if let Some(t) = &e.discover_tribe {
        PowerEffect::DiscoverTribe(tribe(t)?)
    } else if e.discover_power {
        PowerEffect::DiscoverPower
    } else if let Some(d) = e.spell_discount {
        PowerEffect::SpellDiscount(d)
    } else if e.get_random_spell {
        PowerEffect::GetRandomSpell
    } else if e.trigger_battlecry {
        PowerEffect::TriggerBattlecry
    } else if let Some(t) = &e.discard_get_random_minion {
        PowerEffect::DiscardGetRandomMinion { tribe: tribe(t)? }
    } else if let Some(n) = e.get_linked_minions {
        PowerEffect::GetLinkedMinions(n)
    } else if e.refresh_with_spells {
        PowerEffect::RefreshWithSpells
    } else if let Some(s) = &e.trinket_offer {
        PowerEffect::TrinketOffer(match s.as_str() {
            "lesser" => Slot::Lesser,
            "greater" => Slot::Greater,
            other => return Err(format!("unknown trinket slot {other:?}")),
        })
    } else if let Some(n) = e.discover_powers {
        PowerEffect::DiscoverPowers(n)
    } else if let Some(n) = e.buy_milestone {
        if e.becomes.is_none() {
            return Err("buy_milestone needs `becomes`".into());
        }
        PowerEffect::BuyMilestone { count: n, becomes: 0 }
    } else if let Some([a, h]) = e.end_of_turn_ends {
        PowerEffect::EndOfTurnEnds { attack: a, health: h }
    } else if e.choose.unwrap_or(false) {
        PowerEffect::ChooseSubPowers
    } else if let Some(g) = e.gold {
        PowerEffect::Gold(g)
    } else if let Some(c) = e.get_random_spell_cost {
        PowerEffect::GetRandomSpellOfCost(c)
    } else if e.discover_my_tier {
        PowerEffect::DiscoverMyTier
    } else if let Some(n) = e.soc_grant_deathrattle {
        if e.card.is_none() {
            return Err("soc_grant_deathrattle needs `card`".into());
        }
        PowerEffect::SocGrantDeathrattle { count: n, card: 0 }
    } else if let Some(n) = e.soc_double_left_attack {
        PowerEffect::SocDoubleLeftAttack(n)
    } else if let Some(names) = &e.soc_rightmost {
        PowerEffect::SocRightmost { keywords: set(names)? }
    } else if let Some(h) = &e.soc_damage_enemies {
        PowerEffect::SocDamageEnemies { hits: h.hits, damage: h.damage }
    } else if let Some(td) = &e.turn_discovers {
        if td.tiers.is_empty() || td.tiers.len() > 3 {
            return Err("turn_discovers takes one to three tiers".into());
        }
        let mut tiers = [0u8; 3];
        for (i, &t) in td.tiers.iter().enumerate() {
            tiers[i] = t;
        }
        PowerEffect::TurnDiscovers { turn: td.turn, tiers }
    } else if let Some(b) = &e.bananas {
        PowerEffect::Bananarama {
            mine: b.mine,
            others: b.others,
            big_percent: b.big_percent,
            banana: 0,
            big: 0,
        }
    } else if e.dark_gift_start {
        PowerEffect::DarkGiftStart
    } else if let Some(n) = e.dark_gift_every {
        PowerEffect::DarkGiftEvery { turns: n.max(1) }
    } else if let Some([attack, health, every]) = e.buff_per_buys {
        PowerEffect::BuffPerBuys { attack, health, every: every.max(1) as u8 }
    } else if e.discover_magnetic {
        PowerEffect::DiscoverMagnetic
    } else if let Some(c) = &e.get_tribe_cut_cost {
        PowerEffect::GetTribeCutCost { tribe: tribe(&c.tribe)?, gold: c.gold }
    } else if let Some(more) = e.discover_my_tier_costs_more {
        PowerEffect::DiscoverMyTierCostsMore { more }
    } else if e.cast_random_spell {
        PowerEffect::CastRandomSpell
    } else if let Some(p) = &e.prices {
        PowerEffect::Prices { minion: p.minion, refresh: p.refresh, upgrade_extra: p.upgrade_extra }
    } else if let Some(s) = &e.random_tribe_after_sells {
        PowerEffect::RandomTribeAfterSells { every: s.every.max(1), tribe: tribe(&s.tribe)? }
    } else if let Some(b) = &e.card_after_buys {
        let of = match b.of.as_deref() {
            None | Some("cards") => BuyKind::Cards,
            Some("minions") => BuyKind::Minions,
            Some("battlecry") => BuyKind::Battlecry,
            Some(other) => return Err(format!("unknown card_after_buys kind {other:?}")),
        };
        PowerEffect::CardAfterBuys { every: b.every.max(1), of, card: 0 }
    } else if let Some(n) = e.dig_for_golden {
        PowerEffect::DigForGolden { digs: n.max(1) }
    } else if e.discover_below {
        PowerEffect::DiscoverBelow
    } else if let Some(base) = e.growing_gold {
        PowerEffect::GrowingGold { base }
    } else if e.kitty_discover {
        PowerEffect::KittyDiscover
    } else if e.swap_with_shop {
        PowerEffect::SwapWithShop
    } else if let Some([every, discount]) = e.upgrade_discount_elements {
        PowerEffect::UpgradeDiscountAfterElementals { every: every.max(1), discount }
    } else if e.three_wishes {
        PowerEffect::ThreeWishes
    } else if let Some(every) = e.triple_reward_after_tiers {
        PowerEffect::TripleRewardAfterTiers { every: every.max(1) }
    } else if let Some([attack, health, every]) = e.tavern_buff_after_buys {
        PowerEffect::TavernBuffAfterBuys { attack, health, every: every.max(1) as u8 }
    } else if let Some(n) = e.copy_buy_after_minions {
        PowerEffect::CopyBuyAfterMinions { every: n.max(1) }
    } else if let Some(n) = e.stir_the_pot {
        PowerEffect::StirThePot { count: n.max(1) }
    } else if e.steal_all_tavern {
        PowerEffect::StealAllTavern
    } else if e.discover_dead_minion {
        PowerEffect::DiscoverDeadMinion
    } else if let Some(n) = e.hand_copy_every {
        PowerEffect::HandCopyEvery { turns: n.max(1) }
    } else if let Some(gold) = e.champion {
        PowerEffect::Champion { gold }
    } else if e.lucky_roll {
        PowerEffect::LuckyRoll
    } else if e.free_third_spell {
        PowerEffect::FreeThirdSpell
    } else if e.copy_last_spell {
        PowerEffect::CopyLastSpell
    } else if let Some(every) = e.triple_reward_after_attacks {
        PowerEffect::TripleRewardAfterAttacks { every: every.max(1) }
    } else if e.replace_shop_higher {
        PowerEffect::ReplaceShopHigher
    } else if e.temp_reborn {
        PowerEffect::TempReborn
    } else if let Some(price) = e.stay_frosty {
        PowerEffect::StayFrosty { price }
    } else if let Some(count) = e.refresh_high_tier {
        PowerEffect::RefreshHighTier { count: count.max(1) }
    } else if let Some([attack, health]) = e.saturday_cthun {
        PowerEffect::SaturdayCthun { attack, health }
    } else if let Some([attack, health]) = e.hat_trick {
        PowerEffect::HatTrick { attack, health }
    } else if e.burgle_last_opponent {
        PowerEffect::BurgleLastOpponent
    } else if e.extra_dragon {
        PowerEffect::ExtraDragon
    } else if e.replace_same_tier {
        PowerEffect::ReplaceSameTier
    } else if let Some(turns) = e.imprison {
        PowerEffect::Imprison { turns }
    } else if let Some([attack, health]) = e.tavern_lighting {
        PowerEffect::TavernLighting { attack, health }
    } else if let Some([percent, count]) = e.darkmoon_tickets {
        PowerEffect::DarkmoonTickets { percent, count: count.max(1) }
    } else if let Some([attack, health]) = e.steal_shop_as_stats {
        PowerEffect::StealShopAsStats { attack, health }
    } else if e.spirit_swap {
        PowerEffect::SpiritSwap
    } else if e.devour {
        PowerEffect::Devour
    } else if e.ispy_next_opponent {
        PowerEffect::IspyNextOpponent
    } else if e.twice_as_nice {
        PowerEffect::TwiceAsNice
    } else if let Some(times) = e.enhancification {
        PowerEffect::Enhancification { times: times.max(1) }
    } else if e.destroy_tribe_get_random {
        let t = e.tribe.as_deref().ok_or("destroy_tribe_get_random needs a tribe")?;
        PowerEffect::DestroyTribeGetRandom { tribe: tribe(t)? }
    } else if let Some(which) = &e.hymn {
        PowerEffect::Hymn { attack: which.trim() == "attack" }
    } else if e.cloning_gallery {
        PowerEffect::CloningGallery
    } else if e.double_time {
        PowerEffect::DoubleTime { coin: 0 }
    } else if e.wingmen {
        PowerEffect::Wingmen
    } else if e.wax_warband {
        PowerEffect::WaxWarband
    } else if e.start_minion.is_some() {
        PowerEffect::StartMinion { card: 0 }
    } else if let Some(damage) = e.damage_milestone {
        if e.becomes.is_none() {
            return Err("damage_milestone needs `becomes`".into());
        }
        PowerEffect::DamageMilestone { damage: damage.max(1), becomes: 0 }
    } else if let Some(count) = e.random_minions_each_turn {
        let t = e.tribe.as_deref().ok_or("random_minions_each_turn needs a tribe")?;
        PowerEffect::RandomMinionsEachTurn { tribe: tribe(t)?, count: count.max(1) }
    } else if e.take_first_killed {
        PowerEffect::TakeFirstKilled
    } else if e.all_will_burn {
        PowerEffect::AllWillBurn
    } else if let Some(count) = e.attack_milestone {
        if e.becomes.is_none() {
            return Err("attack_milestone needs `becomes`".into());
        }
        PowerEffect::AttackMilestone { count: count.max(1), becomes: 0 }
    } else if e.free_first_minion {
        PowerEffect::FreeFirstMinion
    } else if e.friendly_wager {
        PowerEffect::FriendlyWager { coin: 0 }
    } else if e.embrace_your_rage {
        PowerEffect::EmbraceYourRage
    } else if e.sprout_it_out {
        PowerEffect::SproutItOut
    } else if e.glory_of_combat {
        PowerEffect::GloryOfCombat
    } else if e.power_of_the_storm {
        PowerEffect::PowerOfTheStorm
    } else if e.conviction {
        PowerEffect::Conviction
    } else if let Some([count, attack, health]) = e.stance {
        PowerEffect::Stance { count: count.max(0) as u8, attack, health }
    } else if e.fragrant_phylactery {
        PowerEffect::FragrantPhylactery
    } else if e.lock_and_load {
        PowerEffect::LockAndLoad
    } else if let Some(which) = &e.copy_highest_when_space {
        PowerEffect::CopyHighestWhenSpace { by_health: which.trim() == "health" }
    } else if let Some(every) = e.mech_gyver {
        PowerEffect::MechGyver { every: every.max(1) }
    } else if e.broodmother {
        PowerEffect::Broodmother { card: 0 }
    } else if e.detective_for_hire {
        PowerEffect::DetectiveForHire { coin: 0 }
    } else if e.tentacular {
        PowerEffect::Tentacular { card: 0 }
    } else if e.rapid_reanimation {
        PowerEffect::RapidReanimation
    } else if let Some(n) = e.prize_wall {
        PowerEffect::PrizeWall { every: n.max(1) }
    } else if e.discover_secret {
        PowerEffect::DiscoverSecret
    } else {
        PowerEffect::None
    })
}

/// The card names an effect or a `choose` list carries, once the card set and
/// the power table can be searched.
fn resolve_effect(
    e: &RawPowerEffect,
    plain: PowerEffect,
    cards: &CardDb,
    powers: &HashMap<String, u8>,
    who: &str,
) -> Result<PowerEffect, String> {
    let card = |name: &Option<String>, what: &str| -> Result<CardId, String> {
        let name = name.as_deref().ok_or_else(|| format!("{who}: needs `{what}`"))?;
        cards.by_name(name).ok_or_else(|| format!("{who}: no card {name:?}"))
    };
    Ok(match plain {
        PowerEffect::BuyMilestone { count, .. } => {
            let name = e.becomes.as_deref().ok_or_else(|| format!("{who}: needs `becomes`"))?;
            let becomes =
                powers.get(name).copied().ok_or_else(|| format!("{who}: no power {name:?}"))?;
            PowerEffect::BuyMilestone { count, becomes }
        }
        PowerEffect::DamageMilestone { damage, .. } => {
            let name = e.becomes.as_deref().ok_or_else(|| format!("{who}: needs `becomes`"))?;
            let becomes =
                powers.get(name).copied().ok_or_else(|| format!("{who}: no power {name:?}"))?;
            PowerEffect::DamageMilestone { damage, becomes }
        }
        PowerEffect::AttackMilestone { count, .. } => {
            let name = e.becomes.as_deref().ok_or_else(|| format!("{who}: needs `becomes`"))?;
            let becomes =
                powers.get(name).copied().ok_or_else(|| format!("{who}: no power {name:?}"))?;
            PowerEffect::AttackMilestone { count, becomes }
        }
        PowerEffect::SocGrantDeathrattle { count, .. } => {
            PowerEffect::SocGrantDeathrattle { count, card: card(&e.card, "card")? }
        }
        PowerEffect::StartMinion { .. } => {
            PowerEffect::StartMinion { card: card(&e.start_minion, "start_minion")? }
        }
        PowerEffect::DoubleTime { .. } => {
            PowerEffect::DoubleTime { coin: card(&e.card, "card")? }
        }
        PowerEffect::FriendlyWager { .. } => {
            PowerEffect::FriendlyWager { coin: card(&e.card, "card")? }
        }
        PowerEffect::DetectiveForHire { .. } => {
            PowerEffect::DetectiveForHire { coin: card(&e.card, "card")? }
        }
        PowerEffect::Broodmother { .. } => {
            PowerEffect::Broodmother { card: card(&e.card, "card")? }
        }
        PowerEffect::Tentacular { .. } => {
            PowerEffect::Tentacular { card: card(&e.card, "card")? }
        }
        PowerEffect::CardAfterBuys { every, of, .. } => {
            let b = e.card_after_buys.as_ref().unwrap();
            PowerEffect::CardAfterBuys {
                every,
                of,
                card: cards
                    .by_name(&b.card)
                    .ok_or_else(|| format!("{who}: no card {:?}", b.card))?,
            }
        }
        PowerEffect::Bananarama { mine, others, big_percent, .. } => {
            let b = e.bananas.as_ref().unwrap();
            PowerEffect::Bananarama {
                mine,
                others,
                big_percent,
                banana: card(&Some(b.banana.clone()), "banana")?,
                big: card(&Some(b.big.clone()), "big")?,
            }
        }
        other => other,
    })
}

fn tribe(name: &str) -> Result<Tribe, String> {
    crate::cards::tribe(name.trim().to_lowercase().as_str())
}

/// One power while the table is still being built: its own fields, the effect
/// as parsed, and the names only the second pass can resolve.
struct Pending {
    power: HeroPower,
    raw: RawPowerEffect,
    family: Option<String>,
    returns_to: Option<String>,
    choose: Vec<String>,
    swaps_to: Option<String>,
}

impl HeroDb {
    /// The hero and power tables from heroes.toml, with the card names an
    /// effect carries (Bru'kan's Elemental token, Mukla's Bananas) resolved
    /// against `cards` and the powers an effect names resolved against the
    /// power table itself (Sulfuras, the Invocations, the King of X family).
    pub fn from_toml(s: &str, cards: &CardDb) -> Result<HeroDb, String> {
        #[derive(Deserialize)]
        struct RawFile {
            #[serde(default)]
            hero: Vec<RawHero>,
            #[serde(default)]
            power: Vec<RawExtraPower>,
        }
        let raw: RawFile = toml::from_str(s).map_err(|e| format!("heroes.toml: {e}"))?;
        let mut db = HeroDb {
            heroes: Vec::new(),
            powers: Vec::new(),
            by_card_id: HashMap::new(),
            power_by_card_id: HashMap::new(),
        };
        let mut pending: Vec<Pending> = Vec::new();
        // The starting powers come first, one per hero, so a hero's index is
        // its starting power's index too.
        for h in raw.hero {
            let effect = parse_effect(&h.power.effect).map_err(|e| format!("{}: {e}", h.name))?;
            let power = HeroPower {
                name: h.power.name,
                card_id: h.power.card_id,
                cost: h.power.cost,
                passive: h.power.passive,
                uses_per_turn: h.power.uses_per_turn,
                uses_per_game: if h.power.once_per_game { 1 } else { h.power.uses_per_game },
                unlock_tier: h.power.unlock_tier,
                unlock_turn: h.power.unlock_turn,
                effect,
                parent: None,
                complete_turns: 0,
                cheaper_each_turn: h.power.cheaper_each_turn,
                family: 0,
                skip_turns: h.power.skip_turns,
                options: Vec::new(),
                swaps_to: None,
            };
            pending.push(Pending {
                power,
                raw: h.power.effect,
                family: h.power.family,
                returns_to: None,
                choose: h.power.choose,
                swaps_to: h.power.swaps_each_turn,
            });
            let mut tribes = Tribes::NONE;
            for t in &h.tribes {
                tribes = tribes.with(tribe(t).map_err(|e| format!("{}: {e}", h.name))?);
            }
            let hero = Hero {
                name: h.name,
                card_id: h.card_id.clone(),
                armor: h.armor,
                extra_health: h.extra_health,
                tribes,
                power: db.heroes.len() as u8,
                power_excludes: h.power_excludes,
            };
            if db.by_card_id.insert(h.card_id, db.heroes.len() as u8).is_some() {
                return Err(format!("heroes.toml: hero {} twice", hero.name));
            }
            db.heroes.push(hero);
        }
        for h in raw.power {
            let effect = parse_effect(&h.effect).map_err(|e| format!("{}: {e}", h.name))?;
            let power = HeroPower {
                name: h.name,
                card_id: h.card_id,
                cost: h.cost,
                passive: h.passive,
                uses_per_turn: h.uses_per_turn,
                uses_per_game: if h.once_per_game { 1 } else { h.uses_per_game },
                unlock_tier: h.unlock_tier,
                unlock_turn: h.unlock_turn,
                effect,
                parent: None,
                complete_turns: h.complete_turns,
                cheaper_each_turn: h.cheaper_each_turn,
                family: 0,
                skip_turns: h.skip_turns,
                options: Vec::new(),
                swaps_to: None,
            };
            pending.push(Pending {
                power,
                raw: h.effect,
                family: h.family,
                returns_to: h.returns_to,
                choose: h.choose,
                swaps_to: h.swaps_each_turn,
            });
        }
        // Every card_id is known now, so the names inside the effects and the
        // family names can be turned into numbers.
        db.powers = pending.iter().map(|p| p.power.clone()).collect();
        for (i, p) in pending.iter().enumerate() {
            if db.power_by_card_id.insert(p.power.card_id.clone(), i as u8).is_some() {
                return Err(format!("heroes.toml: power {} twice", p.power.name));
            }
        }
        let mut families: HashMap<&str, u8> = HashMap::new();
        for (i, p) in pending.iter().enumerate() {
            if let Some(name) = p.family.as_deref() {
                let next = families.len() as u8 + 1;
                let id = *families.entry(name).or_insert(next);
                db.powers[i].family = id;
            }
        }
        for (i, p) in pending.iter().enumerate() {
            let plain = p.power.effect;
            db.powers[i].effect = resolve_effect(&p.raw, plain, cards, &db.power_by_card_id, &p.power.name)?;
            if let Some(r) = &p.swaps_to {
                db.powers[i].swaps_to = Some(
                    db.power_by_card_id(r)
                        .ok_or_else(|| format!("{}: no power {r:?}", p.power.name))?,
                );
            }
            if let Some(r) = &p.returns_to {
                db.powers[i].parent = Some(
                    db.power_by_card_id(r)
                        .ok_or_else(|| format!("{}: no power {r:?}", p.power.name))?,
                );
            }
            for c in &p.choose {
                let idx = db
                    .power_by_card_id(c)
                    .ok_or_else(|| format!("{}: no power {c:?}", p.power.name))?;
                db.powers[i].options.push(idx);
            }
        }
        Ok(db)
    }
}
