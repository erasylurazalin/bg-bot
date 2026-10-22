//! Python bindings: a batch of games stepped together, for training.
//!
//! Every seat in a game has a kind. Rule and random seats are played here in
//! Rust. Learner and snapshot seats are played from Python: after every call
//! each game is waiting on one of them, and `observe` says which.
//!
//! Python owns the numpy arrays and passes them in; Rust fills them. Nothing
//! is allocated per step.

// Seat numbers index several per-seat arrays at once.
#![allow(clippy::needless_range_loop)]

use std::sync::Arc;

use bgcore::bots::{random_action, rule_action};
use bgcore::obs::{self, OBS_SIZE};
use bgcore::{Action, Data, Game, N_ACTIONS, PLAYERS, Rng};
use numpy::{PyReadonlyArray1, PyReadwriteArray1, PyReadwriteArray2, PyReadwriteArray3};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use rayon::prelude::*;

const LEARNER: u8 = 0;
const SNAPSHOT: u8 = 1;
const RULE: u8 = 2;
const RANDOM: u8 = 3;

struct Env {
    game: Game,
    kinds: [u8; PLAYERS],
    rng: Rng,
    /// Seats whose placement was already reported.
    reported: [bool; PLAYERS],
}

impl Env {
    fn new(data: &Arc<Data>, seed: u64, lineup: &[u8; 4]) -> Env {
        let mut rng = Rng::new(seed ^ 0x6a09_e667_f3bc_c908);
        let mut kinds = [LEARNER; PLAYERS];
        let mut i = 0;
        for (kind, &n) in lineup.iter().enumerate() {
            for _ in 0..n {
                kinds[i] = kind as u8;
                i += 1;
            }
        }
        rng.shuffle(&mut kinds);
        Env { game: Game::new(data.clone(), seed), kinds, rng, reported: [false; PLAYERS] }
    }

    fn is_python(&self, p: usize) -> bool {
        matches!(self.kinds[p], LEARNER | SNAPSHOT)
    }

    /// Play rule and random seats until a Python seat is up or the game ends.
    fn advance(&mut self) {
        while let Some(p) = self.game.current_player() {
            let a = match self.kinds[p] {
                RULE => rule_action(&self.game),
                RANDOM => random_action(&self.game, &mut self.rng),
                _ => return,
            };
            self.game.step(a).expect("bot picked an illegal action");
        }
    }

    /// Write the placement of every seat knocked out since the last call.
    fn report(&mut self, places: &mut [i8]) {
        for p in 0..PLAYERS {
            let place = self.game.players[p].placement;
            if place != 0 && !self.reported[p] {
                places[p] = place as i8;
                self.reported[p] = true;
            }
        }
    }
}

#[pyclass]
struct VecEnv {
    data: Arc<Data>,
    envs: Vec<Env>,
    lineup: [u8; 4],
    next_seed: u64,
}

fn check_lineup(lineup: [u8; 4]) -> PyResult<[u8; 4]> {
    if lineup.iter().map(|&n| n as usize).sum::<usize>() != PLAYERS || lineup[0] == 0 {
        return Err(PyValueError::new_err("lineup is (learner, snapshot, rule, random) seats, 8 in total, at least 1 learner"));
    }
    Ok(lineup)
}

#[pymethods]
impl VecEnv {
    /// `lineup`: how many seats of each kind (learner, snapshot, rule,
    /// random) a new game gets, shuffled into random seats.
    #[new]
    #[pyo3(signature = (n, seed, lineup, cards_toml=None, rules_toml=None))]
    fn new(n: usize, seed: u64, lineup: [u8; 4], cards_toml: Option<&str>, rules_toml: Option<&str>) -> PyResult<VecEnv> {
        let lineup = check_lineup(lineup)?;
        let data = match (cards_toml, rules_toml) {
            (None, None) => Data::bundled(),
            (Some(c), Some(r)) => Data::from_toml(c, r).map_err(PyValueError::new_err)?,
            _ => return Err(PyValueError::new_err("give both cards_toml and rules_toml, or neither")),
        };
        let data = Arc::new(data);
        let seed = seed.wrapping_mul(1 << 32);
        let mut envs: Vec<Env> = (0..n as u64).map(|i| Env::new(&data, seed + i, &lineup)).collect();
        envs.par_iter_mut().for_each(Env::advance);
        Ok(VecEnv { data, envs, lineup, next_seed: seed + n as u64 })
    }

    /// The lineup for games that start from now on.
    fn set_lineup(&mut self, lineup: [u8; 4]) -> PyResult<()> {
        self.lineup = check_lineup(lineup)?;
        Ok(())
    }

    #[getter]
    fn n(&self) -> usize {
        self.envs.len()
    }

    /// Take `actions[i]` for the waiting seat of game i. Fills
    /// `places[i, seat]` with the placement of every seat knocked out during
    /// this call (0 for the rest), and `reset[i]` for games that ended and
    /// restarted. Raises on an illegal action.
    fn step(
        &mut self,
        py: Python<'_>,
        actions: PyReadonlyArray1<i64>,
        mut places: PyReadwriteArray2<i8>,
        mut reset: PyReadwriteArray1<bool>,
    ) -> PyResult<()> {
        let n = self.envs.len();
        let actions = actions.as_slice()?;
        let places = places.as_slice_mut()?;
        let reset = reset.as_slice_mut()?;
        if actions.len() != n || places.len() != n * PLAYERS || reset.len() != n {
            return Err(PyValueError::new_err("array sizes don't match the number of games"));
        }
        let (data, lineup, base) = (&self.data, self.lineup, self.next_seed);
        let envs = &mut self.envs;
        let bad = py.detach(|| {
            envs.par_iter_mut()
                .zip(actions)
                .zip(places.par_chunks_mut(PLAYERS))
                .zip(reset.par_iter_mut())
                .enumerate()
                .map(|(i, (((env, &a), places), reset))| {
                    places.fill(0);
                    *reset = false;
                    let Some(a) = usize::try_from(a).ok().and_then(Action::from_index) else { return Some(i) };
                    if env.game.step(a).is_err() {
                        return Some(i);
                    }
                    env.advance();
                    env.report(places);
                    if env.game.over {
                        *env = Env::new(data, base + i as u64, &lineup);
                        env.advance();
                        *reset = true;
                    }
                    None
                })
                .find_first(|r| r.is_some())
                .flatten()
        });
        self.next_seed += n as u64;
        match bad {
            Some(i) => Err(PyValueError::new_err(format!("illegal action {} in game {i}", actions[i]))),
            None => Ok(()),
        }
    }

    /// For each game: the waiting seat's observation, legal mask, seat and
    /// kind (0 learner, 1 snapshot).
    fn observe(
        &self,
        py: Python<'_>,
        mut obs: PyReadwriteArray2<f32>,
        mut mask: PyReadwriteArray2<bool>,
        mut seat: PyReadwriteArray1<i64>,
        mut kind: PyReadwriteArray1<i64>,
    ) -> PyResult<()> {
        let n = self.envs.len();
        let obs = obs.as_slice_mut()?;
        let mask = mask.as_slice_mut()?;
        let seat = seat.as_slice_mut()?;
        let kind = kind.as_slice_mut()?;
        if obs.len() != n * OBS_SIZE || mask.len() != n * N_ACTIONS || seat.len() != n || kind.len() != n {
            return Err(PyValueError::new_err("array sizes don't match"));
        }
        py.detach(|| {
            self.envs
                .par_iter()
                .zip(obs.par_chunks_mut(OBS_SIZE))
                .zip(mask.par_chunks_mut(N_ACTIONS))
                .zip(seat.par_iter_mut())
                .zip(kind.par_iter_mut())
                .for_each(|((((env, o), m), s), k)| {
                    let p = env.game.current_player().expect("a waiting game is never over");
                    debug_assert!(env.is_python(p));
                    obs::write(&env.game, p, o);
                    let m: &mut [bool; N_ACTIONS] = m.try_into().unwrap();
                    env.game.legal_mask(m);
                    *s = p as i64;
                    *k = env.kinds[p] as i64;
                });
        });
        Ok(())
    }

    /// Every seat's observation, whoever is waiting, and whether that seat
    /// is a learner still in the game. For bootstrapping values at the end
    /// of a rollout.
    fn observe_all(&self, py: Python<'_>, mut obs: PyReadwriteArray3<f32>, mut live: PyReadwriteArray2<bool>) -> PyResult<()> {
        let n = self.envs.len();
        let obs = obs.as_slice_mut()?;
        let live = live.as_slice_mut()?;
        if obs.len() != n * PLAYERS * OBS_SIZE || live.len() != n * PLAYERS {
            return Err(PyValueError::new_err("array sizes don't match"));
        }
        py.detach(|| {
            self.envs
                .par_iter()
                .zip(obs.par_chunks_mut(PLAYERS * OBS_SIZE))
                .zip(live.par_chunks_mut(PLAYERS))
                .for_each(|((env, o), l)| {
                    for p in 0..PLAYERS {
                        obs::write(&env.game, p, &mut o[p * OBS_SIZE..(p + 1) * OBS_SIZE]);
                        l[p] = env.kinds[p] == LEARNER && env.game.players[p].alive;
                    }
                });
        });
        Ok(())
    }

    /// Seat kinds of every game, [n, 8].
    fn kinds(&self, mut out: PyReadwriteArray2<i64>) -> PyResult<()> {
        let out = out.as_slice_mut()?;
        for (o, env) in out.chunks_mut(PLAYERS).zip(&self.envs) {
            for p in 0..PLAYERS {
                o[p] = env.kinds[p] as i64;
            }
        }
        Ok(())
    }

    /// Card names by id, for logging.
    fn card_names(&self) -> Vec<String> {
        self.data.cards.cards.iter().map(|c| c.name.clone()).collect()
    }
}

#[pymodule]
fn bgpy(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<VecEnv>()?;
    m.add("OBS_SIZE", OBS_SIZE)?;
    m.add("N_ACTIONS", N_ACTIONS)?;
    m.add("PLAYERS", PLAYERS)?;
    m.add("SCALARS", obs::SCALARS)?;
    m.add("MINION_FEATURES", obs::MINION_FEATURES)?;
    m.add("OPP_SCALARS", obs::OPP_SCALARS)?;
    m.add("BOARD", bgcore::BOARD)?;
    m.add("HAND", bgcore::HAND)?;
    m.add("SHOP", bgcore::SHOP)?;
    m.add("DISCOVER", bgcore::DISCOVER)?;
    m.add("LEARNER", LEARNER)?;
    m.add("SNAPSHOT", SNAPSHOT)?;
    m.add("RULE", RULE)?;
    m.add("RANDOM", RANDOM)?;
    Ok(())
}
