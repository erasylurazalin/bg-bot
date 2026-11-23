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
    self, HeroRef, ME, ShopFileSpec, ShopMinionSpec, ShopPlace, ShopPowerSpec, ShopSnapshot,
    ShopStateSpec, ShopStepSpec, ShopTrinketSpec, ShopTurnSpec,
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
    /// Free Refreshes left this turn: the Refresh button's price is 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub free_refreshes: Option<u16>,
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

/// One card as the screen showed it. A missing `attack`/`health` means the
/// card's printed stats, doubled when golden; a missing `keywords` means the
/// card's printed keywords.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ScreenCard {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attack: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub golden: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keywords: Option<Vec<String>>,
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

fn card_label(card: &ScreenCard) -> String {
    card.name
        .clone()
        .or_else(|| card.card_id.clone())
        .unwrap_or_else(|| "an unnamed card".to_string())
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
    let Some(id) = resolve_card(data, ids, card) else {
        missing.push(format!("card not built: {}", card_label(card)));
        return ShopMinionSpec {
            card_id: card.card_id.clone(),
            name: card.name.clone(),
            attack: card.attack.unwrap_or(0),
            health: card.health.unwrap_or(0),
            golden: card.golden.unwrap_or(false),
            keywords: card.keywords.clone().unwrap_or_default(),
            slot: card.slot,
            kind: card.kind.clone(),
            frozen: card.frozen.unwrap_or(false),
            cost: card.cost,
            costs_health: card.costs_health.unwrap_or(false),
            ..Default::default()
        };
    };
    let c = data.cards.get(id);
    let golden = card.golden.unwrap_or(false) || c.always_golden;
    let made = data.cards.make(id, golden);
    let keywords = card
        .keywords
        .clone()
        .unwrap_or_else(|| keyword_names(made.keywords));
    let mut spec = ShopMinionSpec {
        card_id: card.card_id.clone(),
        name: Some(c.name.clone()),
        attack: card.attack.unwrap_or(made.attack),
        health: card.health.unwrap_or(made.health),
        golden,
        keywords,
        slot: card.slot,
        kind: card
            .kind
            .clone()
            .or_else(|| Some(if c.is_spell() { "spell" } else { "minion" }.to_string())),
        frozen: card.frozen.unwrap_or(false),
        costs_health: card.costs_health.unwrap_or(false),
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
        free_refreshes: state.free_refreshes.unwrap_or(0),
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
        card_id: spec.card_id,
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
        ..Default::default()
    }
}

fn shop_place_of_option(data: &Data, ids: &CardIdMap, card: &ScreenCard, id: u64) -> ShopPlace {
    match resolve_card(data, ids, card) {
        Some(_) => {
            let mut ignored = Vec::new();
            shop_place_of_card(data, ids, card, id, &mut ignored)
        }
        None => ShopPlace {
            id,
            card_id: card.card_id.clone(),
            name: card.name.clone(),
            kind: Some(card.kind.clone().unwrap_or_else(|| "minion".to_string())),
            keywords: card.keywords.clone(),
            attack: card.attack.unwrap_or(0),
            health: card.health.unwrap_or(0),
            golden: card.golden.unwrap_or(false),
            ..Default::default()
        },
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
    pub game: Game,
    pub steps: usize,
    pub played: usize,
    pub stop: Option<ScreenStop>,
    pub missing: Vec<String>,
    pub seed: u64,
    /// What the engine did differently from the step's own `seen` state. The
    /// replay carries on; these lines flag the turn.
    pub step_differences: Vec<String>,
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
    let mut missing = Vec::new();
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
    let base = screen_file_spec(file);
    let hero = base.hero(data, &start);
    let lobby = base.lobby();
    let loaded = shop_replay::load(data, &start, hero, lobby, turn.turn, seed, None);
    let mut missing = {
        let mut m = missing;
        m.extend(loaded.missing);
        drop_waiting_missing(&mut m, waiting_known_empty);
        m
    };
    let mut game = loaded.game;
    // A passive power seated after `clear_choices` can queue its start-of-turn
    // Discovers again. The screen's `start` is past them: the turns that have
    // one show it as a step of its own.
    game.players[ME].discover_queue.clear();
    resync_power_uses(data, &mut game, &start);
    let steps = turn.steps.len();
    let mut states = Vec::new();
    let mut step_differences = Vec::new();
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
                states,
            });
        }
        apply_step_price(data, ids, &mut game, screen);
        let (next, miss, _text) = match seated.play_on(data, ids, game.clone()) {
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
                    states,
                });
            }
        };
        missing.extend(miss);
        game = next;
        // Compare the engine's result with what the screen showed before
        // putting that state on the game. The zones are the checker's useful
        // output; the resync below only stops one difference cascading into a
        // stop.
        if let Some(spec) = seen_spec.as_ref() {
            let ours = ShopSnapshot::of_game(data, &game, ME);
            step_differences.extend(step_seen_differences(
                data, turn.turn, screen, spec, &ours, i,
            ));
        }
        // The screen's own seen state goes on only where it saw one.
        if let Some(seen) = &screen.seen {
            if seen.tavern.is_some()
                && let Some(spec) = seen_spec.as_ref()
            {
                shop_replay::place_tavern(data, &mut game, spec, &mut missing);
            }
            if seen.board.is_some()
                && let Some(spec) = seen_spec.as_ref()
            {
                let fake = ShopStepSpec {
                    after: Some(spec.clone()),
                    ..Default::default()
                };
                shop_replay::force_board(data, &mut game, &fake, &mut missing);
            }
            if seen.hand.is_some()
                && let Some(spec) = seen_spec.as_ref()
            {
                let fake = ShopStepSpec {
                    after: Some(spec.clone()),
                    ..Default::default()
                };
                shop_replay::force_hand(data, &mut game, &fake, &mut missing);
            }
            // The step's visible numbers are put on the carried game after
            // they have been compared, so one drift makes one line.
            if let Some(free) = seen.free_refreshes {
                game.players[ME].free_rolls = free;
            }
            if let Some(gold) = seen.gold {
                game.players[ME].gold = gold.clamp(0, u8::MAX as i32) as u8;
            }
            if let Some(tier) = seen.tier {
                game.players[ME].tier = tier.clamp(1, MAX_TIER);
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
        states,
    })
}

// ---- the comparison ----

fn card_text(m: &shop_replay::SnapMinion) -> String {
    let golden = if m.golden { " (golden)" } else { "" };
    format!("{}{golden} {}/{}", m.name, m.attack, m.health)
}

fn compare_ordered(
    label: &str,
    ours: &[shop_replay::SnapMinion],
    screen: &[shop_replay::SnapMinion],
) -> Vec<String> {
    let mut out = Vec::new();
    for i in 0..ours.len().max(screen.len()) {
        let a = ours.get(i);
        let b = screen.get(i);
        if a.map(card_text) == b.map(card_text) {
            continue;
        }
        let a = a.map(card_text).unwrap_or_else(|| "nothing".to_string());
        let b = b.map(card_text).unwrap_or_else(|| "nothing".to_string());
        out.push(format!("{label} {}: engine {a}, screen {b}", i + 1));
    }
    out
}

fn compare_multiset(
    label: &str,
    ours: &[shop_replay::SnapMinion],
    screen: &[shop_replay::SnapMinion],
) -> Vec<String> {
    let mut a: BTreeMap<String, usize> = BTreeMap::new();
    let mut b: BTreeMap<String, usize> = BTreeMap::new();
    for m in ours {
        *a.entry(card_text(m)).or_default() += 1;
    }
    for m in screen {
        *b.entry(card_text(m)).or_default() += 1;
    }
    let mut out = Vec::new();
    for (text, n) in &a {
        let m = b.get(text).copied().unwrap_or(0);
        if *n > m {
            out.push(format!(
                "{label}: engine {text} x{}, screen {}",
                n - m,
                if m == 0 {
                    "no such card".to_string()
                } else {
                    format!("x{m}")
                }
            ));
        }
    }
    for (text, n) in &b {
        let m = a.get(text).copied().unwrap_or(0);
        if *n > m {
            out.push(format!(
                "{label}: screen {text} x{}, engine {}",
                n - m,
                if m == 0 {
                    "no such card".to_string()
                } else {
                    format!("x{m}")
                }
            ));
        }
    }
    out
}

fn without_colon(line: String) -> String {
    line.replacen(": engine ", " engine ", 1)
        .replacen(": screen ", " screen ", 1)
}

/// What the engine did differently from a step's `seen` state, before that
/// state is put onto it. A random Tavern is skipped: its cards are the draw's,
/// not the engine's.
fn step_seen_differences(
    data: &Data,
    turn: u32,
    screen: &ScreenStepSpec,
    seen_spec: &ShopStateSpec,
    ours: &ShopSnapshot,
    step: usize,
) -> Vec<String> {
    let Some(seen) = screen.seen.as_ref() else {
        return Vec::new();
    };
    let mut log_missing = Vec::new();
    let theirs = ShopSnapshot::of_state(data, seen_spec, turn, &mut log_missing);
    let prefix = format!("after step {} ({}): ", step + 1, screen.step);
    let mut out = Vec::new();
    if seen.board.is_some() {
        for line in compare_ordered("board", &ours.board, &theirs.board) {
            out.push(format!("{prefix}{}", without_colon(line)));
        }
    }
    if seen.hand.is_some() {
        for line in compare_multiset("hand", &ours.hand, &theirs.hand) {
            out.push(format!("{prefix}{}", without_colon(line)));
        }
    }
    if seen.tavern.is_some() && !screen.tavern_random && screen.step != "roll" {
        for line in compare_multiset("Tavern", &ours.tavern, &theirs.tavern) {
            out.push(format!("{prefix}{}", without_colon(line)));
        }
    }
    if let Some(gold) = seen.gold
        && ours.gold != gold
    {
        out.push(format!("{prefix}engine Gold {}, screen {gold}", ours.gold));
    }
    if let Some(free) = seen.free_refreshes
        && ours.free_refreshes != free
    {
        out.push(format!(
            "{prefix}engine free Refreshes {}, screen {free}",
            ours.free_refreshes
        ));
    }
    if let Some(tier) = seen.tier
        && ours.tier != tier
    {
        out.push(format!("{prefix}engine tier {}, screen {tier}", ours.tier));
    }
    out
}

/// Compare only the fields the turn's `end` has. The board keeps its order;
/// the hand and Tavern are compared as multisets, as the shop replay does.
pub fn compare_end(
    data: &Data,
    ids: &CardIdMap,
    game: &Game,
    end: &ScreenStateSpec,
    turn: u32,
) -> (Vec<String>, Vec<String>) {
    let ours = ShopSnapshot::of_game(data, game, ME);
    let mut missing = Vec::new();
    let mut next_id = 1u64;
    let spec = state_to_spec(data, ids, end, turn, &mut next_id, &mut missing);
    let screen = ShopSnapshot::of_state(data, &spec, turn, &mut missing);
    let mut diffs = Vec::new();
    if end.board.is_some() {
        diffs.extend(compare_ordered("board", &ours.board, &screen.board));
    }
    if end.hand.is_some() {
        diffs.extend(compare_multiset("hand", &ours.hand, &screen.hand));
    } else if let Some(count) = end.hand_count
        && ours.hand.len() != count as usize
    {
        diffs.push(format!(
            "hand count: engine {}, screen {count}",
            ours.hand.len()
        ));
    }
    if end.tavern.is_some() {
        diffs.extend(compare_multiset("Tavern", &ours.tavern, &screen.tavern));
    }
    if let Some(gold) = end.gold
        && ours.gold != gold
    {
        diffs.push(format!("Gold: engine {}, screen {gold}", ours.gold));
    }
    if let Some(tier) = end.tier
        && ours.tier != tier
    {
        diffs.push(format!("tier: engine {}, screen {tier}", ours.tier));
    }
    if let Some(health) = end.health
        && ours.health != health
    {
        diffs.push(format!("Health: engine {}, screen {health}", ours.health));
    }
    (diffs, missing)
}

// ---- verdicts over several seeds ----

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScreenVerdict {
    Ok,
    OkRandom,
    Flagged,
    Stopped,
}

impl ScreenVerdict {
    pub fn as_str(self) -> &'static str {
        match self {
            ScreenVerdict::Ok => "ok",
            ScreenVerdict::OkRandom => "ok-random",
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
    pub differences: Vec<String>,
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
    let mut results = Vec::new();
    let mut state_sets: Vec<Option<Vec<ShopSnapshot>>> = Vec::new();
    let mut plays = Vec::new();
    for k in 0..n {
        let seed = k + 1;
        match replay_screen_turn(data, ids, file, turn, seed) {
            Ok(run) => {
                let mut differences = run.step_differences.clone();
                let compare_missing = if run.stop.is_none() {
                    match &turn.end {
                        Some(end) => {
                            let (d, m) = compare_end(data, ids, &run.game, end, turn.turn);
                            differences.extend(d);
                            m
                        }
                        None => Vec::new(),
                    }
                } else {
                    Vec::new()
                };
                let mut missing = run.missing.clone();
                missing.extend(compare_missing);
                let engine = Some(ScreenEndState::of(data, &run.game));
                plays.push((run.played, run.stop.clone()));
                state_sets.push(Some(run.states));
                results.push(ScreenSeedResult {
                    seed,
                    played: run.played,
                    stop: run.stop,
                    differences,
                    missing,
                    engine,
                });
            }
            Err(stop) => {
                plays.push((0, Some(stop.clone())));
                state_sets.push(None);
                results.push(ScreenSeedResult {
                    seed,
                    played: 0,
                    stop: Some(stop),
                    differences: Vec::new(),
                    missing: Vec::new(),
                    engine: None,
                });
            }
        }
    }
    let completed = results.iter().filter(|r| r.stop.is_none()).count();
    let matches = results
        .iter()
        .filter(|r| r.stop.is_none() && r.differences.is_empty())
        .count();
    let stop = results
        .first()
        .and_then(|r| r.stop.clone())
        .or_else(|| results.iter().find_map(|r| r.stop.clone()));
    let verdict = if completed == 0 {
        ScreenVerdict::Stopped
    } else if matches == results.len() {
        ScreenVerdict::Ok
    } else if matches > 0 {
        ScreenVerdict::OkRandom
    } else {
        ScreenVerdict::Flagged
    };
    let differences = results
        .iter()
        .find(|r| r.stop.is_none() && !r.differences.is_empty())
        .map(|r| r.differences.clone())
        .unwrap_or_default();
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
    ScreenTurnOutcome {
        file: file.file(),
        game: file.game,
        turn: turn.turn,
        steps: turn.steps.len(),
        verdict,
        stop,
        differences,
        randomness_at,
        seeds: results,
    }
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

fn card_of_spec(data: &Data, ids: &CardIdMap, m: &ShopMinionSpec) -> Option<ScreenCard> {
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
        attack: Some(m.attack),
        health: Some(m.health),
        golden: Some(m.golden),
        keywords: (!m.keywords.is_empty()).then(|| m.keywords.clone()),
        slot: m.slot,
        kind: m.kind.clone(),
        frozen: Some(m.frozen),
        cost,
        costs_health: m.costs_health.then_some(true),
    })
}

fn visible_cost(data: &Data, m: &ShopMinionSpec) -> Option<i32> {
    m.cost_override
        .or(m.cost)
        .or_else(|| m.cost_reduce.map(|r| data.rules.buy_cost as i32 - r))
}

fn state_of_spec(data: &Data, ids: &CardIdMap, state: &ShopStateSpec) -> ScreenStateSpec {
    let hand: Vec<ScreenCard> = state
        .hand
        .iter()
        .filter_map(|m| card_of_spec(data, ids, m))
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
            waiting: p.waiting.as_ref().and_then(|w| card_of_spec(data, ids, w)),
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
                .filter_map(|m| card_of_spec(data, ids, m))
                .collect(),
        ),
        hand: Some(hand),
        tavern: Some(
            state
                .tavern
                .iter()
                .filter_map(|m| card_of_spec(data, ids, m))
                .collect(),
        ),
        gold: Some(state.gold),
        gold_max: Some(state.gold_max),
        free_refreshes: Some(state.free_refreshes),
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
) -> ScreenStepSpec {
    let mut seen = ScreenStateSpec::default();
    let mut has_seen = false;
    // The Gold digits and the Refresh button's price are always on screen, so
    // every step carries them after it.
    if let Some(after) = after {
        seen.gold = Some(after.gold);
        seen.free_refreshes = Some(after.free_refreshes);
        seen.board = Some(
            after
                .board
                .iter()
                .filter_map(|m| card_of_spec(data, ids, m))
                .collect(),
        );
        seen.hand = Some(
            after
                .hand
                .iter()
                .filter_map(|m| card_of_spec(data, ids, m))
                .collect(),
        );
        seen.tavern = Some(
            after
                .tavern
                .iter()
                .filter_map(|m| card_of_spec(data, ids, m))
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
                waiting: p.waiting.as_ref().and_then(|w| card_of_spec(data, ids, w)),
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
                attack: Some(c.attack),
                health: Some(c.health),
                golden: Some(c.golden),
                keywords: c.keywords.clone(),
                slot: c.slot,
                kind: c.kind.clone(),
                frozen: None,
                cost: c.cost,
                costs_health: None,
            };
            // A buy's price is on the card the screen showed. The action file
            // writes it for a Tavern spell; for a discounted minion the step's
            // own paid Gold is the next best evidence.
            if step.step == "buy" && card.cost.is_none() {
                let from_tavern = before.and_then(|before| {
                    before.tavern.iter().find(|m| {
                        (c.card_id.is_some() && m.card_id == c.card_id)
                            || (c.name.is_some() && m.name == c.name)
                            || (c.slot.is_some() && m.slot == c.slot)
                    })
                });
                card.cost = step
                    .paid
                    .filter(|p| p.health <= 0 && p.gold >= 0)
                    .map(|p| p.gold)
                    .or_else(|| from_tavern.and_then(|m| visible_cost(data, m)));
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
                attack: Some(o.attack),
                health: Some(o.health),
                golden: Some(o.golden),
                keywords: o.keywords.clone(),
                slot: o.slot,
                kind: o.kind.clone(),
                frozen: None,
                cost: o.cost,
                costs_health: None,
            })
            .collect(),
        picked: step.picked.as_ref().map(|p| {
            ScreenPick::Card(ScreenCard {
                card_id: p.card_id.clone(),
                name: p.name.clone(),
                attack: Some(p.attack),
                health: Some(p.health),
                golden: Some(p.golden),
                keywords: p.keywords.clone(),
                slot: p.slot,
                kind: p.kind.clone(),
                frozen: None,
                cost: p.cost,
                costs_health: None,
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
                    let out = step_of_spec(data, ids, step, before, step.after.as_ref().or(before));
                    before = step.after.as_ref().or(before);
                    out
                })
                .collect();
            ScreenTurnSpec {
                turn: turn.turn,
                start: state_of_spec(data, ids, &turn.start),
                steps,
                end: turn.end.as_ref().map(|e| state_of_spec(data, ids, e)),
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
