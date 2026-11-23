//! The screen replay: seat a shop turn from the state a video screen showed at
//! its start, play the steps a vision converter read, and compare the engine's
//! end of turn with the state the screen showed before the fight
//! (`docs/real-boards.md` "The screen file" and "The screen replay").
//!
//! There is no Power.log behind a screen file: no entity ids, hidden counters,
//! enchantment numbers or states between steps. The turn is seated once, then
//! each step is mapped from the carried game's own zones and played on it. A
//! seen Tavern or hand replaces the engine's draw, as the whole-turn replay
//! does with the log's.
//!
//! Nothing here changes a rule. What the screen does not show keeps the
//! engine's default for the turn; a step the engine refuses or cannot map
//! stops the turn, and the end comparison flags what differs.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::minion::Keywords;
use crate::real_boards::{self, CardIdMap};
use crate::shop_replay::{
    self, HeroRef, ME, ShopFileSpec, ShopGiftSpec, ShopMinionSpec, ShopPlace, ShopPowerSpec,
    ShopSnapshot, ShopStateSpec, ShopStepSpec, ShopTrinketSpec, ShopTurnSpec,
};
use crate::{Action, CardId, Data, Game, MAX_TIER, Minion};

// ---- the screen file (docs/real-boards.md "The screen file") ----

/// One game as a screen converter writes it. Every field but the ones that
/// name a card or a step is optional: a missing field means "not seen", which
/// is not the same as an empty zone.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ScreenFileSpec {
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub game: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hero: Option<ScreenHero>,
    /// The lobby's tribes, when the screen showed them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lobby: Vec<String>,
    #[serde(default)]
    pub turns: Vec<ScreenTurnSpec>,
}

impl ScreenFileSpec {
    pub fn file(&self) -> String {
        format!("{}-g{}.json", self.source, self.game)
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ScreenHero {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// One state the screen showed: a turn's `start`, a step's optional `seen`,
/// or a turn's `end`. A missing field was not seen.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ScreenStateSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub board: Option<Vec<ScreenCard>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hand: Option<Vec<ScreenCard>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tavern: Option<Vec<ScreenCard>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gold: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gold_max: Option<i32>,
    /// Whether the next Refresh is free: the Refresh button shows a price of
    /// 0. The screen does not show how many free Refreshes are left.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub free_refresh: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upgrade_cost: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub armor: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frozen: Option<bool>,
    /// The main hero power: a card id or the name the screen showed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hero_power: Option<ScreenPower>,
    /// The second power slot, when the hero has two.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hero_power_second: Option<ScreenPower>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trinkets: Option<Vec<ScreenTrinket>>,
    /// Cards in hand the screen could count but not read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hand_count: Option<u32>,
}

/// A stat the screen read: an exact number, the range a "2.1k" display
/// stands for, or `null` when the reader could not tell. A missing field is
/// not the same: it means the card's printed stat.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum ScreenNumber {
    Number(i64),
    Range { min: i64, max: i64 },
    Null,
}

impl ScreenNumber {
    fn concrete(self, default: i32) -> i32 {
        match self {
            ScreenNumber::Number(n) => n.clamp(i32::MIN as i64, i32::MAX as i64) as i32,
            ScreenNumber::Range { min, .. } => min.clamp(i32::MIN as i64, i32::MAX as i64) as i32,
            ScreenNumber::Null => default,
        }
    }

    fn text(self) -> String {
        match self {
            ScreenNumber::Number(n) => n.to_string(),
            ScreenNumber::Range { min, max } => format!("{min}..{max}"),
            ScreenNumber::Null => "null".to_string(),
        }
    }
}

fn deserialize_some_number<'de, D>(d: D) -> Result<Option<ScreenNumber>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    ScreenNumber::deserialize(d).map(Some)
}

/// A Dark Gift the screen showed on a card: its card id or its name, and the
/// countdown when the icon showed one. A plain string is accepted as the id or
/// the name, which is what a video converter reads most easily.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ScreenGift {
    Read(String),
    At {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        card_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        state: Option<i32>,
    },
}

impl ScreenGift {
    fn read(&self) -> (Option<&str>, Option<&str>, Option<i32>) {
        match self {
            // A plain string may be the gift's card id or its name.
            ScreenGift::Read(s) => (Some(s.as_str()), Some(s.as_str()), None),
            ScreenGift::At {
                card_id,
                name,
                state,
            } => (card_id.as_deref(), name.as_deref(), *state),
        }
    }
}

/// One card as the screen showed it. A missing `attack`/`health` means the
/// card's printed stats, doubled when golden; a missing `keywords` means the
/// card's printed keywords. A stat may instead be a number, a range
/// (`{"min": 2050, "max": 2149}`) or `null` for "cannot tell".
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ScreenCard {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_some_number",
        skip_serializing_if = "Option::is_none"
    )]
    pub attack: Option<ScreenNumber>,
    #[serde(
        default,
        deserialize_with = "deserialize_some_number",
        skip_serializing_if = "Option::is_none"
    )]
    pub health: Option<ScreenNumber>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub golden: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keywords: Option<Vec<String>>,
    /// Only these keywords were read: true means the screen showed it, false
    /// means it showed the keyword was absent. A keyword not named is unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyword_reads: Option<BTreeMap<String, bool>>,
    /// 1-based slot in the Tavern.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<u8>,
    /// `minion` or `spell`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frozen: Option<bool>,
    /// The Gold the card shows in the Tavern (a Tavern spell's price, or a
    /// minion's discounted price).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<i32>,
    /// The card's price is paid in Health rather than Gold.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub costs_health: Option<bool>,
    /// The gift a Dark Gift Discover's option or pick carries, as the video
    /// converter reads it. A plain card id or name is enough.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dark_gift: Option<ScreenGift>,
    /// Every gift icon the screen showed on a card. `None` means the icons
    /// were not read; an empty list means the screen showed none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dark_gifts: Option<Vec<ScreenGift>>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ScreenPower {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<i32>,
    /// Presses already used this turn: the button greys out when spent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used_this_turn: Option<u8>,
    /// The counter the power shows ("(1 left!)"), when it shows one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub numbers: Option<Vec<i32>>,
    /// The minion Lock and Load holds. `waiting_seen` says the screen showed
    /// the slot, so no minion means it holds none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiting: Option<ScreenCard>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiting_seen: Option<bool>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub second: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ScreenTrinket {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub greater: bool,
    /// 0 lesser, 1 greater, 2 the hero power's slot, when the screen says it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<u8>,
    /// The counter the trinket shows ("(1 left!)"), when it shows one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub numbers: Option<Vec<i32>>,
}

/// A step's target or named card: its zone and place, close to the action
/// file's `ShopPlace`.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ScreenPlace {
    /// `board`, `hand` or `slot` (the Tavern); the action file calls it
    /// `kind`, and that spelling is accepted too.
    #[serde(
        default,
        alias = "zone",
        alias = "where",
        skip_serializing_if = "Option::is_none"
    )]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// The option a choice took: its index in the offer, or the card the screen
/// showed picked.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
#[allow(clippy::large_enum_variant)] // one card is much larger than an index
pub enum ScreenPick {
    Index(usize),
    At { index: usize },
    Card(ScreenCard),
}

/// One step as a screen converter writes it. The field names follow the action
/// file's where they can; `seen` is what the screen showed right after it.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ScreenStepSpec {
    pub step: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<u8>,
    /// Where a dragged or played minion landed, 1-based.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub which: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub free: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier_after: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub_option: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot_kind: Option<String>,
    /// `discover`, `choose_one`, `dark_gift` or `triple_reward`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card: Option<ScreenCard>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<ScreenPlace>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<ScreenPlace>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<ScreenCard>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picked: Option<ScreenPick>,
    /// This step's Tavern is a random draw (a Roll or another refresh), so its
    /// contents are applied but not compared with the engine.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub tavern_random: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seen: Option<ScreenStateSpec>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ScreenTurnSpec {
    pub turn: u32,
    #[serde(default)]
    pub start: ScreenStateSpec,
    #[serde(default)]
    pub steps: Vec<ScreenStepSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<ScreenStateSpec>,
}

// ---- resolving what the screen showed ----

fn resolve_card(data: &Data, ids: &CardIdMap, card: &ScreenCard) -> Option<CardId> {
    if let Some(card_id) = card.card_id.as_deref() {
        if let Some((logged, _)) = ids.get(card_id) {
            let name = real_boards::resolved_name(card_id, logged);
            if let Some(c) = data.cards.by_name(&name) {
                return Some(c);
            }
        }
        if let Some(name) = card.name.as_deref()
            && let Some(c) = data
                .cards
                .by_name(&real_boards::resolved_name(card_id, name))
        {
            return Some(c);
        }
    }
    if let Some(name) = card.name.as_deref()
        && let Some(c) = data.cards.by_name(name)
    {
        return Some(c);
    }
    card.card_id
        .as_deref()
        .and_then(|id| data.cards.by_name(id))
}

fn keyword_names(keywords: Keywords) -> Vec<String> {
    Keywords::ALL
        .iter()
        .filter(|(bit, _)| keywords.has(*bit))
        .map(|(_, name)| name.to_string())
        .collect()
}

/// The spelling a keyword read uses: `DIVINE_SHIELD`, `divine shield` and
/// `divine-shield` are the same field.
fn keyword_key(name: &str) -> String {
    name.trim().to_ascii_uppercase().replace([' ', '-'], "_")
}

/// A card's keyword list as the screen read it. A full `keywords` list stands
/// on its own; `keyword_reads` then adds or removes only the keywords it
/// names, and leaves every other one unknown (the printed list on a fresh
/// seat).
fn screen_keywords(card: &ScreenCard, made: Option<&Minion>) -> Vec<String> {
    let mut list = card
        .keywords
        .clone()
        .unwrap_or_else(|| made.map_or_else(Vec::new, |m| keyword_names(m.keywords)));
    if let Some(reads) = &card.keyword_reads {
        list.retain(|name| reads.get(&keyword_key(name)).copied().unwrap_or(true));
        for (name, on) in reads {
            if !*on {
                continue;
            }
            let key = keyword_key(name);
            if list.iter().any(|k| keyword_key(k) == key) {
                continue;
            }
            let canonical = Keywords::ALL
                .iter()
                .find(|(_, known)| keyword_key(known) == key)
                .map(|(_, known)| known.to_string())
                .unwrap_or_else(|| name.clone());
            list.push(canonical);
        }
    }
    list
}

fn card_label(card: &ScreenCard) -> String {
    card.name
        .clone()
        .or_else(|| card.card_id.clone())
        .unwrap_or_else(|| "an unnamed card".to_string())
}

/// The gift icons a screen card carries. The bool says the icons were read at
/// all: an unseen list is unknown, which never takes a gift off the engine's
/// card.
fn gift_specs(data: &Data, card: &ScreenCard) -> (Vec<ShopGiftSpec>, bool, Vec<String>) {
    let known = card.dark_gift.is_some() || card.dark_gifts.is_some();
    let mut out = Vec::new();
    let mut unknown = Vec::new();
    let gifts = card
        .dark_gift
        .iter()
        .chain(card.dark_gifts.iter().flatten());
    for gift in gifts {
        let (id, name, state) = gift.read();
        let found = data.gifts.gifts.iter().find(|g| {
            id.is_some_and(|id| g.id == id)
                || name.is_some_and(|name| g.name == name)
                || id.is_some_and(|id| {
                    data.cards
                        .by_name(&real_boards::resolved_name(id, id))
                        .is_some_and(|c| c == g.card)
                })
                || name.is_some_and(|name| data.cards.by_name(name) == Some(g.card))
        });
        match found {
            Some(g) => out.push(ShopGiftSpec {
                card_id: Some(g.id.clone()),
                name: Some(g.name.clone()),
                state,
                landed: None,
            }),
            None => unknown.push(id.or(name).unwrap_or("an unnamed Dark Gift").to_string()),
        }
    }
    (out, known, unknown)
}

/// One screen card as the shop loader reads a file card. Missing stats are the
/// card's printed ones (doubled when golden), and missing keywords are its
/// printed ones.
fn minion_spec_of(
    data: &Data,
    ids: &CardIdMap,
    card: &ScreenCard,
    missing: &mut Vec<String>,
) -> ShopMinionSpec {
    let (gifts, gifts_known, unknown) = gift_specs(data, card);
    for gift in unknown {
        missing.push(format!("Dark Gift not built: {gift}"));
    }
    let Some(id) = resolve_card(data, ids, card) else {
        missing.push(format!("card not built: {}", card_label(card)));
        return ShopMinionSpec {
            card_id: card.card_id.clone(),
            name: card.name.clone(),
            attack: card.attack.map_or(0, |v| v.concrete(0)),
            health: card.health.map_or(0, |v| v.concrete(0)),
            golden: card.golden.unwrap_or(false),
            keywords: screen_keywords(card, None),
            slot: card.slot,
            kind: card.kind.clone(),
            frozen: card.frozen.unwrap_or(false),
            cost: card.cost,
            costs_health: card.costs_health.unwrap_or(false),
            dark_gifts: gifts,
            gifts_known,
            ..Default::default()
        };
    };
    let c = data.cards.get(id);
    let golden = card.golden.unwrap_or(false) || c.always_golden;
    let made = data.cards.make(id, golden);
    let keywords = screen_keywords(card, Some(&made));
    let mut spec = ShopMinionSpec {
        card_id: card.card_id.clone(),
        name: Some(c.name.clone()),
        attack: card.attack.map_or(made.attack, |v| v.concrete(made.attack)),
        health: card.health.map_or(made.health, |v| v.concrete(made.health)),
        golden,
        keywords,
        slot: card.slot,
        kind: card
            .kind
            .clone()
            .or_else(|| Some(if c.is_spell() { "spell" } else { "minion" }.to_string())),
        frozen: card.frozen.unwrap_or(false),
        costs_health: card.costs_health.unwrap_or(false),
        dark_gifts: gifts,
        gifts_known,
        ..Default::default()
    };
    if let Some(cost) = card.cost {
        if c.is_spell() {
            spec.cost = Some(cost);
        } else {
            let rule = data.rules.buy_cost as i32;
            if cost < rule {
                spec.cost_reduce = Some(rule - cost);
            } else if cost > rule {
                spec.cost_override = Some(cost);
            }
        }
    }
    spec
}

fn resolve_power(data: &Data, ids: &CardIdMap, power: &ScreenPower) -> Option<String> {
    if let Some(card_id) = power.card_id.as_deref()
        && data.heroes.power_by_card_id(card_id).is_some()
    {
        return Some(card_id.to_string());
    }
    let logged_name = power
        .card_id
        .as_deref()
        .and_then(|id| ids.get(id))
        .map(|(n, _)| n);
    for name in [power.name.as_deref(), logged_name].into_iter().flatten() {
        if let Some(p) = data.heroes.powers.iter().find(|p| p.name == name) {
            return Some(p.card_id.clone());
        }
    }
    power.card_id.clone()
}

fn power_spec(data: &Data, ids: &CardIdMap, power: &ScreenPower, second: bool) -> ShopPowerSpec {
    ShopPowerSpec {
        card_id: resolve_power(data, ids, power),
        name: power.name.clone(),
        cost: power.cost,
        numbers: power.numbers.clone().unwrap_or_default(),
        used_this_turn: power.used_this_turn.unwrap_or(0),
        waiting: power
            .waiting
            .as_ref()
            .map(|w| minion_spec_of(data, ids, w, &mut Vec::new())),
        second: second || power.second,
    }
}

fn resolve_trinket(data: &Data, ids: &CardIdMap, trinket: &ScreenTrinket) -> Option<String> {
    if let Some(card_id) = trinket.card_id.as_deref()
        && data.trinkets.by_card_id(card_id).is_some()
    {
        return Some(card_id.to_string());
    }
    let logged_name = trinket
        .card_id
        .as_deref()
        .and_then(|id| ids.get(id))
        .map(|(n, _)| n);
    for name in [trinket.name.as_deref(), logged_name].into_iter().flatten() {
        if let Some(t) = data.trinkets.by_name(name) {
            return Some(data.trinkets.get(t).card_id.clone());
        }
    }
    trinket.card_id.clone()
}

fn trinket_spec(data: &Data, ids: &CardIdMap, trinket: &ScreenTrinket) -> ShopTrinketSpec {
    ShopTrinketSpec {
        card_id: resolve_trinket(data, ids, trinket),
        name: trinket.name.clone(),
        greater: trinket.greater || trinket.slot == Some(1),
        numbers: trinket.numbers.clone().unwrap_or_default(),
    }
}

/// A screen state as the shop loader reads a state: unseen zones are empty,
/// unseen numbers take the engine's own default for the turn, and no counter,
/// Dark Gift or enchantment is invented.
fn state_to_spec(
    data: &Data,
    ids: &CardIdMap,
    state: &ScreenStateSpec,
    turn: u32,
    next_id: &mut u64,
    missing: &mut Vec<String>,
) -> ShopStateSpec {
    let mut build = |cards: Option<&Vec<ScreenCard>>, place: u8| {
        let mut out = Vec::new();
        for (k, card) in cards.into_iter().flatten().enumerate() {
            let mut spec = minion_spec_of(data, ids, card, missing);
            spec.id = *next_id;
            *next_id += 1;
            match place {
                0 => spec.position = Some((k + 1) as u8),
                1 => spec.index = Some((k + 1) as u8),
                _ => spec.slot = Some(card.slot.unwrap_or((k + 1) as u8)),
            }
            out.push(spec);
        }
        out
    };
    let board = build(state.board.as_ref(), 0);
    let hand = build(state.hand.as_ref(), 1);
    let mut tavern = build(state.tavern.as_ref(), 2);
    tavern.sort_by_key(|m| (m.slot.unwrap_or(0), m.id));
    let mut hero_powers = Vec::new();
    if let Some(power) = &state.hero_power {
        hero_powers.push(power_spec(data, ids, power, false));
    }
    if let Some(power) = &state.hero_power_second {
        hero_powers.push(power_spec(data, ids, power, true));
    }
    let mut trinkets: Vec<ShopTrinketSpec> = state
        .trinkets
        .as_ref()
        .map(|list| list.iter().map(|t| trinket_spec(data, ids, t)).collect())
        .unwrap_or_default();
    if state
        .trinkets
        .as_ref()
        .is_some_and(|list| list.iter().any(|t| t.slot.is_some()))
    {
        trinkets.sort_by_key(|t| if t.greater { 1 } else { 0 });
    }
    let gold = state.gold.unwrap_or_else(|| data.rules.gold(turn) as i32);
    ShopStateSpec {
        board,
        hand,
        tavern,
        hand_hidden: 0,
        gold,
        gold_max: state
            .gold_max
            .unwrap_or_else(|| data.rules.gold(turn) as i32),
        tier: state.tier.unwrap_or(1),
        upgrade_cost: state.upgrade_cost,
        health: state.health.unwrap_or(data.rules.starting_health),
        armor: state.armor.unwrap_or(0),
        free_refreshes: u16::from(state.free_refresh.unwrap_or(false)),
        fodder_refreshes: Some(Vec::new()),
        frozen: state.frozen.unwrap_or(false),
        hero_powers,
        trinkets,
        counters: Default::default(),
        secrets: Some(Vec::new()),
        won_last_combat: Some(false),
        last_opponent_board: None,
        lobby: Vec::new(),
    }
}

fn screen_file_spec(file: &ScreenFileSpec) -> ShopFileSpec {
    ShopFileSpec {
        source: file.source.clone(),
        game: file.game,
        hero: file.hero.as_ref().map(|h| HeroRef {
            card_id: h.card_id.clone(),
            name: h.name.clone(),
        }),
        lobby: file.lobby.clone(),
        ..Default::default()
    }
}

fn place_of(place: &ScreenPlace) -> ShopPlace {
    ShopPlace {
        card_id: place.card_id.clone(),
        name: place.name.clone(),
        kind: place.kind.clone(),
        position: place.position,
        index: place.index,
        slot: place.slot,
        ..Default::default()
    }
}

fn shop_place_of_card(
    data: &Data,
    ids: &CardIdMap,
    card: &ScreenCard,
    id: u64,
    missing: &mut Vec<String>,
) -> ShopPlace {
    let spec = minion_spec_of(data, ids, card, missing);
    ShopPlace {
        id,
        // A screen card may name only a name. The shop loader resolves a
        // place by its card id first and its name second, so a name in the id
        // slot lets a hand-written screen file's offer build (the card set's
        // own name is what `name` already holds).
        card_id: spec.card_id.clone().or_else(|| spec.name.clone()),
        name: spec.name,
        kind: Some(if spec.kind.as_deref() == Some("spell") {
            "spell".to_string()
        } else {
            "minion".to_string()
        }),
        keywords: Some(spec.keywords),
        attack: spec.attack,
        health: spec.health,
        golden: spec.golden,
        dark_gifts: spec.dark_gifts,
        ..Default::default()
    }
}

fn shop_place_of_option(data: &Data, ids: &CardIdMap, card: &ScreenCard, id: u64) -> ShopPlace {
    match resolve_card(data, ids, card) {
        Some(_) => {
            let mut ignored = Vec::new();
            shop_place_of_card(data, ids, card, id, &mut ignored)
        }
        None => {
            let (gifts, _, _) = gift_specs(data, card);
            ShopPlace {
                id,
                card_id: card.card_id.clone().or_else(|| card.name.clone()),
                name: card.name.clone(),
                kind: Some(card.kind.clone().unwrap_or_else(|| "minion".to_string())),
                keywords: card.keywords.clone(),
                attack: card.attack.map_or(0, |v| v.concrete(0)),
                health: card.health.map_or(0, |v| v.concrete(0)),
                golden: card.golden.unwrap_or(false),
                dark_gifts: gifts,
                ..Default::default()
            }
        }
    }
}

fn same_card(a: &ScreenCard, b: &ScreenCard) -> bool {
    let key = |c: &ScreenCard| {
        (
            c.card_id.clone().unwrap_or_default(),
            c.name.clone().unwrap_or_default(),
        )
    };
    key(a) == key(b)
}

fn tavern_id_at(state: &ShopStateSpec, slot: u8) -> Option<u64> {
    state
        .tavern
        .iter()
        .find(|m| m.slot == Some(slot) || m.slot.map(|s| s as usize) == Some(slot as usize))
        .map(|m| m.id)
}

fn board_id_at(state: &ShopStateSpec, position: u8) -> Option<u64> {
    state
        .board
        .iter()
        .find(|m| m.position == Some(position))
        .map(|m| m.id)
}

fn hand_id_at(state: &ShopStateSpec, index: u8) -> Option<u64> {
    state
        .hand
        .iter()
        .find(|m| m.index == Some(index))
        .map(|m| m.id)
}

/// Whether an exported card is the one a place names. The exported spell slot
/// is the engine's own, after the minion slots, so a screen's slot numbering
/// and the engine's can put different cards in the same slot number.
fn export_card_is(m: &ShopMinionSpec, card_id: Option<&str>, name: Option<&str>) -> bool {
    card_id.is_some_and(|id| m.card_id.as_deref() == Some(id))
        || name.is_some_and(|name| m.name.as_deref() == Some(name))
}

fn zone_id_of(
    state: &ShopStateSpec,
    kind: Option<&str>,
    position: Option<u8>,
    index: Option<u8>,
    slot: Option<u8>,
) -> Option<u64> {
    match kind {
        Some("slot") => slot.and_then(|s| tavern_id_at(state, s)),
        Some("board") => position.and_then(|p| board_id_at(state, p)),
        Some("hand") => index.and_then(|i| hand_id_at(state, i)),
        _ => None,
    }
}

/// One screen step as the shop replay reads a logged step. Its places get the
/// synthetic ids of the carried game's exported state, so the replay's own
/// action mapping (`actions_for`) finds the same card the screen names.
fn step_to_spec(
    data: &Data,
    ids: &CardIdMap,
    screen: &ScreenStepSpec,
    exported: &ShopStateSpec,
    seen: Option<&ShopStateSpec>,
    missing: &mut Vec<String>,
) -> Result<ShopStepSpec, String> {
    let mut out = ShopStepSpec {
        step: screen.step.clone(),
        slot: screen.slot,
        index: screen.index,
        position: screen.position,
        to: screen.to,
        which: screen.which.clone(),
        on: screen.on,
        cost: screen.cost,
        free: screen.free,
        tier_after: screen.tier_after,
        sub_option: screen.sub_option,
        slot_kind: screen.slot_kind.clone(),
        kind: screen.kind.clone(),
        after: seen.cloned(),
        ..Default::default()
    };
    if let Some(card) = &screen.card {
        out.card = Some(shop_place_of_card(data, ids, card, 0, missing));
    }
    if let Some(target) = &screen.target {
        let mut target = place_of(target);
        if target.id == 0 {
            target.id = zone_id_of(
                exported,
                target.kind.as_deref(),
                target.position,
                target.index,
                target.slot,
            )
            .unwrap_or(0);
        }
        out.target = Some(target);
    }
    for (k, option) in screen.options.iter().enumerate() {
        out.options
            .push(shop_place_of_option(data, ids, option, (k + 1) as u64));
    }
    out.picked = match &screen.picked {
        None => None,
        Some(ScreenPick::Index(k)) => Some(out.options.get(*k).cloned().unwrap_or_default()),
        Some(ScreenPick::At { index }) => {
            Some(out.options.get(*index).cloned().unwrap_or_default())
        }
        Some(ScreenPick::Card(card)) => {
            let at = screen.options.iter().position(|o| same_card(o, card));
            match at {
                Some(k) => Some(out.options[k].clone()),
                None => Some(shop_place_of_option(
                    data,
                    ids,
                    card,
                    (out.options.len() + 1) as u64,
                )),
            }
        }
    };
    // A choice's price is on the card the screen showed. The trinket offer's
    // picked option is charged from the step's `paid`.
    if out.paid.is_none() {
        let price = match &screen.picked {
            Some(ScreenPick::Card(card)) => card.cost,
            Some(ScreenPick::Index(k)) => screen.options.get(*k).and_then(|c| c.cost),
            Some(ScreenPick::At { index }) => screen.options.get(*index).and_then(|c| c.cost),
            None => None,
        };
        if let Some(gold) = price {
            out.paid = Some(shop_replay::ShopPaid { gold, health: 0 });
        }
    }
    // A target that named a card but no place: find it in the exported zone.
    if let Some(target) = out.target.as_mut()
        && target.id == 0
        && target.name.is_some()
    {
        let zone = match target.kind.as_deref() {
            Some("slot") => &exported.tavern,
            Some("board") => &exported.board,
            Some("hand") => &exported.hand,
            _ => &exported.board,
        };
        if let Some(m) = zone
            .iter()
            .find(|m| m.name.as_deref() == target.name.as_deref())
        {
            target.id = m.id;
        }
    }
    match screen.step.as_str() {
        "buy" => {
            if out.target.is_none()
                && let Some(slot) = screen.slot
            {
                out.target = Some(ShopPlace {
                    kind: Some("slot".to_string()),
                    slot: Some(slot),
                    ..Default::default()
                });
            }
            if let Some(target) = out.target.as_mut() {
                target.kind.get_or_insert_with(|| "slot".to_string());
                if target.id == 0
                    && let Some(slot) = target.slot.or(screen.slot)
                {
                    target.id = tavern_id_at(exported, slot).unwrap_or(0);
                }
                // The file's slot counts the spell where it stood, while the
                // engine's exported spell slot comes after every minion. Name
                // the card the screen bought, not the card in that engine slot.
                if let Some(card) = out.card.as_ref() {
                    let card_id = card.card_id.as_deref();
                    let name = card.name.as_deref();
                    let current = exported
                        .tavern
                        .iter()
                        .find(|m| Some(m.id) == Some(target.id))
                        .is_some_and(|m| export_card_is(m, card_id, name));
                    if !current
                        && let Some(m) = exported
                            .tavern
                            .iter()
                            .find(|m| export_card_is(m, card_id, name))
                    {
                        target.id = m.id;
                    }
                }
                if out.card.is_none()
                    && let Some(m) = exported
                        .tavern
                        .iter()
                        .find(|m| Some(m.id) == Some(target.id))
                {
                    out.card = Some(ShopPlace {
                        card_id: m.card_id.clone(),
                        name: m.name.clone(),
                        ..Default::default()
                    });
                }
            }
        }
        "sell" | "activate" | "move" => {
            if let Some(position) = screen.position {
                if out.card.is_none()
                    && let Some(m) = exported.board.iter().find(|m| m.position == Some(position))
                {
                    out.card = Some(ShopPlace {
                        card_id: m.card_id.clone(),
                        name: m.name.clone(),
                        ..Default::default()
                    });
                }
                if let Some(card) = out.card.as_mut()
                    && card.id == 0
                    && let Some(m) = exported.board.iter().find(|m| m.position == Some(position))
                {
                    card.id = m.id;
                }
            }
        }
        "play" => {
            if let Some(index) = screen.index {
                if out.card.is_none()
                    && let Some(m) = exported.hand.iter().find(|m| m.index == Some(index))
                {
                    out.card = Some(ShopPlace {
                        card_id: m.card_id.clone(),
                        name: m.name.clone(),
                        ..Default::default()
                    });
                }
                if let Some(card) = out.card.as_mut()
                    && card.id == 0
                    && let Some(m) = exported.hand.iter().find(|m| m.index == Some(index))
                {
                    card.id = m.id;
                }
            }
            let mut landing = screen.to;
            if landing.is_none()
                && let Some(seen) = seen
            {
                // The played card stands on the board the engine had right
                // after the step: its place there is the slot the engine needs.
                let name = out.card.as_ref().and_then(|c| c.name.as_deref());
                if let Some(name) = name
                    && let Some((k, _)) = seen
                        .board
                        .iter()
                        .enumerate()
                        .find(|(_, c)| c.name.as_deref() == Some(name))
                {
                    landing = Some((k + 1) as u8);
                }
            }
            if let Some(to) = landing {
                out.moved = Some(shop_replay::ShopMoved {
                    to: Some(ShopPlace {
                        kind: Some("board".to_string()),
                        position: Some(to),
                        ..Default::default()
                    }),
                    ..Default::default()
                });
            }
        }
        "discover" | "trinket_pick" => {
            if out.kind.is_none() {
                out.kind = Some(if screen.step == "discover" {
                    "discover".to_string()
                } else {
                    "trinket".to_string()
                });
            }
        }
        _ => {}
    }
    if out.options.is_empty() && out.picked.is_some() {
        return Err("the step picked from no options".to_string());
    }
    Ok(out)
}

// ---- carrying the game and playing one turn ----

/// Why the screen replay could not follow the next step.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ScreenStop {
    /// The step's index in the turn's list, 0 based.
    pub step: usize,
    /// The step's kind (`buy`, `play`, `end_turn`, ...).
    pub kind: String,
    /// `start`, `map`, `seat`, `missing`, `illegal` or `ended`.
    pub class: String,
    /// What the screen, the loader or the engine said.
    pub reason: String,
}

/// One turn played forward from its screen start on one seed.
#[derive(Clone, Debug)]
pub struct ScreenTurnRun {
    /// Boxed: a `Game` is about 160 KB, and a chained run keeps one per seed
    /// while it builds the next turn.
    pub game: Box<Game>,
    pub steps: usize,
    pub played: usize,
    pub stop: Option<ScreenStop>,
    pub missing: Vec<String>,
    pub seed: u64,
    /// What the engine did differently from the step's own `seen` state. The
    /// replay carries on; these lines flag the turn.
    pub step_differences: Vec<String>,
    /// The per-value checks of every step this seed played.
    pub step_checks: Vec<ScreenCheckGroup>,
    /// What the carried game differed on before the screen's `start` went on.
    /// These are between-turn lines: they are reported, not flagged.
    pub between_turns: Vec<String>,
    pub between_checks: Option<ScreenCheckGroup>,
    /// The cards whose visible stats changed between turns, and whether any
    /// player counter is uncertain after the fight. Step lines about them say
    /// "(uncertain after the fight)".
    pub uncertain_cards: Vec<String>,
    pub uncertain_counters: bool,
    /// The engine state after every played step, for finding the step whose
    /// randomness splits the seeds.
    pub states: Vec<ShopSnapshot>,
}

/// What the engine ended with, per seed, for `turns.json`.
#[derive(Clone, Debug, Serialize)]
pub struct ScreenEndState {
    pub gold: i32,
    pub tier: u8,
    pub health: i32,
    pub armor: i32,
    pub upgrade_cost: Option<u8>,
    pub board: Vec<ScreenCardRow>,
    pub hand: Vec<ScreenCardRow>,
    pub tavern: Vec<ScreenCardRow>,
    pub powers: Vec<ScreenPowerRow>,
    pub trinkets: Vec<(String, bool)>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ScreenCardRow {
    pub name: String,
    pub attack: i32,
    pub health: i32,
    pub golden: bool,
    pub spell: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct ScreenPowerRow {
    pub card_id: String,
    pub uses_left: u8,
    pub number: i32,
}

impl ScreenEndState {
    pub fn of(data: &Data, game: &Game) -> ScreenEndState {
        let snap = ShopSnapshot::of_game(data, game, ME);
        let row = |m: &shop_replay::SnapMinion| ScreenCardRow {
            name: m.name.clone(),
            attack: m.attack,
            health: m.health,
            golden: m.golden,
            spell: m.spell,
        };
        ScreenEndState {
            gold: snap.gold,
            tier: snap.tier,
            health: snap.health,
            armor: snap.armor,
            upgrade_cost: snap.upgrade_cost,
            board: snap.board.iter().map(row).collect(),
            hand: snap.hand.iter().map(row).collect(),
            tavern: snap.tavern.iter().map(row).collect(),
            powers: snap
                .powers
                .iter()
                .map(|p| ScreenPowerRow {
                    card_id: p.card_id.clone(),
                    uses_left: p.uses_left,
                    number: p.number,
                })
                .collect(),
            trinkets: snap.trinkets.clone(),
        }
    }
}

fn minion_of(data: &Data, m: &Minion, id: u64, place: usize, tavern: bool) -> ShopMinionSpec {
    let card = data.cards.get(m.card);
    ShopMinionSpec {
        id,
        name: Some(card.name.clone()),
        attack: m.attack,
        health: m.health,
        golden: m.golden,
        keywords: keyword_names(m.keywords),
        position: (!tavern).then_some(place as u8),
        index: (!tavern).then_some(place as u8),
        slot: tavern.then_some(place as u8),
        kind: Some(if card.is_spell() { "spell" } else { "minion" }.to_string()),
        frozen: m.frozen,
        ..Default::default()
    }
}

/// The carried game as a `ShopStateSpec` with synthetic ids, the state the
/// step's action mapping is seated from.
fn export_state(data: &Data, game: &Game) -> ShopStateSpec {
    let pl = &game.players[ME];
    let mut next = 1u64;
    let mut board = Vec::new();
    for (k, m) in pl.board.iter().enumerate() {
        board.push(minion_of(data, m, next, k + 1, false));
        next += 1;
    }
    let mut hand = Vec::new();
    for (k, m) in pl.hand.iter().enumerate() {
        let mut spec = minion_of(data, m, next, k + 1, false);
        spec.position = None;
        hand.push(spec);
        next += 1;
    }
    let mut tavern = Vec::new();
    for (k, m) in pl.shop.iter().enumerate() {
        let mut spec = minion_of(data, m, next, k + 1, true);
        spec.index = None;
        spec.position = None;
        tavern.push(spec);
        next += 1;
    }
    if let Some(card) = pl.shop_spell {
        let made = data.cards.make(card, false);
        let mut spec = minion_of(data, &made, next, pl.shop.len() + 1, true);
        spec.kind = Some("spell".to_string());
        tavern.push(spec);
    }
    let mut hero_powers = Vec::new();
    for (slot, st) in [pl.power, pl.power2].into_iter().enumerate() {
        let Some(st) = st else { continue };
        let power = data.heroes.power(st.of);
        hero_powers.push(ShopPowerSpec {
            card_id: Some(power.card_id.clone()),
            name: Some(power.name.clone()),
            numbers: vec![st.progress as i32],
            used_this_turn: power
                .uses_per_turn
                .saturating_add(pl.tp.power_extra_uses)
                .saturating_sub(st.uses_left),
            waiting: pl
                .locked_and_loaded
                .as_ref()
                .map(|m| minion_of(data, m, 0, 1, false)),
            second: slot == 1,
            ..Default::default()
        });
    }
    let trinkets = pl
        .trinkets
        .iter()
        .enumerate()
        .filter(|(_, st)| st.of != 0)
        .map(|(slot, st)| ShopTrinketSpec {
            card_id: Some(data.trinkets.get(st.of - 1).card_id.clone()),
            greater: slot == 1,
            ..Default::default()
        })
        .collect();
    ShopStateSpec {
        board,
        hand,
        tavern,
        hand_hidden: 0,
        gold: pl.gold as i32,
        gold_max: data.rules.gold(game.turn) as i32,
        tier: pl.tier,
        upgrade_cost: (pl.tier < MAX_TIER).then_some(game.upgrade_price(ME) as i32),
        health: pl.health,
        armor: pl.armor,
        free_refreshes: pl.free_rolls,
        fodder_refreshes: Some(pl.fodder_refreshes.iter().copied().collect()),
        frozen: pl.frozen,
        hero_powers,
        trinkets,
        counters: Default::default(),
        secrets: Some(Vec::new()),
        won_last_combat: Some(pl.won_last_combat),
        last_opponent_board: None,
        lobby: Vec::new(),
    }
}

/// The screen said the power's waiting slot was empty. The loader cannot tell
/// that from "the file does not say", so its Tavish miss is dropped here.
fn drop_waiting_missing(missing: &mut Vec<String>, waiting_known_empty: bool) {
    if waiting_known_empty {
        missing.retain(|m| !m.starts_with("the minion locked in hand not in the file"));
    }
}

/// The loader seats `uses_per_turn - used_this_turn`, but a trinket's extra
/// presses are seated after the power. Put them back.
fn resync_power_uses(data: &Data, game: &mut Game, start: &ShopStateSpec) {
    let extra = game.players[ME].tp.power_extra_uses;
    for (slot, spec) in start.hero_powers.iter().enumerate() {
        let st = if slot == 1 {
            game.players[ME].power2.as_mut()
        } else {
            game.players[ME].power.as_mut()
        };
        let Some(st) = st else { continue };
        let power = data.heroes.power(st.of);
        st.uses_left = power
            .uses_per_turn
            .saturating_add(extra)
            .saturating_sub(spec.used_this_turn);
    }
}

/// Put the price the screen showed on the card a buy is about to take onto the
/// carried game. The engine prices a minion from its rules, a spell from the
/// player's discount; the client's own number is what the screen shows.
fn apply_step_price(data: &Data, ids: &CardIdMap, game: &mut Game, screen: &ScreenStepSpec) {
    if screen.step != "buy" {
        return;
    }
    let Some(card) = screen.card.as_ref() else {
        return;
    };
    if card.costs_health == Some(true) {
        game.players[ME].tp.health_price_left = 1;
        return;
    }
    let Some(cost) = card.cost else { return };
    let spell = resolve_card(data, ids, card).is_some_and(|c| data.cards.get(c).is_spell());
    let pl = &mut game.players[ME];
    if spell {
        if let Some(engine) = pl.spell_cost(&data.cards) {
            let want = cost.clamp(0, u8::MAX as i32) as u8;
            if want < engine {
                pl.spell_discount = pl.spell_discount.saturating_add(engine - want);
            }
        }
    } else {
        let rule = data.rules.buy_cost as i32;
        if cost == 0 {
            pl.free_minion_left = pl.free_minion_left.max(1);
        } else {
            pl.free_minion_left = 0;
            if cost != rule {
                pl.tp.tavern_price = cost.clamp(0, u8::MAX as i32) as u8;
            } else {
                pl.tp.tavern_price = 0;
            }
        }
    }
}

/// The last Tavern spell the screen showed played before this step, earlier
/// turns included. The Galaxy's Lens copies it and no single state carries it.
fn last_screen_spell(
    data: &Data,
    ids: &CardIdMap,
    file: &ScreenFileSpec,
    turn: &ScreenTurnSpec,
    before_step: usize,
) -> Option<CardId> {
    let mut last = None;
    for t in &file.turns {
        for (i, step) in t.steps.iter().enumerate() {
            if t.turn == turn.turn && i >= before_step {
                break;
            }
            if step.step != "play" {
                continue;
            }
            let Some(card) = step.card.as_ref().and_then(|c| resolve_card(data, ids, c)) else {
                continue;
            };
            if data.cards.get(card).tavern_spell {
                last = Some(card);
            }
        }
        if t.turn == turn.turn {
            break;
        }
    }
    last
}

/// What the carried game differed on before a turn's screen start went on.
#[derive(Clone, Debug, Default)]
struct BetweenTurn {
    lines: Vec<String>,
    checks: Vec<ScreenCheck>,
    uncertain_cards: Vec<String>,
    /// A player counter a fight could have changed: the start differed from
    /// the carry, so the counter is not proved.
    uncertain_counters: bool,
}

/// Compare the carried game with the screen's turn start. The Tavern is not
/// compared: the engine's start of turn draws a fresh random one, and the
/// screen's draw is put on without being read as a fight result. Everything a
/// fight could have changed is compared as a between-turn line.
fn between_turn_diff(
    data: &Data,
    ids: &CardIdMap,
    start_screen: &ScreenStateSpec,
    game: &Game,
    turn: u32,
) -> BetweenTurn {
    let ours = ShopSnapshot::of_game(data, game, ME);
    let prefix = format!("before turn {turn}: ");
    let mut out = BetweenTurn::default();
    let mut lines: Vec<String> = Vec::new();
    if let Some(board) = start_screen.board.as_ref() {
        let (mut found, mut checks) = compare_board(data, ids, &ours.board, board);
        lines.extend(
            found
                .drain(..)
                .map(|l| format!("{prefix}{}", without_colon(l))),
        );
        out.checks.append(&mut checks);
        for (i, card) in board.iter().enumerate() {
            let Some(a) = ours.board.get(i) else { continue };
            let name = screen_card_name(data, ids, card);
            if a.name != name || a.golden != screen_card_golden(data, ids, card) {
                continue;
            }
            if number_status(a.attack, screen_stat(data, ids, card, true))
                == ScreenCheckStatus::Disagree
                || number_status(a.health, screen_stat(data, ids, card, false))
                    == ScreenCheckStatus::Disagree
            {
                out.uncertain_cards.push(a.name.clone());
            }
        }
    }
    if let Some(hand) = start_screen.hand.as_ref() {
        let (mut found, mut checks) = compare_group(data, ids, "hand", &ours.hand, hand);
        lines.extend(
            found
                .drain(..)
                .map(|l| format!("{prefix}{}", without_colon(l))),
        );
        out.checks.append(&mut checks);
        for b in hand {
            let name = screen_card_name(data, ids, b);
            let golden = screen_card_golden(data, ids, b);
            if let Some(a) = ours
                .hand
                .iter()
                .find(|a| a.name == name && a.golden == golden)
                && (number_status(a.attack, screen_stat(data, ids, b, true))
                    == ScreenCheckStatus::Disagree
                    || number_status(a.health, screen_stat(data, ids, b, false))
                        == ScreenCheckStatus::Disagree)
            {
                out.uncertain_cards.push(a.name.clone());
            }
        }
    }
    let mut checks = std::mem::take(&mut out.checks);
    let mut player = |field: &str, engine: i64, screen: i64| {
        checks.push(ScreenCheck {
            zone: "player".to_string(),
            index: None,
            card: None,
            field: field.to_string(),
            engine: ScreenCheckValue::Number(engine),
            screen: ScreenCheckValue::Number(screen),
            status: if engine == screen {
                ScreenCheckStatus::Agree
            } else {
                ScreenCheckStatus::Disagree
            },
        });
    };
    if let Some(gold) = start_screen.gold {
        player("gold", ours.gold as i64, gold as i64);
        if ours.gold != gold {
            lines.push(format!("{prefix}Gold engine {}, screen {gold}", ours.gold));
        }
    }
    if let Some(tier) = start_screen.tier {
        player("tier", ours.tier as i64, tier as i64);
        if ours.tier != tier {
            lines.push(format!("{prefix}tier engine {}, screen {tier}", ours.tier));
        }
    }
    if let Some(health) = start_screen.health {
        player("health", ours.health as i64, health as i64);
        if ours.health != health {
            lines.push(format!(
                "{prefix}Health engine {}, screen {health}",
                ours.health
            ));
        }
    }
    if let Some(armor) = start_screen.armor {
        player("armor", ours.armor as i64, armor as i64);
        if ours.armor != armor {
            lines.push(format!(
                "{prefix}armor engine {}, screen {armor}",
                ours.armor
            ));
        }
    }
    if let Some(free) = start_screen.free_refresh {
        let engine = ours.free_refreshes > 0;
        checks.push(ScreenCheck {
            zone: "player".to_string(),
            index: None,
            card: None,
            field: "free_refresh".to_string(),
            engine: ScreenCheckValue::Bool(engine),
            screen: ScreenCheckValue::Bool(free),
            status: if engine == free {
                ScreenCheckStatus::Agree
            } else {
                ScreenCheckStatus::Disagree
            },
        });
        if engine != free {
            let says = |f: bool| if f { "free" } else { "paid" };
            lines.push(format!(
                "{prefix}next Refresh engine {}, screen {}",
                says(engine),
                says(free)
            ));
        }
    }
    out.uncertain_cards.sort();
    out.uncertain_cards.dedup();
    out.uncertain_counters = !lines.is_empty();
    out.lines = lines;
    out.checks = checks;
    out
}

/// Put the screen's stat values on a resynced board: a number is taken, a
/// range keeps the engine's value when it holds it and otherwise takes the
/// nearest bound, and `null` keeps the engine's value. `before` is the board
/// as it stood before the screen's cards went on.
fn resync_board_targets(
    data: &Data,
    ids: &CardIdMap,
    game: &mut Game,
    screen: &[ScreenCard],
    before: &[shop_replay::SnapMinion],
) {
    for (i, card) in screen.iter().enumerate() {
        let Some(old) = before.get(i) else { continue };
        if old.name != screen_card_name(data, ids, card)
            || old.golden != screen_card_golden(data, ids, card)
        {
            continue;
        }
        if let Some(m) = game.players[ME].board.get_mut(i) {
            m.attack = number_target(old.attack, screen_stat(data, ids, card, true));
            m.health = number_target(old.health, screen_stat(data, ids, card, false));
        }
    }
}

/// The hand's side of `resync_board_targets`.
fn resync_hand_targets(
    data: &Data,
    ids: &CardIdMap,
    game: &mut Game,
    screen: &[ScreenCard],
    before: &[shop_replay::SnapMinion],
) {
    for (i, card) in screen.iter().enumerate() {
        let Some(old) = before.get(i) else { continue };
        if old.name != screen_card_name(data, ids, card)
            || old.golden != screen_card_golden(data, ids, card)
        {
            continue;
        }
        if let Some(m) = game.players[ME].hand.get_mut(i) {
            m.attack = number_target(old.attack, screen_stat(data, ids, card, true));
            m.health = number_target(old.health, screen_stat(data, ids, card, false));
        }
    }
}

/// Put the visible parts of a screen start onto a carried game after the
/// between-turn comparison. A zone or number the screen did not show is left
/// alone: unseen is unknown.
fn resync_start(
    data: &Data,
    ids: &CardIdMap,
    game: &mut Game,
    screen: &ScreenStateSpec,
    start: &ShopStateSpec,
    missing: &mut Vec<String>,
    keep: bool,
) {
    let before = ShopSnapshot::of_game(data, game, ME);
    if screen.board.is_some() {
        let fake = ShopStepSpec {
            after: Some(start.clone()),
            ..Default::default()
        };
        shop_replay::force_board(data, game, &fake, missing);
        if let Some(board) = screen.board.as_ref() {
            resync_board_targets(data, ids, game, board, &before.board);
        }
    }
    if screen.hand.is_some() {
        let fake = ShopStepSpec {
            after: Some(start.clone()),
            ..Default::default()
        };
        if keep {
            shop_replay::force_hand_keep(data, game, &fake, missing);
        } else {
            shop_replay::force_hand(data, game, &fake, missing);
        }
        if let Some(hand) = screen.hand.as_ref() {
            resync_hand_targets(data, ids, game, hand, &before.hand);
        }
    }
    if screen.tavern.is_some() {
        shop_replay::place_tavern(data, game, start, missing);
    }
    if let Some(gold) = screen.gold {
        game.players[ME].gold = gold.clamp(0, u8::MAX as i32) as u8;
    }
    if let Some(tier) = screen.tier {
        game.players[ME].tier = tier.clamp(1, MAX_TIER);
    }
    if let Some(health) = screen.health {
        game.players[ME].health = health;
    }
    if let Some(armor) = screen.armor {
        game.players[ME].armor = armor;
    }
    if let Some(free) = screen.free_refresh {
        match free {
            true if game.players[ME].free_rolls == 0 => game.players[ME].free_rolls = 1,
            false => game.players[ME].free_rolls = 0,
            _ => {}
        }
    }
    if let Some(frozen) = screen.frozen {
        game.players[ME].frozen = frozen;
    }
    if screen.hero_power.is_some() || screen.hero_power_second.is_some() {
        shop_replay::seat_power_counters(data, &mut game.players[ME], &start.hero_powers);
        resync_power_uses(data, game, start);
    }
    if screen.trinkets.is_some() {
        shop_replay::seat_trinket_counters(data, &mut game.players[ME], &start.trinkets);
    }
}

/// Append "(uncertain after the fight)" to the step lines a fight's hidden
/// change could explain: a line about a card whose visible stats changed
/// between turns, or a stat line while the player's counters are unproved.
fn mark_uncertain(lines: Vec<String>, cards: &[String], counters: bool) -> Vec<String> {
    lines
        .into_iter()
        .map(|line| {
            let card = cards
                .iter()
                .any(|name| !name.is_empty() && line.contains(name));
            let stats = line.contains("board ") || line.contains("hand ");
            if (card || (counters && stats)) && !line.contains("(uncertain after the fight)") {
                format!("{line} (uncertain after the fight)")
            } else {
                line
            }
        })
        .collect()
}

/// Seat the turn's start once, then play each step on the carried game. The
/// error is only the start that cannot be seated; a later step stops the run
/// with its own `ScreenStop`.
pub fn replay_screen_turn(
    data: &Arc<Data>,
    ids: &CardIdMap,
    file: &ScreenFileSpec,
    turn: &ScreenTurnSpec,
    seed: u64,
) -> Result<ScreenTurnRun, ScreenStop> {
    replay_screen_turn_from(data, ids, file, turn, seed, None)
}

/// `replay_screen_turn` with an optional carried game. `Some` is chained mode:
/// the previous turn's engine game is moved into this shop phase with the
/// engine's own turn transition, compared with the screen's `start`, and then
/// has the visible start put on. `None` is the non-chained replay and the
/// first turn of a chain.
pub fn replay_screen_turn_from(
    data: &Arc<Data>,
    ids: &CardIdMap,
    file: &ScreenFileSpec,
    turn: &ScreenTurnSpec,
    seed: u64,
    carry: Option<&Game>,
) -> Result<ScreenTurnRun, ScreenStop> {
    let mut missing = Vec::new();
    let chained = carry.is_some();
    let mut next_id = 1u64;
    let start = state_to_spec(
        data,
        ids,
        &turn.start,
        turn.turn,
        &mut next_id,
        &mut missing,
    );
    let waiting_known_empty = [
        turn.start.hero_power.as_ref(),
        turn.start.hero_power_second.as_ref(),
    ]
    .into_iter()
    .flatten()
    .any(|p| p.waiting_seen == Some(true) && p.waiting.is_none());
    let mut between_turns = Vec::new();
    let mut between_checks = None;
    let mut uncertain_cards = Vec::new();
    let mut uncertain_counters = false;
    let (mut game, loaded_missing): (Box<Game>, Vec<String>) = match carry {
        Some(game) => {
            let mut game = Box::new(game.clone());
            // The engine's own shop-side turn transition: end the shop turn
            // that is still open, then start the next one (Gold, start-of-turn
            // triggers, tier price, the turn number) without a fight. This is
            // `Game::next_shop_turn_without_fight`, the lobster search's own
            // step, so no rule is copied into the replay.
            game.next_shop_turn_without_fight();
            let between = between_turn_diff(data, ids, &turn.start, &game, turn.turn);
            between_turns = between.lines;
            between_checks = Some(ScreenCheckGroup {
                when: "before turn".to_string(),
                step: None,
                kind: None,
                checks: between.checks,
            });
            uncertain_cards = between.uncertain_cards;
            uncertain_counters = between.uncertain_counters;
            resync_start(
                data,
                ids,
                &mut game,
                &turn.start,
                &start,
                &mut missing,
                true,
            );
            (game, Vec::new())
        }
        None => {
            let base = screen_file_spec(file);
            let hero = base.hero(data, &start);
            let lobby = base.lobby();
            let loaded = shop_replay::load(data, &start, hero, lobby, turn.turn, seed, None);
            let mut game = Box::new(loaded.game);
            // A passive power seated after `clear_choices` can queue its
            // start-of-turn Discovers again. The screen's `start` is past them:
            // the turns that have one show it as a step of its own.
            game.players[ME].discover_queue.clear();
            resync_power_uses(data, &mut game, &start);
            (game, loaded.missing)
        }
    };
    missing.extend(loaded_missing);
    drop_waiting_missing(&mut missing, waiting_known_empty);
    let steps = turn.steps.len();
    let mut states = Vec::new();
    let mut step_differences = Vec::new();
    let mut step_checks: Vec<ScreenCheckGroup> = Vec::new();
    for i in 0..steps {
        let screen = &turn.steps[i];
        let last_spell = last_screen_spell(data, ids, file, turn, i);
        if let Some(spell) = last_spell {
            game.players[ME].last_spell = Some(spell);
        }
        let exported = export_state(data, &game);
        let mut seen_id = 1_000_000u64;
        let seen_spec = screen
            .seen
            .as_ref()
            .map(|seen| state_to_spec(data, ids, seen, turn.turn, &mut seen_id, &mut missing));
        let step = match step_to_spec(
            data,
            ids,
            screen,
            &exported,
            seen_spec.as_ref(),
            &mut missing,
        ) {
            Ok(step) => step,
            Err(reason) => {
                return Ok(ScreenTurnRun {
                    game,
                    steps,
                    played: i,
                    stop: Some(ScreenStop {
                        step: i,
                        kind: screen.step.clone(),
                        class: "map".to_string(),
                        reason,
                    }),
                    missing,
                    seed,
                    step_differences,
                    between_turns,
                    step_checks,
                    between_checks,
                    uncertain_cards,
                    uncertain_counters,
                    states,
                });
            }
        };
        let synthetic = ShopFileSpec {
            source: file.source.clone(),
            game: file.game,
            hero: file.hero.as_ref().map(|h| HeroRef {
                card_id: h.card_id.clone(),
                name: h.name.clone(),
            }),
            lobby: file.lobby.clone(),
            turns: vec![ShopTurnSpec {
                turn: turn.turn,
                start: exported,
                actions: vec![step.clone()],
                end: None,
                pending_choice: None,
            }],
            ..Default::default()
        };
        let seated =
            match shop_replay::seat_step(data, ids, &synthetic, &synthetic.turns[0], 0, seed) {
                Ok(seated) => seated,
                Err((reason, miss)) => {
                    missing.extend(miss);
                    return Ok(ScreenTurnRun {
                        game,
                        steps,
                        played: i,
                        stop: Some(ScreenStop {
                            step: i,
                            kind: screen.step.clone(),
                            class: "seat".to_string(),
                            reason,
                        }),
                        missing,
                        seed,
                        step_differences,
                        between_turns,
                        step_checks,
                        between_checks,
                        uncertain_cards,
                        uncertain_counters,
                        states,
                    });
                }
            };
        let mut seated = seated;
        if let Some(spell) = last_spell {
            seated.game.players[ME].last_spell = Some(spell);
        }
        seated.game.players[ME].discover_queue.clear();
        // A two-step power's second click is a target, not a fresh press. The
        // synthetic state cannot carry the carried game's open prompt, so map
        // the click from the screen's own target instead.
        if screen.step == "hero_power" && game.players[ME].pending_target.is_some() {
            let mut actions = Vec::new();
            if let Some(target) = screen.target.as_ref() {
                match target.kind.as_deref() {
                    Some("board") => {
                        if let Some(p) = target.position {
                            actions.push(Action::Target(p.saturating_sub(1)));
                        }
                    }
                    Some("slot") => {
                        if let Some(s) = target.slot {
                            actions.push(Action::TargetShop(s.saturating_sub(1)));
                        }
                    }
                    Some("hand") => {
                        if let Some(i) = target.index {
                            actions.push(Action::Discard(i.saturating_sub(1)));
                        }
                    }
                    _ => {}
                }
            }
            seated.actions = actions;
        }
        drop_waiting_missing(&mut seated.missing, waiting_known_empty);
        if !seated.missing.is_empty() {
            let reason = seated.missing.join("; ");
            missing.extend(seated.missing);
            return Ok(ScreenTurnRun {
                game,
                steps,
                played: i,
                stop: Some(ScreenStop {
                    step: i,
                    kind: screen.step.clone(),
                    class: "missing".to_string(),
                    reason,
                }),
                missing,
                seed,
                step_differences,
                between_turns,
                step_checks,
                between_checks,
                uncertain_cards,
                uncertain_counters,
                states,
            });
        }
        apply_step_price(data, ids, &mut game, screen);
        let free_before = game.players[ME].free_rolls;
        let (next, miss, _text) = match seated.play_on(data, ids, game.as_ref().clone()) {
            Ok(next) => next,
            Err((reason, miss)) => {
                missing.extend(miss);
                return Ok(ScreenTurnRun {
                    game,
                    steps,
                    played: i,
                    stop: Some(ScreenStop {
                        step: i,
                        kind: screen.step.clone(),
                        class: "illegal".to_string(),
                        reason,
                    }),
                    missing,
                    seed,
                    step_differences,
                    between_turns,
                    step_checks,
                    between_checks,
                    uncertain_cards,
                    uncertain_counters,
                    states,
                });
            }
        };
        missing.extend(miss);
        game = Box::new(next);
        // Compare the engine's result with what the screen showed before
        // putting that state on the game. The zones are the checker's useful
        // output; the resync below only stops one difference cascading into a
        // stop.
        let ours = ShopSnapshot::of_game(data, &game, ME);
        if let Some(spec) = seen_spec.as_ref() {
            let (mut lines, mut checks) = step_seen_differences(data, ids, screen, &ours, i);
            // The screen shows whether the next Refresh is free, not how many
            // free ones are left, so the engine holds one at most. A Roll that
            // spent it while the screen still shows a free one only means the
            // screen had more: that is no difference.
            if screen.step == "roll"
                && free_before > 0
                && game.players[ME].free_rolls == 0
                && screen.seen.as_ref().and_then(|s| s.free_refresh) == Some(true)
            {
                lines.retain(|l| !l.contains("next Refresh"));
                checks.retain(|c| c.field != "free_refresh");
            }
            step_differences.extend(mark_uncertain(lines, &uncertain_cards, uncertain_counters));
            step_checks.push(ScreenCheckGroup {
                when: "after step".to_string(),
                step: Some(i + 1),
                kind: Some(screen.step.clone()),
                checks,
            });
            let _ = spec;
        }
        // The screen's own seen state goes on only where it saw one.
        if let Some(seen) = &screen.seen {
            if seen.tavern.is_some()
                && let Some(spec) = seen_spec.as_ref()
            {
                shop_replay::place_tavern(data, &mut game, spec, &mut missing);
            }
            if let Some(board) = seen.board.as_ref()
                && let Some(spec) = seen_spec.as_ref()
            {
                let fake = ShopStepSpec {
                    after: Some(spec.clone()),
                    ..Default::default()
                };
                shop_replay::force_board(data, &mut game, &fake, &mut missing);
                resync_board_targets(data, ids, &mut game, board, &ours.board);
            }
            if let Some(hand) = seen.hand.as_ref()
                && let Some(spec) = seen_spec.as_ref()
            {
                let fake = ShopStepSpec {
                    after: Some(spec.clone()),
                    ..Default::default()
                };
                if chained {
                    shop_replay::force_hand_keep(data, &mut game, &fake, &mut missing);
                } else {
                    shop_replay::force_hand(data, &mut game, &fake, &mut missing);
                }
                resync_hand_targets(data, ids, &mut game, hand, &ours.hand);
            }
            // The step's visible numbers are put on the carried game after
            // they have been compared, so one drift makes one line.
            // A free next Refresh keeps the engine's own count when it has
            // one; the screen cannot say how many are left.
            match seen.free_refresh {
                Some(true) if game.players[ME].free_rolls == 0 => game.players[ME].free_rolls = 1,
                Some(false) => game.players[ME].free_rolls = 0,
                _ => {}
            }
            if let Some(gold) = seen.gold {
                game.players[ME].gold = gold.clamp(0, u8::MAX as i32) as u8;
            }
            if let Some(tier) = seen.tier {
                game.players[ME].tier = tier.clamp(1, MAX_TIER);
            }
            if let Some(spec) = seen_spec.as_ref() {
                if seen.hero_power.is_some() || seen.hero_power_second.is_some() {
                    shop_replay::seat_power_counters(
                        data,
                        &mut game.players[ME],
                        &spec.hero_powers,
                    );
                    resync_power_uses(data, &mut game, spec);
                }
                if seen.trinkets.is_some() {
                    shop_replay::seat_trinket_counters(
                        data,
                        &mut game.players[ME],
                        &spec.trinkets,
                    );
                }
            }
        }
        states.push(ShopSnapshot::of_game(data, &game, ME));
        if game.turn != turn.turn || game.over {
            return Ok(ScreenTurnRun {
                game,
                steps,
                played: i + 1,
                stop: Some(ScreenStop {
                    step: i + 1,
                    kind: turn
                        .steps
                        .get(i + 1)
                        .map(|s| s.step.clone())
                        .unwrap_or_default(),
                    class: "ended".to_string(),
                    reason: format!("the engine ended the turn after step {}", i + 1),
                }),
                missing,
                seed,
                step_differences,
                between_turns,
                step_checks,
                between_checks,
                uncertain_cards,
                uncertain_counters,
                states,
            });
        }
    }
    Ok(ScreenTurnRun {
        game,
        steps,
        played: steps,
        stop: None,
        missing,
        seed,
        step_differences,
        between_turns,
        step_checks,
        between_checks,
        uncertain_cards,
        uncertain_counters,
        states,
    })
}

// ---- the comparison ----

fn card_text(m: &shop_replay::SnapMinion) -> String {
    let golden = if m.golden { " (golden)" } else { "" };
    format!("{}{golden} {}/{}", m.name, m.attack, m.health)
}

fn screen_card_name(data: &Data, ids: &CardIdMap, card: &ScreenCard) -> String {
    resolve_card(data, ids, card)
        .map(|id| data.cards.get(id).name.clone())
        .or_else(|| card.name.clone())
        .or_else(|| card.card_id.clone())
        .unwrap_or_else(|| "an unnamed card".to_string())
}

fn screen_card_golden(data: &Data, ids: &CardIdMap, card: &ScreenCard) -> bool {
    card.golden.unwrap_or(false)
        || resolve_card(data, ids, card).is_some_and(|id| data.cards.get(id).always_golden)
}

fn screen_stat(data: &Data, ids: &CardIdMap, card: &ScreenCard, attack: bool) -> ScreenNumber {
    let given = if attack { card.attack } else { card.health };
    if let Some(value) = given {
        return value;
    }
    let Some(id) = resolve_card(data, ids, card) else {
        return ScreenNumber::Number(0);
    };
    let made = data.cards.make(id, screen_card_golden(data, ids, card));
    ScreenNumber::Number(if attack { made.attack } else { made.health } as i64)
}

fn screen_card_text(data: &Data, ids: &CardIdMap, card: &ScreenCard) -> String {
    let golden = if screen_card_golden(data, ids, card) {
        " (golden)"
    } else {
        ""
    };
    format!(
        "{}{golden} {}/{}",
        screen_card_name(data, ids, card),
        screen_stat(data, ids, card, true).text(),
        screen_stat(data, ids, card, false).text()
    )
}

fn number_check_value(value: ScreenNumber) -> ScreenCheckValue {
    match value {
        ScreenNumber::Number(n) => ScreenCheckValue::Number(n),
        ScreenNumber::Range { min, max } => ScreenCheckValue::Range { min, max },
        ScreenNumber::Null => ScreenCheckValue::Null,
    }
}

fn number_status(engine: i32, screen: ScreenNumber) -> ScreenCheckStatus {
    match screen {
        ScreenNumber::Number(n) => {
            if engine as i64 == n {
                ScreenCheckStatus::Agree
            } else {
                ScreenCheckStatus::Disagree
            }
        }
        ScreenNumber::Range { min, max } => {
            if (min..=max).contains(&(engine as i64)) {
                ScreenCheckStatus::Agree
            } else {
                ScreenCheckStatus::Disagree
            }
        }
        ScreenNumber::Null => ScreenCheckStatus::CannotTell,
    }
}

/// The value a re-sync puts on the engine: the number the screen read, the
/// nearest bound of a range that does not hold the engine's value, or the
/// engine's own value for a range that agrees and for `null`.
fn number_target(engine: i32, screen: ScreenNumber) -> i32 {
    match screen {
        ScreenNumber::Number(n) => n.clamp(i32::MIN as i64, i32::MAX as i64) as i32,
        ScreenNumber::Range { min, max } => {
            if (engine as i64) < min {
                min.clamp(i32::MIN as i64, i32::MAX as i64) as i32
            } else if (engine as i64) > max {
                max.clamp(i32::MIN as i64, i32::MAX as i64) as i32
            } else {
                engine
            }
        }
        ScreenNumber::Null => engine,
    }
}

fn number_check(
    zone: &str,
    index: usize,
    card: &str,
    field: &str,
    engine: i32,
    screen: ScreenNumber,
) -> ScreenCheck {
    ScreenCheck {
        zone: zone.to_string(),
        index: Some(index as u32),
        card: Some(card.to_string()),
        field: field.to_string(),
        engine: ScreenCheckValue::Number(engine as i64),
        screen: number_check_value(screen),
        status: number_status(engine, screen),
    }
}

fn keyword_check(
    zone: &str,
    index: usize,
    card: &str,
    keyword: &str,
    engine: bool,
    screen: bool,
) -> ScreenCheck {
    ScreenCheck {
        zone: zone.to_string(),
        index: Some(index as u32),
        card: Some(card.to_string()),
        field: keyword_key(keyword),
        engine: ScreenCheckValue::Bool(engine),
        screen: ScreenCheckValue::Bool(screen),
        status: if engine == screen {
            ScreenCheckStatus::Agree
        } else {
            ScreenCheckStatus::Disagree
        },
    }
}

/// The keywords the screen read: a full `keywords` list reads every known
/// keyword, and `keyword_reads` then overrides the ones it names.
fn screen_keyword_reads(card: &ScreenCard) -> BTreeMap<String, bool> {
    let mut out = BTreeMap::new();
    if let Some(list) = &card.keywords {
        for (_, name) in Keywords::ALL {
            out.insert(keyword_key(name), false);
        }
        for name in list {
            out.insert(keyword_key(name), true);
        }
    }
    if let Some(reads) = &card.keyword_reads {
        for (name, on) in reads {
            out.insert(keyword_key(name), *on);
        }
    }
    out
}

/// Compare the stats and keywords of one engine card with the screen card it
/// matches. `zone` and `index` are only for the line and the checks.
fn compare_card_values(
    data: &Data,
    ids: &CardIdMap,
    zone: &str,
    index: usize,
    ours: &shop_replay::SnapMinion,
    card: &ScreenCard,
) -> (Vec<String>, Vec<ScreenCheck>) {
    let mut lines = Vec::new();
    let mut checks = Vec::new();
    let name = screen_card_name(data, ids, card);
    let attack = screen_stat(data, ids, card, true);
    let health = screen_stat(data, ids, card, false);
    let attack_status = number_status(ours.attack, attack);
    let health_status = number_status(ours.health, health);
    checks.push(number_check(
        zone,
        index,
        &name,
        "attack",
        ours.attack,
        attack,
    ));
    checks.push(number_check(
        zone,
        index,
        &name,
        "health",
        ours.health,
        health,
    ));
    if attack_status == ScreenCheckStatus::Disagree || health_status == ScreenCheckStatus::Disagree
    {
        lines.push(format!(
            "{zone} {index}: engine {}, screen {}",
            card_text(ours),
            screen_card_text(data, ids, card)
        ));
    }
    let engine_keywords: Vec<String> = ours.keywords.iter().map(|k| keyword_key(k)).collect();
    for (keyword, read) in screen_keyword_reads(card) {
        let engine = engine_keywords.iter().any(|k| k == &keyword);
        checks.push(keyword_check(zone, index, &name, &keyword, engine, read));
        if engine != read {
            let says = |on: bool| if on { "yes" } else { "no" };
            lines.push(format!(
                "{zone} {index}: {name} keyword {keyword} engine {}, screen {}",
                says(engine),
                says(read)
            ));
        }
    }
    (lines, checks)
}

/// Compare a board in order: the screen's first card is the first minion.
fn compare_board(
    data: &Data,
    ids: &CardIdMap,
    ours: &[shop_replay::SnapMinion],
    screen: &[ScreenCard],
) -> (Vec<String>, Vec<ScreenCheck>) {
    let mut lines = Vec::new();
    let mut checks = Vec::new();
    for i in 0..ours.len().max(screen.len()) {
        match (ours.get(i), screen.get(i)) {
            (Some(a), Some(b)) => {
                let name = screen_card_name(data, ids, b);
                let golden = screen_card_golden(data, ids, b);
                if a.name != name || a.golden != golden {
                    lines.push(format!(
                        "board {}: engine {}, screen {}",
                        i + 1,
                        card_text(a),
                        screen_card_text(data, ids, b)
                    ));
                    checks.push(ScreenCheck {
                        zone: "board".to_string(),
                        index: Some((i + 1) as u32),
                        card: Some(a.name.clone()),
                        field: "card".to_string(),
                        engine: ScreenCheckValue::Text(a.name.clone()),
                        screen: ScreenCheckValue::Text(name),
                        status: ScreenCheckStatus::Disagree,
                    });
                    continue;
                }
                let (mut value_lines, mut value_checks) =
                    compare_card_values(data, ids, "board", i + 1, a, b);
                lines.append(&mut value_lines);
                checks.append(&mut value_checks);
            }
            (Some(a), None) => lines.push(format!(
                "board {}: engine {}, screen nothing",
                i + 1,
                card_text(a)
            )),
            (None, Some(b)) => lines.push(format!(
                "board {}: engine nothing, screen {}",
                i + 1,
                screen_card_text(data, ids, b)
            )),
            (None, None) => {}
        }
    }
    (lines, checks)
}

/// Compare a hand or Tavern as a multiset: match cards by name and golden
/// flag, then compare each matched card's stats and the keywords read.
fn compare_group(
    data: &Data,
    ids: &CardIdMap,
    zone: &str,
    ours: &[shop_replay::SnapMinion],
    screen: &[ScreenCard],
) -> (Vec<String>, Vec<ScreenCheck>) {
    let mut lines = Vec::new();
    let mut checks = Vec::new();
    let mut used = vec![false; ours.len()];
    let mut unmatched_screen: Vec<(String, usize)> = Vec::new();
    for (si, b) in screen.iter().enumerate() {
        let name = screen_card_name(data, ids, b);
        let golden = screen_card_golden(data, ids, b);
        let at = ours
            .iter()
            .enumerate()
            .position(|(k, a)| !used[k] && a.name == name && a.golden == golden);
        match at {
            Some(k) => {
                used[k] = true;
                let (mut value_lines, mut value_checks) =
                    compare_card_values(data, ids, zone, si + 1, &ours[k], b);
                lines.append(&mut value_lines);
                checks.append(&mut value_checks);
            }
            None => {
                let text = screen_card_text(data, ids, b);
                if let Some((_, n)) = unmatched_screen.iter_mut().find(|(t, _)| *t == text) {
                    *n += 1;
                } else {
                    unmatched_screen.push((text, 1));
                }
            }
        }
    }
    let mut unmatched_engine: BTreeMap<String, usize> = BTreeMap::new();
    for (k, a) in ours.iter().enumerate() {
        if !used[k] {
            *unmatched_engine.entry(card_text(a)).or_default() += 1;
        }
    }
    for (text, n) in unmatched_engine {
        lines.push(format!("{zone}: engine {text} x{n}, screen no such card"));
    }
    for (text, n) in unmatched_screen {
        lines.push(format!("{zone}: screen {text} x{n}, engine no such card"));
    }
    (lines, checks)
}

fn without_colon(line: String) -> String {
    line.replacen(": engine ", " engine ", 1)
        .replacen(": screen ", " screen ", 1)
}

/// What the engine did differently from a step's `seen` state, before that
/// state is put onto it, and the per-value checks the same comparison made. A
/// random Tavern is skipped: its cards are the draw's, not the engine's.
fn step_seen_differences(
    data: &Data,
    ids: &CardIdMap,
    screen: &ScreenStepSpec,
    ours: &ShopSnapshot,
    step: usize,
) -> (Vec<String>, Vec<ScreenCheck>) {
    let Some(seen) = screen.seen.as_ref() else {
        return (Vec::new(), Vec::new());
    };
    let prefix = format!("after step {} ({}): ", step + 1, screen.step);
    let mut out = Vec::new();
    let mut checks = Vec::new();
    if let Some(board) = seen.board.as_ref() {
        let (lines, mut c) = compare_board(data, ids, &ours.board, board);
        out.extend(
            lines
                .into_iter()
                .map(|l| format!("{prefix}{}", without_colon(l))),
        );
        checks.append(&mut c);
    }
    if let Some(hand) = seen.hand.as_ref() {
        let (lines, mut c) = compare_group(data, ids, "hand", &ours.hand, hand);
        out.extend(
            lines
                .into_iter()
                .map(|l| format!("{prefix}{}", without_colon(l))),
        );
        checks.append(&mut c);
    }
    if let Some(tavern) = seen.tavern.as_ref()
        && !screen.tavern_random
        && screen.step != "roll"
    {
        let (lines, mut c) = compare_group(data, ids, "Tavern", &ours.tavern, tavern);
        out.extend(
            lines
                .into_iter()
                .map(|l| format!("{prefix}{}", without_colon(l))),
        );
        checks.append(&mut c);
    }
    if let Some(gold) = seen.gold {
        checks.push(ScreenCheck {
            zone: "player".to_string(),
            index: None,
            card: None,
            field: "gold".to_string(),
            engine: ScreenCheckValue::Number(ours.gold as i64),
            screen: ScreenCheckValue::Number(gold as i64),
            status: if ours.gold == gold {
                ScreenCheckStatus::Agree
            } else {
                ScreenCheckStatus::Disagree
            },
        });
        if ours.gold != gold {
            out.push(format!("{prefix}engine Gold {}, screen {gold}", ours.gold));
        }
    }
    if let Some(free) = seen.free_refresh {
        let engine = ours.free_refreshes > 0;
        checks.push(ScreenCheck {
            zone: "player".to_string(),
            index: None,
            card: None,
            field: "free_refresh".to_string(),
            engine: ScreenCheckValue::Bool(engine),
            screen: ScreenCheckValue::Bool(free),
            status: if engine == free {
                ScreenCheckStatus::Agree
            } else {
                ScreenCheckStatus::Disagree
            },
        });
        if engine != free {
            let says = |f: bool| if f { "free" } else { "paid" };
            out.push(format!(
                "{prefix}next Refresh engine {}, screen {}",
                says(engine),
                says(free)
            ));
        }
    }
    if let Some(tier) = seen.tier {
        checks.push(ScreenCheck {
            zone: "player".to_string(),
            index: None,
            card: None,
            field: "tier".to_string(),
            engine: ScreenCheckValue::Number(ours.tier as i64),
            screen: ScreenCheckValue::Number(tier as i64),
            status: if ours.tier == tier {
                ScreenCheckStatus::Agree
            } else {
                ScreenCheckStatus::Disagree
            },
        });
        if ours.tier != tier {
            out.push(format!("{prefix}engine tier {}, screen {tier}", ours.tier));
        }
    }
    (out, checks)
}

/// Compare only the fields the turn's `end` has. The board keeps its order;
/// the hand and Tavern are compared by matching cards, as the shop replay does.
pub fn compare_end(
    data: &Data,
    ids: &CardIdMap,
    game: &Game,
    end: &ScreenStateSpec,
    _turn: u32,
) -> (Vec<String>, Vec<ScreenCheck>, Vec<String>) {
    let ours = ShopSnapshot::of_game(data, game, ME);
    let missing = Vec::new();
    let mut diffs = Vec::new();
    let mut checks = Vec::new();
    if let Some(board) = end.board.as_ref() {
        let (lines, mut c) = compare_board(data, ids, &ours.board, board);
        diffs.extend(lines);
        checks.append(&mut c);
    }
    if let Some(hand) = end.hand.as_ref() {
        let (lines, mut c) = compare_group(data, ids, "hand", &ours.hand, hand);
        diffs.extend(lines);
        checks.append(&mut c);
    } else if let Some(count) = end.hand_count
        && ours.hand.len() != count as usize
    {
        diffs.push(format!(
            "hand count: engine {}, screen {count}",
            ours.hand.len()
        ));
    }
    if let Some(tavern) = end.tavern.as_ref() {
        let (lines, mut c) = compare_group(data, ids, "Tavern", &ours.tavern, tavern);
        diffs.extend(lines);
        checks.append(&mut c);
    }
    if let Some(gold) = end.gold {
        checks.push(ScreenCheck {
            zone: "player".to_string(),
            index: None,
            card: None,
            field: "gold".to_string(),
            engine: ScreenCheckValue::Number(ours.gold as i64),
            screen: ScreenCheckValue::Number(gold as i64),
            status: if ours.gold == gold {
                ScreenCheckStatus::Agree
            } else {
                ScreenCheckStatus::Disagree
            },
        });
        if ours.gold != gold {
            diffs.push(format!("Gold: engine {}, screen {gold}", ours.gold));
        }
    }
    if let Some(tier) = end.tier {
        checks.push(ScreenCheck {
            zone: "player".to_string(),
            index: None,
            card: None,
            field: "tier".to_string(),
            engine: ScreenCheckValue::Number(ours.tier as i64),
            screen: ScreenCheckValue::Number(tier as i64),
            status: if ours.tier == tier {
                ScreenCheckStatus::Agree
            } else {
                ScreenCheckStatus::Disagree
            },
        });
        if ours.tier != tier {
            diffs.push(format!("tier: engine {}, screen {tier}", ours.tier));
        }
    }
    if let Some(health) = end.health {
        checks.push(ScreenCheck {
            zone: "player".to_string(),
            index: None,
            card: None,
            field: "health".to_string(),
            engine: ScreenCheckValue::Number(ours.health as i64),
            screen: ScreenCheckValue::Number(health as i64),
            status: if ours.health == health {
                ScreenCheckStatus::Agree
            } else {
                ScreenCheckStatus::Disagree
            },
        });
        if ours.health != health {
            diffs.push(format!("Health: engine {}, screen {health}", ours.health));
        }
    }
    (diffs, checks, missing)
}

// ---- verdicts over several seeds ----

/// One value a comparison checked: a number, a range, `null`, a keyword's
/// presence, or a card name.
#[derive(Clone, Debug, Serialize)]
#[serde(untagged)]
pub enum ScreenCheckValue {
    Number(i64),
    Range { min: i64, max: i64 },
    Bool(bool),
    Text(String),
    Null,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ScreenCheckStatus {
    Agree,
    Disagree,
    CannotTell,
}

/// One per-value check: where the card is, which field, both values and what
/// the comparison said. This is what bg-vision's check mode reads.
#[derive(Clone, Debug, Serialize)]
pub struct ScreenCheck {
    pub zone: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub card: Option<String>,
    pub field: String,
    pub engine: ScreenCheckValue,
    pub screen: ScreenCheckValue,
    pub status: ScreenCheckStatus,
}

/// The checks made at one point in a turn: before it, after a step, or at its
/// end.
#[derive(Clone, Debug, Serialize)]
pub struct ScreenCheckGroup {
    /// `before turn`, `after step` or `end`.
    pub when: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    pub checks: Vec<ScreenCheck>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ScreenLineClass {
    Certain,
    Random,
}

/// One difference line with the class the seeds gave it.
#[derive(Clone, Debug, Serialize)]
pub struct ScreenLine {
    pub text: String,
    pub class: ScreenLineClass,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub uncertain: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScreenVerdict {
    Ok,
    OkRandom,
    Uncertain,
    Flagged,
    Stopped,
}

impl ScreenVerdict {
    pub fn as_str(self) -> &'static str {
        match self {
            ScreenVerdict::Ok => "ok",
            ScreenVerdict::OkRandom => "ok-random",
            ScreenVerdict::Uncertain => "uncertain",
            ScreenVerdict::Flagged => "flagged",
            ScreenVerdict::Stopped => "stopped",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ScreenSeedResult {
    pub seed: u64,
    pub played: usize,
    pub stop: Option<ScreenStop>,
    pub differences: Vec<String>,
    pub missing: Vec<String>,
    pub engine: Option<ScreenEndState>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ScreenSplit {
    pub step: usize,
    pub kind: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ScreenTurnOutcome {
    pub file: String,
    pub game: u32,
    pub turn: u32,
    pub steps: usize,
    pub verdict: ScreenVerdict,
    pub stop: Option<ScreenStop>,
    /// Certain step and end lines that are not uncertain: the lines that flag
    /// the turn.
    pub differences: Vec<String>,
    /// Certain step and end lines that involve a value a fight could have
    /// changed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub uncertain_lines: Vec<String>,
    /// Step and end lines that differ between seeds, or that only some seeds
    /// produce. They do not flag the turn.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub random_lines: Vec<String>,
    /// Chained mode: certain differences between the carried game and the
    /// screen's `start`, before that start was put on. They do not flag the
    /// turn.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub between_turns: Vec<String>,
    /// Chained mode: between-turn lines the seeds did not agree on.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub random_between_turns: Vec<String>,
    /// Every step and end line with its class.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lines: Vec<ScreenLine>,
    /// Every between-turn line with its class.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub between_turn_lines: Vec<ScreenLine>,
    /// The per-value checks of the first completed seed, grouped by where
    /// they were made.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checks: Vec<ScreenCheckGroup>,
    /// Chained mode: the cards and counters a fight could have changed that
    /// turn. Step lines that involve them are marked.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub uncertain: Vec<String>,
    /// Chained mode: this turn was seated fresh because the file skipped one
    /// or the turn before it stopped.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reseated: bool,
    pub randomness_at: Option<ScreenSplit>,
    pub seeds: Vec<ScreenSeedResult>,
}

/// Replay one turn over `seeds` seeds and judge it. A random outcome the screen
/// does not pin down is `ok-random` when some seed matches; the first step
/// whose engine state differs between seeds names the split.
pub fn replay_screen_turn_seeds(
    data: &Arc<Data>,
    ids: &CardIdMap,
    file: &ScreenFileSpec,
    turn: &ScreenTurnSpec,
    seeds: u64,
) -> ScreenTurnOutcome {
    let n = seeds.max(1);
    let runs = (0..n)
        .map(|k| replay_screen_turn(data, ids, file, turn, k + 1))
        .collect();
    outcome_from_runs(data, ids, file, turn, runs)
}

/// Judge one turn from the runs of its seeds. This is the shared tail of the
/// non-chained and the chained replay: the chained one carries each seed's
/// game between turns before it gets here.
#[derive(Clone, Debug)]
struct ComputedSeed {
    seed: u64,
    played: usize,
    stop: Option<ScreenStop>,
    missing: Vec<String>,
    engine: Option<ScreenEndState>,
    step_lines: Vec<String>,
    end_lines: Vec<String>,
    between_lines: Vec<String>,
    step_checks: Vec<ScreenCheckGroup>,
    between_checks: Option<ScreenCheckGroup>,
    end_checks: Option<ScreenCheckGroup>,
    uncertain_cards: Vec<String>,
    uncertain_counters: bool,
    states: Vec<ShopSnapshot>,
    complete: bool,
}

fn outcome_from_runs(
    data: &Arc<Data>,
    ids: &CardIdMap,
    file: &ScreenFileSpec,
    turn: &ScreenTurnSpec,
    runs: Vec<Result<ScreenTurnRun, ScreenStop>>,
) -> ScreenTurnOutcome {
    let mut computed = Vec::new();
    for (k, run) in runs.into_iter().enumerate() {
        let seed = k as u64 + 1;
        match run {
            Ok(run) => {
                let complete = run.stop.is_none();
                let (end_lines, end_checks, compare_missing) = if complete {
                    match &turn.end {
                        Some(end) => {
                            let (d, c, m) = compare_end(data, ids, &run.game, end, turn.turn);
                            let d = mark_uncertain(d, &run.uncertain_cards, run.uncertain_counters);
                            (
                                d,
                                Some(ScreenCheckGroup {
                                    when: "end".to_string(),
                                    step: None,
                                    kind: None,
                                    checks: c,
                                }),
                                m,
                            )
                        }
                        None => (Vec::new(), None, Vec::new()),
                    }
                } else {
                    (Vec::new(), None, Vec::new())
                };
                let mut missing = run.missing.clone();
                missing.extend(compare_missing);
                computed.push(ComputedSeed {
                    seed,
                    played: run.played,
                    stop: run.stop,
                    missing,
                    engine: Some(ScreenEndState::of(data, &run.game)),
                    step_lines: run.step_differences,
                    end_lines,
                    between_lines: run.between_turns,
                    step_checks: run.step_checks,
                    between_checks: run.between_checks,
                    end_checks,
                    uncertain_cards: run.uncertain_cards,
                    uncertain_counters: run.uncertain_counters,
                    states: run.states,
                    complete,
                });
            }
            Err(stop) => computed.push(ComputedSeed {
                seed,
                played: 0,
                stop: Some(stop),
                missing: Vec::new(),
                engine: None,
                step_lines: Vec::new(),
                end_lines: Vec::new(),
                between_lines: Vec::new(),
                step_checks: Vec::new(),
                between_checks: None,
                end_checks: None,
                uncertain_cards: Vec::new(),
                uncertain_counters: false,
                states: Vec::new(),
                complete: false,
            }),
        }
    }

    let completed = computed.iter().filter(|c| c.complete).count();
    let mut line_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut between_counts: BTreeMap<String, usize> = BTreeMap::new();
    for c in computed.iter().filter(|c| c.complete) {
        let mut seen = std::collections::BTreeSet::new();
        for text in c.step_lines.iter().chain(c.end_lines.iter()) {
            if seen.insert(text.clone()) {
                *line_counts.entry(text.clone()).or_default() += 1;
            }
        }
        let mut seen = std::collections::BTreeSet::new();
        for text in &c.between_lines {
            if seen.insert(text.clone()) {
                *between_counts.entry(text.clone()).or_default() += 1;
            }
        }
    }

    let mut lines = Vec::new();
    let mut between_turn_lines = Vec::new();
    let mut differences = Vec::new();
    let mut uncertain_lines = Vec::new();
    let mut random_lines = Vec::new();
    let mut between_turns = Vec::new();
    let mut random_between_turns = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for c in &computed {
        for text in c.step_lines.iter().chain(c.end_lines.iter()) {
            if !seen.insert(text.clone()) {
                continue;
            }
            let class = if completed > 0 && line_counts.get(text).copied().unwrap_or(0) == completed
            {
                ScreenLineClass::Certain
            } else {
                ScreenLineClass::Random
            };
            let uncertain = text.contains("(uncertain after the fight)");
            lines.push(ScreenLine {
                text: text.clone(),
                class,
                uncertain,
            });
            match (class, uncertain) {
                (ScreenLineClass::Certain, true) => uncertain_lines.push(text.clone()),
                (ScreenLineClass::Certain, false) => differences.push(text.clone()),
                (ScreenLineClass::Random, _) => random_lines.push(text.clone()),
            }
        }
    }
    let mut seen = std::collections::BTreeSet::new();
    for c in &computed {
        for text in &c.between_lines {
            if !seen.insert(text.clone()) {
                continue;
            }
            let class =
                if completed > 0 && between_counts.get(text).copied().unwrap_or(0) == completed {
                    ScreenLineClass::Certain
                } else {
                    ScreenLineClass::Random
                };
            between_turn_lines.push(ScreenLine {
                text: text.clone(),
                class,
                uncertain: false,
            });
            match class {
                ScreenLineClass::Certain => between_turns.push(text.clone()),
                ScreenLineClass::Random => random_between_turns.push(text.clone()),
            }
        }
    }

    let stop = computed
        .first()
        .and_then(|c| c.stop.clone())
        .or_else(|| computed.iter().find_map(|c| c.stop.clone()));
    let verdict = if completed == 0 {
        ScreenVerdict::Stopped
    } else if !differences.is_empty() {
        ScreenVerdict::Flagged
    } else if !uncertain_lines.is_empty() {
        ScreenVerdict::Uncertain
    } else if !random_lines.is_empty() || !random_between_turns.is_empty() {
        ScreenVerdict::OkRandom
    } else {
        ScreenVerdict::Ok
    };

    let mut state_sets: Vec<Option<Vec<ShopSnapshot>>> = Vec::new();
    let mut plays = Vec::new();
    for c in &computed {
        state_sets.push(c.complete.then(|| c.states.clone()));
        plays.push((c.played, c.stop.clone()));
    }
    let mut randomness_at = None;
    if verdict == ScreenVerdict::OkRandom {
        let done: Vec<&Vec<ShopSnapshot>> = state_sets.iter().filter_map(|s| s.as_ref()).collect();
        let limit = done.iter().map(|s| s.len()).min().unwrap_or(0);
        for i in 0..limit {
            let first = &done[0][i];
            if done.iter().any(|s| s[i] != *first) {
                randomness_at = Some(ScreenSplit {
                    step: i,
                    kind: turn
                        .steps
                        .get(i)
                        .map(|s| s.step.clone())
                        .unwrap_or_default(),
                });
                break;
            }
        }
        if randomness_at.is_none() {
            let step = plays
                .iter()
                .filter_map(|(_, stop)| stop.as_ref().map(|s| s.step))
                .min();
            if let Some(step) = step {
                randomness_at = Some(ScreenSplit {
                    step,
                    kind: turn
                        .steps
                        .get(step)
                        .map(|s| s.step.clone())
                        .unwrap_or_default(),
                });
            }
        }
    }

    let source = computed
        .iter()
        .find(|c| c.complete)
        .or_else(|| computed.first());
    let mut uncertain = source
        .map(|c| c.uncertain_cards.clone())
        .unwrap_or_default();
    if source.is_some_and(|c| c.uncertain_counters) {
        uncertain.push("player counters".to_string());
    }
    let mut checks = Vec::new();
    if let Some(c) = source {
        if let Some(group) = c.between_checks.clone() {
            checks.push(group);
        }
        checks.extend(c.step_checks.clone());
        if let Some(group) = c.end_checks.clone() {
            checks.push(group);
        }
    }
    let results = computed
        .iter()
        .map(|c| {
            let mut all = c.step_lines.clone();
            all.extend(c.end_lines.clone());
            ScreenSeedResult {
                seed: c.seed,
                played: c.played,
                stop: c.stop.clone(),
                differences: all,
                missing: c.missing.clone(),
                engine: c.engine.clone(),
            }
        })
        .collect();
    ScreenTurnOutcome {
        file: file.file(),
        game: file.game,
        turn: turn.turn,
        steps: turn.steps.len(),
        verdict,
        stop,
        differences,
        uncertain_lines,
        random_lines,
        between_turns,
        random_between_turns,
        lines,
        between_turn_lines,
        checks,
        uncertain,
        reseated: false,
        randomness_at,
        seeds: results,
    }
}

/// Replay a whole file in chained mode. Each seed carries its own game from
/// turn to turn; a turn starts from the engine's end of the previous turn
/// unless the file skipped one or that turn stopped, in which case it is
/// seated fresh from its `start` and marked `reseated`.
pub fn replay_screen_file_chain(
    data: &Arc<Data>,
    ids: &CardIdMap,
    file: &ScreenFileSpec,
    seeds: u64,
) -> Vec<ScreenTurnOutcome> {
    let n = seeds.max(1);
    let turns = file.turns.len();
    let mut per_turn: Vec<Vec<Result<ScreenTurnRun, ScreenStop>>> =
        (0..turns).map(|_| Vec::new()).collect();
    let mut reseated = vec![false; turns];
    for k in 0..n {
        let seed = k + 1;
        let mut carry: Option<Box<Game>> = None;
        let mut previous_turn: Option<u32> = None;
        let mut previous_stopped = false;
        for (i, turn) in file.turns.iter().enumerate() {
            let fresh = i == 0
                || previous_stopped
                || previous_turn.is_some_and(|t| turn.turn != t + 1)
                || carry.as_ref().is_none_or(|g| g.turn + 1 != turn.turn);
            if i > 0 && fresh {
                reseated[i] = true;
            }
            let run = if fresh {
                replay_screen_turn_from(data, ids, file, turn, seed, None)
            } else {
                replay_screen_turn_from(data, ids, file, turn, seed, carry.as_deref())
            };
            match &run {
                Ok(run) if run.stop.is_none() => {
                    carry = Some(run.game.clone());
                    previous_stopped = false;
                }
                _ => {
                    carry = None;
                    previous_stopped = true;
                }
            }
            per_turn[i].push(run);
            previous_turn = Some(turn.turn);
        }
    }
    per_turn
        .into_iter()
        .enumerate()
        .map(|(i, runs)| {
            let mut outcome = outcome_from_runs(data, ids, file, &file.turns[i], runs);
            outcome.reseated = reseated[i];
            outcome
        })
        .collect()
}

/// Parse every screen file of a folder: `files` are the (name, text) pairs,
/// one per game, in the folder's order.
pub fn parse_screen_files(files: &[(String, String)]) -> Result<Vec<ScreenFileSpec>, String> {
    let mut out = Vec::with_capacity(files.len());
    for (name, text) in files {
        let spec: ScreenFileSpec =
            serde_json::from_str(text).map_err(|e| format!("{name}: {e}"))?;
        out.push(spec);
    }
    Ok(out)
}

// ---- a screen file from an action file (the check on Era's logs) ----

/// A stat as `--k-ranges` writes it: 1,000 or more becomes the range the
/// game's one-decimal-of-thousands display gives, and everything else stays
/// exact. 2,050 to 2,149 is "2.1k".
fn written_stat(value: i32, k_ranges: bool) -> ScreenNumber {
    if k_ranges && value >= 1000 {
        let center = ((value as i64 + 50) / 100) * 100;
        ScreenNumber::Range {
            min: center - 50,
            max: center + 49,
        }
    } else {
        ScreenNumber::Number(value as i64)
    }
}

fn card_of_spec(
    data: &Data,
    ids: &CardIdMap,
    m: &ShopMinionSpec,
    k_ranges: bool,
) -> Option<ScreenCard> {
    let name = m
        .name
        .clone()
        .or_else(|| {
            m.card_id
                .as_deref()
                .and_then(|id| ids.get(id))
                .map(|(n, _)| n.to_string())
        })
        .or_else(|| {
            let id = m.card_id.as_deref()?;
            data.cards
                .by_name(&real_boards::resolved_name(id, id))
                .map(|c| data.cards.get(c).name.clone())
        })?;
    let cost = visible_cost(data, m);
    Some(ScreenCard {
        card_id: m.card_id.clone(),
        name: Some(name),
        attack: Some(written_stat(m.attack, k_ranges)),
        health: Some(written_stat(m.health, k_ranges)),
        golden: Some(m.golden),
        keywords: (!m.keywords.is_empty()).then(|| m.keywords.clone()),
        keyword_reads: None,
        slot: m.slot,
        kind: m.kind.clone(),
        frozen: Some(m.frozen),
        cost,
        costs_health: m.costs_health.then_some(true),
        dark_gift: None,
        dark_gifts: None,
    })
}

fn visible_cost(data: &Data, m: &ShopMinionSpec) -> Option<i32> {
    m.cost_override
        .or(m.cost)
        .or_else(|| m.cost_reduce.map(|r| data.rules.buy_cost as i32 - r))
}

fn state_of_spec(
    data: &Data,
    ids: &CardIdMap,
    state: &ShopStateSpec,
    k_ranges: bool,
) -> ScreenStateSpec {
    let hand: Vec<ScreenCard> = state
        .hand
        .iter()
        .filter_map(|m| card_of_spec(data, ids, m, k_ranges))
        .collect();
    let hand_count = if state.hand_hidden > 0 {
        Some(state.hand.len() as u32 + state.hand_hidden)
    } else {
        None
    };
    let mut powers: Vec<ScreenPower> = state
        .hero_powers
        .iter()
        .map(|p| ScreenPower {
            card_id: p.card_id.clone(),
            name: p.name.clone(),
            cost: p.cost,
            used_this_turn: Some(p.used_this_turn),
            numbers: Some(p.numbers.clone()),
            waiting: p
                .waiting
                .as_ref()
                .and_then(|w| card_of_spec(data, ids, w, k_ranges)),
            waiting_seen: Some(true),
            second: p.second,
        })
        .collect();
    powers.sort_by_key(|p| p.second);
    let hero_power = powers.first().cloned();
    let hero_power_second = powers.get(1).cloned();
    ScreenStateSpec {
        board: Some(
            state
                .board
                .iter()
                .filter_map(|m| card_of_spec(data, ids, m, k_ranges))
                .collect(),
        ),
        hand: Some(hand),
        tavern: Some(
            state
                .tavern
                .iter()
                .filter_map(|m| card_of_spec(data, ids, m, k_ranges))
                .collect(),
        ),
        gold: Some(state.gold),
        gold_max: Some(state.gold_max),
        free_refresh: Some(state.free_refreshes > 0),
        tier: Some(state.tier),
        upgrade_cost: state.upgrade_cost,
        health: Some(state.health),
        armor: Some(state.armor),
        frozen: Some(state.frozen),
        hero_power,
        hero_power_second,
        trinkets: Some(
            state
                .trinkets
                .iter()
                .map(|t| ScreenTrinket {
                    card_id: t.card_id.clone(),
                    name: t.name.clone(),
                    greater: t.greater,
                    slot: None,
                    numbers: Some(t.numbers.clone()),
                })
                .collect(),
        ),
        hand_count,
    }
}

fn place_of_spec(data: &Data, ids: &CardIdMap, place: &ShopPlace) -> ScreenPlace {
    let _ = (data, ids);
    ScreenPlace {
        kind: place.kind.clone(),
        position: place.position,
        index: place.index,
        slot: place.slot,
        card_id: place.card_id.clone(),
        name: place.name.clone(),
    }
}

/// A logged gift as the screen file writes the one a Discover showed: its card
/// id and name, and its countdown when the log has one.
fn screen_gift_of(g: &ShopGiftSpec) -> ScreenGift {
    ScreenGift::At {
        card_id: g.card_id.clone(),
        name: g.name.clone(),
        state: g.state,
    }
}

/// A non-Roll step whose Tavern changed by more than the one card the step
/// acted on: a card's Refresh, whose new cards are random.
fn tavern_refreshed(before: Option<&ShopStateSpec>, after: Option<&ShopStateSpec>) -> bool {
    let (Some(before), Some(after)) = (before, after) else {
        return false;
    };
    if before.tavern.is_empty() && after.tavern.is_empty() {
        return false;
    }
    let key = |m: &ShopMinionSpec| (m.card_id.clone(), m.name.clone());
    let mut a: std::collections::HashMap<(Option<String>, Option<String>), usize> =
        std::collections::HashMap::new();
    for m in &before.tavern {
        *a.entry(key(m)).or_default() += 1;
    }
    let mut common = 0;
    for m in &after.tavern {
        if let Some(n) = a.get_mut(&key(m))
            && *n > 0
        {
            *n -= 1;
            common += 1;
        }
    }
    before.tavern.len() + after.tavern.len() - 2 * common > 1
}

fn step_of_spec(
    data: &Data,
    ids: &CardIdMap,
    step: &ShopStepSpec,
    before: Option<&ShopStateSpec>,
    after: Option<&ShopStateSpec>,
    k_ranges: bool,
) -> ScreenStepSpec {
    let mut seen = ScreenStateSpec::default();
    let mut has_seen = false;
    // The Gold digits and the Refresh button's price are always on screen, so
    // every step carries them after it. An end-of-turn state is the next
    // turn's quiet moment, not the shop-end Gold the replay still holds.
    if let Some(after) = after {
        if step.step != "end_turn" {
            seen.gold = Some(after.gold);
        }
        seen.free_refresh = Some(after.free_refreshes > 0);
        seen.board = Some(
            after
                .board
                .iter()
                .filter_map(|m| card_of_spec(data, ids, m, k_ranges))
                .collect(),
        );
        seen.hand = Some(
            after
                .hand
                .iter()
                .filter_map(|m| card_of_spec(data, ids, m, k_ranges))
                .collect(),
        );
        seen.tavern = Some(
            after
                .tavern
                .iter()
                .filter_map(|m| card_of_spec(data, ids, m, k_ranges))
                .collect(),
        );
        seen.frozen = Some(after.frozen);
        seen.trinkets = Some(
            after
                .trinkets
                .iter()
                .map(|t| ScreenTrinket {
                    card_id: t.card_id.clone(),
                    name: t.name.clone(),
                    greater: t.greater,
                    slot: None,
                    numbers: Some(t.numbers.clone()),
                })
                .collect(),
        );
        let mut powers: Vec<ScreenPower> = after
            .hero_powers
            .iter()
            .map(|p| ScreenPower {
                card_id: p.card_id.clone(),
                name: p.name.clone(),
                cost: p.cost,
                used_this_turn: Some(p.used_this_turn),
                numbers: Some(p.numbers.clone()),
                waiting: p
                    .waiting
                    .as_ref()
                    .and_then(|w| card_of_spec(data, ids, w, k_ranges)),
                waiting_seen: Some(true),
                second: p.second,
            })
            .collect();
        powers.sort_by_key(|p| p.second);
        seen.hero_power = powers.first().cloned();
        seen.hero_power_second = powers.get(1).cloned();
        if after.hand_hidden > 0 {
            seen.hand_count = Some(after.hand.len() as u32 + after.hand_hidden);
        }
        has_seen = true;
    }
    ScreenStepSpec {
        step: step.step.clone(),
        slot: step.slot,
        index: step.index,
        position: step.position,
        to: step.to.or_else(|| {
            step.moved
                .as_ref()
                .and_then(|m| m.to.as_ref())
                .and_then(|t| t.position)
        }),
        which: step.which.clone(),
        on: step.on,
        cost: step.cost,
        free: step.free,
        tier_after: step.tier_after,
        sub_option: step.sub_option,
        slot_kind: step.slot_kind.clone(),
        kind: step.kind.clone(),
        card: step.card.as_ref().and_then(|c| {
            let mut card = ScreenCard {
                card_id: c.card_id.clone(),
                name: c.name.clone(),
                attack: Some(written_stat(c.attack, k_ranges)),
                health: Some(written_stat(c.health, k_ranges)),
                golden: Some(c.golden),
                keywords: c.keywords.clone(),
                keyword_reads: None,
                slot: c.slot,
                kind: c.kind.clone(),
                frozen: None,
                cost: c.cost,
                costs_health: None,
                dark_gift: None,
                dark_gifts: None,
            };
            // A buy's price is what the screen showed on the card, and the
            // step's own paid Gold is that price when a discount made it
            // differ from the card's list price.
            if step.step == "buy" {
                let from_tavern = before.and_then(|before| {
                    before.tavern.iter().find(|m| {
                        (c.card_id.is_some() && m.card_id == c.card_id)
                            || (c.name.is_some() && m.name == c.name)
                            || (c.slot.is_some() && m.slot == c.slot)
                    })
                });
                if let Some(paid) = step.paid.filter(|p| p.health <= 0 && p.gold >= 0) {
                    card.cost = Some(paid.gold);
                }
                if card.cost.is_none() {
                    card.cost = from_tavern.and_then(|m| visible_cost(data, m));
                }
                card.costs_health = from_tavern.is_some_and(|m| m.costs_health).then_some(true);
            }
            (card.card_id.is_some() || card.name.is_some()).then_some(card)
        }),
        target: step.target.as_ref().map(|t| place_of_spec(data, ids, t)),
        source: step.source.as_ref().map(|t| place_of_spec(data, ids, t)),
        options: step
            .options
            .iter()
            .map(|o| ScreenCard {
                card_id: o.card_id.clone(),
                name: o.name.clone(),
                attack: Some(written_stat(o.attack, k_ranges)),
                health: Some(written_stat(o.health, k_ranges)),
                golden: Some(o.golden),
                keywords: o.keywords.clone(),
                keyword_reads: None,
                slot: o.slot,
                kind: o.kind.clone(),
                frozen: None,
                cost: o.cost,
                costs_health: None,
                dark_gift: o.dark_gifts.first().map(screen_gift_of),
                dark_gifts: None,
            })
            .collect(),
        picked: step.picked.as_ref().map(|p| {
            // A trinket's picked card can carry the list price while the
            // offer showed the price actually paid. The paid Gold is what the
            // screen showed on the button, so write that. A hero power's
            // trinket Discover is free even when its list price is not.
            let trinket = p
                .card_id
                .as_deref()
                .is_some_and(|id| data.trinkets.by_card_id(id).is_some())
                || p.name
                    .as_deref()
                    .is_some_and(|name| data.trinkets.by_name(name).is_some());
            let cost = if step.step == "trinket_pick" || trinket {
                step.paid
                    .filter(|paid| paid.gold >= 0)
                    .map(|paid| paid.gold)
                    .or_else(|| {
                        step.options
                            .iter()
                            .find(|o| {
                                (p.card_id.is_some() && o.card_id == p.card_id)
                                    || (p.name.is_some() && o.name == p.name)
                            })
                            .and_then(|o| o.cost)
                    })
                    .or(p.cost)
            } else {
                p.cost
            };
            ScreenPick::Card(ScreenCard {
                card_id: p.card_id.clone(),
                name: p.name.clone(),
                attack: Some(written_stat(p.attack, k_ranges)),
                health: Some(written_stat(p.health, k_ranges)),
                golden: Some(p.golden),
                keywords: p.keywords.clone(),
                keyword_reads: None,
                slot: p.slot,
                kind: p.kind.clone(),
                frozen: None,
                cost,
                costs_health: None,
                dark_gift: p.dark_gifts.first().map(screen_gift_of),
                dark_gifts: None,
            })
        }),
        tavern_random: step.step == "roll" || tavern_refreshed(before, after),
        seen: has_seen.then_some(seen),
    }
}

/// A screen file from one action file: only what a screen shows. The turn's
/// start and end keep their cards, stats, golden flags, Gold, tier, Health,
/// hero power and trinkets. A step keeps its place and its choice, and every
/// step's `seen` keeps the board, hand, Tavern, Gold, free Refreshes, hero
/// powers and trinkets the screen showed right after it.
pub fn screen_from_action_file(
    data: &Data,
    ids: &CardIdMap,
    file: &ShopFileSpec,
    k_ranges: bool,
) -> ScreenFileSpec {
    let turns = file
        .turns
        .iter()
        .map(|turn| {
            let mut before: Option<&ShopStateSpec> = Some(&turn.start);
            let steps = turn
                .actions
                .iter()
                .map(|step| {
                    let out = step_of_spec(
                        data,
                        ids,
                        step,
                        before,
                        step.after.as_ref().or(before),
                        k_ranges,
                    );
                    before = step.after.as_ref().or(before);
                    out
                })
                .collect();
            ScreenTurnSpec {
                turn: turn.turn,
                start: state_of_spec(data, ids, &turn.start, k_ranges),
                steps,
                end: turn.end.as_ref().map(|e| {
                    let mut end = state_of_spec(data, ids, e, k_ranges);
                    // The logged end after an end_turn step is the next turn's
                    // quiet moment. Its Gold is not the shop-end Gold.
                    if turn.actions.last().is_some_and(|s| s.step == "end_turn") {
                        end.gold = None;
                    }
                    end
                }),
            }
        })
        .collect();
    ScreenFileSpec {
        source: file.source.clone(),
        game: file.game,
        hero: file.hero.as_ref().map(|h| ScreenHero {
            card_id: h.card_id.clone(),
            name: h.name.clone(),
        }),
        lobby: if file.lobby.is_empty() {
            file.tribes.clone()
        } else {
            file.lobby.clone()
        },
        turns,
    }
}

/// The JSON value of a screen file, for a command that writes one per game.
pub fn screen_file_value(file: &ScreenFileSpec) -> Value {
    serde_json::to_value(file).expect("a screen file serializes")
}
