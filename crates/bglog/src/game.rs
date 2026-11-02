//! One game: every entity the log has named, keyed by entity id.
//!
//! This is the state the reader builds, the same table the game client holds
//! (`Game` in `~/projects/bg/bg/powerlog.py`). A game starts at `CREATE_GAME`
//! and ends at the next one; the reader keeps the latest game only.

use std::collections::{HashMap, HashSet};

use crate::entity::Entity;

/// Every entity of one game, with the players and the names the log used.
#[derive(Clone, Debug, Default)]
pub struct Game {
    /// Entity id to entity. Holders of a `Game` should not assume an order
    /// here: sort by id when printing.
    pub entities: HashMap<i32, Entity>,
    /// The `GameType=` from the game header (`GT_BATTLEGROUNDS`), or empty
    /// while no header has been read.
    pub game_type: String,
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
}

impl Game {
    pub fn new() -> Game {
        Game::default()
    }

    /// The entity with this id, created empty if the log has not named it yet.
    pub fn entity(&mut self, id: i32) -> &mut Entity {
        self.entities.entry(id).or_insert_with(|| Entity::new(id))
    }

    /// The game entity: id 1 unless the log said otherwise.
    pub fn game_entity(&mut self) -> &mut Entity {
        let id = self.names.get("GameEntity").copied().unwrap_or(1);
        self.entity(id)
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
