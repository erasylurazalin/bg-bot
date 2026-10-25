//! Real boards from Era's games (`boards/real/*.json`, docs/real-boards.md),
//! played by real seats instead of shopping (see `Game::new_random_heroes_real`).
//!
//! The JSON texts are parsed here (`parse_board_files`); the callers read the
//! files: train/real_boards.py for training, `bgbench --real-boards` for the
//! fuzzer and the benchmarks.

use std::collections::HashMap;

use arrayvec::ArrayVec;
use serde::Deserialize;

use crate::BOARD;
use crate::cards::CardDb;
use crate::minion::{Keywords, Minion};
use crate::rng::Rng;

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
}

/// One real board: the turn it fought at, the player's Tavern tier then,
/// and its minions. Which player it belonged to (`who` in the file) is
/// already gone: every board of a turn is an equally likely draw.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RealBoardSpec {
    pub turn: u32,
    pub tier: u8,
    pub minions: Vec<RealMinionSpec>,
}

/// A real board ready to be copied onto a seat: fresh minions (no pool
/// copies) with the file's stats, golden flag and keywords.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RealBoard {
    pub tier: u8,
    pub minions: ArrayVec<Minion, BOARD>,
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

impl RealBoards {
    /// Resolve the specs against the card set: each minion becomes a fresh
    /// `Minion` (pool_copies 0, so it takes no pool copy and nothing goes
    /// back when it dies or its seat is knocked out) with the file's attack,
    /// health, golden flag and keywords. A minion whose name matches no card
    /// becomes a Stand-in with those stats and keywords and no effect (Era,
    /// 2026-10-25), and its name is returned.
    pub fn build(specs: &[RealBoardSpec], db: &CardDb) -> (RealBoards, Vec<String>) {
        let mut by_turn = vec![Vec::new(); LATE_TURN as usize];
        let mut unmatched = Vec::new();
        for spec in specs {
            let mut minions = ArrayVec::new();
            for m in &spec.minions {
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
                let mut keywords = Keywords(0);
                for kw in &m.keywords {
                    set_keyword(&mut keywords, kw);
                }
                minion.keywords = keywords;
                if minions.is_full() {
                    // A real board never holds more than BOARD minions; a
                    // file that somehow does loses the tail.
                    continue;
                }
                minions.push(minion);
            }
            // Turn 13 and later all draw from one bucket.
            let t = spec.turn.clamp(1, LATE_TURN) as usize;
            by_turn[t - 1].push(RealBoard { tier: spec.tier, minions });
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
}

#[derive(Deserialize)]
struct RawMinion {
    card_id: String,
    #[serde(default)]
    name: String,
    attack: i32,
    health: i32,
    #[serde(default)]
    golden: bool,
    #[serde(default)]
    keywords: Vec<String>,
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
            let mut minions = Vec::new();
            for m in b.minions {
                // An unknown card id becomes a Stand-in (see `build`).
                let (name, id_golden) = match ids.get(&m.card_id) {
                    Some(hit) => hit,
                    None => {
                        unknown.push(if m.name.is_empty() { m.card_id.clone() } else { m.name.clone() });
                        (STAND_IN, false)
                    }
                };
                minions.push(RealMinionSpec {
                    name: name.to_string(),
                    attack: m.attack,
                    health: m.health,
                    golden: m.golden || id_golden,
                    keywords: m.keywords,
                });
            }
            specs.push(RealBoardSpec { turn: b.turn, tier: b.tier, minions });
        }
    }
    Ok((specs, unknown))
}
