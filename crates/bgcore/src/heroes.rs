//! The hero table: heroes, their armor and their powers, parsed from
//! heroes.toml. Effects are data here as they are in cards.toml; the rules
//! code never matches a hero or power by name. See DESIGN.md "Heroes".
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
    /// it were turn 7 (docs/darkgifts.md).
    DarkGiftStart,
    /// Every `turns` turns, Discover a minion with a Dark Gift (Feel
    /// Devastation). The count lives in the power's `progress`.
    DarkGiftEvery { turns: u8 },
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
    /// The power this one goes back to when it completes, index into
    /// `HeroDb::powers` (Bru'kan's Invocations and Galewing's flightpaths go
    /// back to their chooser). None for a power that stays.
    pub parent: Option<u8>,
    /// Turns this power takes to complete once it is chosen ("In 2 turns,
    /// gain 2 Gold"): 0 means it completes at the end of the next fight.
    pub complete_turns: u8,
    /// Powers that rotate into each other (the King of X family), grouped
    /// under one number; 0 for a power outside a family.
    pub family: u8,
    /// The first turns of the game this power leaves only Freeze and End Turn
    /// (Procrastinate skips two, Expedition Plans one).
    pub skip_turns: u8,
    /// The powers a press may turn this one into (Embrace the Elements,
    /// Dungar's Gryphon), indices into `HeroDb::powers`.
    pub options: Vec<u8>,
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
    unlock_tier: u8,
    #[serde(default)]
    unlock_turn: u32,
    #[serde(default)]
    family: Option<String>,
    #[serde(default)]
    skip_turns: u8,
    #[serde(default)]
    choose: Vec<String>,
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
        PowerEffect::SocGrantDeathrattle { count, .. } => {
            PowerEffect::SocGrantDeathrattle { count, card: card(&e.card, "card")? }
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
                once_per_game: h.power.once_per_game,
                unlock_tier: h.power.unlock_tier,
                unlock_turn: h.power.unlock_turn,
                effect,
                parent: None,
                complete_turns: 0,
                family: 0,
                skip_turns: h.power.skip_turns,
                options: Vec::new(),
            };
            pending.push(Pending {
                power,
                raw: h.power.effect,
                family: h.power.family,
                returns_to: None,
                choose: h.power.choose,
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
                once_per_game: h.once_per_game,
                unlock_tier: h.unlock_tier,
                unlock_turn: h.unlock_turn,
                effect,
                parent: None,
                complete_turns: h.complete_turns,
                family: 0,
                skip_turns: h.skip_turns,
                options: Vec::new(),
            };
            pending.push(Pending {
                power,
                raw: h.effect,
                family: h.family,
                returns_to: h.returns_to,
                choose: h.choose,
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
