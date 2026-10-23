use arrayvec::ArrayVec;
use serde::Deserialize;

use crate::minion::{Attached, CardId, Keywords, MAX_ATTACHED, Minion};
use crate::rng::Rng;
use crate::BOARD;

/// A per-player count that an "improve your future ..." effect raises and
/// scales by. Each variant is one slot in `Counters::improve`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Counter {
    TastyLobster,
    /// Ballers sold this game, shared by Fire, Snow and Air Baller.
    Baller,
}

pub const N_COUNTERS: usize = 2;

impl Counter {
    pub fn index(self) -> usize {
        self as usize
    }
}

fn counter(name: &str) -> Result<Counter, String> {
    match name {
        "tasty_lobster" => Ok(Counter::TastyLobster),
        "baller" => Ok(Counter::Baller),
        other => Err(format!("unknown counter {other:?}")),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tribe {
    Beast,
    Mech,
    Dragon,
    Murloc,
    Demon,
    Pirate,
    Elemental,
}

impl Tribe {
    /// The five tribes the observation has slots for (layout frozen until the
    /// run that redoes the observation for every tribe). Rules use `EVERY`.
    pub const ALL: [Tribe; 5] = [Tribe::Beast, Tribe::Mech, Tribe::Dragon, Tribe::Murloc, Tribe::Demon];
    /// Every tribe a card can have.
    pub const EVERY: [Tribe; 7] = [
        Tribe::Beast,
        Tribe::Mech,
        Tribe::Dragon,
        Tribe::Murloc,
        Tribe::Demon,
        Tribe::Pirate,
        Tribe::Elemental,
    ];

    fn bit(self) -> u8 {
        1 << self as u8
    }
}

/// The tribes of a card: none (neutral), one, two, or all five ("all").
/// Every tribe filter in the game asks `has`, so a two-tribe card counts for
/// both and an "all" card counts for every tribe.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Tribes(u8);

impl Tribes {
    pub const NONE: Tribes = Tribes(0);
    pub const ALL: Tribes = Tribes(0b1111111);

    pub fn of(t: Tribe) -> Tribes {
        Tribes(t.bit())
    }

    pub fn has(self, t: Tribe) -> bool {
        self.0 & t.bit() != 0
    }

    /// Neutral: no tribe at all.
    pub fn is_neutral(self) -> bool {
        self.0 == 0
    }

    /// Whether the two sets share a tribe ("a minion of its type").
    pub fn overlaps(self, other: Tribes) -> bool {
        self.0 & other.0 != 0
    }

    /// One of the card's tribes picked uniformly at random; None when it is
    /// neutral (DESIGN.md "Types": a dual or all-type minion uses one of its
    /// types at random).
    pub fn random_type(self, rng: &mut Rng) -> Option<Tribe> {
        let n = self.0.count_ones();
        if n == 0 {
            return None;
        }
        let mut k = rng.below(n);
        for t in Tribe::EVERY {
            if self.has(t) {
                if k == 0 {
                    return Some(t);
                }
                k -= 1;
            }
        }
        None
    }
}

/// "Give a friendly minion of each type ..." (Misplaced Tea Set, The Last One
/// Standing): for each type in turn, one random minion of that type that has
/// not been picked yet, so every minion gets at most one buff, even a dual or
/// all-type one. `tribes[i]` is the types of board minion `i`, `Tribes::NONE`
/// for one that can't be picked (dead). Returns the picked indexes.
pub fn pick_one_per_type(tribes: &[Tribes], rng: &mut Rng) -> ArrayVec<usize, BOARD> {
    let mut picked: ArrayVec<usize, BOARD> = ArrayVec::new();
    for t in Tribe::EVERY {
        let mut ok: ArrayVec<usize, BOARD> = ArrayVec::new();
        for (i, ts) in tribes.iter().enumerate().take(BOARD) {
            if !picked.contains(&i) && ts.has(t) {
                ok.push(i);
            }
        }
        if ok.is_empty() {
            continue;
        }
        let k = rng.below(ok.len() as u32) as usize;
        picked.push(ok[k]);
    }
    picked
}

/// Which Tavern minion a consuming minion takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ConsumePick {
    /// The player picks one in the Tavern.
    Chosen,
    Random,
    HighestHealth,
}

/// Who does the consuming.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ConsumeBy {
    /// The minion with the effect.
    This,
    /// A friendly minion the player picks, of this tribe if one is given.
    Chosen(Option<Tribe>),
}

/// Battlecry and general minion card text effects. Also what Activate, event
/// triggers, "after your hero takes damage" and some end of turn effects run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Summon `count` copies of a card (a token or a pool minion) next to the
    /// minion. A golden minion summons `count` golden copies, or, with
    /// `golden_doubles_count`, twice as many plain ones. Fights only.
    Summon { card: CardId, count: u8, golden_doubles_count: bool },
    /// Buff one other friendly minion the player picks. `per_keyword` is
    /// added once more for each different Bonus Keyword in the warband.
    BuffChosen {
        tribe: Option<Tribe>,
        attack: i32,
        health: i32,
        per_keyword: (i32, i32),
        random_keyword: bool,
    },
    /// Buff one minion the player picks on the board or in the Tavern
    /// ("another minion", Suspicious Prisonguard).
    BuffAnyMinion { attack: i32, health: i32 },
    /// Set one minion's (board or Tavern) stats (Tyrael).
    SetStatsAnyMinion { attack: i32, health: i32 },
    /// Buff every other friendly minion.
    BuffOthers { tribe: Option<Tribe>, attack: i32, health: i32, per_keyword: (i32, i32) },
    /// Buff every friendly minion, the one with the effect included.
    BuffAll { tribe: Option<Tribe>, attack: i32, health: i32 },
    /// Buff the minion itself.
    BuffSelf { attack: i32, health: i32 },
    /// Buff a random friendly minion (of a tribe, if given). With `improve`
    /// set the buff is multiplied by 1 plus the player's count for that
    /// counter, and the count then goes up by one ("Improve your future ...").
    BuffRandomFriendly { tribe: Option<Tribe>, attack: i32, health: i32, improve: Option<Counter> },
    /// Buff the minion that killed the one with the effect (Fishbait).
    BuffKiller { attack: i32, health: i32 },
    /// The minion gains a keyword.
    GainKeyword { keyword: u8 },
    /// Get a random pool minion of a tribe.
    GetRandomMinion { tribe: Tribe },
    /// Get `count` random Tavern spells that cost `cost`.
    GetRandomSpell { cost: u8, count: u8 },
    /// Summon `count` random pool minions that have a Deathrattle. Fights only.
    SummonRandomDeathrattle { count: u8 },
    /// Trigger your left-most Deathrattle `times` times. Fights only.
    TriggerDeathrattle { times: u8 },
    /// Pick a Tavern card, replace it with a Fishbait and have your left-most
    /// Beast attack it.
    ReplaceWithFishbait { card: CardId },
    /// Refresh the Tavern for free, put a Fishbait in the left-most slot of it and
    /// have your left-most Beast attack it.
    RefreshWithFishbait { card: CardId },
    /// The next Magnetization to this minion this turn happens `n` extra
    /// times.
    ExtraMagnetization(u8),
    /// The minion gains a random Bonus Keyword it doesn't have.
    RandomKeyword,
    /// A minion consumes minions from the Tavern and gains their stats, and
    /// their Bonus Keywords when `keywords` is set.
    Consume { by: ConsumeBy, pick: ConsumePick, keywords: bool, count: u8 },
    /// Buff the Tavern minions (of a tribe, if given), now and, unless
    /// `this_turn`, for the rest of the game.
    TavernBuff { tribe: Option<Tribe>, attack: i32, health: i32, this_turn: bool },
    /// Deal damage to your own hero (never below the self damage floor).
    DamageHero(i32),
    /// Undo the damage your hero just took.
    RewindDamage,
    /// Magnetize a random Volumizer to this minion and get a copy of it.
    MagnetizeVolumizerAndCopy,
    /// Give your Beetles +attack/+health this game.
    BeetleBuff { attack: i32, health: i32 },
    /// Gain extra Gold next turn.
    GoldNextTurn(u8),
    /// Get a random Magnetic Volumizer (2 when golden).
    RandomVolumizer,
    /// The next Tavern spell you buy costs `n` less.
    SpellDiscount(u8),
    /// Get `count` copies of a generated card (`count * 2` when golden).
    GetCard { card: CardId, count: u8 },
    /// Raise the player's Tavern spell bonus for the rest of the game.
    SpellBonus { attack: i32, health: i32 },
    /// Get a random Chromadrake (`2` when golden).
    RandomChromadrake,
    /// Cast a named Tavern spell `times` times for the minion's side.
    /// Fights only run the spell effects the fight knows (Shiny Ring's
    /// board buff and Mighty Dragonbreath).
    CastSpell { card: CardId, times: u8 },
    /// Activate only: trigger a friendly minion's Rally, chosen by the
    /// player, in the shop.
    TriggerRally,
    /// Buff the minion that just attacked (Roaring Recruiter).
    BuffAttacker { attack: i32, health: i32 },
    /// Give `count` random other friendly minions of a tribe stats and a
    /// keyword (Amber Guardian). A golden minion picks `count` twice as many.
    BuffRandomOther { tribe: Option<Tribe>, attack: i32, health: i32, keyword: u8, count: u8 },
    /// Summon the `count` highest-Attack minions of a tribe from hand, for
    /// this combat only (Expert Aviator, Diremuck Forager). The copies leave
    /// the hand for the rest of the fight and are back in it afterwards.
    SummonFromHand { tribe: Tribe, count: u8 },
    /// Give a random minion in hand stats, permanently (Very Hungry
    /// Winterfinner). Fights only.
    BuffRandomHand { attack: i32, health: i32 },
    /// Give the left-most minion in hand stats (Twilight Tidehunter).
    BuffLeftmostHand { attack: i32, health: i32 },
    /// Give the minions of a tribe on the board and in hand stats (Shamanic
    /// Tidecaller).
    BuffTribeBoardAndHand { tribe: Tribe, attack: i32, health: i32 },
    /// Trigger a friendly minion's Battlecry, chosen by the player, in the
    /// shop (Kelp Keeper).
    TriggerBattlecry,
    /// If you control another minion of a tribe, Discover `count` minions of
    /// it (Primalfin Lookout).
    DiscoverTribeIfAnother { tribe: Tribe, count: u8 },
    /// Cast the Tavern spell this minion was taught (Magicfin Apprentice).
    CastTaughtSpell,
    /// Trigger the Battlecries of the adjacent minions this many times
    /// (Young Murk-Eye).
    TriggerAdjacentBattlecries(u8),
    /// Add `count` upcoming Refreshes that each include a Fodder (two when
    /// golden). Only the current turn's coming Refreshes, not the past ones.
    AddFodderRefreshes(u8),
    /// Add Fodders to the very next Refresh (Twisted Wrathguard). Golden adds
    /// twice as many to it.
    AddFodderToNextRefresh(u8),
    /// Every friendly Demon consumes `count` random Tavern minions (golden
    /// doubles the stats taken). Soulkeeping Jailer.
    DemonsConsume { count: u8 },
    /// Raise this minion's `improvements` by `n` (Sacrificial Wrathguard's
    /// "Improve this"). The count is Activates, so golden does not double
    /// it; the Deathrattle's per-improvement number is doubled instead.
    ImproveSelf(u8),
    /// Give every minion in the Tavern stats and one random keyword from
    /// `mask`, the same keyword for all of them (Deft Deserter).
    TavernBuffRandomKeyword { attack: i32, health: i32, mask: u8 },
    /// Discover `count` Demons up to the player's tier; the hero then takes
    /// damage equal to the tier of each one chosen (Imposing Percussionist).
    DiscoverDemonTierDamage(u8),
    /// Steal the `count` highest-Attack minions from the Tavern into hand
    /// (Decoy Conjurer). Golden steals two.
    StealTavern { count: u8 },
    /// Discover `count` Tavern spells up to the player's tier (Rodeo
    /// Performer). Golden discovers two.
    DiscoverSpell { count: u8 },
    /// Rally: gain the Attack of the minion this one attacks, for this fight
    /// (Heroic Underdog). A golden one gains double.
    GainTargetAttack,
    /// Rally: remove these keywords from the minion this one attacks, for
    /// this fight (Sin'dorei Straight Shot).
    StripTargetKeywords { mask: u8 },
    /// Deathrattle: destroy the minion that killed this one, ignoring Divine
    /// Shield (Leeroy the Reckless).
    DestroyKiller,
    /// Deathrattle: give `count` friendly Dragons Divine Shield, picking
    /// Dragons without it first (Nadina the Red).
    ShieldDragons { count: u8 },
    /// Get a random minion of exactly this tier (Highkeeper Ra gets Tier 6).
    /// Golden gets two.
    GetRandomTierMinion { tier: u8 },
    /// Give a friendly minion of each type the stats permanently, one random
    /// minion per type and at most one buff per minion (The Last One
    /// Standing). A golden minion does the whole thing twice.
    BuffEachType { attack: i32, health: i32 },
    /// Rally: deal this minion's Attack to the minion it attacks and to one
    /// random minion next to it (both neighbours when golden). Obsidian
    /// Ravager.
    DamageTargetAndNeighbor,
    /// Give this minion's current stats to the left-most minion in hand
    /// (Futurefin). A golden minion gives double.
    GiveOwnStatsToLeftmostHand,
    /// Get a Lockbox, or open the one in hand `sooner` turns sooner (golden
    /// doubles the number of turns, it is still one Lockbox).
    GetLockbox { sooner: u8 },
    /// Get a random Bounty (two when golden).
    GetRandomBounty,
    /// Give all friendly minions stats; golden minions keep them after the
    /// fight (Ship Master Eudora).
    BuffAllKeepGolden { attack: i32, health: i32 },
    /// Give the minions next to this one +attack, once and again for each
    /// friendly Golden minion (Surfing Sylvar).
    BuffAdjacentPerGolden { attack: i32 },
    /// Buff one other friendly minion the player picks (of a tribe), by
    /// `attack`/`health` plus `per_gold` for each Gold spent this turn
    /// (Lovesick Balladist).
    BuffChosenPerGold { tribe: Option<Tribe>, attack: i32, health: i32, per_gold: (i32, i32) },
    /// Make `count` friendly minions of a tribe (if given) of Tier `max_tier`
    /// or lower Golden on the board, picked by the player (Elite Navigator,
    /// Captain Sanders). Golden doubles `count`.
    MakeGoldenChosen { tribe: Option<Tribe>, max_tier: u8, count: u8 },
    /// Get `count` random Golden minions of exactly this tier from the pool;
    /// they give no Triple Reward (Silent Deliverer). Golden doubles it.
    GetGoldenMinion { tier: u8, count: u8 },
    /// Give the other friendly minions of a tribe +attack/+health, times one
    /// plus the Golden minions the player played this game (Hooktusk).
    BuffOthersPerGolden { tribe: Option<Tribe>, attack: i32, health: i32 },
    /// Add to the player's "after the Tavern is Refreshed this game" amount
    /// (Waveling, En-Djinn Blazer, Easterly Winds). Every explicit Refresh
    /// gives the whole amount to one random Tavern minion. Golden doubles it.
    RefreshBuff { attack: i32, health: i32 },
    /// Gain free Refreshes, which are used before Gold and carry over turns
    /// (Refreshing Anomaly). Golden doubles it.
    FreeRefreshes(u8),
    /// Give friendly minions stats multiplied by one plus the player's count
    /// of `counter`, then raise the count (the Ballers: "Improve your future
    /// Ballers"). Golden doubles the stats, not the count.
    BuffAllImproving { attack: i32, health: i32, counter: Counter },
    /// Activate: this minion gains the stats of the next minion its owner
    /// buys this turn (Living Prison). Armed in `improvements`.
    ArmNextBuy,
    /// Gain the stats of the highest-Health minion in the Tavern (Unbound
    /// Tempest). Golden gains double.
    GainHighestTavernStats,
    /// Give the other friendly minions of a tribe the stats, times one plus
    /// this minion's `improvements` (Ultraviolet Ascendant). Golden doubles.
    BuffOthersPerImprovement { tribe: Option<Tribe>, attack: i32, health: i32 },
}

impl Effect {
    /// Whether the player has to pick something to finish this effect. Event
    /// triggers and end of turn effects can't ask, so they can't use these.
    pub fn needs_choice(self) -> bool {
        matches!(
            self,
            Effect::BuffChosen { .. }
                | Effect::ReplaceWithFishbait { .. }
                | Effect::Consume { by: ConsumeBy::Chosen(_), .. }
                | Effect::Consume { pick: ConsumePick::Chosen, .. }
                | Effect::TriggerRally
                | Effect::TriggerBattlecry
                | Effect::DiscoverDemonTierDamage(_)
                | Effect::BuffAnyMinion { .. }
                | Effect::SetStatsAnyMinion { .. }
                | Effect::BuffChosenPerGold { .. }
                | Effect::MakeGoldenChosen { .. }
        )
    }

    /// Whether the effect needs the player to pick a minion on the board or
    /// in the Tavern ("another minion").
    pub fn needs_any_minion(self) -> bool {
        matches!(self, Effect::BuffAnyMinion { .. } | Effect::SetStatsAnyMinion { .. })
    }

    /// Whether the effect takes a Tavern minion into hand, so it needs one
    /// in the Tavern and room in hand.
    pub fn needs_shop_minion(self) -> bool {
        matches!(self, Effect::StealTavern { .. })
    }

    /// Whether the effect needs a friendly minion with a Rally to be legal
    /// (Sky-hatch Runaway).
    pub fn needs_rally_target(self) -> bool {
        matches!(self, Effect::TriggerRally)
    }

    /// Whether the effect needs another friendly minion with a Battlecry to
    /// be legal (Kelp Keeper).
    pub fn needs_battlecry_target(self) -> bool {
        matches!(self, Effect::TriggerBattlecry)
    }
}

impl Effect {
    /// Whether a fight can run this effect: Rally, "whenever a friendly minion
    /// attacks", Deathrattle, Avenge and damage milestone effects must be.
    pub fn runs_in_fights(self) -> bool {
        match self {
            Effect::BuffOthers { per_keyword, .. } => per_keyword == (0, 0),
            Effect::Summon { .. }
            | Effect::BuffAll { .. }
            | Effect::BuffSelf { .. }
            | Effect::BuffRandomFriendly { .. }
            | Effect::BuffKiller { .. }
            | Effect::GainKeyword { .. }
            | Effect::GetRandomMinion { .. }
            | Effect::SummonRandomDeathrattle { .. }
            | Effect::TriggerDeathrattle { .. }
            | Effect::BeetleBuff { .. }
            | Effect::RandomVolumizer
            | Effect::GetCard { .. }
            | Effect::SpellBonus { .. }
            | Effect::RandomChromadrake
            | Effect::CastSpell { .. }
            | Effect::BuffAttacker { .. }
            | Effect::BuffRandomOther { .. }
            | Effect::SummonFromHand { .. }
            | Effect::BuffRandomHand { .. }
            | Effect::AddFodderRefreshes(_)
            | Effect::GainTargetAttack
            | Effect::StripTargetKeywords { .. }
            | Effect::DestroyKiller
            | Effect::ShieldDragons { .. }
            | Effect::GetRandomTierMinion { .. }
            | Effect::BuffEachType { .. }
            | Effect::GetLockbox { .. }
            | Effect::GetRandomBounty
            | Effect::BuffAllKeepGolden { .. }
            | Effect::RefreshBuff { .. }
            | Effect::BuffOthersPerImprovement { .. }
            | Effect::DamageTargetAndNeighbor => true,
            _ => false,
        }
    }

    /// Whether the effect picks one of the player's Tavern cards, so it can
    /// only be activated while there is one.
    pub fn needs_shop_slot(self) -> bool {
        matches!(self, Effect::ReplaceWithFishbait { .. } | Effect::StealTavern { .. })
    }
}

/// An Activate ability: pay gold, run the effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Activate {
    pub cost: u8,
    pub effect: Effect,
}

/// Things a player does that event triggers count.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EventKind {
    SpellsCast,
    /// Counts gold, so spending 5 adds 5.
    GoldSpent,
    /// Counts damage, so taking 3 adds 3.
    HeroDamage,
    MinionsSold,
    CardsBought,
    /// Elementals played from hand (Unbound Tempest).
    ElementalsPlayed,
}

/// "After N events, do the effect (N left!)". The count lives in the
/// minion's `improvements`. Going past N keeps the remainder, so one big
/// event can fire it more than once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EventTrigger {
    pub kind: EventKind,
    pub every: u8,
    pub effect: Effect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DiscoverKind {
    Tier1Minion,
    MyTierMinion,
    MyTierSpell,
    DeathrattleMinion,
    BattlecryMinion,
}

/// A Tavern spell (or Choose One spell option) that targets a friendly minion.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SpellTarget {
    Buff {
        attack: i32,
        health: i32,
        keyword: u8,
        times: u8,
        if_win_next_turn_buff: Option<(i32, i32)>,
    },
    BuffSameTribe {
        attack: i32,
        health: i32,
    },
    SameTribeMinion,
    SetStats {
        attack: i32,
        health: i32,
    },
    MakeGolden {
        max_tier: u8,
    },
    /// Give the minion stats, once more for each different Bonus Keyword in
    /// the warband (Seafood Stew).
    BuffPerBonusKeyword {
        attack: i32,
        health: i32,
    },
    /// The chosen minion (of a tribe, if given) consumes Tavern minions
    /// (Methodical Madness, Corrupted Cupcakes).
    Consume {
        tribe: Option<Tribe>,
        pick: ConsumePick,
        keywords: bool,
        count: u8,
    },
    /// +attack/+health and Taunt, or remove Taunt if it already has it
    /// (Tricky Trousers).
    TrickyTrousers { attack: i32, health: i32 },
    /// Transform the minion into a random minion exactly one tier higher,
    /// keeping only its Attack and Health (Robust Evolution).
    RobustEvolution,
    /// Tavern minions of the chosen minion's type get stats this game
    /// (Eonar's Favor). A dual or all-type one picks one type at random.
    TavernBuffType { attack: i32, health: i32 },
    /// Refresh the Tavern with minions of the chosen minion's type (Lost
    /// Staff of Hamuul).
    RefreshWithType,
    /// Sell the chosen friendly minion and give its stats to a random other
    /// friendly minion (Channel the Devourer).
    Devourer,
    /// Give the chosen friendly minion of a tribe half (rounded up) the stats
    /// of the highest-Health minion in the Tavern (Arcane Absorption).
    HalfOfHighestTavern { tribe: Tribe },
    /// Give a minion (board or Tavern) stats, plus `per` for each minion of
    /// `tribe` its owner played this turn (Conflagration).
    BuffPerTribePlayed { tribe: Tribe, attack: i32, health: i32, per: (i32, i32) },
}

impl SpellTarget {
    /// Whether this target may also be a minion in the Tavern. Spells that
    /// say "a minion" (no "friendly") reach both, per DESIGN.md; "friendly"
    /// means the board only.
    pub fn can_target_shop(self) -> bool {
        matches!(
            self,
            SpellTarget::Buff { .. }
                | SpellTarget::BuffSameTribe { .. }
                | SpellTarget::SameTribeMinion
                | SpellTarget::SetStats { .. }
                | SpellTarget::BuffPerBonusKeyword { .. }
                | SpellTarget::TrickyTrousers { .. }
                | SpellTarget::RobustEvolution
                | SpellTarget::TavernBuffType { .. }
                | SpellTarget::RefreshWithType
                | SpellTarget::BuffPerTribePlayed { .. }
        )
    }

    /// Whether the chosen minion must have a type (a neutral can't be
    /// picked).
    pub fn needs_type(self) -> bool {
        matches!(
            self,
            SpellTarget::BuffSameTribe { .. }
                | SpellTarget::SameTribeMinion
                | SpellTarget::TavernBuffType { .. }
                | SpellTarget::RefreshWithType
        )
    }
}

/// A Tavern spell (or Choose One spell option) that does not target a minion.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SpellEffect {
    GainGold(u8),
    GoldNextTurn(u8),
    MaxGold(u8),
    FreeRolls(u8),
    Overconfidence { win: u8, tie: u8 },
    BuffShop { attack: i32, health: i32 },
    BuffBoard { attack: i32, health: i32, times: u8, golden_extra: bool },
    BuffRandomBoard { count: u8, attack: i32, health: i32 },
    NextTurnBoardBuff { attack: i32, health: i32, times: u8 },
    RandomTierMinion(u8),
    Discover(DiscoverKind),
    BoonOfBeetles(u8),
    /// Buff every friendly minion, again for Dragons, again for minions
    /// with Divine Shield.
    MightyDragonbreath { attack: i32, health: i32 },
    BroodOfNozdormu,
    UpperHand,
    /// Make a random minion in the Tavern golden.
    GoldenTouch,
    /// Get a random Murloc and a copy of it (Cloning Conch).
    CloningConch,
    /// Give minions in the Tavern stats for the rest of the game (Staff of
    /// Enrichment).
    BuffTavernPermanent { attack: i32, health: i32 },
    /// For each type on the board, give one random friendly minion of it
    /// +attack/+health, at most one buff per minion (Misplaced Tea Set).
    MisplacedTeaSet { attack: i32, health: i32 },
    /// Discover a minion of the most common type on the board (Planar
    /// Telescope).
    PlanarTelescope,
    /// Steal a random minion from the Tavern into hand (Enchanted Lasso).
    StealRandomTavern,
    /// Discover a minion of exactly the player's tier and lock it in hand
    /// for one turn (Search Through Time).
    DiscoverLockedMinion,
    /// Refresh the Tavern with Tavern spells (Saloon's Finest).
    SaloonFinest,
    /// Get `count` copies of a card (Weapons Forge).
    GetCard { card: CardId, count: u8 },
    /// An unplayable Lockbox that counts down from this many turns (its
    /// count lives in the hand minion's `improvements`).
    Lockbox(u8),
    /// Give the left-most minion on the board stats (Selfish Bounty).
    BuffLeftmost { attack: i32, health: i32 },
    /// Get a random minion of the most common type on the board (Friendly
    /// Bounty).
    FriendlyBounty,
    /// Add to the "after the Tavern is Refreshed this game" amount (Easterly
    /// Winds).
    RefreshBuff { attack: i32, health: i32 },
    /// Get one of each of two cards (Temperature Shift).
    GetTwo { a: CardId, b: CardId },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChooseOption {
    MinionTarget {
        tribe: Option<Tribe>,
        attack: i32,
        health: i32,
        keyword: u8,
    },
    SpellBonus {
        attack: i32,
        health: i32,
    },
    SpellTarget(SpellTarget),
    SpellEffect(SpellEffect),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Deathrattle {
    Summon {
        card: CardId,
        count: u8,
        /// When true, a golden minion summons twice as many plain tokens
        /// (Beetles); when false, it summons `count` golden tokens.
        golden_doubles_count: bool,
        beetle_buff: Option<(i32, i32)>,
    },
    SummonByAttack {
        card: CardId,
    },
    CombatBeastBuff {
        attack: i32,
        health: i32,
    },
    RandomVolumizer,
    GetCard {
        card: CardId,
        count: u8,
    },
    /// Any effect a fight can run.
    Effect(Effect),
    /// Give the Tavern minions (of the fight's owner) +attack/+health for the
    /// rest of the game, once more for each `improvements` the dead minion
    /// has (Sacrificial Wrathguard). Golden doubles it.
    TavernBuff {
        attack: i32,
        health: i32,
        per_improvement: (i32, i32),
    },
}

/// "After you play a minion of `tribe`, ..." for the minion that has it.
/// Every effect runs for it; a golden one doubles each.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AfterPlayTribe {
    /// The card being played never counts for itself, and this runs right
    /// after it lands, before its Battlecry (Ultraviolet Ascendant).
    pub exclude_played: bool,
    pub tribe: Tribe,
    pub effects: Vec<Effect>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartOfTurn {
    GainGold(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndOfTurn {
    BuffSelf { attack: i32, health: i32 },
    ResourcefulRobot,
    BuffPerMagnetization { attack: i32, health: i32 },
    ImproveTier,
    GetCard { card: CardId, count: u8 },
    CopyLastSpell,
    /// Any generic effect (one that needs no choice).
    Effect(Effect),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartOfCombat {
    CombatBeastBuff { attack: i32, health: i32 },
    DamageAllOthers(i32),
    /// Any effect a fight can run (Electric Synthesizer, Amber Guardian,
    /// Runic Arcanist).
    Effect(Effect),
    /// Attack at once, before the normal attack rotation (Heroic Broodmother).
    AttackImmediately,
    /// Give your minions stats, improved by a per-minion count that goes up
    /// every time a friendly minion loses Divine Shield (Hopebringer).
    Hopebringer { attack: i32, health: i32, improve: (i32, i32) },
    /// If this minion is in your hand, summon a copy of it (Flighty Scout).
    /// A golden one summons a golden copy, which has double its stats.
    FlightyScout,
    /// Gain the Attack of the highest-Attack minion in your hand, for this
    /// combat (Costume Enthusiast). A golden one gains double.
    GainHighestHandAttack,
    /// Gain the stats of every minion in your hand, for this combat (Choral
    /// Mrrrglr). A golden one counts the hand twice.
    GainHandStats,
}

/// "While this is in your hand, after you play a minion of a tribe, ...".
/// The buff is on the hand card itself and is permanent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WhileInHand {
    pub tribe: Tribe,
    pub attack: i32,
    pub health: i32,
}

/// "Once this reaches `total` Attack, gain `keyword`" (Scarlet Survivor).
/// The minion's `improvements` doubles as the "already fired" flag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AttackThreshold {
    pub total: u16,
    pub keyword: u8,
}

/// What a minion keeps from combat, permanently.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermanentKeep {
    /// This minion keeps its own stat and Bonus Keyword gains (Tarecgosa).
    SelfKeep,
    /// Adjacent Dragons keep theirs (Persistent Poet).
    AdjacentDragons,
}

/// Which attacking minions an attack trigger reacts to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AttackFilter {
    /// Only attackers of this tribe.
    pub tribe: Option<Tribe>,
    /// Only attackers with a Rally effect.
    pub rally: bool,
    /// Another minion: the minion with the trigger doesn't count (Roaring
    /// Recruiter).
    pub exclude_self: bool,
}

/// "Whenever" or "after a friendly minion attacks" (of a kind): run the
/// effect for the minion that has it. Any effect a fight can run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AttackTrigger {
    pub filter: AttackFilter,
    pub effect: Effect,
}

/// "Avenge (N)": after every N friendly minions die in a fight, run the
/// effect. The count starts at 0 in every fight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Avenge {
    pub every: u8,
    pub effect: Effect,
}

/// "Once this deals N damage, ...": the damage dealt so far lives in the
/// minion's `improvements`, and the effect runs once, when it passes `total`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DamageMilestone {
    pub total: u16,
    pub effect: Effect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnSummonBeast {
    Buff { attack: i32, health: i32, improve_permanently: bool },
    MultiplyAttackInCombat,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnPlayMech {
    MagnetizeSatellite { card: CardId },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnPlayOrMagnetizeMech {
    Buff { attack: i32, health: i32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnCastSpellOnMech {
    MagnetizeSatellite { card: CardId, attack: i32, health: i32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnCastSpell {
    BuffDivineShield { attack: i32, health: i32 },
    /// Buff every friendly minion. A golden minion does it twice rather
    /// than twice as much (Timecap'n Hooktail).
    BuffAll { attack: i32, health: i32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnSell {
    GetCard { card: CardId, count: u8 },
    Effect(Effect),
    DiscoverByTier,
    GoldIfLost(u8),
    /// If the minion is Golden when sold, Discover a Tier 7 minion (Greedy
    /// Conniver).
    DiscoverTier7IfGolden,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Aura {
    Brann,
    Titus,
    Drakkari,
    Balinda,
    /// Has Taunt while it has Divine Shield.
    TauntWhileShielded,
    /// Has +30/+30 while its owner's hero has 15 or less Health (Holy
    /// Vanguard). A golden one has +60/+60.
    HolyVanguard,
    /// Whenever its owner Magnetizes to a different minion, the same
    /// Magnetic also Magnetizes to this one (twice when golden). Polarizing
    /// Beatboxer.
    PolarizingBeatboxer,
    /// Your Bounties cast twice (three times when golden). Proud Privateer.
    ProudPrivateer,
}

/// "After you buy a minion, give it +attack/+health and multiply its stats,
/// once per turn" (Stone Age Slab). A golden minion multiplies by
/// `golden_multiply`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AfterBuy {
    pub attack: i32,
    pub health: i32,
    pub multiply: i32,
    pub golden_multiply: i32,
}

/// "After you summon a minion in combat, give it this minion's current
/// Attack and maximum Health, `times` times per combat" (Stalwart Kodo).
/// A golden minion gives double.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AfterSummonGiveStats {
    pub times: u8,
}

#[derive(Clone, Debug)]
pub struct Card {
    pub name: String,
    pub tier: u8,
    pub tribes: Tribes,
    pub attack: i32,
    pub health: i32,
    /// Golden stats, when the data doesn't double the printed ones.
    pub golden_stats: Option<(i32, i32)>,
    pub keywords: Keywords,
    pub token: bool,
    pub spell: bool,
    /// Bought with Health instead of Gold (Hasty Excavation).
    pub pay_health: bool,
    pub cost: u8,
    pub is_beetle: bool,
    pub is_volumizer: bool,
    /// Index in `Game::pool` that holds copies of this card. Shared pool
    /// groups (such as the three Volumizers) all point to their first card's
    /// slot.
    pub pool_slot: CardId,
    pub volumizer_buff: Option<(i32, i32)>,
    pub spell_buff: Option<(i32, i32)>,
    pub battlecry: Option<Effect>,
    pub choose_one: Option<[ChooseOption; 2]>,
    pub deathrattle: Option<Deathrattle>,
    pub start_of_turn: Option<StartOfTurn>,
    pub end_of_turn: Option<EndOfTurn>,
    pub start_of_combat: Option<StartOfCombat>,
    pub rally: Option<Effect>,
    pub on_summon_beast: Option<OnSummonBeast>,
    pub on_play_mech: Option<OnPlayMech>,
    pub on_play_or_magnetize_mech: Option<OnPlayOrMagnetizeMech>,
    pub on_cast_spell_on_mech: Option<OnCastSpellOnMech>,
    pub on_cast_spell: Option<OnCastSpell>,
    /// Whenever the owner casts a spell on this minion (Twilight Tidehunter).
    pub on_cast_spell_on_self: Option<Effect>,
    /// Whenever the owner casts a spell on a minion of `tribe` (Shamanic
    /// Tidecaller).
    pub on_cast_spell_on_tribe: Option<(Tribe, Effect)>,
    /// Whenever this minion takes damage in a fight (Very Hungry
    /// Winterfinner).
    pub when_damaged: Option<Effect>,
    /// While this is in its owner's hand, after they play a minion of a
    /// tribe (Bream Counter).
    pub while_in_hand: Option<WhileInHand>,
    /// After the owner buys a Tavern spell, get a taught 1/1 Murloc token
    /// (Magicfin Mycologist).
    pub teaches_spell: bool,
    /// Tokens that can't be tripled (Magicfin Apprentice).
    pub no_triple: bool,
    /// The 1/1 Murloc token Magicfin Mycologist teaches a spell to.
    pub taught_token: bool,
    pub whenever_friendly_attacks: Option<AttackTrigger>,
    pub after_friendly_attacks: Option<AttackTrigger>,
    pub avenge: Option<Avenge>,
    pub damage_milestone: Option<DamageMilestone>,
    /// Gets this much for every Deathrattle its owner has triggered this
    /// game, wherever it is.
    pub buff_per_deathrattle: Option<(i32, i32)>,
    /// Tavern buffs don't change its stats.
    pub fixed_stats: bool,
    pub activate: Option<Activate>,
    pub on_event: Option<EventTrigger>,
    /// What happens after you trigger a Battlecry (Kalecgos).
    pub after_battlecry: Option<Effect>,
    /// Keeps combat gains permanently (Tarecgosa, Persistent Poet).
    pub permanent_keep: Option<PermanentKeep>,
    /// Gains a keyword once its Attack reaches a threshold (Scarlet
    /// Survivor).
    pub attack_threshold: Option<AttackThreshold>,
    /// Effects that run in order after your hero takes damage.
    pub after_hero_damage: Vec<Effect>,
    /// Effects that run after the owner plays a minion of a tribe (Wrath
    /// Weaver, Insatiable Ur'zul).
    pub after_play_tribe: Option<AfterPlayTribe>,
    /// Stats this minion gains permanently when another friendly Demon deals
    /// damage in a fight (Devout Hellcaller).
    pub after_demon_damage: Option<(i32, i32)>,
    /// A Demon Fodder: on the Refresh it appears in, it feeds itself to a
    /// random friendly Demon and its slot is refilled, if the player has a
    /// Demon then. Otherwise it waits in the Tavern as a plain 2/2 Demon.
    pub is_fodder: bool,
    /// Refreshes each turn that cost Health instead of Gold.
    pub health_refreshes: u8,
    pub on_sell: Option<OnSell>,
    pub aura: Option<Aura>,
    /// Gives each minion its owner summons in combat this minion's stats
    /// (Stalwart Kodo).
    pub after_summon_give_stats: Option<AfterSummonGiveStats>,
    pub spell_target: Option<SpellTarget>,
    pub spell_effect: Option<SpellEffect>,
    /// Always Golden, with its printed stats (Aureate Laureate): it is made
    /// Golden and gives no Triple Reward.
    pub always_golden: bool,
    /// Also damages the enemies next to the one it attacks (Blade Collector).
    pub cleave: bool,
    /// Gets this much for every Golden minion its owner played this game,
    /// wherever it is (Maritime Extortionist).
    pub buff_per_golden_played: Option<(i32, i32)>,
    /// After its owner Discovers a card (Hooktusk).
    pub after_discover: Option<Effect>,
    /// One of the five Bounties, which share one slot of the spell draw.
    pub is_bounty: bool,
    /// After this attacks and kills a minion, it deals the excess damage to an
    /// adjacent enemy, both when golden (Wildfire Elemental).
    pub overkill: bool,
    /// Can stand in for any minion of this tribe in a triple (Elemental of
    /// Surprise).
    pub triple_wildcard: Option<Tribe>,
    /// Whenever its owner plays a minion of this tribe from hand, it gets
    /// Divine Shield until the next shop phase, for good when this is golden
    /// (Ichoron the Protector).
    pub play_shield: Option<Tribe>,
    /// After its owner buys a minion (Stone Age Slab).
    pub after_buy: Option<AfterBuy>,
}

impl Card {
    /// Whether the card has this tribe, always true for no filter.
    pub fn is(&self, tribe: Option<Tribe>) -> bool {
        tribe.is_none_or(|t| self.tribes.has(t))
    }

    pub fn is_spell(&self) -> bool {
        self.spell
    }

    /// Whether an "any minion" draw may show this card: always without a
    /// sandbox rig, otherwise only the rigged tribe and neutral minions.
    pub fn rig_ok(&self, rig: Option<Tribe>) -> bool {
        rig.is_none_or(|t| self.tribes.is_neutral() || self.tribes.has(t))
    }

    /// Whether this spell gives stats to minions (used by the rule bot).
    pub fn gives_stats(&self) -> bool {
        matches!(
            self.spell_target,
            Some(
                SpellTarget::Buff { .. }
                    | SpellTarget::BuffSameTribe { .. }
                    | SpellTarget::BuffPerBonusKeyword { .. }
                    | SpellTarget::BuffPerTribePlayed { .. }
                    | SpellTarget::HalfOfHighestTavern { .. }
            )
        )
            || matches!(
                self.spell_effect,
                Some(
                    SpellEffect::BuffBoard { .. }
                        | SpellEffect::BuffRandomBoard { .. }
                        | SpellEffect::BuffShop { .. }
                        | SpellEffect::NextTurnBoardBuff { .. }
                )
            )
            || self.choose_one.is_some_and(|opts| {
                opts.iter().any(|o| {
                    matches!(
                        o,
                        ChooseOption::SpellTarget(SpellTarget::Buff { .. })
                            | ChooseOption::SpellEffect(SpellEffect::BuffBoard { .. })
                    )
                })
            })
    }

    /// Whether this card reads its owner's hand during a fight, so the
    /// fight needs the hand passed in (Expert Aviator, Flighty Scout,
    /// Diremuck Forager, Costume Enthusiast, Choral Mrrrglr, Very Hungry
    /// Winterfinner).
    pub fn reads_hand(&self) -> bool {
        self.when_damaged.is_some()
            || matches!(
                self.start_of_combat,
                Some(
                    StartOfCombat::FlightyScout
                        | StartOfCombat::GainHighestHandAttack
                        | StartOfCombat::GainHandStats
                        | StartOfCombat::Effect(Effect::SummonFromHand { .. })
                )
            )
            || matches!(self.rally, Some(Effect::SummonFromHand { .. }))
    }

    /// Whether a host Mech needs to store this card in `Minion::attached`
    /// when Magnetized. Satellites and Volumizers have no persistent board
    /// triggers after being Magnetized, so they take no slot.
    pub fn carries_effect(&self) -> bool {
        self.deathrattle.is_some()
            || self.start_of_turn.is_some()
            || self.end_of_turn.is_some()
            || self.start_of_combat.is_some()
            || self.rally.is_some()
            || self.on_summon_beast.is_some()
            || self.on_play_mech.is_some()
            || self.on_play_or_magnetize_mech.is_some()
            || self.on_cast_spell_on_mech.is_some()
            || self.on_cast_spell.is_some()
            || self.on_cast_spell_on_self.is_some()
            || self.on_cast_spell_on_tribe.is_some()
            || self.when_damaged.is_some()
            || self.while_in_hand.is_some()
            || self.whenever_friendly_attacks.is_some()
            || self.after_friendly_attacks.is_some()
            || self.avenge.is_some()
            || self.damage_milestone.is_some()
            || self.activate.is_some()
            || self.on_event.is_some()
            || !self.after_hero_damage.is_empty()
            || self.after_play_tribe.is_some()
            || self.after_demon_damage.is_some()
            || self.health_refreshes > 0
            || self.on_sell.is_some()
            || self.after_discover.is_some()
            || self.play_shield.is_some()
            || self.after_buy.is_some()
            || self.aura.is_some()
            || self.after_summon_give_stats.is_some()
            || self.spell_buff.is_some()
    }
}

/// All cards. Ids `0..pool_cards` are the pool minions, tokens and spells
/// come after.
#[derive(Clone, Debug)]
pub struct CardDb {
    pub cards: Vec<Card>,
    pub pool_cards: usize,
    /// Tier of every card by id, kept compact for the pool draws.
    pub tiers: Vec<u8>,
    /// Pool card ids (ascending) of tier at most the index, for the pool draws.
    pub ids_up_to_tier: Vec<Vec<u8>>,
    pub volumizers: Vec<CardId>,
    pub spells: Vec<CardId>,
    pub beetle_token: Option<CardId>,
    /// The five Chromadrakes "a random Chromadrake" picks from.
    pub chromadrakes: Vec<CardId>,
    /// The 1/1 Murloc Magicfin Mycologist teaches a spell to.
    pub taught_token: Option<CardId>,
    /// The generated Demon Fodder token.
    pub fodder_token: Option<CardId>,
    /// The five Bounties. Only the first is in `spells`: the five are one
    /// slot of every spell draw, and a drawn Bounty is one of them at random.
    pub bounties: Vec<CardId>,
    /// The generated Lockbox spell.
    pub lockbox: Option<CardId>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    minion: Vec<RawCard>,
    #[serde(default)]
    token: Vec<RawCard>,
    #[serde(default)]
    spell: Vec<RawSpell>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCard {
    name: String,
    tier: u8,
    /// One tribe, "neutral" or "all".
    tribe: Option<String>,
    /// Two or more tribes, for multi-tribe cards.
    tribes: Option<Vec<String>>,
    stats: [i32; 2],
    /// Golden stats when they are not exactly double the printed ones (the
    /// data has one such card).
    golden_stats: Option<[i32; 2]>,
    #[serde(default)]
    keywords: Vec<String>,
    #[serde(default)]
    beetle: bool,
    volumizer_buff: Option<[i32; 2]>,
    spell_buff: Option<[i32; 2]>,
    battlecry: Option<RawEffect>,
    choose_one: Option<[RawMinionChooseOption; 2]>,
    deathrattle: Option<RawDeathrattle>,
    start_of_turn: Option<RawStartOfTurn>,
    end_of_turn: Option<RawEndOfTurn>,
    start_of_combat: Option<RawStartOfCombat>,
    rally: Option<RawEffect>,
    on_summon_beast: Option<RawOnSummonBeast>,
    on_play_mech: Option<RawOnPlayMech>,
    on_play_or_magnetize_mech: Option<RawBuff>,
    on_cast_spell_on_mech: Option<RawOnCastSpellOnMech>,
    on_cast_spell: Option<RawOnCastSpell>,
    on_cast_spell_on_self: Option<RawEffect>,
    on_cast_spell_on_tribe: Option<RawOnCastSpellOnTribe>,
    when_damaged: Option<RawEffect>,
    while_in_hand: Option<RawWhileInHand>,
    #[serde(default)]
    teaches_spell: bool,
    #[serde(default)]
    no_triple: bool,
    #[serde(default)]
    taught_token: bool,
    whenever_friendly_attack: Option<RawAttackTrigger>,
    after_friendly_attack: Option<RawAttackTrigger>,
    avenge: Option<RawAvenge>,
    damage_milestone: Option<RawMilestone>,
    buff_per_deathrattle: Option<[i32; 2]>,
    #[serde(default)]
    fixed_stats: bool,
    activate: Option<RawActivate>,
    on_event: Option<RawOnEvent>,
    after_battlecry: Option<RawEffect>,
    /// "self" or "adjacent_dragons".
    permanent_keep: Option<String>,
    attack_threshold: Option<RawAttackThreshold>,
    /// One of the five generated Chromadrakes.
    #[serde(default)]
    chromadrake: bool,
    #[serde(default)]
    after_hero_damage: Vec<RawEffect>,
    after_play_tribe: Option<RawAfterPlayTribe>,
    after_demon_damage: Option<[i32; 2]>,
    /// A generated Demon Fodder token.
    #[serde(default)]
    fodder: bool,
    health_refreshes: Option<u8>,
    on_sell: Option<RawOnSell>,
    aura: Option<String>,
    after_summon_give_stats: Option<RawAfterSummonGiveStats>,
    #[serde(default)]
    always_golden: bool,
    #[serde(default)]
    cleave: bool,
    buff_per_golden_played: Option<[i32; 2]>,
    after_discover: Option<RawEffect>,
    #[serde(default)]
    overkill: bool,
    /// Tribe a triple wildcard stands in for.
    triple_wildcard: Option<String>,
    /// Tribe whose members get Divine Shield when played.
    play_shield: Option<String>,
    after_buy: Option<RawAfterBuy>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAfterBuy {
    buff: [i32; 2],
    multiply: i32,
    golden_multiply: i32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAfterSummonGiveStats {
    times: u8,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAttackThreshold {
    total: u16,
    keyword: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSpell {
    name: String,
    tier: u8,
    cost: u8,
    /// A spell that is never in the Tavern's spell pool (Pointy Arrow).
    #[serde(default)]
    token: bool,
    /// Bought with Health instead of Gold (Hasty Excavation).
    #[serde(default)]
    pay_health: bool,
    /// One of the five Bounties (they share one slot of the spell draw).
    #[serde(default)]
    bounty: bool,
    effect: Option<RawSpellEffect>,
    choose_one: Option<[RawSpellEffect; 2]>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAfterPlayTribe {
    #[serde(default)]
    exclude_played: bool,
    tribe: String,
    effects: Vec<RawEffect>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSpellEffect {
    target: Option<String>,
    /// "chosen", "random" or "highest_health", for a spell whose target
    /// consumes Tavern minions.
    consume: Option<String>,
    #[serde(default)]
    with_keywords: bool,
    /// The tribe the spell's target must have.
    tribe: Option<String>,
    buff: Option<[i32; 2]>,
    keyword: Option<String>,
    times: Option<u8>,
    count: Option<u8>,
    #[serde(default)]
    golden_extra: bool,
    if_win_next_turn_buff: Option<[i32; 2]>,
    next_turn_board_buff: Option<[i32; 2]>,
    same_tribe_minion: Option<bool>,
    set_stats: Option<[i32; 2]>,
    make_golden_max_tier: Option<u8>,
    gold: Option<u8>,
    gold_next_turn: Option<u8>,
    max_gold: Option<u8>,
    free_rolls: Option<u8>,
    overconfidence: Option<[u8; 2]>,
    random_tier_minion: Option<u8>,
    discover: Option<String>,
    boon_of_beetles: Option<u8>,
    start_of_combat: Option<String>,
    golden_touch: Option<bool>,
    /// "Give your minions..., repeat for your Dragons, repeat for your
    /// minions with Divine Shield."
    mighty_dragonbreath: Option<[i32; 2]>,
    /// "Give a minion stats, repeat for each different Bonus Keyword in your
    /// warband."
    #[serde(default)]
    per_bonus_keyword: bool,
    /// Get a random Murloc and a copy of it.
    #[serde(default)]
    cloning_conch: bool,
    /// Give minions in the Tavern stats for the rest of the game.
    tavern_buff_permanent: Option<[i32; 2]>,
    /// Misplaced Tea Set: one minion per type on the board.
    misplaced_tea_set: Option<[i32; 2]>,
    #[serde(default)]
    planar_telescope: bool,
    #[serde(default)]
    steal_random_tavern: bool,
    #[serde(default)]
    discover_locked_minion: bool,
    #[serde(default)]
    saloon_finest: bool,
    /// Get `count` copies of this generated card.
    get: Option<String>,
    /// The chosen minion: toggle Taunt with these stats.
    tricky_trousers: Option<[i32; 2]>,
    #[serde(default)]
    robust_evolution: bool,
    /// Eonar's Favor: Tavern minions of the chosen minion's type.
    tavern_buff_type: Option<[i32; 2]>,
    #[serde(default)]
    refresh_with_type: bool,
    #[serde(default)]
    devourer: bool,
    /// An unplayable Lockbox that opens after this many turns.
    lockbox: Option<u8>,
    /// Give the left-most minion on the board stats (Selfish Bounty).
    buff_leftmost: Option<[i32; 2]>,
    #[serde(default)]
    friendly_bounty: bool,
    /// Add to the "after the Tavern is Refreshed" amount.
    refresh_buff: Option<[i32; 2]>,
    /// Get one of each of two named cards.
    get_pair: Option<[String; 2]>,
    /// Give the chosen friendly minion (of `tribe`) half the stats of the
    /// highest-Health Tavern minion.
    #[serde(default)]
    half_of_highest: bool,
    /// Buff by `buff`, plus this per minion of `tribe` played this turn.
    per_played_this_turn: Option<[i32; 2]>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEffect {
    buff: Option<[i32; 2]>,
    /// "chosen", "others" or "self", for a buff.
    target: Option<String>,
    tribe: Option<String>,
    /// Added once more per different Bonus Keyword in the warband.
    per_keyword: Option<[i32; 2]>,
    #[serde(default)]
    random_keyword: bool,
    /// "chosen", "random" or "highest_health".
    consume: Option<String>,
    /// The consumer is a friendly minion the player picks (of `tribe`).
    #[serde(default)]
    consume_by_chosen: bool,
    #[serde(default)]
    with_keywords: bool,
    tavern_buff: Option<[i32; 2]>,
    #[serde(default)]
    this_turn: bool,
    damage_hero: Option<i32>,
    #[serde(default)]
    rewind: bool,
    #[serde(default)]
    magnetize_volumizer: bool,
    beetle_buff: Option<[i32; 2]>,
    gold_next_turn: Option<u8>,
    random_volumizer: Option<bool>,
    spell_discount: Option<u8>,
    /// Raise the player's Tavern spell bonus this game.
    spell_bonus: Option<[i32; 2]>,
    #[serde(default)]
    random_chromadrake: bool,
    /// Cast a named Tavern spell, `count` times.
    cast: Option<String>,
    #[serde(default)]
    trigger_rally: bool,
    buff_attacker: Option<[i32; 2]>,
    /// Buff random other friendly minions of `tribe`, with `keyword`.
    buff_random_other: Option<[i32; 2]>,
    keyword: Option<String>,
    get: Option<String>,
    count: Option<u8>,
    /// Summon a card (token or pool minion) next to the minion.
    summon: Option<String>,
    /// Summon the highest-Attack minions of this tribe from hand, for this
    /// combat only.
    summon_from_hand: Option<String>,
    #[serde(default)]
    golden_doubles_count: bool,
    /// Get a random pool minion of this tribe.
    random_minion: Option<String>,
    /// Name of the counter this buff scales by and then improves.
    improve_counter: Option<String>,
    /// Buff the minion that killed this one.
    buff_killer: Option<[i32; 2]>,
    gain_keyword: Option<String>,
    /// Get `count` random Tavern spells with this cost.
    random_spell_cost: Option<u8>,
    summon_random_deathrattle: Option<u8>,
    #[serde(default)]
    trigger_deathrattle: bool,
    /// Pick a Tavern card to replace with this card, for the left-most Beast
    /// to attack.
    replace_with: Option<String>,
    /// Refresh the Tavern with this card in it, for the left-most Beast to
    /// attack.
    refresh_with: Option<String>,
    extra_magnetization: Option<u8>,
    /// Give a random minion in hand stats (Winterfinner).
    buff_random_hand: Option<[i32; 2]>,
    /// Give the left-most minion in hand stats (Twilight Tidehunter).
    buff_leftmost_hand: Option<[i32; 2]>,
    /// Trigger a friendly minion's Battlecry, chosen by the player.
    #[serde(default)]
    trigger_battlecry: bool,
    /// If you control another minion of this tribe, Discover `count` of them.
    discover_tribe_if_another: Option<String>,
    /// Cast the Tavern spell this minion was taught.
    #[serde(default)]
    cast_taught: bool,
    /// Trigger the Battlecries of the adjacent minions, `count` times.
    trigger_adjacent_battlecries: Option<u8>,
    /// Add Fodders to the next `n` Refreshes.
    add_fodder_refreshes: Option<u8>,
    /// Add Fodders to the very next Refresh.
    add_fodder_to_next_refresh: Option<u8>,
    /// Your Demons each consume `n` random Tavern minions.
    demons_consume: Option<u8>,
    /// Raise this minion's `improvements` by `n`.
    improve_self: Option<u8>,
    /// Give the Tavern stats and one random keyword out of `keywords`.
    tavern_buff_random_keyword: Option<[i32; 2]>,
    keywords: Option<Vec<String>>,
    /// Discover `n` Demons up to the owner's tier, then damage the hero for
    /// their tiers.
    discover_demon_tier_damage: Option<u8>,
    /// Set a minion's (board or Tavern) stats (Tyrael).
    set_stats_any: Option<[i32; 2]>,
    /// Steal the `n` highest-Attack minions from the Tavern (Decoy Conjurer).
    steal_tavern: Option<u8>,
    /// Discover `n` Tavern spells up to the owner's tier (Rodeo Performer).
    discover_spell: Option<u8>,
    /// Rally: gain the target's Attack (Heroic Underdog).
    #[serde(default)]
    gain_target_attack: bool,
    /// Rally: remove these keywords from the target (Sin'dorei Straight
    /// Shot).
    strip_keywords: Option<Vec<String>>,
    /// Deathrattle: destroy the minion that killed this one (Leeroy the
    /// Reckless).
    #[serde(default)]
    destroy_killer: bool,
    /// Deathrattle: give `n` friendly Dragons Divine Shield (Nadina the
    /// Red).
    shield_dragons: Option<u8>,
    /// Get a random minion of exactly this tier (Highkeeper Ra).
    random_tier_minion: Option<u8>,
    /// Give a friendly minion of each type stats permanently (The Last One
    /// Standing).
    buff_each_type: Option<[i32; 2]>,
    /// Rally: damage the target and a neighbour (Obsidian Ravager).
    #[serde(default)]
    damage_target_and_neighbor: bool,
    /// Give this minion's stats to the left-most minion in hand (Futurefin).
    #[serde(default)]
    give_stats_leftmost_hand: bool,
    /// Get a Lockbox, or open the held one this many turns sooner.
    lockbox_sooner: Option<u8>,
    #[serde(default)]
    random_bounty: bool,
    /// Give all friendly minions the buff; golden minions keep it.
    keep_golden: Option<[i32; 2]>,
    /// Give adjacent minions this much Attack, repeated per Golden minion.
    adjacent_per_golden: Option<i32>,
    /// Added once more for each Gold spent this turn.
    per_gold_spent: Option<[i32; 2]>,
    /// Make friendly minions up to this tier Golden on the board.
    make_golden_max_tier: Option<u8>,
    /// Get random Golden minions of exactly this tier.
    golden_minion_tier: Option<u8>,
    /// Scale a buff by one plus the Golden minions played this game.
    #[serde(default)]
    per_golden_played: bool,
    /// Add to the "after the Tavern is Refreshed this game" amount.
    refresh_buff: Option<[i32; 2]>,
    free_refreshes: Option<u8>,
    /// Multiply a buff by one plus the named counter, then raise it.
    #[serde(default)]
    improving_counter: Option<String>,
    /// Arm this minion to gain the stats of the next minion bought.
    #[serde(default)]
    arm_next_buy: bool,
    #[serde(default)]
    gain_highest_tavern_stats: bool,
    /// Multiply a buff by one plus this minion's `improvements`.
    #[serde(default)]
    per_improvement: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMinionChooseOption {
    buff: Option<[i32; 2]>,
    keyword: Option<String>,
    tribe: Option<String>,
    spell_buff: Option<[i32; 2]>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDeathrattle {
    summon: Option<String>,
    count: Option<u8>,
    #[serde(default)]
    golden_doubles_count: bool,
    beetle_buff: Option<[i32; 2]>,
    summon_by_attack: Option<String>,
    combat_beast_buff: Option<[i32; 2]>,
    random_volumizer: Option<bool>,
    get: Option<String>,
    effect: Option<RawEffect>,
    /// Give the Tavern stats for the rest of the game, improved by the dead
    /// minion's `improvements`.
    tavern_buff: Option<[i32; 2]>,
    per_improvement: Option<[i32; 2]>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStartOfTurn {
    gold: u8,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEndOfTurn {
    buff_self: Option<[i32; 2]>,
    resourceful_robot: Option<bool>,
    buff_per_magnetization: Option<[i32; 2]>,
    improve_tier: Option<bool>,
    get: Option<String>,
    count: Option<u8>,
    copy_last_spell: Option<bool>,
    effect: Option<RawEffect>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStartOfCombat {
    combat_beast_buff: Option<[i32; 2]>,
    damage_all_others: Option<i32>,
    /// Any effect a fight can run.
    effect: Option<RawEffect>,
    #[serde(default)]
    attack_immediately: bool,
    hopebringer: Option<RawHopebringer>,
    #[serde(default)]
    flighty_scout: bool,
    #[serde(default)]
    gain_hand_attack: bool,
    #[serde(default)]
    gain_hand_stats: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawOnCastSpellOnTribe {
    tribe: String,
    buff: [i32; 2],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawWhileInHand {
    /// The tribe of the minion that has to be played.
    after_play: String,
    buff: [i32; 2],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawHopebringer {
    buff: [i32; 2],
    improve: [i32; 2],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAttackTrigger {
    /// Only attackers of this tribe.
    attacker_tribe: Option<String>,
    /// Only attackers with a Rally effect.
    #[serde(default)]
    attacker_rally: bool,
    /// Another minion: the minion with the trigger isn't counted.
    #[serde(default)]
    attacker_another: bool,
    effect: RawEffect,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAvenge {
    every: u8,
    effect: RawEffect,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMilestone {
    total: u16,
    effect: RawEffect,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawOnSummonBeast {
    buff: Option<[i32; 2]>,
    #[serde(default)]
    improve_permanently: bool,
    multiply_attack_in_combat: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawOnPlayMech {
    magnetize_satellite: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawOnCastSpellOnMech {
    magnetize_satellite: String,
    stats: [i32; 2],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawOnCastSpell {
    buff_divine_shield: Option<[i32; 2]>,
    buff_all: Option<[i32; 2]>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBuff {
    buff: [i32; 2],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawActivate {
    cost: u8,
    effect: RawEffect,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawOnEvent {
    /// "spells_cast", "gold_spent", "hero_damage", "minions_sold" or "cards_bought".
    event: String,
    every: u8,
    effect: RawEffect,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawOnSell {
    #[serde(default)]
    discover_tier7_if_golden: bool,
    get: Option<String>,
    count: Option<u8>,
    discover_by_tier: Option<bool>,
    gold_if_lost: Option<u8>,
    effect: Option<RawEffect>,
}

fn tribe(s: &str) -> Result<Tribe, String> {
    match s {
        "beast" => Ok(Tribe::Beast),
        "mech" => Ok(Tribe::Mech),
        "dragon" => Ok(Tribe::Dragon),
        "murloc" => Ok(Tribe::Murloc),
        "demon" => Ok(Tribe::Demon),
        "pirate" => Ok(Tribe::Pirate),
        "elemental" => Ok(Tribe::Elemental),
        _ => Err(format!("unknown tribe {s:?}")),
    }
}

/// One entry of a card's tribe list: a tribe, "neutral" (none) or "all".
fn tribe_set(s: &str) -> Result<Tribes, String> {
    match s {
        "neutral" => Ok(Tribes::NONE),
        "all" => Ok(Tribes::ALL),
        _ => Ok(Tribes::of(tribe(s)?)),
    }
}

fn card_tribes(one: &Option<String>, many: &Option<Vec<String>>) -> Result<Tribes, String> {
    match (one, many) {
        (Some(t), None) => tribe_set(t),
        (None, Some(ts)) => {
            let mut set = Tribes::NONE;
            for t in ts {
                let one = tribe_set(t)?;
                set = Tribes(set.0 | one.0);
            }
            Ok(set)
        }
        (None, None) => Err("needs tribe or tribes".into()),
        (Some(_), Some(_)) => Err("has both tribe and tribes".into()),
    }
}

fn parse_event(s: &str) -> Result<EventKind, String> {
    match s {
        "spells_cast" => Ok(EventKind::SpellsCast),
        "gold_spent" => Ok(EventKind::GoldSpent),
        "hero_damage" => Ok(EventKind::HeroDamage),
        "minions_sold" => Ok(EventKind::MinionsSold),
        "cards_bought" => Ok(EventKind::CardsBought),
        "elementals_played" => Ok(EventKind::ElementalsPlayed),
        other => Err(format!("unknown event {other:?}")),
    }
}

fn keyword_bit(s: &str) -> Result<u8, String> {
    Keywords::ALL
        .iter()
        .find(|(_, n)| *n == s)
        .map(|(b, _)| *b)
        .ok_or_else(|| format!("unknown keyword {s:?}"))
}

fn parse_discover(s: &str) -> Result<DiscoverKind, String> {
    match s {
        "tier_1_minion" => Ok(DiscoverKind::Tier1Minion),
        "tier_minion" => Ok(DiscoverKind::MyTierMinion),
        "tier_spell" => Ok(DiscoverKind::MyTierSpell),
        "deathrattle" => Ok(DiscoverKind::DeathrattleMinion),
        "battlecry" => Ok(DiscoverKind::BattlecryMinion),
        other => Err(format!("unknown discover {other:?}")),
    }
}

fn parse_spell_effect(
    e: &RawSpellEffect,
) -> Result<(Option<SpellTarget>, Option<SpellEffect>), String> {
    let kw = match &e.keyword {
        Some(k) => keyword_bit(k)?,
        None => 0,
    };
    let times = e.times.unwrap_or(1);
    if let Some([attack, health]) = e.mighty_dragonbreath {
        return Ok((None, Some(SpellEffect::MightyDragonbreath { attack, health })));
    }
    match e.target.as_deref() {
        Some("chosen") => {
            if e.half_of_highest {
                let t = e.tribe.as_deref().ok_or("half_of_highest needs a tribe")?;
                return Ok((Some(SpellTarget::HalfOfHighestTavern { tribe: tribe(t)? }), None));
            }
            if let Some(per) = e.per_played_this_turn {
                let [attack, health] = e.buff.ok_or("per_played_this_turn needs buff")?;
                let t = e.tribe.as_deref().ok_or("per_played_this_turn needs a tribe")?;
                return Ok((
                    Some(SpellTarget::BuffPerTribePlayed {
                        tribe: tribe(t)?,
                        attack,
                        health,
                        per: (per[0], per[1]),
                    }),
                    None,
                ));
            }
            if let Some(how) = &e.consume {
                let pick = match how.as_str() {
                    "chosen" => ConsumePick::Chosen,
                    "random" => ConsumePick::Random,
                    "highest_health" => ConsumePick::HighestHealth,
                    other => return Err(format!("unknown spell consume {other:?}")),
                };
                let tribe = e.tribe.as_deref().map(tribe).transpose()?;
                return Ok((
                    Some(SpellTarget::Consume {
                        tribe,
                        pick,
                        keywords: e.with_keywords,
                        count: e.count.unwrap_or(1),
                    }),
                    None,
                ));
            }
            if let Some([attack, health]) = e.buff {
                if e.per_bonus_keyword {
                    return Ok((Some(SpellTarget::BuffPerBonusKeyword { attack, health }), None));
                }
                let if_win = e.if_win_next_turn_buff.map(|[a, h]| (a, h));
                return Ok((
                    Some(SpellTarget::Buff {
                        attack,
                        health,
                        keyword: kw,
                        times,
                        if_win_next_turn_buff: if_win,
                    }),
                    None,
                ));
            }
            if e.same_tribe_minion == Some(true) {
                return Ok((Some(SpellTarget::SameTribeMinion), None));
            }
            if let Some([attack, health]) = e.set_stats {
                return Ok((Some(SpellTarget::SetStats { attack, health }), None));
            }
            if let Some(max_tier) = e.make_golden_max_tier {
                return Ok((Some(SpellTarget::MakeGolden { max_tier }), None));
            }
            if let Some([attack, health]) = e.tricky_trousers {
                return Ok((Some(SpellTarget::TrickyTrousers { attack, health }), None));
            }
            if e.robust_evolution {
                return Ok((Some(SpellTarget::RobustEvolution), None));
            }
            if let Some([attack, health]) = e.tavern_buff_type {
                return Ok((Some(SpellTarget::TavernBuffType { attack, health }), None));
            }
            if e.refresh_with_type {
                return Ok((Some(SpellTarget::RefreshWithType), None));
            }
            if e.devourer {
                return Ok((Some(SpellTarget::Devourer), None));
            }
            Err("invalid chosen spell effect".into())
        }
        Some("chosen_tribe") => {
            let [attack, health] = e.buff.ok_or("chosen_tribe needs buff")?;
            Ok((Some(SpellTarget::BuffSameTribe { attack, health }), None))
        }
        Some("shop") => {
            let [attack, health] = e.buff.ok_or("shop target needs buff")?;
            Ok((None, Some(SpellEffect::BuffShop { attack, health })))
        }
        Some("board") => {
            let [attack, health] = e.buff.ok_or("board target needs buff")?;
            Ok((
                None,
                Some(SpellEffect::BuffBoard {
                    attack,
                    health,
                    times,
                    golden_extra: e.golden_extra,
                }),
            ))
        }
        Some("random_board") => {
            let [attack, health] = e.buff.ok_or("random_board needs buff")?;
            let count = e.count.ok_or("random_board needs count")?;
            Ok((None, Some(SpellEffect::BuffRandomBoard { count, attack, health })))
        }
        Some(other) => Err(format!("unknown spell target {other:?}")),
        None => {
            let eff = if let Some(g) = e.gold {
                SpellEffect::GainGold(g)
            } else if let Some(g) = e.gold_next_turn {
                SpellEffect::GoldNextTurn(g)
            } else if let Some(g) = e.max_gold {
                SpellEffect::MaxGold(g)
            } else if let Some(r) = e.free_rolls {
                SpellEffect::FreeRolls(r)
            } else if let Some([win, tie]) = e.overconfidence {
                SpellEffect::Overconfidence { win, tie }
            } else if let Some([attack, health]) = e.next_turn_board_buff {
                SpellEffect::NextTurnBoardBuff { attack, health, times }
            } else if let Some(t) = e.random_tier_minion {
                SpellEffect::RandomTierMinion(t)
            } else if let Some(d) = &e.discover {
                SpellEffect::Discover(parse_discover(d)?)
            } else if e.golden_touch == Some(true) {
                SpellEffect::GoldenTouch
            } else if e.cloning_conch {
                SpellEffect::CloningConch
            } else if let Some([attack, health]) = e.tavern_buff_permanent {
                SpellEffect::BuffTavernPermanent { attack, health }
            } else if let Some([attack, health]) = e.misplaced_tea_set {
                SpellEffect::MisplacedTeaSet { attack, health }
            } else if e.planar_telescope {
                SpellEffect::PlanarTelescope
            } else if e.steal_random_tavern {
                SpellEffect::StealRandomTavern
            } else if e.discover_locked_minion {
                SpellEffect::DiscoverLockedMinion
            } else if e.saloon_finest {
                SpellEffect::SaloonFinest
            } else if let Some([attack, health]) = e.refresh_buff {
                SpellEffect::RefreshBuff { attack, health }
            } else if let Some(n) = e.lockbox {
                SpellEffect::Lockbox(n)
            } else if let Some([attack, health]) = e.buff_leftmost {
                SpellEffect::BuffLeftmost { attack, health }
            } else if e.friendly_bounty {
                SpellEffect::FriendlyBounty
            } else if let Some(b) = e.boon_of_beetles {
                SpellEffect::BoonOfBeetles(b)
            } else if let Some(soc) = &e.start_of_combat {
                match soc.as_str() {
                    "double_leftmost_attack" => SpellEffect::BroodOfNozdormu,
                    "enemy_health_to_1" => SpellEffect::UpperHand,
                    other => return Err(format!("unknown spell start_of_combat {other:?}")),
                }
            } else {
                return Err("invalid spell effect".into());
            };
            Ok((None, Some(eff)))
        }
    }
}

impl CardDb {
    pub fn from_toml(s: &str) -> Result<CardDb, String> {
        let raw: RawFile = toml::from_str(s).map_err(|e| format!("cards.toml: {e}"))?;
        let pool_cards = raw.minion.len();
        let all_minions: Vec<(RawCard, bool)> = raw
            .minion
            .into_iter()
            .map(|c| (c, false))
            .chain(raw.token.into_iter().map(|c| (c, true)))
            .collect();
        let total_cards = all_minions.len() + raw.spell.len();
        if total_cards > CardId::MAX as usize {
            return Err("cards.toml: too many cards".into());
        }
        if pool_cards > crate::game::MAX_POOL_CARDS {
            return Err(format!(
                "cards.toml: {} pool minions, the limit is {}",
                pool_cards,
                crate::game::MAX_POOL_CARDS
            ));
        }
        let mut names: Vec<String> = all_minions.iter().map(|(c, _)| c.name.clone()).collect();
        names.extend(raw.spell.iter().map(|sp| sp.name.clone()));
        for (i, n) in names.iter().enumerate() {
            if names[..i].contains(n) {
                return Err(format!("cards.toml: {n} appears twice"));
            }
        }
        let token_id = |name: &str| -> Result<CardId, String> {
            let id = names
                .iter()
                .position(|n| n == name)
                .ok_or_else(|| format!("unknown card {name:?}"))?;
            if id >= all_minions.len() || !all_minions[id].1 {
                return Err(format!("{name} is not a token"));
            }
            Ok(id as CardId)
        };
        let generated_id = |name: &str| -> Result<CardId, String> {
            let id = names
                .iter()
                .position(|n| n == name)
                .ok_or_else(|| format!("unknown card {name:?}"))?;
            if id < pool_cards {
                return Err(format!("{name} is a pool minion, not a token or spell"));
            }
            Ok(id as CardId)
        };

        // any minion, pool card or token, to summon
        let minion_id = |name: &str| -> Result<CardId, String> {
            let id = names
                .iter()
                .position(|n| n == name)
                .ok_or_else(|| format!("unknown card {name:?}"))?;
            if id >= all_minions.len() {
                return Err(format!("{name} is a spell, not a minion"));
            }
            Ok(id as CardId)
        };

        // a Tavern spell, for "cast <spell>"
        let spell_id = |name: &str| -> Result<CardId, String> {
            let id = names
                .iter()
                .position(|n| n == name)
                .ok_or_else(|| format!("unknown card {name:?}"))?;
            if id < all_minions.len() {
                return Err(format!("{name} is not a spell"));
            }
            Ok(id as CardId)
        };

        let mut cards = Vec::with_capacity(total_cards);
        let mut volumizers = Vec::new();
        let mut spells = Vec::with_capacity(raw.spell.len());
        let mut first_volumizer_slot: Option<CardId> = None;
        let mut beetle_token: Option<CardId> = None;
        let mut chromadrakes = Vec::new();
        let mut taught_token: Option<CardId> = None;
        let mut fodder_token: Option<CardId> = None;

        let parse_effect = |b: &RawEffect| -> Result<Effect, String> {
            let t = b.tribe.as_deref().map(tribe).transpose()?;
            Ok(if let Some([attack, health]) = b.refresh_buff {
                Effect::RefreshBuff { attack, health }
            } else if let Some(n) = b.free_refreshes {
                Effect::FreeRefreshes(n)
            } else if let Some(name) = &b.improving_counter {
                let [attack, health] = b.buff.ok_or("improving_counter needs buff")?;
                Effect::BuffAllImproving { attack, health, counter: counter(name)? }
            } else if b.arm_next_buy {
                Effect::ArmNextBuy
            } else if b.gain_highest_tavern_stats {
                Effect::GainHighestTavernStats
            } else if b.per_improvement {
                let [attack, health] = b.buff.ok_or("per_improvement needs buff")?;
                Effect::BuffOthersPerImprovement { tribe: t, attack, health }
            } else if let Some(per_gold) = b.per_gold_spent {
                let [attack, health] = b.buff.ok_or("per_gold_spent needs buff")?;
                Effect::BuffChosenPerGold {
                    tribe: t,
                    attack,
                    health,
                    per_gold: (per_gold[0], per_gold[1]),
                }
            } else if b.per_golden_played {
                let [attack, health] = b.buff.ok_or("per_golden_played needs buff")?;
                Effect::BuffOthersPerGolden { tribe: t, attack, health }
            } else if let Some([attack, health]) = b.keep_golden {
                Effect::BuffAllKeepGolden { attack, health }
            } else if let Some(attack) = b.adjacent_per_golden {
                Effect::BuffAdjacentPerGolden { attack }
            } else if let Some(max_tier) = b.make_golden_max_tier {
                Effect::MakeGoldenChosen { tribe: t, max_tier, count: b.count.unwrap_or(1) }
            } else if let Some(tier) = b.golden_minion_tier {
                Effect::GetGoldenMinion { tier, count: b.count.unwrap_or(1) }
            } else if let Some(n) = b.lockbox_sooner {
                Effect::GetLockbox { sooner: n }
            } else if b.random_bounty {
                Effect::GetRandomBounty
            } else if let Some([attack, health]) = b.buff {
                let per_keyword = b.per_keyword.map_or((0, 0), |[a, h]| (a, h));
                match b.target.as_deref() {
                    Some("chosen") => Effect::BuffChosen {
                        tribe: t,
                        attack,
                        health,
                        per_keyword,
                        random_keyword: b.random_keyword,
                    },
                    Some("others") => Effect::BuffOthers { tribe: t, attack, health, per_keyword },
                    Some("self") => Effect::BuffSelf { attack, health },
                    Some("all") => Effect::BuffAll { tribe: t, attack, health },
                    Some("any") => Effect::BuffAnyMinion { attack, health },
                    Some("random") => {
                        let improve = b.improve_counter.as_deref().map(counter).transpose()?;
                        Effect::BuffRandomFriendly { tribe: t, attack, health, improve }
                    }
                    other => return Err(format!("battlecry target {other:?}")),
                }
            } else if let Some([attack, health]) = b.buff_killer {
                Effect::BuffKiller { attack, health }
            } else if let Some(name) = &b.summon {
                Effect::Summon {
                    card: minion_id(name)?,
                    count: b.count.unwrap_or(1),
                    golden_doubles_count: b.golden_doubles_count,
                }
            } else if let Some(name) = &b.summon_from_hand {
                Effect::SummonFromHand { tribe: tribe(name)?, count: b.count.unwrap_or(1) }
            } else if let Some([attack, health]) = b.buff_random_hand {
                Effect::BuffRandomHand { attack, health }
            } else if let Some([attack, health]) = b.buff_leftmost_hand {
                Effect::BuffLeftmostHand { attack, health }
            } else if b.trigger_battlecry {
                Effect::TriggerBattlecry
            } else if let Some(name) = &b.discover_tribe_if_another {
                Effect::DiscoverTribeIfAnother {
                    tribe: tribe(name)?,
                    count: b.count.unwrap_or(1),
                }
            } else if b.cast_taught {
                Effect::CastTaughtSpell
            } else if let Some(n) = b.trigger_adjacent_battlecries {
                Effect::TriggerAdjacentBattlecries(n)
            } else if let Some(name) = &b.random_minion {
                Effect::GetRandomMinion { tribe: tribe(name)? }
            } else if let Some(k) = &b.gain_keyword {
                Effect::GainKeyword { keyword: keyword_bit(k)? }
            } else if let Some(cost) = b.random_spell_cost {
                Effect::GetRandomSpell { cost, count: b.count.unwrap_or(1) }
            } else if let Some(n) = b.summon_random_deathrattle {
                Effect::SummonRandomDeathrattle { count: n }
            } else if b.trigger_deathrattle {
                Effect::TriggerDeathrattle { times: b.count.unwrap_or(1) }
            } else if let Some(name) = &b.replace_with {
                Effect::ReplaceWithFishbait { card: token_id(name)? }
            } else if let Some(name) = &b.refresh_with {
                Effect::RefreshWithFishbait { card: token_id(name)? }
            } else if let Some(n) = b.extra_magnetization {
                Effect::ExtraMagnetization(n)
            } else if b.random_keyword {
                Effect::RandomKeyword
            } else if let Some(how) = &b.consume {
                let pick = match how.as_str() {
                    "chosen" => ConsumePick::Chosen,
                    "random" => ConsumePick::Random,
                    "highest_health" => ConsumePick::HighestHealth,
                    other => return Err(format!("unknown consume {other:?}")),
                };
                let by = if b.consume_by_chosen { ConsumeBy::Chosen(t) } else { ConsumeBy::This };
                Effect::Consume {
                    by,
                    pick,
                    keywords: b.with_keywords,
                    count: b.count.unwrap_or(1),
                }
            } else if let Some([attack, health]) = b.tavern_buff {
                Effect::TavernBuff { tribe: t, attack, health, this_turn: b.this_turn }
            } else if let Some([attack, health]) = b.tavern_buff_random_keyword {
                let mut mask = 0u8;
                for k in b.keywords.as_deref().unwrap_or(&[]) {
                    mask |= keyword_bit(k)?;
                }
                Effect::TavernBuffRandomKeyword { attack, health, mask }
            } else if let Some(n) = b.add_fodder_refreshes {
                Effect::AddFodderRefreshes(n)
            } else if let Some(n) = b.add_fodder_to_next_refresh {
                Effect::AddFodderToNextRefresh(n)
            } else if let Some(n) = b.demons_consume {
                Effect::DemonsConsume { count: n }
            } else if let Some(n) = b.improve_self {
                Effect::ImproveSelf(n)
            } else if let Some(n) = b.discover_demon_tier_damage {
                Effect::DiscoverDemonTierDamage(n)
            } else if let Some(n) = b.damage_hero {
                Effect::DamageHero(n)
            } else if b.rewind {
                Effect::RewindDamage
            } else if b.magnetize_volumizer {
                Effect::MagnetizeVolumizerAndCopy
            } else if let Some([attack, health]) = b.beetle_buff {
                Effect::BeetleBuff { attack, health }
            } else if let Some(g) = b.gold_next_turn {
                Effect::GoldNextTurn(g)
            } else if b.random_volumizer == Some(true) {
                Effect::RandomVolumizer
            } else if let Some(d) = b.spell_discount {
                Effect::SpellDiscount(d)
            } else if let Some([attack, health]) = b.spell_bonus {
                Effect::SpellBonus { attack, health }
            } else if b.random_chromadrake {
                Effect::RandomChromadrake
            } else if let Some(name) = &b.cast {
                Effect::CastSpell { card: spell_id(name)?, times: b.count.unwrap_or(1) }
            } else if b.trigger_rally {
                Effect::TriggerRally
            } else if let Some([attack, health]) = b.buff_attacker {
                Effect::BuffAttacker { attack, health }
            } else if let Some([attack, health]) = b.buff_random_other {
                let keyword = b.keyword.as_deref().map(keyword_bit).transpose()?.unwrap_or(0);
                Effect::BuffRandomOther {
                    tribe: t,
                    attack,
                    health,
                    keyword,
                    count: b.count.unwrap_or(1),
                }
            } else if let Some([attack, health]) = b.set_stats_any {
                Effect::SetStatsAnyMinion { attack, health }
            } else if let Some(n) = b.steal_tavern {
                Effect::StealTavern { count: n }
            } else if let Some(n) = b.discover_spell {
                Effect::DiscoverSpell { count: n }
            } else if b.gain_target_attack {
                Effect::GainTargetAttack
            } else if let Some(keys) = &b.strip_keywords {
                let mut mask = 0u8;
                for k in keys {
                    mask |= keyword_bit(k)?;
                }
                Effect::StripTargetKeywords { mask }
            } else if b.destroy_killer {
                Effect::DestroyKiller
            } else if let Some(n) = b.shield_dragons {
                Effect::ShieldDragons { count: n }
            } else if let Some(tier) = b.random_tier_minion {
                Effect::GetRandomTierMinion { tier }
            } else if let Some([attack, health]) = b.buff_each_type {
                Effect::BuffEachType { attack, health }
            } else if b.damage_target_and_neighbor {
                Effect::DamageTargetAndNeighbor
            } else if b.give_stats_leftmost_hand {
                Effect::GiveOwnStatsToLeftmostHand
            } else if let Some(name) = &b.get {
                let card = generated_id(name)?;
                Effect::GetCard { card, count: b.count.unwrap_or(1) }
            } else {
                return Err("invalid battlecry".into());
            })
        };

        for (idx, (c, token)) in all_minions.iter().enumerate() {
            let id = idx as CardId;
            let err = |e: String| format!("cards.toml, {}: {e}", c.name);
            if !(1..=crate::CARD_MAX_TIER).contains(&c.tier) {
                return Err(err(format!("tier {} is not 1 to {}", c.tier, crate::CARD_MAX_TIER)));
            }
            let mut keywords = Keywords::default();
            for k in &c.keywords {
                keywords.set(keyword_bit(k).map_err(&err)?);
            }
            if c.beetle && *token {
                beetle_token = Some(id);
            }
            if c.chromadrake && *token {
                chromadrakes.push(id);
            }
            if c.taught_token && *token {
                taught_token = Some(id);
            }
            if c.fodder && *token {
                fodder_token = Some(id);
            }
            let is_volumizer = c.volumizer_buff.is_some();
            let pool_slot = if is_volumizer {
                volumizers.push(id);
                *first_volumizer_slot.get_or_insert(id)
            } else {
                id
            };
            let volumizer_buff = c.volumizer_buff.map(|[a, h]| (a, h));
            let spell_buff = c.spell_buff.map(|[a, h]| (a, h));

            let battlecry = c.battlecry.as_ref().map(&parse_effect).transpose().map_err(&err)?;

            let activate = match &c.activate {
                None => None,
                Some(a) => {
                    let effect = parse_effect(&a.effect).map_err(&err)?;
                    Some(Activate { cost: a.cost, effect })
                }
            };

            let on_event = match &c.on_event {
                None => None,
                Some(e) => {
                    let effect = parse_effect(&e.effect).map_err(&err)?;
                    if effect.needs_choice() {
                        return Err(err("an event trigger can't ask for a choice".into()));
                    }
                    if e.every == 0 {
                        return Err(err("on_event every must be at least 1".into()));
                    }
                    Some(EventTrigger {
                        kind: parse_event(&e.event).map_err(&err)?,
                        every: e.every,
                        effect,
                    })
                }
            };

            let after_battlecry = match &c.after_battlecry {
                None => None,
                Some(e) => {
                    let effect = parse_effect(e).map_err(&err)?;
                    if effect.needs_choice() {
                        return Err(err("an after_battlecry effect can't ask for a choice".into()));
                    }
                    Some(effect)
                }
            };

            let permanent_keep = match c.permanent_keep.as_deref() {
                None => None,
                Some("self") => Some(PermanentKeep::SelfKeep),
                Some("adjacent_dragons") => Some(PermanentKeep::AdjacentDragons),
                Some(other) => return Err(err(format!("unknown permanent_keep {other:?}"))),
            };

            let attack_threshold = match &c.attack_threshold {
                None => None,
                Some(t) => Some(AttackThreshold {
                    total: t.total,
                    keyword: keyword_bit(&t.keyword).map_err(&err)?,
                }),
            };

            let mut after_hero_damage = Vec::new();
            for e in &c.after_hero_damage {
                let effect = parse_effect(e).map_err(&err)?;
                if effect.needs_choice() {
                    return Err(err("after_hero_damage can't ask for a choice".into()));
                }
                after_hero_damage.push(effect);
            }

            let after_play_tribe = match &c.after_play_tribe {
                None => None,
                Some(p) => {
                    let mut effects = Vec::new();
                    for e in &p.effects {
                        let effect = parse_effect(e).map_err(&err)?;
                        if effect.needs_choice() {
                            return Err(err("after_play_tribe can't ask for a choice".into()));
                        }
                        effects.push(effect);
                    }
                    Some(AfterPlayTribe {
                        exclude_played: p.exclude_played,
                        tribe: tribe(&p.tribe).map_err(&err)?,
                        effects,
                    })
                }
            };

            let choose_one = match &c.choose_one {
                None => None,
                Some([o1, o2]) => {
                    let parse_opt = |o: &RawMinionChooseOption| -> Result<ChooseOption, String> {
                        if let Some([attack, health]) = o.buff {
                            let kw = o.keyword.as_deref().ok_or("choose_one buff needs keyword")?;
                            Ok(ChooseOption::MinionTarget {
                                tribe: o.tribe.as_deref().map(tribe).transpose()?,
                                attack,
                                health,
                                keyword: keyword_bit(kw)?,
                            })
                        } else if let Some([attack, health]) = o.spell_buff {
                            Ok(ChooseOption::SpellBonus { attack, health })
                        } else {
                            Err("invalid minion choose_one".into())
                        }
                    };
                    Some([parse_opt(o1).map_err(&err)?, parse_opt(o2).map_err(&err)?])
                }
            };

            let deathrattle = match &c.deathrattle {
                None => None,
                Some(d) => Some(if let Some(name) = &d.summon {
                    let card = token_id(name).map_err(&err)?;
                    Deathrattle::Summon {
                        card,
                        count: d.count.unwrap_or(1),
                        golden_doubles_count: d.golden_doubles_count,
                        beetle_buff: d.beetle_buff.map(|[a, h]| (a, h)),
                    }
                } else if let Some(name) = &d.summon_by_attack {
                    let card = token_id(name).map_err(&err)?;
                    Deathrattle::SummonByAttack { card }
                } else if let Some([attack, health]) = d.combat_beast_buff {
                    Deathrattle::CombatBeastBuff { attack, health }
                } else if d.random_volumizer == Some(true) {
                    Deathrattle::RandomVolumizer
                } else if let Some(name) = &d.get {
                    let card = generated_id(name).map_err(&err)?;
                    Deathrattle::GetCard { card, count: d.count.unwrap_or(1) }
                } else if let Some([attack, health]) = d.tavern_buff {
                    Deathrattle::TavernBuff {
                        attack,
                        health,
                        per_improvement: d.per_improvement.map_or((0, 0), |[a, h]| (a, h)),
                    }
                } else if let Some(e) = &d.effect {
                    let effect = parse_effect(e).map_err(&err)?;
                    if !effect.runs_in_fights() {
                        return Err(err("this deathrattle effect can't run in a fight".into()));
                    }
                    Deathrattle::Effect(effect)
                } else {
                    return Err(err("invalid deathrattle".into()));
                }),
            };

            let start_of_turn = c.start_of_turn.as_ref().map(|s| StartOfTurn::GainGold(s.gold));

            let end_of_turn = match &c.end_of_turn {
                None => None,
                Some(e) => Some(if let Some([attack, health]) = e.buff_self {
                    EndOfTurn::BuffSelf { attack, health }
                } else if e.resourceful_robot == Some(true) {
                    EndOfTurn::ResourcefulRobot
                } else if let Some([attack, health]) = e.buff_per_magnetization {
                    EndOfTurn::BuffPerMagnetization { attack, health }
                } else if e.improve_tier == Some(true) {
                    EndOfTurn::ImproveTier
                } else if let Some(name) = &e.get {
                    let card = generated_id(name).map_err(&err)?;
                    EndOfTurn::GetCard { card, count: e.count.unwrap_or(1) }
                } else if e.copy_last_spell == Some(true) {
                    EndOfTurn::CopyLastSpell
                } else if let Some(eff) = &e.effect {
                    let effect = parse_effect(eff).map_err(&err)?;
                    if effect.needs_choice() {
                        return Err(err("an end of turn effect can't ask for a choice".into()));
                    }
                    EndOfTurn::Effect(effect)
                } else {
                    return Err(err("invalid end_of_turn".into()));
                }),
            };

            let start_of_combat = match &c.start_of_combat {
                None => None,
                Some(s) => Some(if let Some([attack, health]) = s.combat_beast_buff {
                    StartOfCombat::CombatBeastBuff { attack, health }
                } else if let Some(dmg) = s.damage_all_others {
                    StartOfCombat::DamageAllOthers(dmg)
                } else if s.attack_immediately {
                    StartOfCombat::AttackImmediately
                } else if let Some(h) = &s.hopebringer {
                    StartOfCombat::Hopebringer {
                        attack: h.buff[0],
                        health: h.buff[1],
                        improve: (h.improve[0], h.improve[1]),
                    }
                } else if s.flighty_scout {
                    StartOfCombat::FlightyScout
                } else if s.gain_hand_attack {
                    StartOfCombat::GainHighestHandAttack
                } else if s.gain_hand_stats {
                    StartOfCombat::GainHandStats
                } else if let Some(eff) = &s.effect {
                    let effect = parse_effect(eff).map_err(&err)?;
                    if !effect.runs_in_fights() {
                        return Err(err("this start_of_combat effect can't run in a fight".into()));
                    }
                    StartOfCombat::Effect(effect)
                } else {
                    return Err(err("invalid start_of_combat".into()));
                }),
            };

            let fight_effect = |e: &RawEffect, what: &str| -> Result<Effect, String> {
                let effect = parse_effect(e).map_err(&err)?;
                if !effect.runs_in_fights() {
                    return Err(err(format!("this {what} effect can't run in a fight")));
                }
                Ok(effect)
            };

            let rally = c.rally.as_ref().map(|e| fight_effect(e, "rally")).transpose()?;

            let attack_trigger = |t: &RawAttackTrigger, what: &str| -> Result<AttackTrigger, String> {
                Ok(AttackTrigger {
                    filter: AttackFilter {
                        tribe: t.attacker_tribe.as_deref().map(tribe).transpose().map_err(&err)?,
                        rally: t.attacker_rally,
                        exclude_self: t.attacker_another,
                    },
                    effect: fight_effect(&t.effect, what)?,
                })
            };
            let whenever_friendly_attacks = c
                .whenever_friendly_attack
                .as_ref()
                .map(|t| attack_trigger(t, "attack trigger"))
                .transpose()?;
            let after_friendly_attacks = c
                .after_friendly_attack
                .as_ref()
                .map(|t| attack_trigger(t, "attack trigger"))
                .transpose()?;
            let avenge = match &c.avenge {
                None => None,
                Some(a) if a.every == 0 => return Err(err("avenge every must be at least 1".into())),
                Some(a) => Some(Avenge { every: a.every, effect: fight_effect(&a.effect, "avenge")? }),
            };
            let damage_milestone = match &c.damage_milestone {
                None => None,
                Some(m) => Some(DamageMilestone {
                    total: m.total,
                    effect: fight_effect(&m.effect, "damage milestone")?,
                }),
            };

            let on_summon_beast = match &c.on_summon_beast {
                None => None,
                Some(s) => Some(if let Some([attack, health]) = s.buff {
                    OnSummonBeast::Buff {
                        attack,
                        health,
                        improve_permanently: s.improve_permanently,
                    }
                } else if s.multiply_attack_in_combat == Some(true) {
                    OnSummonBeast::MultiplyAttackInCombat
                } else {
                    return Err(err("invalid on_summon_beast".into()));
                }),
            };

            let on_play_mech = match &c.on_play_mech {
                None => None,
                Some(p) => {
                    let card = token_id(&p.magnetize_satellite).map_err(&err)?;
                    Some(OnPlayMech::MagnetizeSatellite { card })
                }
            };

            let on_play_or_magnetize_mech = c
                .on_play_or_magnetize_mech
                .as_ref()
                .map(|b| OnPlayOrMagnetizeMech::Buff { attack: b.buff[0], health: b.buff[1] });

            let on_cast_spell_on_mech = match &c.on_cast_spell_on_mech {
                None => None,
                Some(g) => {
                    let card = token_id(&g.magnetize_satellite).map_err(&err)?;
                    Some(OnCastSpellOnMech::MagnetizeSatellite {
                        card,
                        attack: g.stats[0],
                        health: g.stats[1],
                    })
                }
            };

            let on_cast_spell_on_self = match &c.on_cast_spell_on_self {
                None => None,
                Some(e) => {
                    let effect = parse_effect(e).map_err(&err)?;
                    if effect.needs_choice() {
                        return Err(err("an on_cast_spell_on_self effect can't ask for a choice".into()));
                    }
                    Some(effect)
                }
            };

            let on_cast_spell_on_tribe = match &c.on_cast_spell_on_tribe {
                None => None,
                Some(t) => Some((
                    tribe(&t.tribe).map_err(&err)?,
                    Effect::BuffTribeBoardAndHand {
                        tribe: tribe(&t.tribe).map_err(&err)?,
                        attack: t.buff[0],
                        health: t.buff[1],
                    },
                )),
            };

            let when_damaged = match &c.when_damaged {
                None => None,
                Some(e) => {
                    let effect = parse_effect(e).map_err(&err)?;
                    if !effect.runs_in_fights() {
                        return Err(err("this when_damaged effect can't run in a fight".into()));
                    }
                    Some(effect)
                }
            };

            let while_in_hand = match &c.while_in_hand {
                None => None,
                Some(w) => Some(WhileInHand {
                    tribe: tribe(&w.after_play).map_err(&err)?,
                    attack: w.buff[0],
                    health: w.buff[1],
                }),
            };

            let on_cast_spell = match &c.on_cast_spell {
                None => None,
                Some(cs) => Some(if let Some([attack, health]) = cs.buff_divine_shield {
                    OnCastSpell::BuffDivineShield { attack, health }
                } else if let Some([attack, health]) = cs.buff_all {
                    OnCastSpell::BuffAll { attack, health }
                } else {
                    return Err(err("invalid on_cast_spell".into()));
                }),
            };

            let on_sell = match &c.on_sell {
                None => None,
                Some(s) => Some(if s.discover_tier7_if_golden {
                    OnSell::DiscoverTier7IfGolden
                } else if let Some(name) = &s.get {
                    let card = generated_id(name).map_err(&err)?;
                    OnSell::GetCard { card, count: s.count.unwrap_or(1) }
                } else if s.discover_by_tier == Some(true) {
                    OnSell::DiscoverByTier
                } else if let Some(g) = s.gold_if_lost {
                    OnSell::GoldIfLost(g)
                } else if let Some(e) = &s.effect {
                    OnSell::Effect(parse_effect(e).map_err(&err)?)
                } else {
                    return Err(err("invalid on_sell".into()));
                }),
            };

            let aura = match c.aura.as_deref() {
                None => None,
                Some("brann") => Some(Aura::Brann),
                Some("titus") => Some(Aura::Titus),
                Some("drakkari") => Some(Aura::Drakkari),
                Some("balinda") => Some(Aura::Balinda),
                Some("taunt_while_shielded") => Some(Aura::TauntWhileShielded),
                Some("holy_vanguard") => Some(Aura::HolyVanguard),
                Some("polarizing_beatboxer") => Some(Aura::PolarizingBeatboxer),
                Some("proud_privateer") => Some(Aura::ProudPrivateer),
                Some(other) => return Err(err(format!("unknown aura {other:?}"))),
            };

            let after_discover = match &c.after_discover {
                None => None,
                Some(e) => {
                    let effect = parse_effect(e).map_err(&err)?;
                    if effect.needs_choice() {
                        return Err(err("an after_discover effect can't ask for a choice".into()));
                    }
                    Some(effect)
                }
            };

            cards.push(Card {
                name: c.name.clone(),
                tier: c.tier,
                tribes: card_tribes(&c.tribe, &c.tribes).map_err(&err)?,
                attack: c.stats[0],
                health: c.stats[1],
                golden_stats: c.golden_stats.map(|[a, h]| (a, h)),
                keywords,
                token: *token,
                spell: false,
                pay_health: false,
                cost: 0,
                is_beetle: c.beetle,
                is_volumizer,
                pool_slot,
                volumizer_buff,
                spell_buff,
                battlecry,
                choose_one,
                deathrattle,
                start_of_turn,
                end_of_turn,
                start_of_combat,
                rally,
                on_summon_beast,
                on_play_mech,
                on_play_or_magnetize_mech,
                on_cast_spell_on_mech,
                on_cast_spell,
                on_cast_spell_on_self,
                on_cast_spell_on_tribe,
                when_damaged,
                while_in_hand,
                teaches_spell: c.teaches_spell,
                no_triple: c.no_triple,
                taught_token: c.taught_token,
                whenever_friendly_attacks,
                after_friendly_attacks,
                avenge,
                damage_milestone,
                buff_per_deathrattle: c.buff_per_deathrattle.map(|[a, h]| (a, h)),
                fixed_stats: c.fixed_stats,
                activate,
                on_event,
                after_battlecry,
                permanent_keep,
                attack_threshold,
                after_hero_damage,
                after_play_tribe,
                after_demon_damage: c.after_demon_damage.map(|[a, h]| (a, h)),
                is_fodder: c.fodder,
                health_refreshes: c.health_refreshes.unwrap_or(0),
                on_sell,
                aura,
                after_summon_give_stats: c
                    .after_summon_give_stats
                    .as_ref()
                    .map(|a| AfterSummonGiveStats { times: a.times }),
                spell_target: None,
                spell_effect: None,
                always_golden: c.always_golden,
                cleave: c.cleave,
                buff_per_golden_played: c.buff_per_golden_played.map(|[a, h]| (a, h)),
                after_discover,
                is_bounty: false,
                overkill: c.overkill,
                triple_wildcard: c
                    .triple_wildcard
                    .as_deref()
                    .map(tribe)
                    .transpose()
                    .map_err(&err)?,
                play_shield: c.play_shield.as_deref().map(tribe).transpose().map_err(&err)?,
                after_buy: c.after_buy.as_ref().map(|a| AfterBuy {
                    attack: a.buff[0],
                    health: a.buff[1],
                    multiply: a.multiply,
                    golden_multiply: a.golden_multiply,
                }),
            });
        }

        let mut bounties: Vec<CardId> = Vec::new();
        for sp in &raw.spell {
            let id = cards.len() as CardId;
            let err = |e: String| format!("cards.toml, {}: {e}", sp.name);
            if !(1..=crate::MAX_TIER).contains(&sp.tier) {
                return Err(err(format!("tier {} is not 1 to 6", sp.tier)));
            }
            let (spell_target, spell_effect) = match &sp.effect {
                Some(e) if e.get.is_some() => {
                    let card = generated_id(e.get.as_deref().unwrap()).map_err(&err)?;
                    (None, Some(SpellEffect::GetCard { card, count: e.count.unwrap_or(1) }))
                }
                // Two cards that may be pool minions (generated copies of
                // them, with no pool copy): Temperature Shift.
                Some(e) if e.get_pair.is_some() => {
                    let [a, b] = e.get_pair.as_ref().unwrap();
                    let a = minion_id(a).map_err(&err)?;
                    let b = minion_id(b).map_err(&err)?;
                    (None, Some(SpellEffect::GetTwo { a, b }))
                }
                Some(e) => parse_spell_effect(e).map_err(&err)?,
                None => (None, None),
            };
            let choose_one = match &sp.choose_one {
                None => None,
                Some([o1, o2]) => {
                    let to_opt = |o: &RawSpellEffect| -> Result<ChooseOption, String> {
                        match parse_spell_effect(o)? {
                            (Some(st), None) => Ok(ChooseOption::SpellTarget(st)),
                            (None, Some(se)) => Ok(ChooseOption::SpellEffect(se)),
                            _ => Err("invalid spell choose_one option".into()),
                        }
                    };
                    Some([to_opt(o1).map_err(&err)?, to_opt(o2).map_err(&err)?])
                }
            };
            if spell_target.is_none() && spell_effect.is_none() && choose_one.is_none() {
                return Err(err("spell has no effect or choose_one".into()));
            }
            if sp.bounty {
                // The five Bounties are one slot of the spell draw: only the
                // first stands in `spells`.
                if bounties.is_empty() && !sp.token {
                    spells.push(id);
                }
                bounties.push(id);
            } else if !sp.token {
                spells.push(id);
            }
            cards.push(Card {
                name: sp.name.clone(),
                tier: sp.tier,
                tribes: Tribes::NONE,
                attack: 0,
                health: 0,
                golden_stats: None,
                keywords: Keywords::default(),
                token: sp.token,
                spell: true,
                pay_health: sp.pay_health,
                cost: sp.cost,
                is_beetle: false,
                is_volumizer: false,
                pool_slot: id,
                volumizer_buff: None,
                spell_buff: None,
                battlecry: None,
                choose_one,
                deathrattle: None,
                start_of_turn: None,
                end_of_turn: None,
                start_of_combat: None,
                rally: None,
                on_summon_beast: None,
                on_play_mech: None,
                on_play_or_magnetize_mech: None,
                on_cast_spell_on_mech: None,
                on_cast_spell: None,
                on_cast_spell_on_self: None,
                on_cast_spell_on_tribe: None,
                when_damaged: None,
                while_in_hand: None,
                teaches_spell: false,
                no_triple: false,
                taught_token: false,
                whenever_friendly_attacks: None,
                after_friendly_attacks: None,
                avenge: None,
                damage_milestone: None,
                buff_per_deathrattle: None,
                fixed_stats: false,
                activate: None,
                on_event: None,
                after_battlecry: None,
                permanent_keep: None,
                attack_threshold: None,
                after_hero_damage: Vec::new(),
                after_play_tribe: None,
                after_demon_damage: None,
                is_fodder: false,
                health_refreshes: 0,
                on_sell: None,
                aura: None,
                after_summon_give_stats: None,
                spell_target,
                spell_effect,
                always_golden: false,
                cleave: false,
                buff_per_golden_played: None,
                after_discover: None,
                is_bounty: sp.bounty,
                overkill: false,
                triple_wildcard: None,
                play_shield: None,
                after_buy: None,
            });
        }

        let lockbox = cards
            .iter()
            .position(|c| matches!(c.spell_effect, Some(SpellEffect::Lockbox(_))))
            .map(|i| i as CardId);
        let tiers: Vec<u8> = cards.iter().map(|c| c.tier).collect();
        let ids_up_to_tier = (0..=crate::CARD_MAX_TIER)
            .map(|hi| (0..pool_cards).filter(|&c| tiers[c] <= hi).map(|c| c as u8).collect())
            .collect();
        Ok(CardDb {
            cards,
            tiers,
            ids_up_to_tier,
            pool_cards,
            volumizers,
            spells,
            beetle_token,
            chromadrakes,
            taught_token,
            fodder_token,
            bounties,
            lockbox,
        })
    }

    /// One of the five Chromadrakes, uniformly.
    pub fn random_chromadrake(&self, rng: &mut Rng) -> Option<CardId> {
        if self.chromadrakes.is_empty() {
            None
        } else {
            let k = rng.below(self.chromadrakes.len() as u32) as usize;
            Some(self.chromadrakes[k])
        }
    }

    pub fn get(&self, id: CardId) -> &Card {
        &self.cards[id as usize]
    }

    pub fn by_name(&self, name: &str) -> Option<CardId> {
        if let Some(i) = self.cards.iter().position(|c| c.name == name) {
            return Some(i as CardId);
        }
        let alt = match name {
            "Goldrinn the Great Wolf" | "Goldrinn" => "Goldrinn, the Great Wolf",
            "Beast" => "Foraging Bat",
            _ => return None,
        };
        self.cards.iter().position(|c| c.name == alt).map(|i| i as CardId)
    }

    pub fn random_volumizer(&self, rng: &mut Rng) -> Option<CardId> {
        if self.volumizers.is_empty() {
            None
        } else {
            let k = rng.below(self.volumizers.len() as u32) as usize;
            Some(self.volumizers[k])
        }
    }

    /// A random pool minion of a tribe, of Tier `max_tier` or lower (0 means
    /// any Tier). "Random minion" cards pick up to the player's current tier.
    pub fn random_pool_minion(
        &self,
        tribe: Tribe,
        max_tier: u8,
        rng: &mut Rng,
    ) -> Option<CardId> {
        let max_tier = if max_tier == 0 { crate::MAX_TIER } else { max_tier };
        let ok = |c: usize| self.cards[c].tribes.has(tribe) && self.cards[c].tier <= max_tier;
        let count = (0..self.pool_cards).filter(|&c| ok(c)).count() as u32;
        if count == 0 {
            return None;
        }
        let k = rng.below(count) as usize;
        (0..self.pool_cards).filter(|&c| ok(c)).nth(k).map(|c| c as CardId)
    }

    /// A random pool minion that has a Deathrattle, of Tier `max_tier` or
    /// lower (0 means any Tier).
    ///
    /// This is an "any minion" draw, so a sandbox `rig` narrows it (see
    /// `Card::rig_ok`), falling back to every candidate when none is left.
    pub fn random_deathrattle_minion(
        &self,
        max_tier: u8,
        rig: Option<Tribe>,
        rng: &mut Rng,
    ) -> Option<CardId> {
        let max_tier = if max_tier == 0 { crate::MAX_TIER } else { max_tier };
        self.pick_rigged(
            |c| self.cards[c].deathrattle.is_some() && self.cards[c].tier <= max_tier,
            rig,
            rng,
        )
    }

    /// One pool card out of those `ok` accepts, uniformly. With a `rig`
    /// only rig-legal cards count, unless there are none, then all do.
    fn pick_rigged(
        &self,
        ok: impl Fn(usize) -> bool,
        rig: Option<Tribe>,
        rng: &mut Rng,
    ) -> Option<CardId> {
        let rigged = |c: usize| ok(c) && self.cards[c].rig_ok(rig);
        let use_rig = rig.is_some() && (0..self.pool_cards).any(rigged);
        let ok = |c: usize| if use_rig { rigged(c) } else { ok(c) };
        let count = (0..self.pool_cards).filter(|&c| ok(c)).count() as u32;
        if count == 0 {
            return None;
        }
        let k = rng.below(count) as usize;
        (0..self.pool_cards).filter(|&c| ok(c)).nth(k).map(|c| c as CardId)
    }

    /// Whether a spell may be drawn when `exclude` is already taken. The
    /// shared Bounty slot is open while one of the five is not excluded.
    fn spell_open(&self, id: CardId, exclude: &[CardId]) -> bool {
        if self.cards[id as usize].is_bounty {
            self.bounties.iter().any(|b| !exclude.contains(b))
        } else {
            !exclude.contains(&id)
        }
    }

    /// A drawn Bounty slot becomes a random one of the five (not in
    /// `exclude`); any other spell is itself.
    fn resolve_bounty(&self, id: CardId, exclude: &[CardId], rng: &mut Rng) -> CardId {
        if !self.cards[id as usize].is_bounty {
            return id;
        }
        let n = self.bounties.iter().filter(|b| !exclude.contains(b)).count() as u32;
        if n == 0 {
            return id;
        }
        let k = rng.below(n) as usize;
        self.bounties.iter().copied().filter(|b| !exclude.contains(b)).nth(k).unwrap_or(id)
    }

    /// "A random Bounty": one of the five, uniformly.
    pub fn random_bounty(&self, rng: &mut Rng) -> Option<CardId> {
        if self.bounties.is_empty() {
            return None;
        }
        Some(self.bounties[rng.below(self.bounties.len() as u32) as usize])
    }

    /// A random spell that costs `cost`, of any Tier.
    pub fn random_spell_of_cost(&self, cost: u8, rng: &mut Rng) -> Option<CardId> {
        let ok = |id: CardId| self.cards[id as usize].cost == cost;
        let count = self.spells.iter().filter(|&&id| ok(id)).count() as u32;
        if count == 0 {
            return None;
        }
        let k = rng.below(count) as usize;
        let id = self.spells.iter().copied().filter(|&id| ok(id)).nth(k)?;
        Some(self.resolve_bounty(id, &[], rng))
    }

    /// A random pool minion sharing a tribe with `tribes`, other than
    /// `exclude`, of Tier `max_tier` or lower.
    pub fn random_different_minion(
        &self,
        tribes: Tribes,
        exclude: CardId,
        max_tier: u8,
        rng: &mut Rng,
    ) -> Option<CardId> {
        let max_tier = if max_tier == 0 { crate::MAX_TIER } else { max_tier };
        let ok = |c: usize| {
            self.cards[c].tribes.overlaps(tribes)
                && c as CardId != exclude
                && self.cards[c].tier <= max_tier
        };
        let count = (0..self.pool_cards).filter(|&c| ok(c)).count() as u32;
        if count == 0 {
            return None;
        }
        let k = rng.below(count) as usize;
        (0..self.pool_cards).filter(|&c| ok(c)).nth(k).map(|c| c as CardId)
    }

    /// A random tier 7 minion, for the cards that say "a Tier 7 minion"
    /// (none in the lobby yet, Greedy Conniver is the first). Never
    /// reachable any other way: every other draw stops at the Tavern tiers.
    pub fn random_tier7_minion(&self, rig: Option<Tribe>, rng: &mut Rng) -> Option<CardId> {
        self.random_tier_minion(crate::CARD_MAX_TIER, rig, rng)
    }

    /// A random pool minion of exactly `tier`, narrowed by a sandbox `rig`
    /// like `random_deathrattle_minion`.
    pub fn random_tier_minion(&self, tier: u8, rig: Option<Tribe>, rng: &mut Rng) -> Option<CardId> {
        self.pick_rigged(|c| self.cards[c].tier == tier, rig, rng)
    }

    pub fn random_spell(&self, max_tier: u8, rng: &mut Rng) -> Option<CardId> {
        let count = self
            .spells
            .iter()
            .filter(|&&id| self.cards[id as usize].tier <= max_tier)
            .count() as u32;
        if count == 0 {
            return None;
        }
        let k = rng.below(count) as usize;
        let id = self
            .spells
            .iter()
            .copied()
            .filter(|&id| self.cards[id as usize].tier <= max_tier)
            .nth(k)?;
        Some(self.resolve_bounty(id, &[], rng))
    }

    /// A random Tavern spell of Tier `max_tier` or lower, other than the
    /// cards in `exclude`.
    pub fn random_spell_up_to(
        &self,
        max_tier: u8,
        exclude: &[CardId],
        rng: &mut Rng,
    ) -> Option<CardId> {
        let ok = |id: CardId| self.cards[id as usize].tier <= max_tier && self.spell_open(id, exclude);
        let count = self.spells.iter().filter(|&&id| ok(id)).count() as u32;
        if count == 0 {
            return None;
        }
        let k = rng.below(count) as usize;
        let id = self.spells.iter().copied().filter(|&id| ok(id)).nth(k)?;
        Some(self.resolve_bounty(id, exclude, rng))
    }

    pub fn random_tier_spell(
        &self,
        tier: u8,
        exclude: &[CardId],
        rng: &mut Rng,
    ) -> Option<CardId> {
        let ok = |id: CardId| self.cards[id as usize].tier == tier && self.spell_open(id, exclude);
        let count = self.spells.iter().filter(|&&id| ok(id)).count() as u32;
        if count == 0 {
            return None;
        }
        let k = rng.below(count) as usize;
        let id = self.spells.iter().copied().filter(|&id| ok(id)).nth(k)?;
        Some(self.resolve_bounty(id, exclude, rng))
    }

    /// A fresh generated copy of a card (`pool_copies = 0`).
    pub fn make(&self, id: CardId, golden: bool) -> Minion {
        let c = self.get(id);
        // Aureate Laureate is always Golden, and gives no Triple Reward.
        let golden = golden || c.always_golden;
        let m = if golden { 2 } else { 1 };
        let (attack, health) = if golden {
            c.golden_stats.unwrap_or((c.attack * 2, c.health * 2))
        } else {
            (c.attack * m, c.health * m)
        };
        Minion {
            card: id,
            attack,
            health,
            keywords: c.keywords,
            golden,
            no_reward: c.always_golden,
            temp_shield: false,
            pool_copies: 0,
            winners_bread: 0,
            magnetizations: 0,
            volumizers: u16::from(c.is_volumizer),
            // A Lockbox's `improvements` is the turns it has left.
            improvements: match c.spell_effect {
                Some(SpellEffect::Lockbox(n)) => n as u16,
                _ => 0,
            },
            extra_magnetize: 0,
            taught_spell: None,
            locked_turns: 0,
            activated: false,
            attached: [Attached::default(); MAX_ATTACHED],
            n_attached: 0,
        }
    }
}
