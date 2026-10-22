//! Python bindings: a batch of games stepped together, for training.
//!
//! Every seat in a game has a kind. Standard, rush and random seats are played
//! here in Rust. Learner and snapshot seats are played from Python: after every
//! call each game is waiting on one of them, and `observe` says which.
//!
//! Python owns the numpy arrays and passes them in; Rust fills them. Nothing
//! is allocated per step.

// Seat numbers index several per-seat arrays at once.
#![allow(clippy::needless_range_loop)]

use std::sync::Arc;

use bgcore::bots::{random_action, rush_action, standard_action};
use bgcore::obs::{self, OBS_SIZE};
use bgcore::{ACTION_KINDS, Action, Data, Game, MAX_TIER, N_ACTIONS, PLAYERS, Rng, TRIBES};
use numpy::{PyReadonlyArray1, PyReadwriteArray1, PyReadwriteArray2, PyReadwriteArray3};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use rayon::prelude::*;

const LEARNER: u8 = 0;
const SNAPSHOT: u8 = 1;
const STANDARD: u8 = 2;
const RUSH: u8 = 3;
const RANDOM: u8 = 4;
const SEAT_KINDS: usize = 5;

struct Env {
    game: Game,
    kinds: [u8; PLAYERS],
    rng: Rng,
    /// Seats whose placement was already reported.
    reported: [bool; PLAYERS],
}

impl Env {
    fn new(data: &Arc<Data>, seed: u64, lineup: &[u8; SEAT_KINDS]) -> Env {
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
                STANDARD => standard_action(&self.game),
                RUSH => rush_action(&self.game),
                RANDOM => random_action(&self.game, &mut self.rng),
                _ => return,
            };
            self.game.step(a).expect("bot picked an illegal action");
        }
    }

    /// Write the placement, tier turns and final board tribe counts of every
    /// seat that finished since the last call.
    fn report(&mut self, places: &mut [i8], tier_turns: &mut [i8], tribes: &mut [i8]) {
        let nt = MAX_TIER as usize;
        for p in 0..PLAYERS {
            let pl = &self.game.players[p];
            if pl.placement != 0 && !self.reported[p] {
                places[p] = pl.placement as i8;
                for t in 0..nt {
                    tier_turns[p * nt + t] = pl.tier_turns[t] as i8;
                }
                for k in 0..TRIBES {
                    tribes[p * TRIBES + k] = pl.final_tribes[k] as i8;
                }
                self.reported[p] = true;
            }
        }
    }
}

#[pyclass]
struct VecEnv {
    data: Arc<Data>,
    envs: Vec<Env>,
    lineup: [u8; SEAT_KINDS],
    next_seed: u64,
}

fn check_lineup(lineup: [u8; SEAT_KINDS]) -> PyResult<[u8; SEAT_KINDS]> {
    if lineup.iter().map(|&n| n as usize).sum::<usize>() != PLAYERS || lineup[0] == 0 {
        return Err(PyValueError::new_err(
            "lineup is (learner, snapshot, standard, rush, random) seats, 8 in total, at least 1 learner",
        ));
    }
    Ok(lineup)
}

#[pymethods]
impl VecEnv {
    /// `lineup`: how many seats of each kind (learner, snapshot, standard,
    /// rush, random) a new game gets, shuffled into random seats.
    #[new]
    #[pyo3(signature = (n, seed, lineup, cards_toml=None, rules_toml=None))]
    fn new(
        n: usize,
        seed: u64,
        lineup: [u8; SEAT_KINDS],
        cards_toml: Option<&str>,
        rules_toml: Option<&str>,
    ) -> PyResult<VecEnv> {
        let lineup = check_lineup(lineup)?;
        let data = match (cards_toml, rules_toml) {
            (None, None) => Data::bundled(),
            (Some(c), Some(r)) => Data::from_toml(c, r).map_err(PyValueError::new_err)?,
            _ => {
                return Err(PyValueError::new_err("give both cards_toml and rules_toml, or neither"));
            }
        };
        let data = Arc::new(data);
        let seed = seed.wrapping_mul(1 << 32);
        let mut envs: Vec<Env> =
            (0..n as u64).map(|i| Env::new(&data, seed + i, &lineup)).collect();
        envs.par_iter_mut().for_each(Env::advance);
        Ok(VecEnv { data, envs, lineup, next_seed: seed + n as u64 })
    }

    /// The lineup for games that start from now on.
    fn set_lineup(&mut self, lineup: [u8; SEAT_KINDS]) -> PyResult<()> {
        self.lineup = check_lineup(lineup)?;
        Ok(())
    }

    #[getter]
    fn n(&self) -> usize {
        self.envs.len()
    }

    /// Take `actions[i]` for the waiting seat of game i. Fills
    /// `places[i, seat]`, `tier_turns[i, seat, 0..6]` and
    /// `tribes[i, seat, 0..3]` for every seat knocked out or finished during
    /// this call (0 for the rest), and `reset[i]` for games that ended and
    /// restarted. Raises on an illegal action.
    fn step(
        &mut self,
        py: Python<'_>,
        actions: PyReadonlyArray1<i64>,
        mut places: PyReadwriteArray2<i8>,
        mut tier_turns: PyReadwriteArray3<i8>,
        mut tribes: PyReadwriteArray3<i8>,
        mut reset: PyReadwriteArray1<bool>,
    ) -> PyResult<()> {
        let n = self.envs.len();
        let nt = MAX_TIER as usize;
        let actions = actions.as_slice()?;
        let places = places.as_slice_mut()?;
        let tier_turns = tier_turns.as_slice_mut()?;
        let tribes = tribes.as_slice_mut()?;
        let reset = reset.as_slice_mut()?;
        if actions.len() != n
            || places.len() != n * PLAYERS
            || tier_turns.len() != n * PLAYERS * nt
            || tribes.len() != n * PLAYERS * TRIBES
            || reset.len() != n
        {
            return Err(PyValueError::new_err("array sizes don't match the number of games"));
        }
        let (data, lineup, base) = (&self.data, self.lineup, self.next_seed);
        let envs = &mut self.envs;
        let bad = py.detach(|| {
            envs.par_iter_mut()
                .zip(actions)
                .zip(places.par_chunks_mut(PLAYERS))
                .zip(tier_turns.par_chunks_mut(PLAYERS * nt))
                .zip(tribes.par_chunks_mut(PLAYERS * TRIBES))
                .zip(reset.par_iter_mut())
                .enumerate()
                .map(|(i, (((((env, &a), places), tier_turns), tribes), reset))| {
                    places.fill(0);
                    tier_turns.fill(0);
                    tribes.fill(0);
                    *reset = false;
                    let Some(a) = usize::try_from(a).ok().and_then(Action::from_index) else {
                        return Some(i);
                    };
                    if env.game.step(a).is_err() {
                        return Some(i);
                    }
                    env.advance();
                    env.report(places, tier_turns, tribes);
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
            Some(i) => {
                Err(PyValueError::new_err(format!("illegal action {} in game {i}", actions[i])))
            }
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
        if obs.len() != n * OBS_SIZE
            || mask.len() != n * N_ACTIONS
            || seat.len() != n
            || kind.len() != n
        {
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
    fn observe_all(
        &self,
        py: Python<'_>,
        mut obs: PyReadwriteArray3<f32>,
        mut live: PyReadwriteArray2<bool>,
    ) -> PyResult<()> {
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
    let n_cards = Data::bundled().cards.cards.len();
    m.add_class::<VecEnv>()?;
    m.add("OBS_SIZE", OBS_SIZE)?;
    m.add("N_ACTIONS", N_ACTIONS)?;
    m.add("N_CARDS", n_cards)?;
    m.add("PLAYERS", PLAYERS)?;
    m.add("MAX_TIER", MAX_TIER as usize)?;
    m.add("TRIBES", TRIBES)?;
    m.add("SCALARS", obs::SCALARS)?;
    m.add("SLOT_FEATURES", obs::SLOT_FEATURES)?;
    m.add("MINION_FEATURES", obs::MINION_FEATURES)?;
    m.add("OPP_SCALARS", obs::OPP_SCALARS)?;
    m.add("BOARD", bgcore::BOARD)?;
    m.add("HAND", bgcore::HAND)?;
    m.add("SHOP", bgcore::SHOP)?;
    m.add("SHOP_SPELL", obs::SHOP_SPELL)?;
    m.add("DISCOVER", bgcore::DISCOVER)?;
    m.add("OWN_SLOTS", obs::OWN_SLOTS)?;
    m.add("OPP_SLOTS", obs::OPP_SLOTS)?;
    m.add("TOTAL_SLOTS", obs::TOTAL_SLOTS)?;
    m.add("OWN_END", obs::OWN_END)?;
    m.add("OPP_BLOCK", obs::OPP_BLOCK)?;
    m.add("SHOP_START", obs::SHOP_START)?;
    m.add("SHOP_SPELL_START", obs::SHOP_SPELL_START)?;
    m.add("HAND_START", obs::HAND_START)?;
    m.add("BOARD_START", obs::BOARD_START)?;
    m.add("DISCOVER_START", obs::DISCOVER_START)?;
    m.add("SCALAR_GOLD", obs::SCALAR_GOLD)?;
    m.add("SCALAR_TIER", obs::SCALAR_TIER)?;
    m.add("SCALAR_UPGRADE_COST", obs::SCALAR_UPGRADE_COST)?;
    m.add("SCALAR_HEALTH", obs::SCALAR_HEALTH)?;
    m.add("SCALAR_TURN", obs::SCALAR_TURN)?;
    m.add("SCALAR_FROZEN", obs::SCALAR_FROZEN)?;
    m.add("SCALAR_PENDING_TARGET", obs::SCALAR_PENDING_TARGET)?;
    m.add("SCALAR_PENDING_TARGET_SOURCE", obs::SCALAR_PENDING_TARGET_SOURCE)?;
    m.add("SCALAR_PENDING_DISCOVER", obs::SCALAR_PENDING_DISCOVER)?;
    m.add("SCALAR_ACTIONS_LEFT", obs::SCALAR_ACTIONS_LEFT)?;
    m.add("SCALAR_ALIVE", obs::SCALAR_ALIVE)?;
    m.add("SCALAR_GHOST", obs::SCALAR_GHOST)?;
    m.add("SCALAR_VOLUMIZER_ATTACK", obs::SCALAR_VOLUMIZER_ATTACK)?;
    m.add("SCALAR_VOLUMIZER_HEALTH", obs::SCALAR_VOLUMIZER_HEALTH)?;
    m.add("SCALAR_BEETLE_ATTACK", obs::SCALAR_BEETLE_ATTACK)?;
    m.add("SCALAR_BEETLE_HEALTH", obs::SCALAR_BEETLE_HEALTH)?;
    m.add("SCALAR_PATIENT_SCOUT_TIER", obs::SCALAR_PATIENT_SCOUT_TIER)?;
    m.add("SCALAR_GOLD_NEXT_TURN", obs::SCALAR_GOLD_NEXT_TURN)?;
    m.add("SCALAR_EXTRA_MAX_GOLD", obs::SCALAR_EXTRA_MAX_GOLD)?;
    m.add("SCALAR_FREE_ROLLS", obs::SCALAR_FREE_ROLLS)?;
    m.add("SCALAR_SPELL_DISCOUNT", obs::SCALAR_SPELL_DISCOUNT)?;
    m.add("SCALAR_SPELL_ATTACK", obs::SCALAR_SPELL_ATTACK)?;
    m.add("SCALAR_SPELL_HEALTH", obs::SCALAR_SPELL_HEALTH)?;
    m.add("SCALAR_BOON_OF_BEETLES", obs::SCALAR_BOON_OF_BEETLES)?;
    m.add("SCALAR_BROOD_OF_NOZDORMU", obs::SCALAR_BROOD_OF_NOZDORMU)?;
    m.add("SCALAR_UPPER_HAND", obs::SCALAR_UPPER_HAND)?;
    m.add("SCALAR_LAST_SPELL", obs::SCALAR_LAST_SPELL)?;
    m.add("SCALAR_WON_LAST_COMBAT", obs::SCALAR_WON_LAST_COMBAT)?;
    m.add("SCALAR_LOST_LAST_COMBAT", obs::SCALAR_LOST_LAST_COMBAT)?;
    m.add("SCALAR_PENDING_CHOOSE_ONE", obs::SCALAR_PENDING_CHOOSE_ONE)?;
    m.add("SLOT_PRESENT", obs::SLOT_PRESENT)?;
    m.add("SLOT_CARD_ID", obs::SLOT_CARD_ID)?;
    m.add("SLOT_IS_SPELL", obs::SLOT_IS_SPELL)?;
    m.add("SLOT_ATTACK", obs::SLOT_ATTACK)?;
    m.add("SLOT_HEALTH", obs::SLOT_HEALTH)?;
    m.add("SLOT_GOLDEN", obs::SLOT_GOLDEN)?;
    m.add("SLOT_TIER", obs::SLOT_TIER)?;
    m.add("SLOT_BEAST", obs::SLOT_BEAST)?;
    m.add("SLOT_MECH", obs::SLOT_MECH)?;
    m.add("SLOT_NEUTRAL", obs::SLOT_NEUTRAL)?;
    m.add("SLOT_KEYWORDS", obs::SLOT_KEYWORDS)?;
    m.add("SLOT_MAGNETIZATIONS", obs::SLOT_MAGNETIZATIONS)?;
    m.add("SLOT_VOLUMIZERS", obs::SLOT_VOLUMIZERS)?;
    m.add("SLOT_IMPROVEMENTS", obs::SLOT_IMPROVEMENTS)?;
    m.add("SLOT_FROM_POOL", obs::SLOT_FROM_POOL)?;
    m.add("SLOT_WINNERS_BREAD", obs::SLOT_WINNERS_BREAD)?;
    m.add("OPP_ALIVE", obs::OPP_ALIVE)?;
    m.add("OPP_HEALTH", obs::OPP_HEALTH)?;
    m.add("OPP_TIER", obs::OPP_TIER)?;
    m.add("OPP_SEEN", obs::OPP_SEEN)?;
    m.add("OPP_TURNS_SINCE", obs::OPP_TURNS_SINCE)?;
    m.add("OPP_NEXT", obs::OPP_NEXT)?;
    m.add("ACTION_KINDS", ACTION_KINDS.to_vec())?;
    m.add("END_TURN", Action::EndTurn.index())?;
    m.add("LEARNER", LEARNER)?;
    m.add("SNAPSHOT", SNAPSHOT)?;
    m.add("STANDARD", STANDARD)?;
    m.add("RUSH", RUSH)?;
    m.add("RANDOM", RANDOM)?;
    m.add("SEAT_KINDS", SEAT_KINDS)?;
    Ok(())
}
