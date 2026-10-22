use serde::Deserialize;

use crate::minion::{Attached, CardId, Keywords, MAX_ATTACHED, Minion};
use crate::rng::Rng;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tribe {
    Beast,
    Mech,
    Neutral,
}

/// Battlecry and general card text effects.
/// Phase B will extend these trigger enums with spell-cast triggers and
/// spell-generating effects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Summon `count` copies of a token where the minion stood.
    Summon { card: CardId, count: u8 },
    /// Buff one other friendly minion the player picks.
    BuffChosen { tribe: Option<Tribe>, attack: i32, health: i32 },
    /// Buff every other friendly minion.
    BuffOthers { tribe: Option<Tribe>, attack: i32, health: i32 },
    /// Give your Beetles +attack/+health this game.
    BeetleBuff { attack: i32, health: i32 },
    /// Gain extra Gold next turn.
    GoldNextTurn(u8),
    /// Get a random Magnetic Volumizer (2 when golden).
    RandomVolumizer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ChooseOption {
    pub tribe: Option<Tribe>,
    pub attack: i32,
    pub health: i32,
    pub keyword: u8,
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
pub enum AfterFriendlyAttack {
    BeetleBuff { attack: i32, health: i32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AfterBuy {
    AutoReveille { every: u8 },
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
}

#[derive(Clone, Debug)]
pub struct Card {
    pub name: String,
    pub tier: u8,
    pub tribe: Tribe,
    pub attack: i32,
    pub health: i32,
    pub keywords: Keywords,
    pub token: bool,
    pub is_beetle: bool,
    pub is_volumizer: bool,
    /// Index in `Game::pool` that holds copies of this card. Shared pool
    /// groups (such as the three Volumizers) all point to their first card's
    /// slot.
    pub pool_slot: CardId,
    pub volumizer_buff: Option<(i32, i32)>,
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
    pub after_friendly_attack: Option<AfterFriendlyAttack>,
    pub after_buy: Option<AfterBuy>,
    pub on_sell: Option<OnSell>,
    pub aura: Option<Aura>,
}

impl Card {
    pub fn is(&self, tribe: Option<Tribe>) -> bool {
        tribe.is_none_or(|t| self.tribe == t)
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
            || self.after_friendly_attack.is_some()
            || self.after_buy.is_some()
            || self.on_sell.is_some()
            || self.aura.is_some()
    }
}

/// All cards. Ids `0..pool_cards` are the pool minions, tokens come after.
#[derive(Clone, Debug)]
pub struct CardDb {
    pub cards: Vec<Card>,
    pub pool_cards: usize,
    pub volumizers: Vec<CardId>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    minion: Vec<RawCard>,
    #[serde(default)]
    token: Vec<RawCard>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCard {
    name: String,
    tier: u8,
    tribe: String,
    stats: [i32; 2],
    #[serde(default)]
    keywords: Vec<String>,
    #[serde(default)]
    beetle: bool,
    volumizer_buff: Option<[i32; 2]>,
    battlecry: Option<RawBattlecry>,
    choose_one: Option<[RawChooseOption; 2]>,
    deathrattle: Option<RawDeathrattle>,
    start_of_turn: Option<RawStartOfTurn>,
    end_of_turn: Option<RawEndOfTurn>,
    start_of_combat: Option<RawStartOfCombat>,
    rally: Option<RawRally>,
    on_summon_beast: Option<RawOnSummonBeast>,
    on_play_mech: Option<RawOnPlayMech>,
    on_play_or_magnetize_mech: Option<RawBuff>,
    after_friendly_attack: Option<RawBeetleBuff>,
    after_buy: Option<RawAfterBuy>,
    on_sell: Option<RawOnSell>,
    aura: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBattlecry {
    buff: Option<[i32; 2]>,
    target: Option<String>,
    tribe: Option<String>,
    beetle_buff: Option<[i32; 2]>,
    gold_next_turn: Option<u8>,
    random_volumizer: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawChooseOption {
    buff: [i32; 2],
    keyword: String,
    tribe: Option<String>,
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
struct RawAfterBuy {
    every: u8,
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
        "neutral" => Ok(Tribe::Neutral),
        _ => Err(format!("unknown tribe {s:?}")),
    }
}

fn keyword_bit(s: &str) -> Result<u8, String> {
    Keywords::ALL
        .iter()
        .find(|(_, n)| *n == s)
        .map(|(b, _)| *b)
        .ok_or_else(|| format!("unknown keyword {s:?}"))
}

impl CardDb {
    pub fn from_toml(s: &str) -> Result<CardDb, String> {
        let raw: RawFile = toml::from_str(s).map_err(|e| format!("cards.toml: {e}"))?;
        let pool_cards = raw.minion.len();
        let all: Vec<(RawCard, bool)> = raw
            .minion
            .into_iter()
            .map(|c| (c, false))
            .chain(raw.token.into_iter().map(|c| (c, true)))
            .collect();
        if all.len() > CardId::MAX as usize {
            return Err("cards.toml: too many cards".into());
        }
        let names: Vec<String> = all.iter().map(|(c, _)| c.name.clone()).collect();
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
            if !all[id].1 {
                return Err(format!("{name} is not a token"));
            }
            Ok(id as CardId)
        };

        let mut cards = Vec::with_capacity(all.len());
        let mut volumizers = Vec::new();
        let mut first_volumizer_slot: Option<CardId> = None;

        for (idx, (c, token)) in all.iter().enumerate() {
            let id = idx as CardId;
            let err = |e: String| format!("cards.toml, {}: {e}", c.name);
            if !(1..=crate::MAX_TIER).contains(&c.tier) {
                return Err(err(format!("tier {} is not 1 to 6", c.tier)));
            }
            let mut keywords = Keywords::default();
            for k in &c.keywords {
                keywords.set(keyword_bit(k).map_err(&err)?);
            }
            let is_volumizer = c.volumizer_buff.is_some();
            let pool_slot = if is_volumizer {
                volumizers.push(id);
                *first_volumizer_slot.get_or_insert(id)
            } else {
                id
            };
            let volumizer_buff = c.volumizer_buff.map(|[a, h]| (a, h));

            let battlecry = match &c.battlecry {
                None => None,
                Some(b) => Some(if let Some([attack, health]) = b.buff {
                    let t = b.tribe.as_deref().map(tribe).transpose().map_err(&err)?;
                    match b.target.as_deref() {
                        Some("chosen") => Effect::BuffChosen { tribe: t, attack, health },
                        Some("others") => Effect::BuffOthers { tribe: t, attack, health },
                        other => return Err(err(format!("battlecry target {other:?}"))),
                    }
                } else if let Some([attack, health]) = b.beetle_buff {
                    Effect::BeetleBuff { attack, health }
                } else if let Some(g) = b.gold_next_turn {
                    Effect::GoldNextTurn(g)
                } else if b.random_volumizer == Some(true) {
                    Effect::RandomVolumizer
                } else {
                    return Err(err("invalid battlecry".into()));
                }),
            };

            let choose_one = match &c.choose_one {
                None => None,
                Some([o1, o2]) => {
                    let parse_opt = |o: &RawChooseOption| -> Result<ChooseOption, String> {
                        Ok(ChooseOption {
                            tribe: o.tribe.as_deref().map(tribe).transpose()?,
                            attack: o.buff[0],
                            health: o.buff[1],
                            keyword: keyword_bit(&o.keyword)?,
                        })
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

            let after_friendly_attack = c.after_friendly_attack.as_ref().map(|b| {
                AfterFriendlyAttack::BeetleBuff {
                    attack: b.beetle_buff[0],
                    health: b.beetle_buff[1],
                }
            });

            let after_buy = c.after_buy.as_ref().map(|b| AfterBuy::AutoReveille { every: b.every });

            let on_sell = match &c.on_sell {
                None => None,
                Some(s) => Some(if let Some(name) = &s.get {
                    let card = token_id(name).map_err(&err)?;
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
                Some(other) => return Err(err(format!("unknown aura {other:?}"))),
            };

            cards.push(Card {
                name: c.name.clone(),
                tier: c.tier,
                tribe: tribe(&c.tribe).map_err(&err)?,
                attack: c.stats[0],
                health: c.stats[1],
                keywords,
                token: *token,
                is_beetle: c.beetle,
                is_volumizer,
                pool_slot,
                volumizer_buff,
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
                after_friendly_attack,
                after_buy,
                on_sell,
                aura,
            });
        }

        Ok(CardDb { cards, pool_cards, volumizers })
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
            .filter(|&c| self.cards[c].tribe == tribe)
            .count() as u32;
        if count == 0 {
            return None;
        }
        let k = rng.below(count) as usize;
        (0..self.pool_cards)
            .filter(|&c| self.cards[c].tribe == tribe)
            .nth(k)
            .map(|c| c as CardId)
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
            magnetizations: 0,
            volumizers: u16::from(c.is_volumizer),
            improvements: 0,
            attached: [Attached::default(); MAX_ATTACHED],
            n_attached: 0,
        }
    }
}
