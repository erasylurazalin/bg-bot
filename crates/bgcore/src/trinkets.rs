//! Trinkets: what the lesser and greater slots do, parsed from
//! trinkets.toml. Effects are data here as they are in cards.toml, and the
//! rules code never matches a trinket by name. See DESIGN.md "Trinkets and
//! Dark Gifts (2026-10-30)" and docs/ENGINE.md "Trinkets".
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
    /// A Tavern spell cast, wherever the cast comes from.
    CastSpell,
    /// A Tavern spell cast on a friendly minion.
    CastSpellOnMinion,
    /// Gold spent, counted by the amount (a 3 Gold buy adds 3).
    GoldSpent,
    /// A card discarded.
    Discard,
}

/// The player holds a trinket with an effect at all (any hook or rule).
pub const HAS_TRINKET: u16 = 1 << 15;
/// A trinket wants the start of the turn (a `start_of_turn` list or a
/// "every N turns" rule, which counts there).
pub const STARTS_TURN: u16 = 1 << 14;
/// A trinket wants the end of the turn (an `end_of_turn` list or a rule).
pub const ENDS_TURN: u16 = 1 << 13;

impl Event {
    /// The bit a player's trinket mask uses for this event.
    pub fn bit(self) -> u16 {
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
            Event::CastSpellOnMinion => 1 << 7,
            Event::GoldSpent => 1 << 8,
            Event::Discard => 1 << 9,
        }
    }
}

/// One thing a trinket does. The doc names the card a key was written for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Act {
    /// Get `count` generated copies of a card (`get`).
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

/// "Start of Combat: give your minions of a tribe these keywords". The fight
/// has no trinket entity, so this leaves through `Counters`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Soc {
    pub tribe: Option<Tribe>,
    pub keywords: u8,
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
    pub on_cast_spell: Vec<Act>,
    pub on_cast_spell_on_minion: Vec<Act>,
    pub on_discard: Vec<Act>,
    /// Only discard events of a spell (Shath'Yar Shrine).
    pub discard_spells_only: bool,
    pub rule: Option<Rule>,
    pub soc: Option<Soc>,
}

impl Trinket {
    /// The events this trinket listens for, as a bit mask: the player ORs
    /// its trinkets' masks together, so a hook can answer "nothing here"
    /// from one `u16` on the player and never read this table (shared by
    /// every game on every thread) until something matches.
    pub fn event_mask(&self) -> u16 {
        let mut m = 0;
        let mut any = false;
        let list = |on: bool, e: Event, m: &mut u16, any: &mut bool| {
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
        list(!self.on_cast_spell_on_minion.is_empty(), Event::CastSpellOnMinion, &mut m, &mut any);
        list(!self.on_discard.is_empty(), Event::Discard, &mut m, &mut any);
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
        if self.soc.is_some() { any = true; }
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
            Event::CastSpellOnMinion => !self.on_cast_spell_on_minion.is_empty(),
            Event::Discard => !self.on_discard.is_empty(),
            Event::GoldSpent => false,
            Event::Turns => !self.end_of_turn.is_empty() || self.rule.is_some(),
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
struct RawSoc {
    #[serde(default)]
    tribe: Option<String>,
    keywords: Vec<String>,
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
    on_cast_spell_on_minion: Vec<RawAct>,
    #[serde(default)]
    on_discard: Vec<RawAct>,
    #[serde(default)]
    discard_spells_only: bool,
    #[serde(default)]
    rule: Option<RawRule>,
    #[serde(default)]
    soc: Option<RawSoc>,
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
                } else {
                    return Err(err(
                        "an act table with no known key (see trinkets.rs `Act`)".into(),
                    ));
                })
            };
            let acts = |v: &[RawAct]| -> Result<Vec<Act>, String> {
                v.iter().map(act).collect()
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
                        "cast_spell_on_minion" => Event::CastSpellOnMinion,
                        "gold_spent" => Event::GoldSpent,
                        "discard" => Event::Discard,
                        other => match other.strip_prefix("play_") {
                            Some(t) => Event::PlayTribe(tribe(t).map_err(err)?),
                            None => return Err(err(format!("unknown event {other:?}"))),
                        },
                    };
                    if r.every == 0 {
                        return Err(err("a rule needs every >= 1".into()));
                    }
                    Some(Rule { event, every: r.every, once: r.once, acts: acts(&r.acts)? })
                }
            };
            let soc = match &t.soc {
                None => None,
                Some(s) => Some(Soc {
                    tribe: s.tribe.as_deref().map(tribe).transpose().map_err(err)?,
                    keywords: keywords(&s.keywords).map_err(err)?,
                }),
            };
            let trinket = Trinket {
                name: t.name,
                card_id: t.card_id.clone(),
                cost: t.cost,
                slot,
                tribes,
                mechanic,
                uses_per_turn: t.uses_per_turn,
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
                on_cast_spell_on_minion: acts(&t.on_cast_spell_on_minion)?,
                on_discard: acts(&t.on_discard)?,
                discard_spells_only: t.discard_spells_only,
                rule,
                soc,
            };
            if db.by_card_id.insert(t.card_id, db.trinkets.len() as u8).is_some() {
                return Err(format!("trinkets.toml: {} twice", trinket.name));
            }
            db.trinkets.push(trinket);
        }
        Ok(db)
    }
}
