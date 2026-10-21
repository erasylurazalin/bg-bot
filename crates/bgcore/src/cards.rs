use serde::Deserialize;

use crate::minion::{CardId, Keywords, Minion};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tribe {
    Beast,
    Mech,
    Neutral,
}

/// Card text as data. New mechanics add variants here, not code per card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Summon `count` copies of a token where the minion stood.
    Summon { card: CardId, count: u8 },
    /// Buff one other friendly minion the player picks.
    BuffChosen { tribe: Option<Tribe>, attack: i32, health: i32 },
    /// Buff every other friendly minion.
    BuffOthers { tribe: Option<Tribe>, attack: i32, health: i32 },
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
    pub battlecry: Option<Effect>,
    pub deathrattle: Option<Effect>,
}

impl Card {
    pub fn is(&self, tribe: Option<Tribe>) -> bool {
        tribe.is_none_or(|t| self.tribe == t)
    }
}

/// All cards. Ids `0..pool_cards` are the pool minions, tokens come after.
#[derive(Clone, Debug)]
pub struct CardDb {
    pub cards: Vec<Card>,
    pub pool_cards: usize,
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
    battlecry: Option<RawEffect>,
    deathrattle: Option<RawEffect>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEffect {
    summon: Option<String>,
    count: Option<u8>,
    buff: Option<[i32; 2]>,
    target: Option<String>,
    tribe: Option<String>,
}

fn tribe(s: &str) -> Result<Tribe, String> {
    match s {
        "beast" => Ok(Tribe::Beast),
        "mech" => Ok(Tribe::Mech),
        "neutral" => Ok(Tribe::Neutral),
        _ => Err(format!("unknown tribe {s:?}")),
    }
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
        let id_of = |name: &str| -> Result<CardId, String> {
            names
                .iter()
                .position(|n| n == name)
                .map(|i| i as CardId)
                .ok_or_else(|| format!("unknown card {name:?}"))
        };
        let mut cards = Vec::with_capacity(all.len());
        for (c, token) in &all {
            let err = |e: String| format!("cards.toml, {}: {e}", c.name);
            if !(1..=crate::MAX_TIER).contains(&c.tier) {
                return Err(err(format!("tier {} is not 1 to 6", c.tier)));
            }
            let mut keywords = Keywords::default();
            for k in &c.keywords {
                let bit = Keywords::ALL
                    .iter()
                    .find(|(_, n)| n == k)
                    .ok_or_else(|| err(format!("unknown keyword {k:?}")))?;
                keywords.set(bit.0);
            }
            let battlecry = match &c.battlecry {
                None => None,
                Some(e) => {
                    if e.summon.is_some() || e.count.is_some() {
                        return Err(err("a battlecry can only buff".into()));
                    }
                    let [attack, health] = e.buff.ok_or_else(|| err("battlecry needs buff".into()))?;
                    let t = e.tribe.as_deref().map(tribe).transpose().map_err(err)?;
                    Some(match e.target.as_deref() {
                        Some("chosen") => Effect::BuffChosen { tribe: t, attack, health },
                        Some("others") => Effect::BuffOthers { tribe: t, attack, health },
                        other => return Err(err(format!("battlecry target {other:?}"))),
                    })
                }
            };
            let deathrattle = match &c.deathrattle {
                None => None,
                Some(e) => {
                    if e.buff.is_some() || e.target.is_some() || e.tribe.is_some() {
                        return Err(err("a deathrattle can only summon".into()));
                    }
                    let name = e.summon.as_deref().ok_or_else(|| err("deathrattle needs summon".into()))?;
                    let card = id_of(name).map_err(err)?;
                    if !all[card as usize].1 {
                        return Err(err(format!("{name} is not a token")));
                    }
                    Some(Effect::Summon { card, count: e.count.unwrap_or(1) })
                }
            };
            cards.push(Card {
                name: c.name.clone(),
                tier: c.tier,
                tribe: tribe(&c.tribe).map_err(err)?,
                attack: c.stats[0],
                health: c.stats[1],
                keywords,
                token: *token,
                battlecry,
                deathrattle,
            });
        }
        for (i, n) in names.iter().enumerate() {
            if names[..i].contains(n) {
                return Err(format!("cards.toml: {n} appears twice"));
            }
        }
        Ok(CardDb { cards, pool_cards })
    }

    pub fn get(&self, id: CardId) -> &Card {
        &self.cards[id as usize]
    }

    pub fn by_name(&self, name: &str) -> Option<CardId> {
        self.cards.iter().position(|c| c.name == name).map(|i| i as CardId)
    }

    /// A fresh copy of a card, as the shop or a summon makes it.
    pub fn make(&self, id: CardId, golden: bool) -> Minion {
        let c = self.get(id);
        let m = if golden { 2 } else { 1 };
        Minion { card: id, attack: c.attack * m, health: c.health * m, keywords: c.keywords, golden }
    }
}
