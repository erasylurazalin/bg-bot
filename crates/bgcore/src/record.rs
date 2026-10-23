//! Recorded sandbox games: the end-of-turn boards of one game as a TOML
//! file, and the loader that reads one back into `Minion`s.
//!
//! A file has a header (rigged, tribe when rigged, label, seed, free rolls, date) and one
//! `[[turn]]` per fought turn, each with its `[[turn.board]]` minions in
//! order and its `[[turn.hand]]`. A minion always has `card`, `golden`,
//! `attack`, `health` and `keywords`; every other `Minion` field is written
//! only when it differs from a fresh copy of the card, and the loader
//! starts from a fresh copy, so a missing field means its default.

use serde::{Deserialize, Serialize};

use crate::cards::CardDb;
use crate::game::SandboxFight;
use crate::minion::{Attached, Keywords, Minion};

/// How a mirror fight ended, for the player's side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FightResult {
    Win,
    Loss,
    Tie,
}

impl FightResult {
    pub fn as_str(self) -> &'static str {
        match self {
            FightResult::Win => "win",
            FightResult::Loss => "loss",
            FightResult::Tie => "tie",
        }
    }

    fn parse(s: &str) -> Result<FightResult, String> {
        match s {
            "win" => Ok(FightResult::Win),
            "loss" => Ok(FightResult::Loss),
            "tie" => Ok(FightResult::Tie),
            other => Err(format!("unknown fight result {other:?}")),
        }
    }
}

/// One recorded end of turn: the board that went into the mirror fight and
/// the hand beside it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TurnRecord {
    pub turn: u32,
    pub tier: u8,
    pub health: i32,
    pub result: FightResult,
    pub board: Vec<Minion>,
    pub hand: Vec<Minion>,
}

impl TurnRecord {
    pub fn from_fight(f: &SandboxFight) -> TurnRecord {
        TurnRecord {
            turn: f.turn,
            tier: f.tier,
            health: f.health,
            result: match f.winner {
                Some(0) => FightResult::Win,
                Some(_) => FightResult::Loss,
                None => FightResult::Tie,
            },
            board: f.board.to_vec(),
            hand: f.hand.to_vec(),
        }
    }
}

/// A whole recorded game.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameRecord {
    /// The tribe the Tavern was rigged to, None for a normal full pool.
    pub tribe: Option<String>,
    pub label: String,
    pub seed: u64,
    pub free_rolls: u8,
    /// The day the game was played, as `YYYY-MM-DD`.
    pub date: String,
    pub turns: Vec<TurnRecord>,
}

#[derive(Serialize, Deserialize)]
struct FileGame {
    /// Whether the Tavern and the draws were rigged to `tribe`.
    #[serde(default)]
    rigged: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tribe: Option<String>,
    label: String,
    seed: i64,
    free_rolls: u8,
    date: String,
    #[serde(default)]
    turn: Vec<FileTurn>,
}

#[derive(Serialize, Deserialize)]
struct FileTurn {
    turn: u32,
    tier: u8,
    health: i32,
    result: String,
    #[serde(default)]
    board: Vec<FileMinion>,
    #[serde(default)]
    hand: Vec<FileMinion>,
}

#[derive(Serialize, Deserialize)]
struct FileAttached {
    card: String,
    golden: bool,
}

#[derive(Serialize, Deserialize)]
struct FileMinion {
    card: String,
    golden: bool,
    attack: i32,
    health: i32,
    keywords: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pool_copies: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    winners_bread: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    magnetizations: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    volumizers: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    improvements: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    extra_magnetize: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    taught_spell: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    locked_turns: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    activated: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    no_reward: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    temp_shield: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    attached: Vec<FileAttached>,
}

fn keyword_names(k: Keywords) -> Vec<String> {
    Keywords::ALL.iter().filter(|(bit, _)| k.has(*bit)).map(|(_, n)| n.to_string()).collect()
}

fn to_file_minion(db: &CardDb, m: &Minion) -> FileMinion {
    let base = db.make(m.card, m.golden);
    let differs = |a: u64, b: u64| (a != b).then_some(a);
    FileMinion {
        card: db.get(m.card).name.clone(),
        golden: m.golden,
        attack: m.attack,
        health: m.health,
        keywords: keyword_names(m.keywords),
        pool_copies: differs(m.pool_copies as u64, base.pool_copies as u64).map(|v| v as u8),
        winners_bread: differs(m.winners_bread as u64, 0).map(|v| v as u8),
        magnetizations: differs(m.magnetizations as u64, 0).map(|v| v as u16),
        volumizers: differs(m.volumizers as u64, base.volumizers as u64).map(|v| v as u16),
        improvements: differs(m.improvements as u64, 0).map(|v| v as u16),
        extra_magnetize: differs(m.extra_magnetize as u64, 0).map(|v| v as u8),
        taught_spell: m.taught_spell.map(|c| db.get(c).name.clone()),
        locked_turns: differs(m.locked_turns as u64, 0).map(|v| v as u8),
        activated: m.activated.then_some(true),
        no_reward: (m.no_reward != base.no_reward).then_some(m.no_reward),
        temp_shield: m.temp_shield.then_some(true),
        attached: m
            .attached_slice()
            .iter()
            .map(|a| FileAttached { card: db.get(a.card).name.clone(), golden: a.golden })
            .collect(),
    }
}

fn card_id(db: &CardDb, name: &str) -> Result<u16, String> {
    db.by_name(name).ok_or_else(|| format!("unknown card {name:?}"))
}

fn from_file_minion(db: &CardDb, f: &FileMinion) -> Result<Minion, String> {
    let id = card_id(db, &f.card)?;
    let mut m = db.make(id, f.golden);
    m.attack = f.attack;
    m.health = f.health;
    let mut kw = Keywords::default();
    for name in &f.keywords {
        let (bit, _) = Keywords::ALL
            .iter()
            .find(|(_, n)| n == name)
            .ok_or_else(|| format!("{}: unknown keyword {name:?}", f.card))?;
        kw.set(*bit);
    }
    m.keywords = kw;
    if let Some(v) = f.pool_copies {
        m.pool_copies = v;
    }
    if let Some(v) = f.winners_bread {
        m.winners_bread = v;
    }
    if let Some(v) = f.magnetizations {
        m.magnetizations = v;
    }
    if let Some(v) = f.volumizers {
        m.volumizers = v;
    }
    if let Some(v) = f.improvements {
        m.improvements = v;
    }
    if let Some(v) = f.extra_magnetize {
        m.extra_magnetize = v;
    }
    if let Some(name) = &f.taught_spell {
        m.taught_spell = Some(card_id(db, name)?);
    }
    if let Some(v) = f.locked_turns {
        m.locked_turns = v;
    }
    if let Some(v) = f.activated {
        m.activated = v;
    }
    if let Some(v) = f.no_reward {
        m.no_reward = v;
    }
    if let Some(v) = f.temp_shield {
        m.temp_shield = v;
    }
    for a in &f.attached {
        m.push_attached(Attached { card: card_id(db, &a.card)?, golden: a.golden });
    }
    Ok(m)
}

/// The record as TOML text.
pub fn to_toml(db: &CardDb, g: &GameRecord) -> Result<String, String> {
    let seed = i64::try_from(g.seed).map_err(|_| "seed too large for a TOML integer".to_string())?;
    let file = FileGame {
        rigged: Some(g.tribe.is_some()),
        tribe: g.tribe.clone(),
        label: g.label.clone(),
        seed,
        free_rolls: g.free_rolls,
        date: g.date.clone(),
        turn: g
            .turns
            .iter()
            .map(|t| FileTurn {
                turn: t.turn,
                tier: t.tier,
                health: t.health,
                result: t.result.as_str().to_string(),
                board: t.board.iter().map(|m| to_file_minion(db, m)).collect(),
                hand: t.hand.iter().map(|m| to_file_minion(db, m)).collect(),
            })
            .collect(),
    };
    toml::to_string(&file).map_err(|e| format!("writing the record: {e}"))
}

/// Read a recorded game back: per turn, the board and hand as `Minion`s.
pub fn load_str(db: &CardDb, text: &str) -> Result<GameRecord, String> {
    let file: FileGame = toml::from_str(text).map_err(|e| format!("reading the record: {e}"))?;
    let mut turns = Vec::new();
    for t in &file.turn {
        let minions = |v: &[FileMinion]| -> Result<Vec<Minion>, String> {
            v.iter().map(|m| from_file_minion(db, m)).collect()
        };
        turns.push(TurnRecord {
            turn: t.turn,
            tier: t.tier,
            health: t.health,
            result: FightResult::parse(&t.result)?,
            board: minions(&t.board).map_err(|e| format!("turn {}: {e}", t.turn))?,
            hand: minions(&t.hand).map_err(|e| format!("turn {}: {e}", t.turn))?,
        });
    }
    let tribe = if file.rigged.unwrap_or(file.tribe.is_some()) { file.tribe } else { None };
    Ok(GameRecord {
        tribe,
        label: file.label,
        seed: u64::try_from(file.seed).map_err(|_| "negative seed".to_string())?,
        free_rolls: file.free_rolls,
        date: file.date,
        turns,
    })
}

/// Read a recorded game from a file.
pub fn load_file(db: &CardDb, path: &std::path::Path) -> Result<GameRecord, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    load_str(db, &text)
}

/// Today's date (UTC) as `YYYY-MM-DD`.
pub fn today() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    date_from_days((secs / 86_400) as i64)
}

/// Civil date of a day count since 1970-01-01 (Howard Hinnant's algorithm).
fn date_from_days(days: i64) -> String {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::date_from_days;

    #[test]
    fn dates() {
        assert_eq!(date_from_days(0), "1970-01-01");
        assert_eq!(date_from_days(20_000), "2024-10-04");
        assert_eq!(date_from_days(11_016), "2000-02-29");
    }
}
