use serde::Deserialize;

use crate::minion::{Attached, CardId, Keywords, MAX_ATTACHED, Minion};
use crate::rng::Rng;

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
    /// Summon `count` copies of a token where the minion stood.
    Summon { card: CardId, count: u8 },
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
    /// Buff the minion itself.
    BuffSelf { attack: i32, health: i32 },
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
}

impl Effect {
    /// Whether the player has to pick something to finish this effect. Event
    /// triggers and end of turn effects can't ask, so they can't use these.
    pub fn needs_choice(self) -> bool {
        matches!(
            self,
            Effect::BuffChosen { .. }
                | Effect::Consume { by: ConsumeBy::Chosen(_), .. }
                | Effect::Consume { pick: ConsumePick::Chosen, .. }
        )
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
    BroodOfNozdormu,
    UpperHand,
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rally {
    Summon { card: CardId, count: u8, golden_doubles_count: bool },
    RandomMinion(Tribe),
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AfterFriendlyAttack {
    BeetleBuff { attack: i32, health: i32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnSell {
    GetCard { card: CardId, count: u8 },
    DiscoverByTier,
    GoldIfLost(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Aura {
    Brann,
    Titus,
    Drakkari,
    Balinda,
}

#[derive(Clone, Debug)]
pub struct Card {
    pub name: String,
    pub tier: u8,
    pub tribes: Tribes,
    pub attack: i32,
    pub health: i32,
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
    pub rally: Option<Rally>,
    pub on_summon_beast: Option<OnSummonBeast>,
    pub on_play_mech: Option<OnPlayMech>,
    pub on_play_or_magnetize_mech: Option<OnPlayOrMagnetizeMech>,
    pub on_cast_spell_on_mech: Option<OnCastSpellOnMech>,
    pub on_cast_spell: Option<OnCastSpell>,
    pub after_friendly_attack: Option<AfterFriendlyAttack>,
    pub activate: Option<Activate>,
    pub on_event: Option<EventTrigger>,
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
        matches!(self.spell_target, Some(SpellTarget::Buff { .. } | SpellTarget::BuffSameTribe { .. }))
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
            || self.after_friendly_attack.is_some()
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
    rally: Option<RawRally>,
    on_summon_beast: Option<RawOnSummonBeast>,
    on_play_mech: Option<RawOnPlayMech>,
    on_play_or_magnetize_mech: Option<RawBuff>,
    on_cast_spell_on_mech: Option<RawOnCastSpellOnMech>,
    on_cast_spell: Option<RawOnCastSpell>,
    after_friendly_attack: Option<RawBeetleBuff>,
    activate: Option<RawActivate>,
    on_event: Option<RawOnEvent>,
    #[serde(default)]
    after_hero_damage: Vec<RawEffect>,
    health_refreshes: Option<u8>,
    on_sell: Option<RawOnSell>,
    aura: Option<String>,
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
    get: Option<String>,
    count: Option<u8>,
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
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRally {
    summon: Option<String>,
    count: Option<u8>,
    #[serde(default)]
    golden_doubles_count: bool,
    random_minion: Option<String>,
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
    buff_divine_shield: [i32; 2],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBuff {
    buff: [i32; 2],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBeetleBuff {
    beetle_buff: [i32; 2],
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
    match e.target.as_deref() {
        Some("chosen") => {
            if let Some([attack, health]) = e.buff {
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

        let mut cards = Vec::with_capacity(total_cards);
        let mut volumizers = Vec::new();
        let mut spells = Vec::with_capacity(raw.spell.len());
        let mut first_volumizer_slot: Option<CardId> = None;
        let mut beetle_token: Option<CardId> = None;

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
                    other => return Err(format!("battlecry target {other:?}")),
                }
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
                } else {
                    return Err(err("invalid start_of_combat".into()));
                }),
            };

            let rally = match &c.rally {
                None => None,
                Some(r) => Some(if let Some(name) = &r.summon {
                    let card = token_id(name).map_err(&err)?;
                    Rally::Summon {
                        card,
                        count: r.count.unwrap_or(1),
                        golden_doubles_count: r.golden_doubles_count,
                    }
                } else if let Some(t) = &r.random_minion {
                    Rally::RandomMinion(tribe(t).map_err(&err)?)
                } else {
                    return Err(err("invalid rally".into()));
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

            let on_cast_spell = c.on_cast_spell.as_ref().map(|cs| {
                OnCastSpell::BuffDivineShield {
                    attack: cs.buff_divine_shield[0],
                    health: cs.buff_divine_shield[1],
                }
            });

            let after_friendly_attack = c.after_friendly_attack.as_ref().map(|b| {
                AfterFriendlyAttack::BeetleBuff {
                    attack: b.beetle_buff[0],
                    health: b.beetle_buff[1],
                }
            });

            let on_sell = match &c.on_sell {
                None => None,
                Some(s) => Some(if let Some(name) = &s.get {
                    let card = generated_id(name).map_err(&err)?;
                    OnSell::GetCard { card, count: s.count.unwrap_or(1) }
                } else if s.discover_by_tier == Some(true) {
                    OnSell::DiscoverByTier
                } else if let Some(g) = s.gold_if_lost {
                    OnSell::GoldIfLost(g)
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
                Some(other) => return Err(err(format!("unknown aura {other:?}"))),
            };

            cards.push(Card {
                name: c.name.clone(),
                tier: c.tier,
                tribes: card_tribes(&c.tribe, &c.tribes).map_err(&err)?,
                attack: c.stats[0],
                health: c.stats[1],
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
                after_friendly_attack,
                activate,
                on_event,
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
                after_friendly_attack: None,
                activate: None,
                on_event: None,
                after_hero_damage: Vec::new(),
                health_refreshes: 0,
                on_sell: None,
                aura: None,
                spell_target,
                spell_effect,
            });
        }

        Ok(CardDb { cards, pool_cards, volumizers, spells, beetle_token })
    }

    pub fn get(&self, id: CardId) -> &Card {
        &self.cards[id as usize]
    }

    pub fn by_name(&self, name: &str) -> Option<CardId> {
        if let Some(i) = self.cards.iter().position(|c| c.name == name) {
            return Some(i as CardId);
        }
        let alt = match name {
            "Goldrinn, the Great Wolf" | "Goldrinn" => "Goldrinn the Great Wolf",
            "Half-Shell" => "Turtle",
            "Aberrant Tentacle" => "Tentacle",
            "Beast" => "Foraging Bat",
            "Rats" => "Rat",
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
        Minion {
            card: id,
            attack: c.attack * m,
            health: c.health * m,
            keywords: c.keywords,
            golden,
            pool_copies: 0,
            winners_bread: 0,
            magnetizations: 0,
            volumizers: u16::from(c.is_volumizer),
            improvements: 0,
            attached: [Attached::default(); MAX_ATTACHED],
            n_attached: 0,
        }
    }
}
