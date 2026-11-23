//! One game: every entity the log has named, keyed by entity id.
//!
//! This is the state the reader builds, the same table the game client holds
//! (`Game` in the older tracker's `powerlog.py`). A game starts at `CREATE_GAME`
//! and ends at the next one; the reader keeps the latest game only.

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};

use crate::entity::Entity;

/// Every entity of one game, with the players and the names the log used.
#[derive(Clone, Debug, Default)]
pub struct Game {
    /// Entity id to entity. A hash map has no order: read it through
    /// [`Game::entities_in_order`] when the order the log created entities in
    /// matters, as it does for the Battlegrounds views (the Python reads the
    /// values of its dict, which is the creation order).
    pub entities: HashMap<i32, Entity>,
    /// The entity ids in the order [`Game::entity`] first made each of them.
    pub created: Vec<i32>,
    /// The `GameType=` from the game header (`GT_BATTLEGROUNDS`), or empty
    /// while no header has been read.
    pub game_type: String,
    /// The `BuildNumber=` from the game header: the client build the game was
    /// played on (254477 is patch 37.0, 255790 is 37.0.2). 0 while no header
    /// has been read.
    pub build: i64,
    /// PlayerID to player name from the game header, in the order the header
    /// lists them. A `Vec` rather than a map because the order is what decides
    /// which player a repeated header name belongs to.
    pub player_names: Vec<(i32, String)>,
    /// PlayerID to the entity id of that player's `Player` entity.
    pub players: HashMap<i32, i32>,
    /// The PlayerIDs with a real account behind them: in Battlegrounds the
    /// other player entities stand in for whoever you fight.
    pub real_players: HashSet<i32>,
    /// A name as the log spelled it (`GameEntity`, `Bartender Bob`) to the
    /// entity id it means.
    pub names: HashMap<String, i32>,
    /// Entity id to the name the log last used for it.
    pub last_name: HashMap<i32, String>,
    /// The clock of the line being read, as the log prints it
    /// (`18:23:44.1234567`, local time), and how many midnights the log has
    /// passed since its first line: [`Game::time`] turns both into a date.
    pub clock: String,
    pub days: u32,
}

impl Game {
    pub fn new() -> Game {
        Game::default()
    }

    /// The line being read as a local date and time
    /// (`2026-11-17T18:23:44.1234567`), from the date the log started on
    /// (its folder's name, `Hearthstone_2026_11_17_18_04_17`). Empty while no
    /// line has been read or when the name carries no date.
    pub fn time(&self, log_name: &str) -> String {
        local_time(log_name, &self.clock, self.days)
    }

    /// The entity with this id, created empty if the log has not named it yet.
    pub fn entity(&mut self, id: i32) -> &mut Entity {
        match self.entities.entry(id) {
            Entry::Occupied(slot) => slot.into_mut(),
            Entry::Vacant(slot) => {
                self.created.push(id);
                slot.insert(Entity::new(id))
            }
        }
    }

    /// Every entity, in the order the log created them.
    pub fn entities_in_order(&self) -> impl Iterator<Item = &Entity> {
        self.created.iter().filter_map(|id| self.entities.get(id))
    }

    /// The game entity, if the log has made one: id 1 unless the log said
    /// otherwise (`GameEntity EntityID=`).
    pub fn game_entity(&self) -> Option<&Entity> {
        self.entities
            .get(&self.names.get("GameEntity").copied().unwrap_or(1))
    }

    /// A tag of the game entity, 0 when the tag or the entity is missing (the
    /// Python's `game_entity["TAG"]`).
    pub fn game_tag(&self, tag: &str) -> i64 {
        self.game_entity().map(|e| e.int(tag)).unwrap_or(0)
    }

    /// The text of a game entity tag, "" when the tag or the entity is missing.
    pub fn game_text(&self, tag: &str) -> &str {
        self.game_entity().map(|e| e.text(tag)).unwrap_or("")
    }

    /// The `Player` entity of a PlayerID, if the log has created it.
    pub fn player_entity(&self, player_id: i32) -> Option<&Entity> {
        match self.players.get(&player_id) {
            Some(&eid) if eid != 0 => self.entities.get(&eid),
            _ => None,
        }
    }

    /// The name the game header gave a PlayerID.
    pub fn player_name(&self, player_id: i32) -> Option<&str> {
        self.player_names
            .iter()
            .find(|(pid, _)| *pid == player_id)
            .map(|(_, name)| name.as_str())
    }

    /// Set a header name, keeping the place of a PlayerID the header names
    /// twice (the Python assigns into a dict).
    pub fn set_player_name(&mut self, player_id: i32, name: String) {
        match self.player_names.iter_mut().find(|(pid, _)| *pid == player_id) {
            Some(slot) => slot.1 = name,
            None => self.player_names.push((player_id, name)),
        }
    }

    /// The PlayerID of the first player with no real account behind it: the
    /// stand-in the log plays the opponent on, and the player whose entity the
    /// opponent's views are read from (the Python's `Tracker._stand_in`).
    /// Unlike [`Game::stand_in`] it does not ask that there be only one.
    pub fn stand_in_player(&self) -> Option<i32> {
        self.players
            .keys()
            .copied()
            .find(|player| !self.real_players.contains(player))
    }

    /// The entity id of the only player with no real account behind it: the
    /// stand-in the other players' names get read as. `None` when there is no
    /// such player or more than one (the order does not matter then).
    pub fn stand_in(&self) -> Option<i32> {
        let mut stand_ins = self
            .players
            .iter()
            .filter(|(pid, _)| !self.real_players.contains(pid))
            .map(|(_, &eid)| eid);
        let first = stand_ins.next()?;
        if stand_ins.next().is_some() {
            return None;
        }
        Some(first)
    }
}

/// A clock and the midnights passed, as a local date and time from the date
/// the log started on (see [`Game::time`]). Empty when either is missing.
pub fn local_time(log_name: &str, clock: &str, days: u32) -> String {
    match start_date(log_name) {
        Some(date) if !clock.is_empty() => {
            let (y, m, d) = add_days(date, days);
            format!("{y:04}-{m:02}-{d:02}T{clock}")
        }
        _ => String::new(),
    }
}

/// The date in a log folder's name: `Hearthstone_2026_11_17_18_04_17`, with
/// any prefix before it (`laptop-`) and any suffix after (`-g2`).
pub fn start_date(log_name: &str) -> Option<(u32, u32, u32)> {
    let at = log_name.find("Hearthstone_")? + "Hearthstone_".len();
    let mut parts = log_name[at..].split('_');
    let y = parts.next()?.parse().ok()?;
    let m = parts.next()?.parse().ok()?;
    let d = parts.next()?.parse().ok()?;
    ((1..=12).contains(&m) && (1..=31).contains(&d)).then_some((y, m, d))
}

/// A date plus some days, by the Gregorian calendar.
fn add_days((mut y, mut m, mut d): (u32, u32, u32), days: u32) -> (u32, u32, u32) {
    for _ in 0..days {
        let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
        let len = match m {
            2 if leap => 29,
            2 => 28,
            4 | 6 | 9 | 11 => 30,
            _ => 31,
        };
        d += 1;
        if d > len {
            d = 1;
            m += 1;
            if m > 12 {
                m = 1;
                y += 1;
            }
        }
    }
    (y, m, d)
}
