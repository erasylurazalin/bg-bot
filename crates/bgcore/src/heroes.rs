//! The hero table: heroes, their armor and their powers, parsed from
//! heroes.toml. Effects are data here as they are in cards.toml; the rules
//! code never matches a hero or power by name. See DESIGN.md "Heroes".

use std::collections::HashMap;

use serde::Deserialize;

use crate::cards::{Tribe, Tribes};
use crate::minion::Keywords;

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
    /// The whole power works once per game ("Once per game").
    pub once_per_game: bool,
    /// Unlocks at this Tavern tier, 0 for no lock.
    pub unlock_tier: u8,
    /// Unlocks at this turn, 0 for no lock.
    pub unlock_turn: u32,
    pub effect: PowerEffect,
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
    pub power: HeroPower,
    /// Heroes whose powers this hero's power Discover never offers, by hero
    /// card_id (Sir Finley's own list: himself).
    pub power_excludes: Vec<String>,
}

/// Every built hero. Player.hero and PowerState.of index into `heroes`.
#[derive(Clone, Debug, Default)]
pub struct HeroDb {
    pub heroes: Vec<Hero>,
    by_card_id: HashMap<String, u8>,
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

    pub fn get(&self, i: u8) -> &Hero {
        &self.heroes[i as usize]
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
    unlock_tier: u8,
    #[serde(default)]
    unlock_turn: u32,
    #[serde(default)]
    effect: RawPowerEffect,
}

fn one() -> u8 {
    1
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
}

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
    } else {
        PowerEffect::None
    })
}

fn tribe(name: &str) -> Result<Tribe, String> {
    crate::cards::tribe(name.trim().to_lowercase().as_str())
}

impl HeroDb {
    pub fn from_toml(s: &str) -> Result<HeroDb, String> {
        #[derive(Deserialize)]
        struct RawFile {
            #[serde(default)]
            hero: Vec<RawHero>,
        }
        let raw: RawFile = toml::from_str(s).map_err(|e| format!("heroes.toml: {e}"))?;
        let mut db = HeroDb { heroes: Vec::new(), by_card_id: HashMap::new() };
        for h in raw.hero {
            let power = HeroPower {
                name: h.power.name,
                card_id: h.power.card_id,
                cost: h.power.cost,
                passive: h.power.passive,
                uses_per_turn: h.power.uses_per_turn,
                once_per_game: h.power.once_per_game,
                unlock_tier: h.power.unlock_tier,
                unlock_turn: h.power.unlock_turn,
                effect: parse_effect(&h.power.effect).map_err(|e| format!("{}: {e}", h.name))?,
            };
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
                power,
                power_excludes: h.power_excludes,
            };
            if db.by_card_id.insert(h.card_id, db.heroes.len() as u8).is_some() {
                return Err(format!("heroes.toml: hero {} twice", hero.name));
            }
            db.heroes.push(hero);
        }
        Ok(db)
    }
}
