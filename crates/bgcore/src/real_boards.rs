//! Real boards from Era's games (`boards/real/*.json`, docs/real-boards.md),
//! played by real seats instead of shopping (see `Game::new_random_heroes_real`).
//!
//! The JSON texts are parsed here (`parse_board_files`); the callers read the
//! files: train/real_boards.py for training, `bgbench --real-boards` for the
//! fuzzer and the benchmarks.

use std::collections::HashMap;

use arrayvec::ArrayVec;
use serde::Deserialize;

use crate::cards::{CardDb, Counter, N_COUNTERS};
use crate::fight::Deity;
use crate::minion::{Keywords, Minion};
use crate::rng::Rng;
use crate::{BOARD, HAND};

/// Turn from which every later turn draws from one shared pool of boards:
/// games rarely last that long, so those turns stay thin (Era's rule).
pub const LATE_TURN: u32 = 13;

/// One minion of a real board as its loader reads it: the card's name and
/// the attack, health, golden flag and keywords the file shows. A Dark
/// Gift's stats are already inside attack and health; its effect is not
/// built, so it is lost.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RealMinionSpec {
    pub name: String,
    pub attack: i32,
    pub health: i32,
    pub golden: bool,
    /// Keyword names as the file writes them (case-insensitive here;
    /// POISONOUS counts as Venomous, anything unknown is dropped).
    pub keywords: Vec<String>,
    /// The card's current numbers, the {0} {1} {2} {3} of its text
    /// (TAG_SCRIPT_DATA_NUM_1..4 in the log): Lurking Leviathan's growth,
    /// Spark Snapper's Satellite size, Tasty Lobster's buff. Empty when the
    /// log showed none.
    pub numbers: Vec<i32>,
    /// The Dark Gifts on it, by the gift's own card_id
    /// (`BG36_MidGameEffect_000t` and its kin, the file's `dark_gifts`).
    /// Their stats are already in attack and health; the effect is attached
    /// here, with the counter the log does not show set to the board's turn.
    pub dark_gifts: Vec<String>,
}

/// A real board's Deity: the minion it awakens and the stats it has now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RealDeitySpec {
    pub name: String,
    pub attack: i32,
    pub health: i32,
    pub golden: bool,
}

/// The player-level numbers a side had at the first attack, in the shape
/// `Player` (and through it `fight::Counters`) reads them. A number the log
/// did not show is the base value: 0 spells cast, no spell bonus, Blood Gems
/// +1/+1 (Era's rule, 2026-10-26). The Beast bonus, the Boon of Beetles and
/// the other hero-power numbers are not carried: no real seat has a hero.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RealCounters {
    pub gem_attack: i32,
    pub gem_health: i32,
    pub beetle_attack: i32,
    pub beetle_health: i32,
    pub undead_attack: i32,
    pub undead_health: i32,
    pub spell_attack: i32,
    pub spell_health: i32,
    pub spells_cast: u32,
    pub knight_deaths: u32,
    /// The "improve your future ..." counts (`Counter::index`), which the
    /// fight scales their next buff by.
    pub improve: [u16; N_COUNTERS],
}

impl Default for RealCounters {
    fn default() -> RealCounters {
        RealCounters {
            gem_attack: 1,
            gem_health: 1,
            beetle_attack: 0,
            beetle_health: 0,
            undead_attack: 0,
            undead_health: 0,
            spell_attack: 0,
            spell_health: 0,
            spells_cast: 0,
            knight_deaths: 0,
            improve: [0; N_COUNTERS],
        }
    }
}

impl RealCounters {
    /// Take one of the numbers a file holds, by its key: `gem_attack`,
    /// `gem_health`, `beetle_attack`, `beetle_health`, `undead_attack`,
    /// `undead_health`, `spell_attack`, `spell_health`, `spells_cast`,
    /// `knight_deaths`, `tasty_lobster`, `baller`. A key that is not in the
    /// file leaves its field at the base value; a key no engine number uses
    /// is ignored.
    pub fn set(&mut self, key: &str, value: i32) {
        let count = |v: i32| v.max(0) as u32;
        let step = |v: i32| v.max(0).min(u16::MAX as i32) as u16;
        match key {
            "gem_attack" => self.gem_attack = value,
            "gem_health" => self.gem_health = value,
            "beetle_attack" => self.beetle_attack = value,
            "beetle_health" => self.beetle_health = value,
            "undead_attack" => self.undead_attack = value,
            "undead_health" => self.undead_health = value,
            "spell_attack" => self.spell_attack = value,
            "spell_health" => self.spell_health = value,
            "spells_cast" => self.spells_cast = count(value),
            "knight_deaths" => self.knight_deaths = count(value),
            "tasty_lobster" => self.improve[Counter::TastyLobster.index()] = step(value),
            "baller" => self.improve[Counter::Baller.index()] = step(value),
            _ => {}
        }
    }
}

/// One real board: the turn it fought at, the player's Tavern tier then, its
/// minions, the minions in that player's hand, and the side-wide numbers.
/// Which player it belonged to (`who` in the file) is already gone: every
/// board of a turn is an equally likely draw. `Default` is a board with
/// nothing on it and every number at its base value.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RealBoardSpec {
    pub turn: u32,
    pub tier: u8,
    pub minions: Vec<RealMinionSpec>,
    pub hand: Vec<RealMinionSpec>,
    pub counters: RealCounters,
    pub deity: Option<RealDeitySpec>,
}

/// A real board ready to be copied onto a seat: fresh minions (no pool
/// copies) with the file's stats, golden flag, keywords and scaling state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RealBoard {
    pub tier: u8,
    pub minions: ArrayVec<Minion, BOARD>,
    pub hand: ArrayVec<Minion, HAND>,
    pub counters: RealCounters,
    pub deity: Option<Deity>,
}

/// Real boards grouped by turn for the seat draws. Turn `LATE_TURN` and every
/// later turn share one bucket; a turn whose bucket is empty draws from the
/// nearest earlier turn that has boards (Era's rules).
#[derive(Clone, Debug, Default)]
pub struct RealBoards {
    /// Buckets for turns 1..=LATE_TURN, index 0 being turn 1. The last bucket
    /// holds every board of `LATE_TURN` or later.
    by_turn: Vec<Vec<RealBoard>>,
}

/// The token a real board's unknown minion becomes.
pub const STAND_IN: &str = "Stand-in";

/// A card's `improvements` (`Minion.improvements`), from its current numbers:
/// the cards whose scaling the engine models as a count, and which of their
/// numbers holds it. Every other card keeps 0 whatever numbers it carries (a
/// Scarlet Survivor's {0} is its Attack threshold, a Forest Rover's are its
/// own stats): only the cards listed here are read.
fn improvements(name: &str, golden: bool, numbers: &[i32]) -> u16 {
    let mul = if golden { 2 } else { 1 };
    let n = |i: usize| numbers.get(i).copied().unwrap_or(0);
    let steps = match name {
        // "Whenever you summon a Beast, give it +3 Attack and improve this
        // permanently": {0} counts the growth (+3 a play, doubled golden) and
        // the fight's summon buff is 3 * (1 + the count) * golden.
        "Lurking Leviathan" => n(0) / (3 * mul),
        // "Whenever you play a Mech, Magnetize a 2/3 Satellite to it and
        // improve this": {0} is the Satellite's Attack growth (+2 a play,
        // doubled golden) and the Satellite is 2/3 * (1 + count) * golden.
        "Spark Snapper" => n(0) / (2 * mul),
        // "Start of Combat: Give your other Elementals +3/+3. (Improves
        // after every ...)": {2} is the count (only seen at 0 so far).
        "Ultraviolet Ascendant" => n(2),
        // "Discover a Tier {0} minion. (Improves each turn!)": {0} is the
        // tier now, which is 1 + the count.
        "Patient Scout" => n(0) - 1,
        _ => 0,
    };
    steps.clamp(0, u16::MAX as i32) as u16
}

/// Tasty Lobster's "improve your future Tasty Lobsters": the file's numbers
/// are the pair its Deathrattle gives now, printed 2/1 (doubled for a golden
/// one), and the fight scales it by 1 + the count.
const LOBSTER: &str = "Tasty Lobster";
const LOBSTER_PRINTED: i32 = 2;

/// A Tasty Lobster's "improve your future Tasty Lobsters" count, from the
/// pair its Deathrattle gives now ([attack, health]).
fn lobster_count(numbers: &[i32], golden: bool) -> u16 {
    let attack = numbers.first().copied().unwrap_or(0);
    let per_play = LOBSTER_PRINTED * if golden { 2 } else { 1 };
    ((attack / per_play - 1).max(0)).min(u16::MAX as i32) as u16
}

impl RealBoards {
    /// Resolve the specs against the card set: each minion becomes a fresh
    /// `Minion` (pool_copies 0, so it takes no pool copy and nothing goes
    /// back when it dies or its seat is knocked out) with the file's attack,
    /// health, golden flag and keywords, and the scaling state its numbers
    /// stand for (`Minion.improvements`, the Tasty Lobster count). A minion
    /// whose name matches no card becomes a Stand-in with those stats and
    /// keywords and no effect (Era, 2026-10-25), and its name is returned.
    pub fn build(
        specs: &[RealBoardSpec],
        db: &CardDb,
        gifts: &crate::darkgifts::GiftDb,
    ) -> (RealBoards, Vec<String>) {
        let mut by_turn = vec![Vec::new(); LATE_TURN as usize];
        let mut unmatched = Vec::new();
        let make = |m: &RealMinionSpec, turn: u32, unmatched: &mut Vec<String>| {
            let id = match db.by_name(&m.name) {
                Some(id) => id,
                None => {
                    unmatched.push(m.name.clone());
                    db.by_name(STAND_IN).expect("cards.toml has the Stand-in token")
                }
            };
            let mut minion = db.make(id, m.golden);
            minion.attack = m.attack;
            minion.health = m.health;
            minion.improvements = improvements(&m.name, m.golden, &m.numbers);
            let mut keywords = Keywords(0);
            for kw in &m.keywords {
                set_keyword(&mut keywords, kw);
            }
            minion.keywords = keywords;
            // A Dark Gift the board carried: the landing's stats are already
            // in the file's attack and health, so only the gift itself comes
            // along. The counter the log does not carry is set as if the gift
            // had landed on this board's turn.
            for gift_id in &m.dark_gifts {
                let Some(i) = gifts.gifts.iter().position(|g| &g.id == gift_id) else {
                    continue;
                };
                let card = gifts.gifts[i].card;
                let c = db.get(card);
                let state = match c.countdown {
                    Some(cd) => cd.turns,
                    None => turn.min(crate::minion::GIFT_STATE_MAX as u32) as u8,
                };
                minion.push_gift(crate::minion::GiftOn::new(card, state));
            }
            minion
        };
        for spec in specs {
            let mut minions = ArrayVec::new();
            let mut lobster = 0;
            for m in &spec.minions {
                if minions.is_full() {
                    // A real board never holds more than BOARD minions; a
                    // file that somehow does loses the tail.
                    continue;
                }
                if m.name == LOBSTER {
                    lobster = lobster.max(lobster_count(&m.numbers, m.golden));
                }
                minions.push(make(m, spec.turn, &mut unmatched));
            }
            let mut hand = ArrayVec::new();
            for m in &spec.hand {
                if hand.is_full() {
                    continue;
                }
                if m.name == LOBSTER {
                    lobster = lobster.max(lobster_count(&m.numbers, m.golden));
                }
                hand.push(make(m, spec.turn, &mut unmatched));
            }
            let mut counters = spec.counters;
            // Every Lobster shares one count: the log's numbers on any of
            // them, the best of them kept (a Lobster sold is invisible).
            counters.improve[Counter::TastyLobster.index()] =
                counters.improve[Counter::TastyLobster.index()].max(lobster);
            let deity = spec.deity.as_ref().and_then(|d| {
                let card = db.by_name(&d.name)?;
                Some(Deity { card, attack: d.attack, health: d.health, golden: d.golden })
            });
            // Turn 13 and later all draw from one bucket.
            let t = spec.turn.clamp(1, LATE_TURN) as usize;
            // A board whose log gave no Tavern tier (0) takes the highest
            // tier among its minions, at least 1 (2026-10-31: three such
            // boards made a real seat's shop checks index tier 0).
            let tier = if spec.tier > 0 {
                spec.tier.min(crate::MAX_TIER)
            } else {
                minions.iter().map(|m| db.get(m.card).tier).max().unwrap_or(1).clamp(1, crate::MAX_TIER)
            };
            by_turn[t - 1].push(RealBoard { tier, minions, hand, counters, deity });
        }
        (RealBoards { by_turn }, unmatched)
    }

    fn bucket(&self, turn: u32) -> &[RealBoard] {
        let n = self.by_turn.len();
        // Start at this turn's bucket (the shared late one from LATE_TURN on)
        // and fall back to the nearest earlier turn that has boards; a turn
        // with nothing earlier at all takes the nearest later one.
        let mut t = (turn.clamp(1, LATE_TURN) as usize).min(n);
        while t > 0 && self.by_turn[t - 1].is_empty() {
            t -= 1;
        }
        if t == 0 {
            t = 1 + self.by_turn.iter().position(|b| !b.is_empty()).unwrap_or(0);
        }
        &self.by_turn[t - 1]
    }

    /// A fresh random board for a real seat on `turn`, from every board of
    /// that turn (Era's and opponents' alike, equally likely).
    pub fn draw(&self, turn: u32, rng: &mut Rng) -> &RealBoard {
        let b = self.bucket(turn);
        &b[rng.below(b.len() as u32) as usize]
    }

    /// The boards a real seat draws from on `turn` (after the late-turn and
    /// fallback rules); empty only for an empty set.
    pub fn boards_at(&self, turn: u32) -> &[RealBoard] {
        self.bucket(turn)
    }

    /// How many boards each turn bucket 1..=LATE_TURN holds as itself (so
    /// before the late-turn pool and the fallback rules), for load summaries.
    pub fn counts_by_turn(&self) -> Vec<usize> {
        self.by_turn.iter().map(|b| b.len()).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.by_turn.iter().all(|b| b.is_empty())
    }

    pub fn len(&self) -> usize {
        self.by_turn.iter().map(|b| b.len()).sum()
    }
}

/// Set the keyword `name` stands for, ignoring anything the engine has no
/// keyword for (the tracker also writes CLEAVE, which is a card property
/// here, not a minion keyword).
fn set_keyword(keywords: &mut Keywords, name: &str) {
    let upper = name.to_ascii_uppercase();
    if upper == "POISONOUS" {
        // The engine has one Venomous keyword; Poisonous only loses its
        // permanence.
        keywords.set(Keywords::VENOMOUS);
        return;
    }
    if let Some(&(bit, _)) = Keywords::ALL.iter().find(|&&(bit, k)| {
        bit != Keywords::MAGNETIC && k.eq_ignore_ascii_case(name)
    }) {
        keywords.set(bit);
    }
}

// ---- reading the board files (the format is docs/real-boards.md) ----

#[derive(Deserialize)]
struct RawFile {
    boards: Vec<RawBoard>,
}

#[derive(Deserialize)]
struct RawBoard {
    turn: u32,
    tier: u8,
    minions: Vec<RawMinion>,
    /// The minions in the player's hand at the first attack (the opponent's
    /// too). A file without it has no hand.
    #[serde(default)]
    hand: Vec<RawMinion>,
    /// The side's player-level numbers, by key (see `RealCounters::set`). A
    /// number the log did not show is not there and keeps its base value, so
    /// an older file without the object loads fine.
    #[serde(default)]
    counters: HashMap<String, i32>,
    /// The side's Deity, when it had one.
    #[serde(default)]
    deity: Option<RawDeity>,
}

#[derive(Deserialize)]
struct RawDeity {
    card_id: String,
    #[serde(default)]
    name: String,
    attack: i32,
    health: i32,
}

#[derive(Deserialize)]
struct RawMinion {
    card_id: String,
    /// The Dark Gifts on it, as gift card_ids.
    #[serde(default)]
    dark_gifts: Vec<String>,
    #[serde(default)]
    name: String,
    attack: i32,
    health: i32,
    #[serde(default)]
    golden: bool,
    #[serde(default)]
    keywords: Vec<String>,
    /// The card's current numbers, TAG_SCRIPT_DATA_NUM_1..4 as the log shows
    /// them, or nothing when the card has none.
    #[serde(default)]
    numbers: Vec<i32>,
}

#[derive(Deserialize)]
struct RawRefFile {
    cards: Vec<RawRefCard>,
}

#[derive(Deserialize)]
struct RawRefCard {
    card_id: String,
    name: String,
    #[serde(default)]
    is_golden: bool,
    #[serde(default)]
    normal_card_id: Option<String>,
}

/// card id to card name, from `ref/bg-cards.json` (docs/card-data.md): the
/// one mapping both loaders share. A golden id (the game's `_G` and
/// `TB_BaconUps_*` forms) maps to the base card's name.
#[derive(Debug, Default)]
pub struct CardIdMap {
    by_id: HashMap<String, (String, bool)>,
}

impl CardIdMap {
    pub fn parse(json: &str) -> Result<CardIdMap, String> {
        let raw: RawRefFile =
            serde_json::from_str(json).map_err(|e| format!("ref card file: {e}"))?;
        let mut names: HashMap<String, String> = HashMap::new();
        for c in &raw.cards {
            if !c.is_golden {
                names.insert(c.card_id.clone(), c.name.clone());
            }
        }
        let mut by_id = HashMap::new();
        for c in raw.cards {
            let name = if c.is_golden {
                // The golden card's name is the base's; where the export
                // points at the base, take that name for sure.
                match c.normal_card_id.as_deref().and_then(|b| names.get(b)) {
                    Some(base) => base.clone(),
                    None => c.name,
                }
            } else {
                c.name
            };
            by_id.insert(c.card_id, (name, c.is_golden));
        }
        Ok(CardIdMap { by_id })
    }

    /// The card a file minion's id stands for: its name, and whether the id
    /// itself is a golden card.
    pub fn get(&self, card_id: &str) -> Option<(&str, bool)> {
        self.by_id.get(card_id).map(|(name, golden)| (name.as_str(), *golden))
    }
}

/// Parse board files into specs for `RealBoards::build`. `files` are the
/// (name, text) pairs of one JSON file per game; `ids` is the card id map.
/// Returns the specs and the minions whose card id no source knows, by the
/// name their file shows (the id itself when the file shows none).
pub fn parse_board_files(
    files: &[(String, String)],
    ids: &CardIdMap,
) -> Result<(Vec<RealBoardSpec>, Vec<String>), String> {
    let mut specs = Vec::new();
    let mut unknown = Vec::new();
    for (file, text) in files {
        let raw: RawFile = serde_json::from_str(text).map_err(|e| format!("{file}: {e}"))?;
        for b in raw.boards {
            let side = |list: Vec<RawMinion>, unknown: &mut Vec<String>| {
                let mut out = Vec::new();
                for m in list {
                    // An unknown card id becomes a Stand-in (see `build`).
                    let (name, id_golden) = match ids.get(&m.card_id) {
                        Some(hit) => hit,
                        None => {
                            unknown.push(if m.name.is_empty() {
                                m.card_id.clone()
                            } else {
                                m.name.clone()
                            });
                            (STAND_IN, false)
                        }
                    };
                    out.push(RealMinionSpec {
                        name: name.to_string(),
                        attack: m.attack,
                        health: m.health,
                        golden: m.golden || id_golden,
                        keywords: m.keywords,
                        numbers: m.numbers,
                        dark_gifts: m.dark_gifts,
                    });
                }
                out
            };
            let minions = side(b.minions, &mut unknown);
            let hand = side(b.hand, &mut unknown);
            let mut counters = RealCounters::default();
            for (key, value) in &b.counters {
                counters.set(key, *value);
            }
            // The deity's card id goes through the same map; one we cannot
            // resolve leaves the board without a Deity.
            let deity = b.deity.and_then(|d| {
                let (name, golden) = match ids.get(&d.card_id) {
                    Some((name, golden)) => (name.to_string(), golden),
                    // An id the ref data lacks (a new Deity): its name still
                    // works if the card set has it.
                    None if !d.name.is_empty() => (d.name, false),
                    None => return None,
                };
                Some(RealDeitySpec { name, attack: d.attack, health: d.health, golden })
            });
            specs.push(RealBoardSpec { turn: b.turn, tier: b.tier, minions, hand, counters, deity });
        }
    }
    Ok((specs, unknown))
}
