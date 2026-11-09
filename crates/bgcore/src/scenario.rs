//! Scenario starts for training (`docs/briefs/comp-starts.md`): a game can
//! begin at a later turn with a real board already on the board and a comp's
//! core cards in hand, so the network gets to practice playing an assembled
//! comp. This is a training mode; normal games never read anything here.
//!
//! The choice of turn, comps, Health and lobby is drawn from a `Rng` seeded
//! from the game's seed, so a scenario game is deterministic for its seed.
//! The comp's lobby tribes are always in the lobby, and the highest core card
//! sets a floor for the tier.

use crate::cards::{Tribe, Tribes};
use crate::comps::{Comp, Comps};
use crate::minion::CardId;
use crate::{Data, HAND, PLAYERS, Rng};

/// The deadline curve the scenario tier starts from: 0 for turns 1 to 4,
/// tier 3 on turns 5 and 6, tier 4 on turn 7, tier 5 on turn 8 and tier 6
/// from turn 9 (the brief leaves the late turns to judgement).
pub fn deadline_tier(turn: u32) -> u8 {
    match turn {
        0..=4 => 0,
        5 | 6 => 3,
        7 => 4,
        8 => 5,
        _ => 6,
    }
}

/// One seat a scenario game fills: its comp, the core cards it starts with in
/// hand (highest weight first, at most `HAND`), the tier and Health.
#[derive(Clone, Debug)]
pub struct ScenarioSeat {
    pub comp: usize,
    pub core: Vec<CardId>,
    pub tier: u8,
    pub health: i32,
}

/// What one scenario game starts with.
#[derive(Clone, Debug)]
pub struct ScenarioPlan {
    pub turn: u32,
    pub lobby: Tribes,
    /// One entry per player, None for a seat with a normal start.
    pub seats: [Option<ScenarioSeat>; PLAYERS],
}

/// A comp file, the comps scenario games may draw, the turn range and the
/// share of games that are scenario games.
#[derive(Clone, Debug)]
pub struct ScenarioConfig {
    pub comps: Comps,
    pub choices: Vec<usize>,
    pub turn_min: u32,
    pub turn_max: u32,
    pub share: f64,
}

impl ScenarioConfig {
    pub fn new(
        comps: Comps,
        names: &[String],
        turn_min: u32,
        turn_max: u32,
        share: f64,
    ) -> Result<ScenarioConfig, String> {
        if names.is_empty() {
            return Err("scenario: no comp names".to_string());
        }
        if turn_min < 1 || turn_max < turn_min {
            return Err(format!(
                "scenario: turns {turn_min},{turn_max}: need 1 <= MIN <= MAX"
            ));
        }
        if !(0.0..=1.0).contains(&share) {
            return Err(format!("scenario: share {share} is not between 0 and 1"));
        }
        let mut choices = Vec::with_capacity(names.len());
        for name in names {
            let i = comps
                .index_of(name)
                .ok_or_else(|| format!("scenario: no comp named {name:?}"))?;
            choices.push(i);
        }
        Ok(ScenarioConfig { comps, choices, turn_min, turn_max, share })
    }

    /// A scenario plan for one game, or None when this game is a normal one.
    /// `seats` are the players to fill (learner and snapshot seats).
    pub fn plan(&self, seed: u64, data: &Data, seats: &[usize]) -> Option<ScenarioPlan> {
        if seats.is_empty() || self.share <= 0.0 {
            return None;
        }
        let mut rng = Rng::new(seed ^ 0x5343_454e_4152_4900);
        if self.share < 1.0 {
            let take = (self.share * 1_000_000.0) as u32;
            if rng.below(1_000_000) >= take {
                return None;
            }
        }
        let turn = self.turn_min + rng.below(self.turn_max - self.turn_min + 1);
        // A few comps need two lobby tribes each, and a core card can bring a
        // tribe of its own, so four independent draws can want more than the
        // five a lobby holds. Draw the set again (deterministically) until it
        // fits; after many tries give every seat the first draw's comp.
        let mut picked: Vec<usize> = Vec::with_capacity(seats.len());
        for _ in 0..128 {
            picked.clear();
            for _ in 0..seats.len() {
                let k = rng.below(self.choices.len() as u32) as usize;
                picked.push(self.choices[k]);
            }
            if lobby_of(&self.comps, &picked, data).len() <= 5 {
                break;
            }
        }
        if picked.is_empty() {
            return None;
        }
        if lobby_of(&self.comps, &picked, data).len() > 5 {
            let first = picked[0];
            picked.fill(first);
        }
        let mut lobby = lobby_of(&self.comps, &picked, data);
        while lobby.len() < 5 {
            let t = Tribe::EVERY[rng.below(Tribe::EVERY.len() as u32) as usize];
            if !lobby.has(t) {
                lobby = lobby.with(t);
            }
        }
        let mut out: [Option<ScenarioSeat>; PLAYERS] = std::array::from_fn(|_| None);
        for (p, &ci) in seats.iter().zip(&picked) {
            let comp = &self.comps.comps[ci];
            out[*p] = Some(ScenarioSeat {
                comp: ci,
                core: core_hand(comp),
                tier: deadline_tier(turn)
                    .max(core_tier(comp, data))
                    .min(crate::MAX_TIER),
                health: 15 + rng.below(21) as i32,
            });
        }
        Some(ScenarioPlan { turn, lobby, seats: out })
    }
}

/// The lobby tribes the picked comps need. Besides `comp.lobby` this holds
/// every core card's own tribe and lobby tie, because the scenario puts every
/// core card in hand and only cards whose lobby test passes are in the pool.
fn lobby_of(comps: &Comps, picked: &[usize], data: &Data) -> Tribes {
    let mut lobby = Tribes::NONE;
    for &i in picked {
        let comp = &comps.comps[i];
        for &t in &comp.lobby {
            lobby = lobby.with(t);
        }
        for core in &comp.core {
            let card = data.cards.get(core.id);
            for &t in &Tribe::EVERY {
                if card.tribes.has(t) {
                    lobby = lobby.with(t);
                }
            }
            if let Some(t) = card.lobby {
                lobby = lobby.with(t);
            }
        }
    }
    lobby
}

/// The comp's core cards, highest weight first, at most a hand's worth.
fn core_hand(comp: &Comp) -> Vec<CardId> {
    let mut core: Vec<_> = comp.core.iter().collect();
    core.sort_by(|a, b| b.weight.total_cmp(&a.weight));
    core.into_iter().take(HAND).map(|c| c.id).collect()
}

fn core_tier(comp: &Comp, data: &Data) -> u8 {
    comp.core
        .iter()
        .map(|c| data.cards.get(c.id).tier)
        .max()
        .unwrap_or(1)
}
