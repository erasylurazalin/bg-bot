//! Real boards from Era's games (`boards/real/*.json`, docs/real-boards.md),
//! played by real seats instead of shopping (see `Game::new_random_heroes_real`).
//!
//! The JSON texts are parsed here (`parse_board_files`); the callers read the
//! files: train/real_boards.py for training, `bgbench --real-boards` for the
//! fuzzer and the benchmarks.

use std::collections::{BTreeMap, HashMap};

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
    /// Marked by Rapid Reanimation (the file's `reanimate`): the minion the
    /// power destroys at Start of Combat and brings back as an exact copy.
    pub reanimate: bool,
}

/// A real board's Deity: the minion it awakens and the stats it has now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RealDeitySpec {
    pub name: String,
    pub attack: i32,
    pub health: i32,
    pub golden: bool,
}

/// Which side of a fight a board belonged to, the file's `who`. Only the
/// fight check (`crate::fight_check`) reads it: a real seat draws both kinds
/// alike.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum RealSide {
    #[default]
    Me,
    Opp,
}

/// The result the file records for a side: `result` on every board.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum RealResult {
    Win,
    Lose,
    Tie,
    /// An older file, or a board that never fought.
    #[default]
    Unknown,
}

/// A side's hero power as the file carries it at the first attack: the
/// power's card id, its name, and its current numbers (the {0} {1} {2} {3} of
/// its text). None of this reaches a real seat yet.
///
/// `waiting` is the minion the power still owes the board, which only Lock and
/// Load has (it steals a Tavern minion and summons it in the next combat);
/// `waiting_known` says whether the file says so. A file written before the
/// key existed has neither, and a fight with Lock and Load in it is then
/// incomplete because the engine cannot tell whether a minion was coming.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RealPowerSpec {
    pub card_id: String,
    pub name: String,
    pub numbers: Vec<i32>,
    pub waiting: Option<RealMinionSpec>,
    pub waiting_known: bool,
}

/// A side's trinket as the file carries it: the card id, whether it sits in
/// the greater slot, and its current numbers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RealTrinketSpec {
    pub card_id: String,
    pub name: String,
    pub greater: bool,
    pub numbers: Vec<i32>,
}

/// A quest reward a side had (the file's `rewards`). The engine models no
/// quest, so a fight with one is incomplete.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RealRewardSpec {
    pub card_id: String,
    pub name: String,
    pub numbers: Vec<i32>,
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
    /// Gold this player has spent this game (Lovesick Balladist and the other
    /// "Gold spent" cards read it). Only a shop step reads it.
    pub gold_spent: u32,
    /// The Tavern's "this game" buff: minions in Bob's Tavern have this much
    /// more, and the ones dealt later get it too (Staff of Enrichment and its
    /// kin). Only a shop step reads it.
    pub tavern_attack: i32,
    pub tavern_health: i32,
    /// The same for Elementals only (Obsidian Ravager and its kin).
    pub tavern_elemental_attack: i32,
    pub tavern_elemental_health: i32,
    /// "After the Tavern is Refreshed this game" (Waveling, En-Djinn Blazer,
    /// Easterly Winds): what one Refresh gives one random Tavern minion. Only
    /// a shop step reads it.
    pub refresh_attack: i32,
    pub refresh_health: i32,
    /// Blood Gem Barrage casts this game: every Refresh gives this much, as
    /// Blood Gems, to every minion in the Tavern. Only a shop step reads it.
    pub barrage_attack: i32,
    pub barrage_health: i32,
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
            gold_spent: 0,
            tavern_attack: 0,
            tavern_health: 0,
            tavern_elemental_attack: 0,
            tavern_elemental_health: 0,
            refresh_attack: 0,
            refresh_health: 0,
            barrage_attack: 0,
            barrage_health: 0,
        }
    }
}

impl RealCounters {
    /// Take one of the numbers a file holds, by its key: `gem_attack`,
    /// `gem_health`, `beetle_attack`, `beetle_health`, `undead_attack`,
    /// `undead_health`, `spell_attack`, `spell_health`, `spells_cast`,
    /// `knight_deaths`, `tasty_lobster`, `baller`, and the numbers only a shop
    /// step reads: `gold_spent`, `tavern_attack`, `tavern_health`,
    /// `tavern_elemental_attack`, `tavern_elemental_health`, `refresh_attack`,
    /// `refresh_health`, `barrage_attack`, `barrage_health`. A key that is not
    /// in the file leaves its field at the base value; a key no engine number
    /// uses is ignored.
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
            "gold_spent" => self.gold_spent = count(value),
            "tavern_attack" => self.tavern_attack = value,
            "tavern_health" => self.tavern_health = value,
            "tavern_elemental_attack" => self.tavern_elemental_attack = value,
            "tavern_elemental_health" => self.tavern_elemental_health = value,
            "refresh_attack" => self.refresh_attack = value,
            "refresh_health" => self.refresh_health = value,
            "barrage_attack" => self.barrage_attack = value,
            "barrage_health" => self.barrage_health = value,
            _ => {}
        }
    }
}

/// One real board: the turn it fought at, the player's Tavern tier then, its
/// minions, the minions in that player's hand, and the side-wide numbers.
/// Which player it belonged to (`who` in the file) is already gone: every
/// board of a turn is an equally likely draw. `Default` is a board with
/// nothing on it and every number at its base value.
///
/// The file's other half is here too and no real seat reads it yet: the game
/// it came from, which side it was, its result, its Health and armor, its
/// hero power, its trinkets and its quest rewards. The fight check
/// (`crate::fight_check`) is what plays a board with all of it
/// (docs/real-boards.md "The fight check").
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RealBoardSpec {
    pub turn: u32,
    pub tier: u8,
    pub minions: Vec<RealMinionSpec>,
    pub hand: Vec<RealMinionSpec>,
    pub counters: RealCounters,
    pub deity: Option<RealDeitySpec>,
    /// The game the board came from (the file's `source` and `game`), which
    /// is how a fight's two boards are found and how the damage a fight did
    /// is compared with the next turn's Health.
    pub source: String,
    pub game: u32,
    pub side: RealSide,
    pub result: RealResult,
    /// The player's Health and armor at the first attack.
    pub health: i32,
    pub armor: i32,
    pub power: Option<RealPowerSpec>,
    pub trinkets: Vec<RealTrinketSpec>,
    pub rewards: Vec<RealRewardSpec>,
    /// The Secret zone: the card ids in the order they were got, which is the
    /// order they fire in. None in a file written before the key existed, and
    /// then a fight with Prestidigitation is incomplete (the engine cannot
    /// tell whether any Secret was up); an empty list means no Secret at all.
    pub secrets: Option<Vec<String>>,
    /// The cards in the hand the log showed without a card id: they are not
    /// in `hand` and nothing can be built for them.
    pub hand_hidden: u32,
}

/// One fight as the files hold it: Era's board and the opponent's on the same
/// turn of the same game, with what the game recorded. The result is the one
/// from Era's side.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RealFightSpec {
    pub me: RealBoardSpec,
    pub opp: RealBoardSpec,
    /// Era's next board in the same game after this fight: its turn and the
    /// Health and armor it shows. The drop from this fight's board to it is
    /// the damage the fight and the next shop phase did (the fight check
    /// reports it; a Health price or an Ice Block moves it too).
    pub next: Option<(u32, i32, i32)>,
}

impl RealFightSpec {
    /// The game file both boards came from, as the collector names it.
    pub fn file(&self) -> String {
        format!("{}-g{}.json", self.me.source, self.me.game)
    }

    pub fn turn(&self) -> u32 {
        self.me.turn
    }

    /// The result the game recorded, from Era's side.
    pub fn result(&self) -> RealResult {
        self.me.result
    }

    /// The damage Era's hero is seen to have taken after this fight: the drop
    /// in Health plus armor from this turn's board to the next one, when the
    /// game recorded both. None when the next board is missing or an older
    /// file has no Health.
    pub fn real_damage(&self) -> Option<i32> {
        let (_, health, armor) = self.next?;
        if self.me.health <= 0 || health <= 0 {
            return None;
        }
        let before = self.me.health + self.me.armor;
        let after = health + armor;
        Some((before - after).max(0))
    }
}


/// A real board ready to be copied onto a seat: fresh minions (no pool
/// copies) with the file's stats, golden flag, keywords and scaling state.
/// The rest of the side as it went into the fight is here too and a real seat
/// plays without it (flat 30 Health and 10 armor, no power, no trinket); the
/// fight check (`crate::fight_check`) is what seats all of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RealBoard {
    pub tier: u8,
    pub minions: ArrayVec<Minion, BOARD>,
    pub hand: ArrayVec<Minion, HAND>,
    pub counters: RealCounters,
    pub deity: Option<Deity>,
    /// The player's Health and armor at the first attack, 0 when the file
    /// did not show them (an older file).
    pub health: i32,
    pub armor: i32,
    /// The hero power, trinkets and quest rewards, by card id: resolving them
    /// against the hero and trinket tables is the fight check's job.
    pub power: Option<RealPowerSpec>,
    pub trinkets: Vec<RealTrinketSpec>,
    pub rewards: Vec<RealRewardSpec>,
    /// The Secret zone, by card id in the order the Secrets were got, or None
    /// when the file does not say (see `RealBoardSpec::secrets`).
    pub secrets: Option<Vec<String>>,
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
        // "Unplayable. In {0} turns, break this open and get a random Golden
        // minion with a type!": {0} is the turns left, which the engine counts
        // down in `improvements` and opens at 0.
        "Lockbox" => n(0),
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

/// One minion of a real board, built exactly as the file writes it: a fresh
/// `Minion` (pool_copies 0, so it takes no pool copy and nothing goes back
/// when it dies) with the file's attack, health, golden flag, keywords, Dark
/// Gifts and the scaling state its numbers stand for
/// (`Minion.improvements`). A minion whose name matches no card becomes a
/// Stand-in with those stats and keywords and no effect (Era, 2026-10-25), and
/// its name is returned. `turn` is the board's turn, which a Dark Gift without
/// a countdown uses as the turn it landed.
fn make_real_minion(
    m: &RealMinionSpec,
    turn: u32,
    db: &CardDb,
    gifts: &crate::darkgifts::GiftDb,
) -> (Minion, Option<String>) {
    let mut unbuilt = None;
    let id = match db.by_name(&m.name) {
        Some(id) => id,
        None => {
            unbuilt = Some(m.name.clone());
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
    minion.reanimate = m.reanimate;
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
    (minion, unbuilt)
}

impl RealBoard {
    /// One minion built as the file writes it (`make_real_minion`): the fight
    /// check uses it for the minion Lock and Load holds, which is not on a
    /// board. The name comes back when the card set has no such card.
    pub fn build_minion(
        m: &RealMinionSpec,
        turn: u32,
        db: &CardDb,
        gifts: &crate::darkgifts::GiftDb,
    ) -> (Minion, Option<String>) {
        make_real_minion(m, turn, db, gifts)
    }

    /// One board resolved against the card set: each minion becomes a fresh
    /// `Minion` (pool_copies 0, so it takes no pool copy and nothing goes
    /// back when it dies or its seat is knocked out) with the file's attack,
    /// health, golden flag and keywords, and the scaling state its numbers
    /// stand for (`Minion.improvements`, the Tasty Lobster count). A minion
    /// whose name matches no card becomes a Stand-in with those stats and
    /// keywords and no effect (Era, 2026-10-25), and its name is returned.
    /// `RealBoards::build` puts every board of a folder through this, and so
    /// does the fight check, one board at a time.
    pub fn build_one(
        spec: &RealBoardSpec,
        db: &CardDb,
        gifts: &crate::darkgifts::GiftDb,
    ) -> (RealBoard, Vec<String>) {
        let mut unmatched = Vec::new();
        let make = |m: &RealMinionSpec, turn: u32, unmatched: &mut Vec<String>| {
            let (minion, unbuilt) = make_real_minion(m, turn, db, gifts);
            if let Some(name) = unbuilt {
                unmatched.push(name);
            }
            minion
        };
        let mut minions = ArrayVec::new();
        let mut lobster = 0;
        for m in &spec.minions {
            if minions.is_full() {
                // A real board never holds more than BOARD minions; a file
                // that somehow does loses the tail.
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
        // Every Lobster shares one count: the log's numbers on any of them,
        // the best of them kept (a Lobster sold is invisible).
        counters.improve[Counter::TastyLobster.index()] =
            counters.improve[Counter::TastyLobster.index()].max(lobster);
        let deity = spec.deity.as_ref().and_then(|d| {
            let card = db.by_name(&d.name)?;
            Some(Deity { card, attack: d.attack, health: d.health, golden: d.golden })
        });
        // A board whose log gave no Tavern tier (0) takes the highest tier
        // among its minions, at least 1 (2026-10-31: three such boards made a
        // real seat's shop checks index tier 0).
        let tier = if spec.tier > 0 {
            spec.tier.min(crate::MAX_TIER)
        } else {
            minions.iter().map(|m| db.get(m.card).tier).max().unwrap_or(1).clamp(1, crate::MAX_TIER)
        };
        (RealBoard {
            tier,
            minions,
            hand,
            counters,
            deity,
            health: spec.health,
            armor: spec.armor,
            power: spec.power.clone(),
            trinkets: spec.trinkets.clone(),
            rewards: spec.rewards.clone(),
            secrets: spec.secrets.clone(),
        }, unmatched)
    }
}

impl RealBoards {
    /// Resolve the specs against the card set, one board at a time
    /// (`RealBoard::build_one`), and bucket them by turn. The names of the
    /// minions no card matches come back with them.
    pub fn build(
        specs: &[RealBoardSpec],
        db: &CardDb,
        gifts: &crate::darkgifts::GiftDb,
    ) -> (RealBoards, Vec<String>) {
        let mut by_turn = vec![Vec::new(); LATE_TURN as usize];
        let mut unmatched = Vec::new();
        for spec in specs {
            let (board, missing) = RealBoard::build_one(spec, db, gifts);
            unmatched.extend(missing);
            // Turn 13 and later all draw from one bucket.
            let t = spec.turn.clamp(1, LATE_TURN) as usize;
            by_turn[t - 1].push(board);
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
    /// The log folder the game came from (`boards/real/<source>-g<N>.json`).
    #[serde(default)]
    source: String,
    #[serde(default)]
    game: u32,
    boards: Vec<RawBoard>,
}

#[derive(Deserialize)]
struct RawBoard {
    turn: u32,
    tier: u8,
    /// `me` or `opp`: whose board this was. Files before the fight check
    /// always wrote it; a board without one counts as Era's.
    #[serde(default)]
    who: String,
    /// `win`, `lose` or `tie` for the side that owned the board.
    #[serde(default)]
    result: String,
    /// The player's Health and armor at the first attack. An older file
    /// without them keeps the rules' starting Health.
    #[serde(default)]
    health: Option<i32>,
    #[serde(default)]
    armor: i32,
    /// The side's hero power at the first attack.
    #[serde(default)]
    hero_power: Option<RawPower>,
    #[serde(default)]
    trinkets: Vec<RawTrinket>,
    #[serde(default)]
    rewards: Vec<RawReward>,
    /// The Secret zone, by card id in the order the Secrets were got. A file
    /// without the key does not say what was up.
    #[serde(default)]
    secrets: Option<Vec<String>>,
    /// The cards in the hand the log did not name.
    #[serde(default)]
    hand_hidden: u32,
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
struct RawPower {
    #[serde(default)]
    card_id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    numbers: Vec<i32>,
    /// The minion the power still owes the board (Lock and Load): a board
    /// minion object, or null when it owes none. A file without the key was
    /// written before it was collected, and then nothing can be said.
    #[serde(default, deserialize_with = "present_or_null")]
    waiting: Option<Option<RawMinion>>,
}

/// Read a field that may be an object, null, or absent as those three things:
/// serde's `Option` alone reads null and absent the same way, and the fight
/// check has to tell "the power owes no minion" from "the file does not say".
fn present_or_null<'de, T, D>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    Option::<T>::deserialize(de).map(Some)
}

#[derive(Deserialize)]
struct RawTrinket {
    #[serde(default)]
    card_id: String,
    #[serde(default)]
    name: String,
    /// Whether it sits in the greater slot.
    #[serde(default)]
    greater: bool,
    #[serde(default)]
    numbers: Vec<i32>,
}

#[derive(Deserialize)]
struct RawReward {
    #[serde(default)]
    card_id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    numbers: Vec<i32>,
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
    /// Marked by Rapid Reanimation: the minion the power destroys at Start of
    /// Combat and brings back as an exact copy.
    #[serde(default)]
    reanimate: bool,
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
            let minions = b
                .minions
                .into_iter()
                .map(|m| real_minion(m, ids, &mut unknown))
                .collect();
            let hand = b.hand.into_iter().map(|m| real_minion(m, ids, &mut unknown)).collect();
            // The hero power's waiting minion (Lock and Load) goes through the
            // same card id map, so its stats are the ones the file shows.
            let power = match b.hero_power {
                None => None,
                Some(p) => {
                    let (waiting, waiting_known) = match p.waiting {
                        None => (None, false),
                        Some(None) => (None, true),
                        Some(Some(m)) => (Some(real_minion(m, ids, &mut unknown)), true),
                    };
                    Some(RealPowerSpec {
                        card_id: p.card_id,
                        name: p.name,
                        numbers: p.numbers,
                        waiting,
                        waiting_known,
                    })
                }
            };
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
            specs.push(RealBoardSpec {
                turn: b.turn,
                tier: b.tier,
                minions,
                hand,
                counters,
                deity,
                source: raw.source.clone(),
                game: raw.game,
                side: if b.who.eq_ignore_ascii_case("opp") { RealSide::Opp } else { RealSide::Me },
                result: parse_result(&b.result),
                health: b.health.unwrap_or(0),
                armor: b.armor,
                power,
                trinkets: b
                    .trinkets
                    .into_iter()
                    .map(|t| RealTrinketSpec {
                        card_id: t.card_id,
                        name: t.name,
                        greater: t.greater,
                        numbers: t.numbers,
                    })
                    .collect(),
                rewards: b
                    .rewards
                    .into_iter()
                    .map(|r| RealRewardSpec {
                        card_id: r.card_id,
                        name: r.name,
                        numbers: r.numbers,
                    })
                    .collect(),
                secrets: b.secrets,
                hand_hidden: b.hand_hidden,
            });
        }
    }
    Ok((specs, unknown))
}

/// The result the file records for a side.
/// One minion of a file as a spec, its card id mapped through the ref data: an
/// unknown id becomes a Stand-in (see `RealBoard::build_one`) and its name goes
/// to `unknown`. Boards, hands and the minion Lock and Load holds all come
/// through here.
fn real_minion(m: RawMinion, ids: &CardIdMap, unknown: &mut Vec<String>) -> RealMinionSpec {
    let (name, id_golden) = match ids.get(&m.card_id) {
        Some(hit) => hit,
        None => {
            unknown.push(if m.name.is_empty() { m.card_id.clone() } else { m.name.clone() });
            (STAND_IN, false)
        }
    };
    RealMinionSpec {
        name: name.to_string(),
        attack: m.attack,
        health: m.health,
        golden: m.golden || id_golden,
        keywords: m.keywords,
        numbers: m.numbers,
        dark_gifts: m.dark_gifts,
        reanimate: m.reanimate,
    }
}

fn parse_result(s: &str) -> RealResult {
    match s {
        "win" => RealResult::Win,
        "lose" => RealResult::Lose,
        "tie" => RealResult::Tie,
        _ => RealResult::Unknown,
    }
}

/// One turn of one game: Era's board and the opponent's board for it, either
/// of which can be missing from a folder.
type TurnBoards = BTreeMap<(String, u32, u32), (Option<RealBoardSpec>, Option<RealBoardSpec>)>;

/// Era's boards of one game, by turn, with the Health and armor each shows.
type GameBoards = BTreeMap<(String, u32), Vec<(u32, i32, i32)>>;

/// Pair every board with the opponent board of the same turn of the same
/// game: what the fight check plays. The order is the files' (by source, game
/// and turn), so a run is repeatable. Boards with no partner come back as a
/// line saying so, by the file and turn (a folder may hold a single side of a
/// turn, and an older file has no `who` at all).
pub fn pair_fights(specs: Vec<RealBoardSpec>) -> (Vec<RealFightSpec>, Vec<String>) {
    let mut turns: TurnBoards = BTreeMap::new();
    let mut unpaired = Vec::new();
    for spec in specs {
        let key = (spec.source.clone(), spec.game, spec.turn);
        let slot = turns.entry(key).or_default();
        // A turn has one board per side; a second one is left out and said so.
        let side = spec.side;
        let (who, taken) = match side {
            RealSide::Me => ("Era's", slot.0.is_some()),
            RealSide::Opp => ("the opponent's", slot.1.is_some()),
        };
        if taken {
            unpaired.push(format!(
                "{}-g{}.json turn {}: a second {who} board, left out",
                spec.source, spec.game, spec.turn
            ));
            continue;
        }
        match side {
            RealSide::Me => slot.0 = Some(spec),
            RealSide::Opp => slot.1 = Some(spec),
        }
    }
    let mut fights = Vec::new();
    // Era's boards of each game, by turn, to find the board after a fight.
    let mut after: GameBoards = BTreeMap::new();
    for ((source, game, turn), (me, _)) in &turns {
        if let Some(me) = me {
            after
                .entry((source.clone(), *game))
                .or_default()
                .push((*turn, me.health, me.armor));
        }
    }
    for v in after.values_mut() {
        v.sort_unstable();
    }
    for ((source, game, turn), (me, opp)) in turns {
        match (me, opp) {
            (Some(me), Some(opp)) => {
                let next = after.get(&(source.clone(), game)).and_then(|boards| {
                    let at = boards.partition_point(|&(t, _, _)| t <= turn);
                    boards.get(at).copied()
                });
                fights.push(RealFightSpec { me, opp, next });
            }
            (me, opp) => unpaired.push(format!(
                "{source}-g{game}.json turn {turn}: {} board with no opponent board",
                if me.is_some() { "only Era's" } else if opp.is_some() { "only the opponent's" } else { "no" }
            )),
        }
    }
    (fights, unpaired)
}
