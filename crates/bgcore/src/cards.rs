use serde::Deserialize;

use crate::minion::{Attached, CardId, Keywords, MAX_ATTACHED, Minion};
use crate::rng::Rng;

/// A per-player count that an "improve your future ..." effect raises and
/// scales by. Each variant is one slot in `Counters::improve`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Counter {
    TastyLobster,
}

pub const N_COUNTERS: usize = 1;

impl Counter {
    pub fn index(self) -> usize {
        self as usize
    }
}

fn counter(name: &str) -> Result<Counter, String> {
    match name {
        "tasty_lobster" => Ok(Counter::TastyLobster),
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
}

impl Tribe {
    pub const ALL: [Tribe; 5] = [Tribe::Beast, Tribe::Mech, Tribe::Dragon, Tribe::Murloc, Tribe::Demon];

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
    pub const ALL: Tribes = Tribes(0b11111);

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
        )
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
            | Effect::BuffRandomHand { .. } => true,
            _ => false,
        }
    }

    /// Whether the effect picks one of the player's Tavern cards, so it can
    /// only be activated while there is one.
    pub fn needs_shop_slot(self) -> bool {
        matches!(self, Effect::ReplaceWithFishbait { .. })
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Aura {
    Brann,
    Titus,
    Drakkari,
    Balinda,
    /// Has Taunt while it has Divine Shield.
    TauntWhileShielded,
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
    /// Refreshes each turn that cost Health instead of Gold.
    pub health_refreshes: u8,
    pub on_sell: Option<OnSell>,
    pub aura: Option<Aura>,
    pub spell_target: Option<SpellTarget>,
    pub spell_effect: Option<SpellEffect>,
}

impl Card {
    /// Whether the card has this tribe, always true for no filter.
    pub fn is(&self, tribe: Option<Tribe>) -> bool {
        tribe.is_none_or(|t| self.tribes.has(t))
    }

    pub fn is_spell(&self) -> bool {
        self.spell
    }

    /// Whether this spell gives stats to minions (used by the rule bot).
    pub fn gives_stats(&self) -> bool {
        matches!(
            self.spell_target,
            Some(
                SpellTarget::Buff { .. }
                    | SpellTarget::BuffSameTribe { .. }
                    | SpellTarget::BuffPerBonusKeyword { .. }
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
            || self.health_refreshes > 0
            || self.on_sell.is_some()
            || self.aura.is_some()
            || self.spell_buff.is_some()
    }
}

/// All cards. Ids `0..pool_cards` are the pool minions, tokens and spells
/// come after.
#[derive(Clone, Debug)]
pub struct CardDb {
    pub cards: Vec<Card>,
    pub pool_cards: usize,
    pub volumizers: Vec<CardId>,
    pub spells: Vec<CardId>,
    pub beetle_token: Option<CardId>,
    /// The five Chromadrakes "a random Chromadrake" picks from.
    pub chromadrakes: Vec<CardId>,
    /// The 1/1 Murloc Magicfin Mycologist teaches a spell to.
    pub taught_token: Option<CardId>,
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
    health_refreshes: Option<u8>,
    on_sell: Option<RawOnSell>,
    aura: Option<String>,
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
    effect: Option<RawSpellEffect>,
    choose_one: Option<[RawSpellEffect; 2]>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSpellEffect {
    target: Option<String>,
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

        let parse_effect = |b: &RawEffect| -> Result<Effect, String> {
            let t = b.tribe.as_deref().map(tribe).transpose()?;
            Ok(if let Some([attack, health]) = b.buff {
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
            if !(1..=crate::MAX_TIER).contains(&c.tier) {
                return Err(err(format!("tier {} is not 1 to 6", c.tier)));
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
                Some(s) => Some(if let Some(name) = &s.get {
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
                Some(other) => return Err(err(format!("unknown aura {other:?}"))),
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
                health_refreshes: c.health_refreshes.unwrap_or(0),
                on_sell,
                aura,
                spell_target: None,
                spell_effect: None,
            });
        }

        for sp in &raw.spell {
            let id = cards.len() as CardId;
            let err = |e: String| format!("cards.toml, {}: {e}", sp.name);
            if !(1..=crate::MAX_TIER).contains(&sp.tier) {
                return Err(err(format!("tier {} is not 1 to 6", sp.tier)));
            }
            let (spell_target, spell_effect) = match &sp.effect {
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
            spells.push(id);
            cards.push(Card {
                name: sp.name.clone(),
                tier: sp.tier,
                tribes: Tribes::NONE,
                attack: 0,
                health: 0,
                golden_stats: None,
                keywords: Keywords::default(),
                token: false,
                spell: true,
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
                health_refreshes: 0,
                on_sell: None,
                aura: None,
                spell_target,
                spell_effect,
            });
        }

        Ok(CardDb {
            cards,
            pool_cards,
            volumizers,
            spells,
            beetle_token,
            chromadrakes,
            taught_token,
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

    pub fn random_pool_minion(&self, tribe: Tribe, rng: &mut Rng) -> Option<CardId> {
        let count = (0..self.pool_cards)
            .filter(|&c| self.cards[c].tribes.has(tribe))
            .count() as u32;
        if count == 0 {
            return None;
        }
        let k = rng.below(count) as usize;
        (0..self.pool_cards)
            .filter(|&c| self.cards[c].tribes.has(tribe))
            .nth(k)
            .map(|c| c as CardId)
    }

    /// A random pool minion that has a Deathrattle, of Tier `max_tier` or
    /// lower (0 means any Tier).
    pub fn random_deathrattle_minion(&self, max_tier: u8, rng: &mut Rng) -> Option<CardId> {
        let max_tier = if max_tier == 0 { crate::MAX_TIER } else { max_tier };
        let ok = |c: usize| self.cards[c].deathrattle.is_some() && self.cards[c].tier <= max_tier;
        let count = (0..self.pool_cards).filter(|&c| ok(c)).count() as u32;
        if count == 0 {
            return None;
        }
        let k = rng.below(count) as usize;
        (0..self.pool_cards).filter(|&c| ok(c)).nth(k).map(|c| c as CardId)
    }

    /// A random spell that costs `cost`, of any Tier.
    pub fn random_spell_of_cost(&self, cost: u8, rng: &mut Rng) -> Option<CardId> {
        let ok = |id: CardId| self.cards[id as usize].cost == cost;
        let count = self.spells.iter().filter(|&&id| ok(id)).count() as u32;
        if count == 0 {
            return None;
        }
        let k = rng.below(count) as usize;
        self.spells.iter().copied().filter(|&id| ok(id)).nth(k)
    }

    /// A random pool minion sharing a tribe with `tribes`, other than `exclude`.
    pub fn random_different_minion(
        &self,
        tribes: Tribes,
        exclude: CardId,
        rng: &mut Rng,
    ) -> Option<CardId> {
        let ok = |c: usize| self.cards[c].tribes.overlaps(tribes) && c as CardId != exclude;
        let count = (0..self.pool_cards).filter(|&c| ok(c)).count() as u32;
        if count == 0 {
            return None;
        }
        let k = rng.below(count) as usize;
        (0..self.pool_cards).filter(|&c| ok(c)).nth(k).map(|c| c as CardId)
    }

    pub fn random_tier_minion(&self, tier: u8, rng: &mut Rng) -> Option<CardId> {
        let count = (0..self.pool_cards)
            .filter(|&c| self.cards[c].tier == tier)
            .count() as u32;
        if count == 0 {
            return None;
        }
        let k = rng.below(count) as usize;
        (0..self.pool_cards)
            .filter(|&c| self.cards[c].tier == tier)
            .nth(k)
            .map(|c| c as CardId)
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
        self.spells
            .iter()
            .copied()
            .filter(|&id| self.cards[id as usize].tier <= max_tier)
            .nth(k)
    }

    pub fn random_tier_spell(
        &self,
        tier: u8,
        exclude: &[CardId],
        rng: &mut Rng,
    ) -> Option<CardId> {
        let ok = |id: CardId| self.cards[id as usize].tier == tier && !exclude.contains(&id);
        let count = self.spells.iter().filter(|&&id| ok(id)).count() as u32;
        if count == 0 {
            return None;
        }
        let k = rng.below(count) as usize;
        self.spells.iter().copied().filter(|&id| ok(id)).nth(k)
    }

    /// A fresh generated copy of a card (`pool_copies = 0`).
    pub fn make(&self, id: CardId, golden: bool) -> Minion {
        let c = self.get(id);
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
            pool_copies: 0,
            winners_bread: 0,
            magnetizations: 0,
            volumizers: u16::from(c.is_volumizer),
            improvements: 0,
            extra_magnetize: 0,
            taught_spell: None,
            attached: [Attached::default(); MAX_ATTACHED],
            n_attached: 0,
        }
    }
}
