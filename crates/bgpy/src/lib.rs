//! Python bindings: a batch of games stepped together, for training.
//!
//! Every seat in a game has a kind. Standard, rush, random and real seats are
//! played here in Rust. Learner and snapshot seats are played from Python:
//! after every call each game is waiting on one of them, and `observe` says
//! which.
//!
//! Python owns the numpy arrays and passes them in; Rust fills them. Nothing
//! is allocated per step.

// Seat numbers index several per-seat arrays at once.
#![allow(clippy::needless_range_loop)]

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use bgcore::bots::{random_action, rush_action, standard_action};
use bgcore::comps::Comps;
use bgcore::overlay::{Overlay, OVERLAY_FLAGS, ShellLine};
use bgcore::obs::{self, OBS_SIZE, OldBoards};
use bgcore::real_boards::{
    RealBoard, RealBoardSpec, RealBoards, RealCounters, RealDeitySpec, RealMinionSpec,
};
use bgcore::shop_replay::{SeatedStep, ShopSnapshot};
use bgcore::search::Position;
use bgcore::{
    ACTION_KINDS, Action, BOARD, Data, Game, Keywords, MAX_TIER, N_ACTIONS, PLAYERS, Rng, TRIBES,
};
use numpy::{PyReadonlyArray1, PyReadwriteArray1, PyReadwriteArray2, PyReadwriteArray3};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use rayon::prelude::*;

const LEARNER: u8 = 0;
const SNAPSHOT: u8 = 1;
const STANDARD: u8 = 2;
const RUSH: u8 = 3;
const RANDOM: u8 = 4;
const REAL: u8 = 5;
const SEAT_KINDS: usize = 6;

/// How many threads the overlay's line search gets. The game steps and
/// observation use the global pool (one thread per core); the line search on
/// the same pool wakes every worker, and the spinning workers then delay the
/// network forward that follows. A small dedicated pool keeps that work off
/// the global pool and leaves the forward pass alone.
const OVERLAY_THREADS: usize = 4;

static OVERLAY_POOL: OnceLock<rayon::ThreadPool> = OnceLock::new();

fn overlay_pool() -> &'static rayon::ThreadPool {
    OVERLAY_POOL.get_or_init(|| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(OVERLAY_THREADS)
            .stack_size(64 << 20)
            .build()
            .expect("the overlay thread pool")
    })
}

/// One minion for `VecEnv(real_boards=..)`, as train/real_boards.py loads
/// it: (name, attack, health, golden, keywords, numbers).
type PyMinion = (String, i32, i32, bool, Vec<String>, Vec<i32>, Vec<String>);

/// One board for `VecEnv(real_boards=..)`:
/// (turn, tier, minions, hand, counters, deity). `counters` holds the
/// side-wide numbers the log showed, by the board file's keys (a missing one
/// is the base value); `deity` is (name, attack, health, golden) or None.
type PyBoard = (
    u32,
    u8,
    Vec<PyMinion>,
    Vec<PyMinion>,
    HashMap<String, i32>,
    Option<(String, i32, i32, bool)>,
);

type PyBoards = Vec<PyBoard>;

/// A board's minions (or hand) as `RealBoards::build` takes them.
fn minion_specs(list: Vec<PyMinion>) -> Vec<RealMinionSpec> {
    list.into_iter()
        .map(|(name, attack, health, golden, keywords, numbers, dark_gifts)| RealMinionSpec {
            name,
            attack,
            health,
            golden,
            keywords,
            numbers,
            dark_gifts,
            // Training hands boards in from the beasts' names and numbers; the
            // Rapid Reanimation mark, the no-reward mark and a Holy Vanguard
            // aura pair are the replay's (they come from the action files).
            reanimate: false,
            no_reward: false,
            dark_gift_states: Vec::new(),
            vanguard_aura: [0, 0],
        })
        .collect()
}

/// The specs `VecEnv(real_boards=..)` and the era-turn runs read: one board
/// per (turn, tier, minions, hand, counters, deity) tuple from
/// train/real_boards.py.
fn real_board_specs(raw: PyBoards) -> Vec<RealBoardSpec> {
    raw.into_iter()
        .map(|(turn, tier, minions, hand, counters, deity)| RealBoardSpec {
            turn,
            tier,
            minions: minion_specs(minions),
            hand: minion_specs(hand),
            counters: real_counters(counters),
            deity: deity.map(|(name, attack, health, golden)| RealDeitySpec {
                name,
                attack,
                health,
                golden,
            }),
            // The rest of the file's board (the game it came from, its side
            // and result, Health, armor, power, trinkets, quest rewards) is for
            // the fight check, not for a seat.
            ..RealBoardSpec::default()
        })
        .collect()
}

/// A board's side-wide numbers, by the board file's keys.
fn real_counters(map: HashMap<String, i32>) -> RealCounters {
    let mut counters = RealCounters::default();
    for (key, value) in &map {
        counters.set(key, *value);
    }
    counters
}

struct Env {
    game: Game,
    kinds: [u8; PLAYERS],
    rng: Rng,
    /// Seats whose placement was already reported.
    reported: [bool; PLAYERS],
    /// Each seat's board card ids at its last decision, -1 for an empty slot.
    /// `remember_boards` updates this; `final_boards` returns it after a
    /// game ends.
    last_board: [[i16; BOARD]; PLAYERS],
    /// The Shell line in progress, per seat.
    shell_lines: [ShellLine; PLAYERS],
}

impl Env {
    fn new(
        data: &Arc<Data>,
        seed: u64,
        lineup: &[u8; SEAT_KINDS],
        boards: Option<&Arc<RealBoards>>,
    ) -> Env {
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
        let real: Vec<usize> = (0..PLAYERS).filter(|&p| kinds[p] == REAL).collect();
        let game = if real.is_empty() {
            Game::new_random_heroes(data.clone(), seed)
        } else {
            Game::new_random_heroes_real(
                data.clone(),
                seed,
                &real,
                boards.expect("real seats need real boards").clone(),
            )
        };
        Env {
            game,
            kinds,
            rng,
            reported: [false; PLAYERS],
            last_board: [[-1; BOARD]; PLAYERS],
            shell_lines: [ShellLine::default(); PLAYERS],
        }
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
            if self.game.step(a).is_err() {
                // Name the state, not only the action: an illegal scripted
                // action is a bug in the bot or the engine, and this is all a
                // training run leaves behind (2026-11-02, ten test batches).
                let pl = &self.game.players[p];
                let names = |ms: &[bgcore::Minion]| -> Vec<String> {
                    ms.iter().map(|m| self.game.data.cards.get(m.card).name.clone()).collect()
                };
                panic!(
                    "bot picked an illegal action {a:?}: turn {} player {p} gold {} tier {} \
                     power {:?} board {:?} hand {:?} shop {:?} spell {:?} \
                     pending_target {:?} pending_shop_target {:?} pending_choose_one {:?} \
                     pending_discard {:?} discover {:?}",
                    self.game.turn, pl.gold, pl.tier, pl.power,
                    names(&pl.board), names(&pl.hand), names(&pl.shop),
                    pl.shop_spell.map(|c| self.game.data.cards.get(c).name.clone()),
                    pl.pending_target, pl.pending_shop_target, pl.pending_choose_one,
                    pl.pending_discard, pl.discover.len(),
                );
            }
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
    boards: Option<Arc<RealBoards>>,
    next_seed: u64,
    /// Whether opponents' last-seen boards are written (see `obs::OldBoards`).
    old_boards: OldBoards,
    /// The comp rules, when the overlay is on.
    overlay_rules: Option<Arc<Overlay>>,
    /// The last completed game's final board card ids per seat, -1 empty.
    finals: Vec<[[i16; BOARD]; PLAYERS]>,
}

fn check_lineup(lineup: [u8; SEAT_KINDS], boards: bool) -> PyResult<[u8; SEAT_KINDS]> {
    if lineup.iter().map(|&n| n as usize).sum::<usize>() != PLAYERS || lineup[0] == 0 {
        return Err(PyValueError::new_err(
            "lineup is (learner, snapshot, standard, rush, random, real) seats, 8 in total, at least 1 learner",
        ));
    }
    if lineup[REAL as usize] > 0 && !boards {
        return Err(PyValueError::new_err(
            "real seats need boards: pass real_boards= (train/real_boards.py loads them)",
        ));
    }
    Ok(lineup)
}

#[pymethods]
impl VecEnv {
    /// `lineup`: how many seats of each kind (learner, snapshot, standard,
    /// rush, random, real) a new game gets, shuffled into random seats.
    /// `real_boards`: the boards real seats play, as train/real_boards.py
    /// loads them ((turn, tier, minions, hand, counters, deity) with
    /// (name, attack, health, golden, keywords, numbers) each); required
    /// when the lineup has real seats.
    /// `old_boards`: "keep" (the default) writes every opponent's last-seen
    /// board in the observation, however old; "drop" writes it only for the
    /// opponent fought last turn and for the one fought this turn when the
    /// board is at most `OLD_BOARD_AGE` turns old, zeros for the rest. The
    /// observation keeps its size either way (see `bgcore::obs::OldBoards`).
    #[new]
    #[pyo3(signature = (n, seed, lineup, cards_toml=None, rules_toml=None, heroes_toml=None, real_boards=None, old_boards=None))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        n: usize,
        seed: u64,
        lineup: [u8; SEAT_KINDS],
        cards_toml: Option<&str>,
        rules_toml: Option<&str>,
        heroes_toml: Option<&str>,
        real_boards: Option<PyBoards>,
        old_boards: Option<&str>,
    ) -> PyResult<VecEnv> {
        let data = match (cards_toml, rules_toml, heroes_toml) {
            (None, None, None) => Data::bundled(),
            (Some(c), Some(r), h) => {
                Data::from_toml_with_heroes(c, r, h.unwrap_or("")).map_err(PyValueError::new_err)?
            }
            _ => {
                return Err(PyValueError::new_err(
                    "give cards_toml and rules_toml (heroes_toml optional), or none of them",
                ));
            }
        };
        let data = Arc::new(data);
        let boards = real_boards.map(|raw| {
            let specs = real_board_specs(raw);
            // Names that match no card are dropped here; train/real_boards.py
            // reports them (it knows both stages of the mapping), so this
            // stays quiet: every eval VecEnv would repeat the line.
            let (boards, _) = RealBoards::build(&specs, &data.cards, &data.gifts);
            Arc::new(boards)
        });
        let lineup = check_lineup(lineup, boards.is_some())?;
        let old_boards = match old_boards {
            None | Some("keep") => OldBoards::Keep,
            Some("drop") => OldBoards::Drop,
            Some(other) => {
                return Err(PyValueError::new_err(format!(
                    "old_boards is keep or drop, not {other}"
                )));
            }
        };
        let seed = seed.wrapping_mul(1 << 32);
        let mut envs: Vec<Env> =
            (0..n as u64).map(|i| Env::new(&data, seed + i, &lineup, boards.as_ref())).collect();
        envs.par_iter_mut().for_each(Env::advance);
        Ok(VecEnv {
            data,
            envs,
            lineup,
            boards,
            next_seed: seed + n as u64,
            old_boards,
            overlay_rules: None,
            finals: vec![[[-1; BOARD]; PLAYERS]; n],
        })
    }

    /// The lineup for games that start from now on.
    fn set_lineup(&mut self, lineup: [u8; SEAT_KINDS]) -> PyResult<()> {
        self.lineup = check_lineup(lineup, self.boards.is_some())?;
        Ok(())
    }

    #[getter]
    fn n(&self) -> usize {
        self.envs.len()
    }

    /// Load the comp library for the overlay. `bonus` is B, the payoff
    /// purchase/play bonus. `dominance` says whether the dominance layer runs
    /// (it is on by default). The overlay is off until this is called.
    #[pyo3(signature = (comps, bonus, dominance=true))]
    fn set_overlay(&mut self, comps: &str, bonus: f32, dominance: bool) -> PyResult<()> {
        let text = std::fs::read_to_string(comps)
            .map_err(|e| PyValueError::new_err(format!("{comps}: {e}")))?;
        let comps = Comps::from_toml(&text, &self.data).map_err(PyValueError::new_err)?;
        let mut rules = Overlay::new(comps, bonus, &self.data);
        rules.set_dominance(dominance);
        self.overlay_rules = Some(Arc::new(rules));
        Ok(())
    }

    fn clear_overlay(&mut self) {
        self.overlay_rules = None;
    }

    /// The overlay's forced action and logit bonus for every game and seat,
    /// in one call. `forced[i, p]` is an action index or -1; `bonus[i, p, a]`
    /// is added to the logits (and may be -inf, which masks the action);
    /// `flags[i, p, f]` holds the `overlay` flag constants. Only the current
    /// player of each game is computed, because the engine's legality check
    /// is current-player only; the other seats are -1 and zero.
    #[pyo3(signature = (forced, bonus, flags))]
    fn overlay(
        &mut self,
        py: Python<'_>,
        mut forced: PyReadwriteArray2<i64>,
        mut bonus: PyReadwriteArray3<f32>,
        mut flags: PyReadwriteArray3<i32>,
    ) -> PyResult<()> {
        let n = self.envs.len();
        let forced = forced.as_slice_mut()?;
        let bonus = bonus.as_slice_mut()?;
        let flags = flags.as_slice_mut()?;
        if forced.len() != n * PLAYERS
            || bonus.len() != n * PLAYERS * N_ACTIONS
            || flags.len() != n * PLAYERS * OVERLAY_FLAGS
        {
            return Err(PyValueError::new_err("overlay array sizes don't match"));
        }
        let rules = self.overlay_rules.clone();
        let envs = &mut self.envs;
        py.detach(|| {
            overlay_pool().install(|| {
                envs
                    .par_iter_mut()
                    .zip(forced.par_chunks_mut(PLAYERS))
                    .zip(bonus.par_chunks_mut(PLAYERS * N_ACTIONS))
                    .zip(flags.par_chunks_mut(PLAYERS * OVERLAY_FLAGS))
                    .for_each(|(((env, forced), bonus), flags)| {
                        forced.fill(-1);
                        bonus.fill(0.0);
                        flags.fill(0);
                        let Some(rules) = &rules else { return };
                        let Some(p) = env.game.current_player() else { return };
                        let mut row = [0.0f32; N_ACTIONS];
                        let mut f = [0i32; OVERLAY_FLAGS];
                        if let Some(a) = rules.decide(&env.game, p, &mut env.shell_lines[p], &mut row, &mut f) {
                            forced[p] = a.index() as i64;
                        }
                        bonus[p * N_ACTIONS..(p + 1) * N_ACTIONS].copy_from_slice(&row);
                        flags[p * OVERLAY_FLAGS..(p + 1) * OVERLAY_FLAGS].copy_from_slice(&f);
                    });
            });
        });
        Ok(())
    }

    /// Snapshot the current player's board card ids in every game. Call it
    /// after `observe` and before `step`; the last snapshot before combat is
    /// the board the player finished the turn with. Eval-only bookkeeping;
    /// training does not need it.
    fn remember_boards(&mut self) {
        self.envs.par_iter_mut().for_each(|env| {
            let Some(p) = env.game.current_player() else { return };
            let mut row = [-1i16; BOARD];
            for (k, m) in env.game.players[p].board.iter().enumerate() {
                row[k] = m.card as i16;
            }
            env.last_board[p] = row;
        });
    }

    /// The final board card ids of the last completed game, for every seat:
    /// `[game][seat][slot]`, -1 for an empty slot. Read after `step` reports
    /// a reset.
    fn final_boards(&self) -> Vec<Vec<Vec<i16>>> {
        self.finals
            .iter()
            .map(|game| game.iter().map(|row| row.to_vec()).collect())
            .collect()
    }

    /// Take `actions[i]` for the waiting seat of game i. Fills
    /// `places[i, seat]`, `tier_turns[i, seat, 0..6]` and
    /// `tribes[i, seat, 0..TRIBES]` (the ten tribes in `Tribe::EVERY` order,
    /// then neutral) for every seat knocked out or finished during this call,
    /// including a hero knocked out by its own effect damage mid-turn (0 for
    /// the rest), and `reset[i]` for games that ended and restarted. Raises
    /// on an illegal action.
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
        let (data, lineup, boards, base) = (&self.data, self.lineup, &self.boards, self.next_seed);
        let envs = &mut self.envs;
        let finals = &mut self.finals;
        let bad = py.detach(|| {
            envs.par_iter_mut()
                .zip(actions)
                .zip(places.par_chunks_mut(PLAYERS))
                .zip(tier_turns.par_chunks_mut(PLAYERS * nt))
                .zip(tribes.par_chunks_mut(PLAYERS * TRIBES))
                .zip(reset.par_iter_mut())
                .zip(finals.par_iter_mut())
                .enumerate()
                .map(|(i, ((((((env, &a), places), tier_turns), tribes), reset), finals))| {
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
                        *finals = env.last_board;
                        *env = Env::new(data, base + i as u64, &lineup, boards.as_ref());
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
        let old_boards = self.old_boards;
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
                    obs::write_with(&env.game, p, o, old_boards);
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
        let old_boards = self.old_boards;
        py.detach(|| {
            self.envs
                .par_iter()
                .zip(obs.par_chunks_mut(PLAYERS * OBS_SIZE))
                .zip(live.par_chunks_mut(PLAYERS))
                .for_each(|((env, o), l)| {
                    for p in 0..PLAYERS {
                        obs::write_with(
                            &env.game,
                            p,
                            &mut o[p * OBS_SIZE..(p + 1) * OBS_SIZE],
                            old_boards,
                        );
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

    /// Every seat of game `i` as it stands now, for looking at what bots
    /// build (train/show_boards.py): (kind, alive, placement, tier, hero,
    /// board), board minions as (name, golden, attack, health, keywords).
    /// Read only; not used in training.
    #[allow(clippy::type_complexity)]
    fn seats(
        &self,
        i: usize,
    ) -> PyResult<Vec<(u8, bool, u8, u8, String, Vec<(String, bool, i32, i32, String)>)>> {
        let env = self.envs.get(i).ok_or_else(|| PyValueError::new_err("no such game"))?;
        let data = &self.data;
        Ok((0..PLAYERS)
            .map(|p| {
                let pl = &env.game.players[p];
                let hero = pl
                    .hero
                    .and_then(|h| data.heroes.heroes.get(h as usize))
                    .map(|h| h.name.clone())
                    .unwrap_or_default();
                let board = pl
                    .board
                    .iter()
                    .map(|m| {
                        let kw: Vec<&str> = Keywords::ALL
                            .iter()
                            .filter(|(k, _)| m.keywords.has(*k))
                            .map(|(_, n)| *n)
                            .collect();
                        (
                            data.cards.cards[m.card as usize].name.clone(),
                            m.golden,
                            m.attack,
                            m.health,
                            kw.join(","),
                        )
                    })
                    .collect();
                (env.kinds[p], pl.alive, pl.placement, pl.tier, hero, board)
            })
            .collect())
    }

    /// Card names by id, for logging and for matching a checkpoint's card
    /// embedding by name.
    fn card_names(&self) -> Vec<String> {
        self.data.cards.cards.iter().map(|c| c.name.clone()).collect()
    }

    /// Hero names by index, for matching a checkpoint's hero embedding by
    /// name.
    fn hero_names(&self) -> Vec<String> {
        self.data.heroes.heroes.iter().map(|h| h.name.clone()).collect()
    }

    /// A search batch holding a copy of game `i` at its waiting seat
    /// (train/search.py). The copy has its own game, RNG and pool: nothing the
    /// search does to it touches the game here, which is stepped with the
    /// action the search picks, exactly as without the search.
    fn search_copy(&self, i: usize) -> PyResult<SearchBatch> {
        let env = self.envs.get(i).ok_or_else(|| PyValueError::new_err("no such game"))?;
        let p = env.game.current_player().ok_or_else(|| PyValueError::new_err("game over"))?;
        Ok(SearchBatch::new(env.game.clone(), p, self.data.clone(), self.boards.clone()))
    }
}

// ---- the search (train/search.py) ----

/// One slot of a `SearchBatch`: a game to try one candidate on, and the
/// position its line is compared against (`SearchBatch::mark_all`).
struct SearchSlot {
    game: Game,
    mark: Option<Position>,
}

/// The search's copies of one game's shop decision: every candidate gets a
/// slot, and the slots are stepped together so the network sees one big
/// batch. `docs/briefs/` "Search v1", `crates/bgcore/src/search.rs`.
#[pyclass]
struct SearchBatch {
    data: Arc<Data>,
    /// The seat the search plays, the same in every slot.
    p: usize,
    /// The turn the decision belongs to: every fight of a candidate is one of
    /// this turn's real boards.
    turn: u32,
    /// Every pool card at its full copies: a playout's Refresh puts them back
    /// (`bgcore::search::full_pool`).
    full_pool: Vec<u8>,
    /// The real games' boards every candidate of this decision fights, by
    /// index into `boards_at(turn)`. Empty without real boards.
    boards: Option<Arc<RealBoards>>,
    fight_boards: Vec<usize>,
    /// The seed the fights start from: the same boards and seeds for every
    /// candidate of one decision.
    seed: u64,
    slots: Vec<Option<SearchSlot>>,
}

impl SearchBatch {
    fn new(game: Game, p: usize, data: Arc<Data>, boards: Option<Arc<RealBoards>>) -> SearchBatch {
        let turn = game.turn;
        let full_pool = bgcore::search::full_pool(&game);
        // The opponent this player is paired against, for the boards to
        // fight: its tier and the tribe its leaderboard line shows.
        let (opp_tier, opp_tribe) = match game.players[p].opponent {
            Some(bgcore::Opponent::Player(q)) => {
                let line = game.players[q as usize].leaderboard;
                (Some(game.players[q as usize].tier), if line.mixed { None } else { line.top })
            }
            Some(bgcore::Opponent::Ghost(_)) => (game.ghost.as_ref().map(|g| g.tier), None),
            None => (None, None),
        };
        let fight_boards = boards.as_ref().map_or_else(Vec::new, |b| {
            bgcore::search::fight_boards(&data, b, turn, opp_tier, opp_tribe, FIGHT_BOARDS)
        });
        SearchBatch {
            data,
            p,
            turn,
            full_pool,
            boards,
            fight_boards,
            seed: 0,
            // Slot 0 is the position every candidate starts from.
            slots: vec![Some(SearchSlot { game, mark: None })],
        }
    }

    fn slot(&self, i: usize) -> PyResult<&SearchSlot> {
        self.slots
            .get(i)
            .and_then(|s| s.as_ref())
            .ok_or_else(|| PyValueError::new_err(format!("slot {i} is empty")))
    }

    fn slot_mut(&mut self, i: usize) -> PyResult<&mut SearchSlot> {
        self.slots
            .get_mut(i)
            .and_then(|s| s.as_mut())
            .ok_or_else(|| PyValueError::new_err(format!("slot {i} is empty")))
    }
}

/// Boards per decision the search fights, when Python has not set
/// `SearchBatch.fights` (the brief's default).
const FIGHT_BOARDS: usize = 20;

#[pymethods]
impl SearchBatch {
    /// The seat the search plays.
    #[getter]
    fn p(&self) -> usize {
        self.p
    }

    #[getter]
    fn turn(&self) -> u32 {
        self.turn
    }

    /// How many real boards each candidate fights (0 without real boards).
    #[getter]
    fn n_fights(&self) -> usize {
        self.fight_boards.len()
    }

    /// How many boards of this turn's bucket the fight draws from.
    #[getter]
    fn bucket(&self) -> usize {
        self.boards.as_ref().map_or(0, |b| b.boards_at(self.turn).len())
    }

    /// Take `n` boards per candidate, evenly spread, and a seed for the
    /// fights. Every candidate of this batch fights the same boards with the
    /// same seeds.
    fn set_fights(&mut self, n: usize, seed: u64) {
        self.seed = seed;
        let Some(boards) = self.boards.as_ref() else {
            self.fight_boards = Vec::new();
            return;
        };
        let turn = self.turn;
        let (tier, tribe) = self.opponent_hint();
        self.fight_boards = bgcore::search::fight_boards(&self.data, boards, turn, tier, tribe, n);
    }

    fn len(&self) -> usize {
        self.slots.len()
    }

    /// A fresh batch with a copy of slot 0: the position again, with none of
    /// the scratch slots (a turn to play from it, in train/eval_search.py).
    fn copy(&self) -> PyResult<SearchBatch> {
        let game = self.slot(0)?.game.clone();
        Ok(SearchBatch::new(game, self.p, self.data.clone(), self.boards.clone()))
    }

    /// Add `n` empty slots.
    fn grow(&mut self, n: usize) {
        for _ in 0..n {
            self.slots.push(None);
        }
    }

    /// Drop every slot from `n` on.
    fn truncate(&mut self, n: usize) {
        self.slots.truncate(n);
    }

    /// Slot `dst` becomes a copy of slot `src` (a new game copy: from here on
    /// the two step apart).
    fn set(&mut self, dst: usize, src: usize) -> PyResult<()> {
        let game = self.slot(src)?.game.clone();
        let mark = None;
        let slot = self
            .slots
            .get_mut(dst)
            .ok_or_else(|| PyValueError::new_err("no such slot"))?;
        *slot = Some(SearchSlot { game, mark });
        Ok(())
    }

    /// Append a copy of slot `src` and return its index.
    fn push_copy(&mut self, src: usize) -> PyResult<usize> {
        let game = self.slot(src)?.game.clone();
        self.slots.push(Some(SearchSlot { game, mark: None }));
        Ok(self.slots.len() - 1)
    }

    /// Take one action index on one slot. Raises when the action is not
    /// legal: the caller built a line the engine refuses.
    fn act(&mut self, slot: usize, actions: Vec<i64>) -> PyResult<()> {
        let s = self.slot_mut(slot)?;
        for a in actions {
            let action = usize::try_from(a).ok().and_then(Action::from_index);
            let Some(action) = action else {
                return Err(PyValueError::new_err(format!("no action {a}")));
            };
            s.game
                .step(action)
                .map_err(|_| PyValueError::new_err(format!("illegal action {a} in slot {slot}")))?;
        }
        Ok(())
    }

    /// Take one action index per slot, -1 to leave a slot alone. A Refresh
    /// draws from the full pool (a real player only roughly knows which
    /// copies are gone); a slot whose shop turn has ended is left alone.
    fn step_all(&mut self, py: Python<'_>, actions: PyReadonlyArray1<i64>) -> PyResult<()> {
        let actions = actions.as_slice()?;
        if actions.len() != self.slots.len() {
            return Err(PyValueError::new_err("one action per slot"));
        }
        let p = self.p;
        let pool = &self.full_pool;
        let bad = py.detach(|| {
            self.slots
                .par_iter_mut()
                .zip(actions.par_iter())
                .enumerate()
                .filter_map(|(i, (slot, &a))| {
                    let slot = slot.as_mut()?;
                    if a < 0 || !slot.game.shopping(p) {
                        return None;
                    }
                    let Some(action) = usize::try_from(a).ok().and_then(Action::from_index) else {
                        return Some(i);
                    };
                    if matches!(action, Action::Roll) {
                        slot.game.pool.copy_from_slice(pool);
                        slot.game.pool_untracked = true;
                    }
                    if slot.game.step(action).is_err() {
                        return Some(i);
                    }
                    None
                })
                .min()
        });
        match bad {
            Some(i) => Err(PyValueError::new_err(format!("illegal search action in slot {i}"))),
            None => Ok(()),
        }
    }

    /// End the shop turn of the slots `which` marks (every slot when the
    /// array is empty), so a finished line sits at the position the coming
    /// fight is played from.
    fn end_all(&mut self, which: PyReadonlyArray1<bool>) -> PyResult<()> {
        let p = self.p;
        let which = which.as_slice()?;
        if !which.is_empty() && which.len() != self.slots.len() {
            return Err(PyValueError::new_err("one flag per slot"));
        }
        for (i, slot) in self.slots.iter_mut().enumerate() {
            let Some(slot) = slot else { continue };
            if !which.is_empty() && !which[i] {
                continue;
            }
            slot.game.end_shop_turn(p);
        }
        Ok(())
    }

    /// The observation, legal mask and state of every slot:
    /// `state` is 0 for a shop decision, 1 for an answer the network takes
    /// (a Discover, a target, a trinket or hero pick) and 2 for a finished
    /// turn or a game that is over.
    fn observe_all(
        &self,
        py: Python<'_>,
        mut obs: PyReadwriteArray2<f32>,
        mut mask: PyReadwriteArray2<bool>,
        mut state: PyReadwriteArray1<i8>,
    ) -> PyResult<()> {
        let n = self.slots.len();
        let obs = obs.as_slice_mut()?;
        let mask = mask.as_slice_mut()?;
        let state = state.as_slice_mut()?;
        if obs.len() != n * OBS_SIZE || mask.len() != n * N_ACTIONS || state.len() != n {
            return Err(PyValueError::new_err("array sizes don't match the number of slots"));
        }
        let p = self.p;
        py.detach(|| {
            self.slots
                .par_iter()
                .zip(obs.par_chunks_mut(OBS_SIZE))
                .zip(mask.par_chunks_mut(N_ACTIONS))
                .zip(state.par_iter_mut())
                .for_each(|(((slot, o), m), s)| {
                    let Some(slot) = slot else {
                        o.fill(0.0);
                        m.fill(false);
                        *s = 2;
                        return;
                    };
                    let g = &slot.game;
                    obs::write(g, p, o);
                    let m: &mut [bool; N_ACTIONS] = m.try_into().unwrap();
                    g.legal_mask(m);
                    *s = if g.shopping(p) {
                        0
                    } else if g.players[p].alive && !g.over && !g.players[p].done {
                        1
                    } else {
                        2
                    };
                });
        });
        Ok(())
    }

    /// What a slot's position holds, as the search reads it: Gold, Tavern
    /// tier, Health, armor, the upgrade price, free Refreshes, the freeze, and
    /// the hand, shop and board as `(card id, Attack, Health)` (the hand and
    /// board with golden). Read only.
    fn describe(
        &self,
        slot: usize,
    ) -> PyResult<Describe> {
        Ok(describe_player(&self.slot(slot)?.game, self.p))
    }

    /// The turn a slot's game is on.
    fn slot_turn(&self, slot: usize) -> PyResult<u32> {
        Ok(self.slot(slot)?.game.turn)
    }

    /// The card names of the game's card set, by id.
    fn card_names(&self) -> Vec<String> {
        self.data.cards.cards.iter().map(|c| c.name.clone()).collect()
    }

    /// Remember every slot's position, so `dominating` says which slots have
    /// got at least as good since.
    fn mark_all(&mut self) {
        let p = self.p;
        for slot in self.slots.iter_mut().flatten() {
            slot.mark = Some(Position::of(&slot.game, p));
        }
    }

    /// For every slot: does its position dominate the one it was marked at?
    /// A dominating slot is re-marked at its new position, so the next call
    /// judges from there. `blocked` (when asked for) says why not.
    fn dominating(
        &mut self,
        mut out: PyReadwriteArray1<bool>,
        mut better: PyReadwriteArray1<bool>,
    ) -> PyResult<()> {
        let n = self.slots.len();
        let out = out.as_slice_mut()?;
        let better = better.as_slice_mut()?;
        if out.len() != n || better.len() != n {
            return Err(PyValueError::new_err("array sizes don't match the number of slots"));
        }
        for (i, slot) in self.slots.iter_mut().enumerate() {
            let Some(slot) = slot.as_mut() else {
                out[i] = false;
                better[i] = false;
                continue;
            };
            let Some(mark) = slot.mark.as_ref() else {
                out[i] = false;
                better[i] = false;
                continue;
            };
            let d = mark.judge(&slot.game);
            out[i] = d.dominates;
            better[i] = d.better;
            if d.dominates {
                slot.mark = Some(Position::of(&slot.game, self.p));
            }
        }
        Ok(())
    }

    /// Judge slot `after` against slot `base`: does it dominate, is anything
    /// better, and what blocked it. The base is read fresh, so a caller that
    /// judged once and then changed the base gets what it asks for.
    fn dominance(&self, base: usize, after: usize) -> PyResult<(bool, bool, Vec<String>)> {
        let b = self.slot(base)?;
        let a = self.slot(after)?;
        let pos = Position::of(&b.game, self.p);
        let d = pos.judge(&a.game);
        Ok((d.dominates, d.better, d.blocked))
    }

    /// Mark one slot, for a line that starts from a position other than the
    /// batch's own.
    fn mark(&mut self, slot: usize) -> PyResult<()> {
        let p = self.p;
        let s = self.slot_mut(slot)?;
        s.mark = Some(Position::of(&s.game, p));
        Ok(())
    }

    /// The Health changes of the coming fight for every slot: the sum over
    /// the boards, 0 when the player wins or ties one. The same boards and
    /// the same seeds for every slot, so two candidates are compared on the
    /// same fights.
    fn fights(&self, py: Python<'_>, mut out: PyReadwriteArray1<i32>) -> PyResult<()> {
        let out = out.as_slice_mut()?;
        if out.len() != self.slots.len() {
            return Err(PyValueError::new_err("one number per slot"));
        }
        let Some(boards) = self.boards.as_ref() else {
            out.fill(0);
            return Ok(());
        };
        let all = boards.boards_at(self.turn);
        let picks = &self.fight_boards;
        if picks.is_empty() {
            out.fill(0);
            return Ok(());
        }
        let (data, p, seed) = (&self.data, self.p, self.seed);
        py.detach(|| {
            self.slots
                .par_iter()
                .zip(out.par_iter_mut())
                .for_each(|(slot, total)| {
                    let Some(slot) = slot else {
                        *total = 0;
                        return;
                    };
                    let g = &slot.game;
                    let mut sum = 0;
                    for (k, &i) in picks.iter().enumerate() {
                        let mut rng = Rng::new(seed ^ (k as u64 + 1).wrapping_mul(0x9e37_79b9_7f4a_7c15));
                        sum += bgcore::search::fight_health_change(data, g, p, &all[i], &mut rng);
                    }
                    *total = sum;
                });
        });
        Ok(())
    }
}

impl SearchBatch {
    /// The next opponent's tier and top tribe, for the boards to fight: from
    /// slot 0, the decision's own position.
    fn opponent_hint(&self) -> (Option<u8>, Option<bgcore::Tribe>) {
        let Some(slot) = self.slots.first().and_then(|s| s.as_ref()) else {
            return (None, None);
        };
        let g = &slot.game;
        match g.players[self.p].opponent {
            Some(bgcore::Opponent::Player(q)) => {
                let line = g.players[q as usize].leaderboard;
                (Some(g.players[q as usize].tier), if line.mixed { None } else { line.top })
            }
            Some(bgcore::Opponent::Ghost(_)) => (g.ghost.as_ref().map(|q| q.tier), None),
            None => (None, None),
        }
    }
}

/// What one player's position holds, as the search and the judges read it
/// (`SearchBatch.describe`, `EraTurnPath.describe`): Gold, Tavern tier,
/// Health, armor, the upgrade price, free Refreshes, the freeze, and the hand,
/// shop and board as `(card id, Attack, Health)` (the hand and board with
/// golden).
type Describe = (
    u8,
    u8,
    i32,
    i32,
    u8,
    u16,
    bool,
    Vec<(u16, i32, i32, bool)>,
    Vec<(u16, i32, i32)>,
    Vec<(u16, i32, i32, bool)>,
);

#[allow(clippy::type_complexity)]
fn describe_player(g: &Game, p: usize) -> Describe {
    let pl = &g.players[p];
    (
        pl.gold,
        pl.tier,
        pl.health,
        pl.armor,
        pl.upgrade_cost,
        pl.free_rolls,
        pl.frozen,
        pl.hand.iter().map(|m| (m.card, m.attack, m.health, m.golden)).collect(),
        pl.shop.iter().map(|m| (m.card, m.attack, m.health)).collect(),
        pl.board.iter().map(|m| (m.card, m.attack, m.health, m.golden)).collect(),
    )
}

// ---- positions a test asks for (train/eval_search.py) ----

/// Hand a zone back to the pool, so a hand-built position keeps the
/// accounting exact.
fn give_back_all(pool: &mut [u8], cards: &bgcore::CardDb, held: Vec<bgcore::Minion>) {
    for m in held {
        let card = cards.get(m.card);
        if m.pool_copies > 0 && !card.token && !card.is_spell() {
            pool[card.pool_slot as usize] += m.pool_copies;
        }
    }
}

fn take_card(g: &mut Game, name: &str) -> PyResult<bgcore::Minion> {
    let c = g
        .data
        .cards
        .by_name(name)
        .ok_or_else(|| PyValueError::new_err(format!("no card {name}")))?;
    let card = g.data.cards.get(c);
    let (slot, copies) = (card.pool_slot as usize, !card.token && !card.is_spell());
    let copies = if copies && slot < g.pool.len() {
        g.pool[slot] -= 1;
        1
    } else {
        0
    };
    let cards = &g.data.cards;
    Ok(g.players[0].make_card(cards, c, false, copies))
}

/// A one-player sandbox position with the board, hand and Tavern a test
/// names, for the search's built positions (`--warden-test`): every card at
/// its own stats, `gold` in hand, the Tavern at `tier`. The sandbox fights a
/// copy of the player's own board and nothing else, which is why it is the
/// right place for a judged line: no opponent and no turn needs to pass.
#[pyfunction]
#[pyo3(signature = (seed, board, shop, hand=Vec::new(), gold=10, tier=5, lost_last_combat=false))]
fn sandbox_position(
    seed: u64,
    board: Vec<String>,
    shop: Vec<String>,
    hand: Vec<String>,
    gold: u8,
    tier: u8,
    lost_last_combat: bool,
) -> PyResult<SearchBatch> {
    let data = Arc::new(Data::bundled());
    let mut g = Game::new_sandbox(data.clone(), seed, Some(bgcore::Tribes::ALL), 0);
    if g.current_player() != Some(0) {
        return Err(PyValueError::new_err("the sandbox position is not player 0's turn"));
    }
    {
        let cards = &g.data.cards;
        let mut pool = std::mem::take(&mut g.pool);
        let shop: Vec<bgcore::Minion> = g.players[0].shop.drain(..).collect();
        let board: Vec<bgcore::Minion> = g.players[0].board.drain(..).collect();
        let hand: Vec<bgcore::Minion> = g.players[0].hand.drain(..).collect();
        give_back_all(&mut pool, cards, shop);
        give_back_all(&mut pool, cards, board);
        give_back_all(&mut pool, cards, hand);
        g.pool = pool;
    }
    for name in &board {
        let m = take_card(&mut g, name)?;
        let _ = g.players[0].board.try_push(m);
    }
    for name in &hand {
        let m = take_card(&mut g, name)?;
        let _ = g.players[0].hand.try_push(m);
    }
    for name in &shop {
        let m = take_card(&mut g, name)?;
        let _ = g.players[0].shop.try_push(m);
    }
    let pl = &mut g.players[0];
    pl.gold = gold;
    pl.tier = tier;
    pl.upgrade_cost = 0;
    pl.lost_last_combat = lost_last_combat;
    Ok(SearchBatch::new(g, 0, data, None))
}

/// The positions Era's logged shop turns start from, one batch each: every
/// turn from `from_turn` on in the action files of `dir`, seated exactly as
/// the shop replay seats the turn's first step (the search brief's cycle-rate
/// test). `labels` says which file and turn each batch is.
/// A logged turn as a batch, and its label: the file, the turn, and what the
/// loader could not build.
type ReplayTurn = (Vec<SearchBatch>, Vec<(String, u32, Vec<String>)>);

#[pyfunction]
#[pyo3(signature = (dir, from_turn=6, seed=1, ref_cards="ref/bg-cards.json"))]
#[allow(clippy::type_complexity)]
fn shop_replay_turns(
    dir: &str,
    from_turn: u32,
    seed: u64,
    ref_cards: &str,
) -> PyResult<ReplayTurn> {
    let data = Arc::new(Data::bundled());
    let (specs, ids, bad) = replay_specs(dir, ref_cards)?;
    let mut batches = Vec::new();
    let mut labels = Vec::new();
    for file in &specs {
        for turn in &file.turns {
            if turn.turn < from_turn || turn.actions.is_empty() {
                continue;
            }
            let Some((game, missing)) =
                bgcore::shop_replay::turn_start_game(&data, &ids, file, turn, seed)
            else {
                continue;
            };
            batches.push(SearchBatch::new(game, bgcore::shop_replay::ME, data.clone(), None));
            labels.push((file.file(), turn.turn, missing));
        }
    }
    for line in &bad {
        labels.push((line.clone(), 0, vec!["the file does not load".to_string()]));
    }
    Ok((batches, labels))
}

/// The `*.json` files of a folder, with their text: the collector's hidden
/// state file (`.state.json`) is not one of them.
fn action_files(dir: &str) -> PyResult<Vec<(String, String)>> {
    Ok(std::fs::read_dir(dir)
        .map_err(|e| PyValueError::new_err(format!("{dir}: {e}")))?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|n| n.ends_with(".json") && !n.starts_with('.'))
        })
        .map(|e| {
            let name = e.file_name().to_str().unwrap_or_default().to_string();
            let text = std::fs::read_to_string(e.path()).unwrap_or_default();
            (name, text)
        })
        .collect())
}

/// Every action file of `dir`, parsed and seated as `shop_replay_turns` seats
/// it: the loader's card id map, and the files it cannot build (a logged value
/// past what its field holds) as `file: why` lines, left out rather than
/// stopping the whole run. Both `shop_replay_turns` and `era_replays` read
/// their turns through this.
fn replay_specs(
    dir: &str,
    ref_cards: &str,
) -> PyResult<(Vec<bgcore::shop_replay::ShopFileSpec>, bgcore::CardIdMap, Vec<String>)> {
    let ids = bgcore::CardIdMap::parse(
        &std::fs::read_to_string(ref_cards)
            .map_err(|e| PyValueError::new_err(format!("{ref_cards}: {e}")))?,
    )
    .map_err(PyValueError::new_err)?;
    let mut files = action_files(dir)?;
    files.sort();
    // One file at a time, so a file the loader cannot build (a logged value
    // past what its field holds) is skipped and named rather than stopping
    // the whole test.
    let mut specs = Vec::new();
    let mut bad: Vec<String> = Vec::new();
    for file in &files {
        match bgcore::shop_replay::parse_action_files(std::slice::from_ref(file)) {
            Ok(mut one) => specs.append(&mut one),
            Err(e) => bad.push(format!("{}: {e}", file.0)),
        }
    }
    bgcore::shop_replay::seat_taught_spells(&mut specs, &ids);
    Ok((specs, ids, bad))
}


// ---- Era's own turns, seated step by step (train/era_turns.py) ----

/// Era's own seat in a replayed game, the seat the shop replay plays.
const ME: usize = bgcore::shop_replay::ME;

/// One of Era's logged turns, played from her own start. The state is seated
/// from the log before every decision, so while the bot takes Era's steps the
/// shop it sees, the Tavern a Refresh deals it and the offers a Discover shows
/// are the log's own, never the engine's draw. The first action that is not
/// Era's leaves her path, and from then on the engine draws: its rng comes from
/// the turn's seed, which is a repeat's seed.
#[pyclass]
struct EraTurnPath {
    data: Arc<Data>,
    ids: Arc<bgcore::CardIdMap>,
    file: Arc<bgcore::shop_replay::ShopFileSpec>,
    turn_index: usize,
    turn: u32,
    seed: u64,
    /// The real boards the search's own judging fights use, if any.
    boards: Option<Arc<RealBoards>>,
    /// The state the bot decides from.
    game: Game,
    /// The next logged step to play.
    step: usize,
    /// The seated state at the start of step `step`, while it is known.
    seated: Option<SeatedStep>,
    /// Whether a matched step is followed by the next step's own seating (the
    /// bot's path, the log's outcomes) or by what the engine played from it
    /// (the fidelity check, Era's steps replayed without help).
    reseat: bool,
    /// The step of the turn the bot first did something else, -1 while it has
    /// not left Era's path.
    left_at: i64,
    left_why: String,
    /// Whether the turn is over.
    done: bool,
    /// How many of Era's steps the bot has played.
    steps_on_path: usize,
    /// What the turn has bought, played and sold, by card.
    bought: Vec<u16>,
    played: Vec<u16>,
    sold: Vec<u16>,
    levelled: bool,
    /// What the loader could not use, or could not seat.
    missing: Vec<String>,
}

impl EraTurnPath {
    /// Seat logged step `i` as the state the bot decides from, keeping the
    /// seated step to play it if the bot takes it.
    fn seat_index(&mut self, i: usize) -> Result<(), (String, Vec<String>)> {
        let seated = bgcore::shop_replay::seat_step(
            &self.data,
            &self.ids,
            &self.file,
            &self.file.turns[self.turn_index],
            i,
            self.seed,
        )?;
        if self.reseat {
            // The bot decides from the log's own before-state. The fidelity
            // pass keeps the state it has carried forward instead.
            self.game = seated.game.clone();
        }
        self.seated = Some(seated);
        Ok(())
    }

    /// Note that the turn left Era's path at the next logged step.
    fn leave(&mut self, why: String) {
        if self.left_at < 0 {
            self.left_at = self.step as i64;
            self.left_why = why;
        }
    }

    /// Era's next decision, as the action index she picked. Steps the engine
    /// asks nothing about (a step with no action of its own, or one whose
    /// click the engine takes itself) are played as the log has them, since
    /// the bot cannot make that decision. None when Era's logged steps are
    /// over, or the next one cannot be seated.
    fn era_decision(&mut self) -> PyResult<Option<usize>> {
        while !self.done && self.left_at < 0 {
            let n = self.file.turns[self.turn_index].actions.len();
            if self.step >= n {
                return Ok(None);
            }
            if self.seated.is_none()
                && let Err((why, miss)) = self.seat_index(self.step)
            {
                self.missing.extend(miss);
                self.missing
                    .push(format!("step {} cannot be seated: {why}", self.step));
                self.leave("the logged step cannot be seated".to_string());
                return Ok(None);
            }
            let seated = self.seated.as_ref().expect("seated just above");
            if seated.step.step == "end_turn" {
                return Ok(Some(Action::EndTurn.index()));
            }
            let mut mask = [false; N_ACTIONS];
            self.game.legal_mask(&mut mask);
            match seated.decision_index() {
                Some(d) if mask[d] => return Ok(Some(d)),
                _ => {
                    // Nothing to ask here: play the log's own step.
                    self.advance()?;
                }
            }
        }
        Ok(None)
    }

    /// Play Era's next logged step, then put the state at the next step's own
    /// seating while that is what keeps the log's outcomes (the bot's path).
    fn advance(&mut self) -> PyResult<()> {
        let seated = self
            .seated
            .take()
            .ok_or_else(|| PyValueError::new_err("no logged step is seated"))?;
        let index = self.step;
        let n = self.file.turns[self.turn_index].actions.len();
        self.step += 1;
        self.steps_on_path += 1;
        for a in &seated.actions {
            self.track_action(&seated.game, *a);
        }
        let is_end = seated.step.step == "end_turn";
        let carried = self.game.clone();
        let (game, missing, _text) = seated
            .play_on(&self.data, &self.ids, carried)
            .map_err(|(why, miss)| {
                PyValueError::new_err(format!(
                    "step {index} cannot be played: {why}{}",
                    if miss.is_empty() {
                        String::new()
                    } else {
                        format!(" ({})", miss.join("; "))
                    }
                ))
            })?;
        self.missing.extend(missing);
        self.game = game;
        if is_end || self.game.players[ME].done || self.game.turn != self.turn || self.game.over {
            self.done = true;
            return Ok(());
        }
        if self.step >= n {
            // The log's turn ends here (an end_turn step almost always closes
            // it, and its own effects have run above).
            self.done = true;
            return Ok(());
        }
        if self.reseat && let Err((why, miss)) = self.seat_index(self.step) {
            self.missing.extend(miss);
            self.missing
                .push(format!("step {} cannot be seated: {why}", self.step));
            self.leave("the next logged step cannot be seated".to_string());
        }
        Ok(())
    }

    /// Remember what one action bought, played or sold, so the turn's cycles
    /// (a card bought or played and sold in the same turn) can be counted. The
    /// state is the one the action was taken from.
    fn track_action(&mut self, g: &Game, action: Action) {
        let pl = &g.players[ME];
        match action {
            Action::Buy(slot) => {
                if let Some(m) = pl.shop.get(slot as usize) {
                    self.bought.push(m.card);
                }
            }
            Action::BuySpell => {
                if let Some(card) = pl.shop_spell {
                    self.bought.push(card);
                }
            }
            Action::Sell(slot) => {
                if let Some(m) = pl.board.get(slot as usize) {
                    self.sold.push(m.card);
                }
            }
            Action::Play { hand, .. } => {
                if let Some(m) = pl.hand.get(hand as usize) {
                    self.played.push(m.card);
                }
            }
            Action::Level => self.levelled = true,
            _ => {}
        }
    }

    /// Play one action of the bot's. While it is Era's own next decision the
    /// logged step is played instead, with the log's outcomes; anything else
    /// leaves her path. Returns whether the turn is still on it.
    fn take_action(&mut self, a: i64) -> PyResult<bool> {
        if self.done {
            return Err(PyValueError::new_err("the turn is over"));
        }
        let Some(action) = usize::try_from(a).ok().and_then(Action::from_index) else {
            return Err(PyValueError::new_err(format!("no action {a}")));
        };
        match self.era_decision()? {
            Some(era) if era == action.index() => {
                self.advance()?;
                return Ok(self.left_at < 0);
            }
            Some(era) => {
                let era = Action::from_index(era).map_or("?".to_string(), |a| format!("{a:?}"));
                self.leave(format!(
                    "at step {} the bot played {action:?}, Era played {era}",
                    self.step
                ));
            }
            None => {
                if matches!(action, Action::EndTurn) {
                    // Era's logged steps are over and her turn is ending: the
                    // bot ends its own too, still on her path.
                    self.done = true;
                    self.game.end_shop_turn(ME);
                    return Ok(self.left_at < 0);
                }
                self.leave(format!(
                    "Era's logged steps were over and the bot played {action:?}"
                ));
            }
        }
        self.play_free(action)?;
        Ok(false)
    }

    /// Play one action the engine draws for: the turn is off Era's path.
    fn play_free(&mut self, action: Action) -> PyResult<()> {
        let game = self.game.clone();
        self.track_action(&game, action);
        if matches!(action, Action::EndTurn) {
            // The engine's End Turn fights; the judged position is the board
            // that goes into the fight, so only the shop turn ends here.
            self.done = true;
            self.game.end_shop_turn(ME);
            return Ok(());
        }
        self.game
            .step(action)
            .map_err(|_| PyValueError::new_err(format!("illegal action {action:?}")))?;
        if self.game.players[ME].done || self.game.turn != self.turn || self.game.over {
            self.done = true;
        }
        Ok(())
    }
}

#[pymethods]
impl EraTurnPath {
    #[getter]
    fn turn(&self) -> u32 {
        self.turn
    }

    /// The action file's name, as `boards/real` names the same game.
    #[getter]
    fn file(&self) -> String {
        self.file.file()
    }

    #[getter]
    fn source(&self) -> String {
        self.file.source.clone()
    }

    #[getter]
    fn game(&self) -> u32 {
        self.file.game
    }

    /// How many steps the log has for the turn.
    #[getter]
    fn logged_steps(&self) -> usize {
        self.file.turns[self.turn_index].actions.len()
    }

    /// How many of Era's steps the bot played (the ones it took).
    #[getter]
    fn steps_on_path(&self) -> usize {
        self.steps_on_path
    }

    /// The step the turn left Era's path at, -1 while it has not.
    #[getter]
    fn left_at(&self) -> i64 {
        self.left_at
    }

    /// What the bot did differently, in words.
    #[getter]
    fn left_why(&self) -> String {
        self.left_why.clone()
    }

    #[getter]
    fn on_path(&self) -> bool {
        self.left_at < 0
    }

    #[getter]
    fn done(&self) -> bool {
        self.done
    }

    /// What the loader could not use, or could not seat, in the replay's words.
    #[getter]
    fn missing(&self) -> Vec<String> {
        self.missing.clone()
    }

    /// The cards the turn has bought, played and sold, by card id: a cycle is
    /// a card in `bought` or `played` and in `sold`.
    #[getter]
    fn bought(&self) -> Vec<u16> {
        self.bought.clone()
    }

    #[getter]
    fn played(&self) -> Vec<u16> {
        self.played.clone()
    }

    #[getter]
    fn sold(&self) -> Vec<u16> {
        self.sold.clone()
    }

    /// Whether the bot pressed the tier up button this turn.
    #[getter]
    fn levelled(&self) -> bool {
        self.levelled
    }

    /// 0 for a shop decision, 1 for an answer the network takes (a Discover, a
    /// target, a trinket or hero pick) and 2 for a finished turn.
    #[getter]
    fn state(&self) -> i8 {
        if self.done || !self.game.players[ME].alive || self.game.over {
            2
        } else if self.game.shopping(ME) {
            0
        } else {
            1
        }
    }

    /// The actions the engine's own cap still allows this turn.
    #[getter]
    fn actions_left(&self) -> usize {
        (self.data.rules.max_actions_per_turn as usize)
            .saturating_sub(self.game.players[ME].actions as usize)
    }

    fn card_names(&self) -> Vec<String> {
        self.data.cards.cards.iter().map(|c| c.name.clone()).collect()
    }

    /// Era's next logged decision as an action index, -1 when the turn has no
    /// logged step left (or it cannot be seated).
    fn era_action(&mut self) -> PyResult<i64> {
        Ok(self.era_decision()?.map_or(-1, |d| d as i64))
    }

    /// The observation and legal mask of the state the bot decides from.
    fn observe(
        &self,
        mut obs: PyReadwriteArray1<f32>,
        mut mask: PyReadwriteArray1<bool>,
    ) -> PyResult<()> {
        let o = obs.as_slice_mut()?;
        if o.len() != OBS_SIZE {
            return Err(PyValueError::new_err("one observation"));
        }
        obs::write(&self.game, ME, o);
        let m: &mut [bool; N_ACTIONS] = mask
            .as_slice_mut()?
            .try_into()
            .map_err(|_| PyValueError::new_err("one legal mask"))?;
        self.game.legal_mask(m);
        Ok(())
    }

    fn describe(&self) -> Describe {
        describe_player(&self.game, ME)
    }

    /// Play one action. Returns false once the turn has left Era's path (and
    /// stays false), true while the bot is still on it.
    fn act(&mut self, a: i64) -> PyResult<bool> {
        self.take_action(a)
    }

    /// End the shop turn without a fight, so the position is the one the fight
    /// after it is fought from (the bot's own end of turn).
    fn finish(&mut self) {
        if self.done {
            return;
        }
        self.game.end_shop_turn(ME);
        self.done = true;
    }

    /// Play every remaining logged step as Era played it: the fidelity check
    /// (a player that always plays Era's next step), and Era's own end-of-turn
    /// position.
    fn play_era(&mut self) -> PyResult<()> {
        loop {
            if self.done {
                break;
            }
            match self.era_decision()? {
                Some(d) => {
                    self.take_action(d as i64)?;
                }
                None => {
                    self.finish();
                    break;
                }
            }
        }
        Ok(())
    }

    /// What the engine's own state shows differently from the log's end-of-turn
    /// state, in the replay's words: empty when the turn is reproduced exactly.
    fn diffs(&self) -> Vec<String> {
        let turn = &self.file.turns[self.turn_index];
        let Some(end) = turn.end.as_ref() else {
            return vec!["the log has no end state for the turn".to_string()];
        };
        let mut missing = Vec::new();
        let log = ShopSnapshot::of_state(&self.data, end, turn.turn, &mut missing);
        let ours = ShopSnapshot::of_game(&self.data, &self.game, ME);
        let mut diffs = bgcore::shop_replay::compare(&ours, &log);
        diffs.extend(missing);
        diffs
    }

    /// A `SearchBatch` at the position the bot decides from: the search's own
    /// entry point, with the same real boards its fights use.
    fn batch(&self) -> PyResult<SearchBatch> {
        Ok(SearchBatch::new(self.game.clone(), ME, self.data.clone(), self.boards.clone()))
    }
}

/// One action file of Era's games, ready to seat any of its turns
/// (`train/era_turns.py`).
#[pyclass]
struct EraReplay {
    data: Arc<Data>,
    ids: Arc<bgcore::CardIdMap>,
    spec: Arc<bgcore::shop_replay::ShopFileSpec>,
    name: String,
    boards: Option<Arc<RealBoards>>,
}

#[pymethods]
impl EraReplay {
    /// The file's name, as `boards/real` names the same game.
    #[getter]
    fn file(&self) -> String {
        self.name.clone()
    }

    #[getter]
    fn source(&self) -> String {
        self.spec.source.clone()
    }

    #[getter]
    fn game(&self) -> u32 {
        self.spec.game
    }

    /// The logged turns from `from_turn` on that have steps to seat, in the
    /// file's order.
    fn turns(&self, from_turn: u32) -> Vec<u32> {
        self.spec
            .turns
            .iter()
            .filter(|t| t.turn >= from_turn && !t.actions.is_empty())
            .map(|t| t.turn)
            .collect()
    }

    /// The turn seated as a path: the position at its first step, and the
    /// machinery to play it with the log's own outcomes. `seed` is the turn's
    /// own (a repeat's seed), and seeds the engine's draws once the path is
    /// left.
    #[pyo3(signature = (turn, seed, reseat=true))]
    fn path(&self, turn: u32, seed: u64, reseat: bool) -> PyResult<EraTurnPath> {
        let turn_index = self
            .spec
            .turns
            .iter()
            .position(|t| t.turn == turn)
            .ok_or_else(|| PyValueError::new_err(format!("{} has no turn {turn}", self.name)))?;
        let seated = bgcore::shop_replay::seat_step(
            &self.data,
            &self.ids,
            &self.spec,
            &self.spec.turns[turn_index],
            0,
            seed,
        )
        .map_err(|(why, miss)| {
            PyValueError::new_err(format!(
                "turn {turn} cannot be seated: {why}{}",
                if miss.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", miss.join("; "))
                }
            ))
        })?;
        let missing = seated.missing.clone();
        let game = seated.game.clone();
        Ok(EraTurnPath {
            data: self.data.clone(),
            ids: self.ids.clone(),
            file: self.spec.clone(),
            turn_index,
            turn,
            seed,
            boards: self.boards.clone(),
            game,
            step: 0,
            seated: Some(seated),
            reseat,
            left_at: -1,
            left_why: String::new(),
            done: false,
            steps_on_path: 0,
            bought: Vec::new(),
            played: Vec::new(),
            sold: Vec::new(),
            levelled: false,
            missing,
        })
    }
}

/// Every action file of `dir` as a replay to seat turns from, with the files
/// the loader cannot build as `file: why` lines. `real_boards` is
/// train/real_boards.py's spec list, the boards the search's own judging
/// fights; without it the search plays blind, as it does for a cycle-rate run.
#[pyfunction]
#[pyo3(signature = (dir, ref_cards, real_boards=None))]
fn era_replays(
    dir: &str,
    ref_cards: &str,
    real_boards: Option<PyBoards>,
) -> PyResult<(Vec<EraReplay>, Vec<String>)> {
    let data = Arc::new(Data::bundled());
    let (specs, ids, bad) = replay_specs(dir, ref_cards)?;
    let ids = Arc::new(ids);
    let boards = real_boards.map(|raw| {
        let specs = real_board_specs(raw);
        let (boards, _) = RealBoards::build(&specs, &data.cards, &data.gifts);
        Arc::new(boards)
    });
    let replays = specs
        .into_iter()
        .map(|spec| {
            let name = spec.file();
            EraReplay {
                data: data.clone(),
                ids: ids.clone(),
                spec: Arc::new(spec),
                name,
                boards: boards.clone(),
            }
        })
        .collect();
    Ok((replays, bad))
}


/// The comp planner as a player for Era's turns (`train/era_turns.py
/// --planner`). One planner per seat or per repeat; `reset` starts a turn or a
/// game with a fresh state.
#[pyclass]
struct Planner {
    inner: bgcore::planner::Planner,
    data: Arc<Data>,
}

#[pymethods]
impl Planner {
    /// `comps`: the comp library TOML (data/comps/comps.toml). `real_boards`:
    /// the same specs a VecEnv gets, so the planner fights the same boards.
    #[new]
    #[pyo3(signature = (comps, seed=0, real_boards=None))]
    fn new(comps: &str, seed: u64, real_boards: Option<PyBoards>) -> PyResult<Planner> {
        let data = Arc::new(Data::bundled());
        let text = std::fs::read_to_string(comps)
            .map_err(|e| PyValueError::new_err(format!("{comps}: {e}")))?;
        let comps = Comps::from_toml(&text, &data).map_err(PyValueError::new_err)?;
        let mut inner = bgcore::planner::Planner::new(comps, seed, &data);
        if let Some(raw) = real_boards {
            let specs = real_board_specs(raw);
            let (boards, _) = RealBoards::build(&specs, &data.cards, &data.gifts);
            inner.set_boards(Some(Arc::new(boards)));
        }
        Ok(Planner { inner, data })
    }

    fn reset(&mut self, seed: u64) {
        self.inner.reset(seed);
    }

    /// The next action for the path's current position.
    fn action(&mut self, path: &EraTurnPath) -> PyResult<i64> {
        if path.data.cards.cards.len() != self.data.cards.cards.len() {
            return Err(PyValueError::new_err(
                "the planner was built from a different card set than the path",
            ));
        }
        if path.boards.is_some() {
            self.inner.set_boards(path.boards.clone());
        }
        Ok(self.inner.action(&path.game, ME).index() as i64)
    }

    /// The comp committed to, if any.
    #[getter]
    fn comp(&self) -> Option<String> {
        self.inner.comp_name().map(str::to_string)
    }

    /// The comp the planner is leaning to before committing.
    #[getter]
    fn lean(&self) -> Option<String> {
        self.inner.lean_name().map(str::to_string)
    }

    #[getter]
    fn committed_turn(&self) -> Option<u32> {
        self.inner.state.committed_turn
    }

    #[getter]
    fn switched_turn(&self) -> Option<u32> {
        self.inner.state.switched_turn
    }

    #[getter]
    fn shell_cycles(&self) -> usize {
        self.inner.state.shell_cycle_turns.len()
    }

    #[getter]
    fn shell_chances(&self) -> usize {
        self.inner.state.shell_chance_turns.len()
    }

    fn level_turns(&self) -> Vec<(u32, u8)> {
        self.inner.state.level_turns.clone()
    }
}

/// The boards of Era's real games, for judging an end-of-turn position: every
/// board of `boards/real`, with the file and game it came from, so the board
/// Era really fought next can be found by (file, turn) and a turn's typical
/// boards drawn the way `RealBoards` draws them.
#[pyclass]
struct RealFights {
    data: Arc<Data>,
    boards: RealBoards,
    /// Every spec's board, in the files' order, so a place found by key and a
    /// place found by `places_at` are the same board.
    flat: Vec<RealBoard>,
    /// The file, game, turn and side of every flat board, in the same order.
    keys: Vec<(String, u32, u32, bool)>,
}

impl RealFights {
    fn index_of(&self, source: &str, game: u32, turn: u32, opponent: bool) -> Option<usize> {
        self.keys
            .iter()
            .position(|(s, g, t, o)| s == source && *g == game && *t == turn && *o == opponent)
    }
}

#[pymethods]
impl RealFights {
    /// Read every board file of `dir`: the game each board came from and its
    /// side survive (the files' own keys), and the minions no card matches come
    /// back by name.
    #[staticmethod]
    fn load(dir: &str, ref_cards: &str) -> PyResult<(RealFights, Vec<String>)> {
        let data = Arc::new(Data::bundled());
        let ids = bgcore::CardIdMap::parse(
            &std::fs::read_to_string(ref_cards)
                .map_err(|e| PyValueError::new_err(format!("{ref_cards}: {e}")))?,
        )
        .map_err(PyValueError::new_err)?;
        let mut files = action_files(dir)?;
        files.sort();
        let (specs, unknown) =
            bgcore::real_boards::parse_board_files(&files, &ids).map_err(PyValueError::new_err)?;
        let keys = specs
            .iter()
            .map(|s| {
                (
                    s.source.clone(),
                    s.game,
                    s.turn,
                    matches!(s.side, bgcore::real_boards::RealSide::Opp),
                )
            })
            .collect();
        let (boards, unbuilt) = RealBoards::build(&specs, &data.cards, &data.gifts);
        // The flat list in the files' order: `places_at` names the places of a
        // turn's bucket, so the same board is reachable both ways.
        let mut flat: Vec<Option<RealBoard>> = vec![None; specs.len()];
        for (place, board) in boards.indexed() {
            flat[place] = Some(board.clone());
        }
        let mut out = Vec::with_capacity(specs.len());
        for (i, board) in flat.into_iter().enumerate() {
            let Some(board) = board else {
                return Err(PyValueError::new_err(format!("board {i} was not built")));
            };
            out.push(board);
        }
        let mut unknown = unknown;
        unknown.extend(unbuilt);
        Ok((
            RealFights { data, boards, flat: out, keys },
            unknown,
        ))
    }

    fn len(&self) -> usize {
        self.flat.len()
    }

    /// The boards of `turn` a typical draw fights, as flat indices: the same
    /// evenly spread list for every position judged on that turn (the rule the
    /// search's own judging uses, `bgcore::search::fight_boards`).
    fn typical(&self, turn: u32, n: usize) -> Vec<usize> {
        let places = self.boards.places_at(turn);
        bgcore::search::fight_boards(&self.data, &self.boards, turn, None, None, n)
            .into_iter()
            .filter_map(|i| places.get(i).copied())
            .collect()
    }

    /// The board Era fought after `turn` of the game `source`+`game`, as a flat
    /// index, or -1 when the folder holds none (the fight was a ghost, or the
    /// log has no opponent board for it).
    fn opponent(&self, source: &str, game: u32, turn: u32) -> i64 {
        self.index_of(source, game, turn, true)
            .map_or(-1, |i| i as i64)
    }

    /// Fight the batch's slot board against the boards `which` names, one
    /// fight each, with the seeds `seed` gives: the same boards and seeds for
    /// every position judged, so a stronger board simply wins more. Returns
    /// (wins, ties, losses, total Health change).
    fn fight(
        &self,
        batch: &SearchBatch,
        slot: usize,
        which: Vec<usize>,
        seed: u64,
    ) -> PyResult<(i64, i64, i64, i64)> {
        let game = &batch.slot(slot)?.game;
        let p = batch.p;
        let mut wins = 0;
        let mut ties = 0;
        let mut losses = 0;
        let mut health = 0;
        for (k, &i) in which.iter().enumerate() {
            let board = self
                .flat
                .get(i)
                .ok_or_else(|| PyValueError::new_err(format!("no real board {i}")))?;
            let mut rng = Rng::new(seed ^ (k as u64 + 1).wrapping_mul(0x9e37_79b9_7f4a_7c15));
            let out = bgcore::search::fight_outcome(&self.data, game, p, board, &mut rng);
            match out.outcome {
                1 => wins += 1,
                0 => ties += 1,
                _ => losses += 1,
            }
            health += out.health_change;
        }
        Ok((wins, ties, losses, health as i64))
    }

    /// The top tribe of Era's last board of the game, by the leaderboard's rule
    /// (a shared top is "mixed", an empty board "none"). The comp label of a
    /// game the comps run has no name for.
    fn final_tribe(&self, source: &str, game: u32) -> String {
        let mut best: Option<(u32, usize)> = None;
        for (i, (s, g, t, opp)) in self.keys.iter().enumerate() {
            if s != source || *g != game || *opp {
                continue;
            }
            if best.is_none_or(|(bt, _)| *t > bt) {
                best = Some((*t, i));
            }
        }
        let Some((_, i)) = best else {
            return "none".to_string();
        };
        let line = bgcore::game::Leaderboard::of(&self.data.cards, &self.flat[i].minions);
        match (line.top, line.mixed) {
            (Some(t), false) => bgcore::shop_replay::tribe_name(t).to_string(),
            _ if line.mixed => "mixed".to_string(),
            _ => "none".to_string(),
        }
    }
}

#[pymodule]
fn bgpy(m: &Bound<'_, PyModule>) -> PyResult<()> {
    // A game step inlines into rayon's splitting helper with a 64 KB frame,
    // nested once per split and again for stolen jobs: the default 2 MB
    // worker stack overflows (a bare segfault, since Python has no Rust
    // overflow handler). Fails harmlessly if a pool already exists.
    let _ = rayon::ThreadPoolBuilder::new().stack_size(64 << 20).build_global();
    let n_cards = Data::bundled().cards.cards.len();
    let n_heroes = Data::bundled().heroes.heroes.len();
    let n_powers = Data::bundled().heroes.powers.len();
    let n_trinkets = Data::bundled().trinkets.len();
    m.add_class::<VecEnv>()?;
    m.add_class::<SearchBatch>()?;
    m.add_class::<EraTurnPath>()?;
    m.add_class::<EraReplay>()?;
    m.add_class::<Planner>()?;
    m.add_class::<RealFights>()?;
    m.add_function(wrap_pyfunction!(sandbox_position, m)?)?;
    m.add_function(wrap_pyfunction!(shop_replay_turns, m)?)?;
    m.add_function(wrap_pyfunction!(era_replays, m)?)?;
    m.add("N_HEROES", n_heroes)?;
    m.add("N_POWERS", n_powers)?;
    m.add("N_TRINKETS", n_trinkets)?;
    m.add("OBS_SIZE", OBS_SIZE)?;
    m.add("N_ACTIONS", N_ACTIONS)?;
    m.add("N_CARDS", n_cards)?;
    m.add("PLAYERS", PLAYERS)?;
    m.add("OVERLAY_FLAGS", OVERLAY_FLAGS)?;
    m.add("MAX_TIER", MAX_TIER as usize)?;
    m.add("TRIBES", TRIBES)?;
    m.add("BOARD", bgcore::BOARD)?;
    m.add("HAND", bgcore::HAND)?;
    m.add("SHOP", bgcore::SHOP)?;
    m.add("DISCOVER", bgcore::DISCOVER)?;
    m.add("CHOICES", bgcore::CHOICES)?;
    // every layout constant of the observation, by its name in obs.rs
    m.add("SHOP_SPELL", obs::SHOP_SPELL)?;
    m.add("SLOT_FEATURES", obs::SLOT_FEATURES)?;
    m.add("MINION_FEATURES", obs::MINION_FEATURES)?;
    m.add("SCALARS", obs::SCALARS)?;
    m.add("OPP_SCALARS", obs::OPP_SCALARS)?;
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
    m.add("SCALAR_ACTIONS_LEFT", obs::SCALAR_ACTIONS_LEFT)?;
    m.add("SCALAR_ALIVE", obs::SCALAR_ALIVE)?;
    m.add("SCALAR_GHOST", obs::SCALAR_GHOST)?;
    m.add("SCALAR_WON_LAST_COMBAT", obs::SCALAR_WON_LAST_COMBAT)?;
    m.add("SCALAR_LOST_LAST_COMBAT", obs::SCALAR_LOST_LAST_COMBAT)?;
    m.add("SCALAR_GOLD_NEXT_TURN", obs::SCALAR_GOLD_NEXT_TURN)?;
    m.add("SCALAR_EXTRA_MAX_GOLD", obs::SCALAR_EXTRA_MAX_GOLD)?;
    m.add("SCALAR_FREE_ROLLS", obs::SCALAR_FREE_ROLLS)?;
    m.add("SCALAR_NEXT_ROLL_HEALTH", obs::SCALAR_NEXT_ROLL_HEALTH)?;
    m.add("SCALAR_HEALTH_REFRESHES_LEFT", obs::SCALAR_HEALTH_REFRESHES_LEFT)?;
    m.add("SCALAR_CAN_REWIND", obs::SCALAR_CAN_REWIND)?;
    m.add("SCALAR_SPELL_DISCOUNT", obs::SCALAR_SPELL_DISCOUNT)?;
    m.add("SCALAR_SHOP_SPELL_COST", obs::SCALAR_SHOP_SPELL_COST)?;
    m.add("SCALAR_SHOP_SPELL_PAY_HEALTH", obs::SCALAR_SHOP_SPELL_PAY_HEALTH)?;
    m.add("SCALAR_SPELL_ATTACK", obs::SCALAR_SPELL_ATTACK)?;
    m.add("SCALAR_SPELL_HEALTH", obs::SCALAR_SPELL_HEALTH)?;
    m.add("SCALAR_VOLUMIZER_ATTACK", obs::SCALAR_VOLUMIZER_ATTACK)?;
    m.add("SCALAR_VOLUMIZER_HEALTH", obs::SCALAR_VOLUMIZER_HEALTH)?;
    m.add("SCALAR_BEETLE_ATTACK", obs::SCALAR_BEETLE_ATTACK)?;
    m.add("SCALAR_BEETLE_HEALTH", obs::SCALAR_BEETLE_HEALTH)?;
    m.add("SCALAR_BOON_OF_BEETLES", obs::SCALAR_BOON_OF_BEETLES)?;
    m.add("SCALAR_BROOD_OF_NOZDORMU", obs::SCALAR_BROOD_OF_NOZDORMU)?;
    m.add("SCALAR_UPPER_HAND", obs::SCALAR_UPPER_HAND)?;
    m.add("SCALAR_PATIENT_SCOUT_TIER", obs::SCALAR_PATIENT_SCOUT_TIER)?;
    m.add("SCALAR_TASTY_LOBSTER", obs::SCALAR_TASTY_LOBSTER)?;
    m.add("SCALAR_BEAST_BONUS_ATTACK", obs::SCALAR_BEAST_BONUS_ATTACK)?;
    m.add("SCALAR_BEAST_BONUS_HEALTH", obs::SCALAR_BEAST_BONUS_HEALTH)?;
    m.add("SCALAR_DEATHRATTLES", obs::SCALAR_DEATHRATTLES)?;
    m.add("SCALAR_FODDER_NEXT", obs::SCALAR_FODDER_NEXT)?;
    m.add("SCALAR_FODDER_QUEUED", obs::SCALAR_FODDER_QUEUED)?;
    m.add("SCALAR_FODDER_REFRESHES", obs::SCALAR_FODDER_REFRESHES)?;
    m.add("SCALAR_MAGICFIN_LEFT", obs::SCALAR_MAGICFIN_LEFT)?;
    m.add("SCALAR_OVERCONFIDENCE_WIN", obs::SCALAR_OVERCONFIDENCE_WIN)?;
    m.add("SCALAR_OVERCONFIDENCE_TIE", obs::SCALAR_OVERCONFIDENCE_TIE)?;
    m.add("SCALAR_WINNERS_BREAD_ATTACK", obs::SCALAR_WINNERS_BREAD_ATTACK)?;
    m.add("SCALAR_WINNERS_BREAD_HEALTH", obs::SCALAR_WINNERS_BREAD_HEALTH)?;
    m.add("SCALAR_NEXT_TURN_BUFF_ATTACK", obs::SCALAR_NEXT_TURN_BUFF_ATTACK)?;
    m.add("SCALAR_NEXT_TURN_BUFF_HEALTH", obs::SCALAR_NEXT_TURN_BUFF_HEALTH)?;
    m.add("SCALAR_TAVERN_BUFFS", obs::SCALAR_TAVERN_BUFFS)?;
    m.add("TAVERN_BUFF_FILTERS", obs::TAVERN_BUFF_FILTERS)?;
    m.add("SCALAR_LAST_SPELL", obs::SCALAR_LAST_SPELL)?;
    m.add("SCALAR_LOBBY", obs::SCALAR_LOBBY)?;
    m.add("LOBBY_TRIBES", obs::LOBBY_TRIBES)?;
    m.add("SCALAR_GEM_ATTACK", obs::SCALAR_GEM_ATTACK)?;
    m.add("SCALAR_GEM_HEALTH", obs::SCALAR_GEM_HEALTH)?;
    m.add("SCALAR_BARRAGE_ATTACK", obs::SCALAR_BARRAGE_ATTACK)?;
    m.add("SCALAR_BARRAGE_HEALTH", obs::SCALAR_BARRAGE_HEALTH)?;
    m.add("SCALAR_REFRESH_ATTACK", obs::SCALAR_REFRESH_ATTACK)?;
    m.add("SCALAR_REFRESH_HEALTH", obs::SCALAR_REFRESH_HEALTH)?;
    m.add("SCALAR_BALLER", obs::SCALAR_BALLER)?;
    m.add("SCALAR_UNDEAD_ATTACK", obs::SCALAR_UNDEAD_ATTACK)?;
    m.add("SCALAR_UNDEAD_HEALTH", obs::SCALAR_UNDEAD_HEALTH)?;
    m.add("SCALAR_KNIGHT_DEATHS", obs::SCALAR_KNIGHT_DEATHS)?;
    m.add("SCALAR_ELEMENTALS_PLAYED", obs::SCALAR_ELEMENTALS_PLAYED)?;
    m.add("SCALAR_GOLDEN_PLAYED", obs::SCALAR_GOLDEN_PLAYED)?;
    m.add("SCALAR_GOLD_SPENT", obs::SCALAR_GOLD_SPENT)?;
    m.add("SCALAR_DISCARDED", obs::SCALAR_DISCARDED)?;
    m.add("SCALAR_SPELLS_CAST", obs::SCALAR_SPELLS_CAST)?;
    m.add("SCALAR_TRAILBLAZER_COVERS", obs::SCALAR_TRAILBLAZER_COVERS)?;
    m.add("SCALAR_SHARING_IS_CARING", obs::SCALAR_SHARING_IS_CARING)?;
    m.add("SCALAR_LAST_DAMAGE", obs::SCALAR_LAST_DAMAGE)?;
    m.add("SCALAR_DEITY", obs::SCALAR_DEITY)?;
    m.add("DEITY_FEATURES", obs::DEITY_FEATURES)?;
    m.add("SCALAR_DEITY_CTHUN", obs::SCALAR_DEITY_CTHUN)?;
    m.add("SCALAR_DEITY_YSHAARJ", obs::SCALAR_DEITY_YSHAARJ)?;
    m.add("SCALAR_DEITY_ATTACK", obs::SCALAR_DEITY_ATTACK)?;
    m.add("SCALAR_DEITY_HEALTH", obs::SCALAR_DEITY_HEALTH)?;
    m.add("SCALAR_DEITY_GOLDEN", obs::SCALAR_DEITY_GOLDEN)?;
    m.add("SCALAR_PENDING_TARGET", obs::SCALAR_PENDING_TARGET)?;
    m.add("SCALAR_PENDING_TARGET_SHOP", obs::SCALAR_PENDING_TARGET_SHOP)?;
    m.add("SCALAR_PENDING_SHOP_SLOT", obs::SCALAR_PENDING_SHOP_SLOT)?;
    m.add("SCALAR_PENDING_CHOOSE_ONE", obs::SCALAR_PENDING_CHOOSE_ONE)?;
    m.add("SCALAR_PENDING_DISCOVER", obs::SCALAR_PENDING_DISCOVER)?;
    m.add("SCALAR_PENDING_DISCARD", obs::SCALAR_PENDING_DISCARD)?;
    m.add("SCALAR_PENDING_SOURCE", obs::SCALAR_PENDING_SOURCE)?;
    m.add("SCALAR_PENDING_CARD", obs::SCALAR_PENDING_CARD)?;
    m.add("SCALAR_PENDING_RUNS", obs::SCALAR_PENDING_RUNS)?;
    m.add("SCALAR_PENDING_ATTACK", obs::SCALAR_PENDING_ATTACK)?;
    m.add("SCALAR_PENDING_HEALTH", obs::SCALAR_PENDING_HEALTH)?;
    m.add("SCALAR_TARGET_KIND", obs::SCALAR_TARGET_KIND)?;
    m.add("TARGET_KINDS", obs::TARGET_KINDS)?;
    m.add("SCALAR_SHOP_CONSUME_LEFT", obs::SCALAR_SHOP_CONSUME_LEFT)?;
    m.add("SCALAR_SHOP_FISHBAIT", obs::SCALAR_SHOP_FISHBAIT)?;
    m.add("SCALAR_DISCOVER_LOCKED", obs::SCALAR_DISCOVER_LOCKED)?;
    m.add("SCALAR_DISCOVER_DAMAGE", obs::SCALAR_DISCOVER_DAMAGE)?;
    m.add("SCALAR_DISCOVER_DOOMED", obs::SCALAR_DISCOVER_DOOMED)?;
    m.add("SCALAR_DISCOVER_BOTH", obs::SCALAR_DISCOVER_BOTH)?;
    m.add("SCALAR_DISCOVER_QUEUE", obs::SCALAR_DISCOVER_QUEUE)?;
    m.add("SCALAR_CHOOSE_ONE", obs::SCALAR_CHOOSE_ONE)?;
    m.add("CHOOSE_FEATURES", obs::CHOOSE_FEATURES)?;
    m.add("SLOT_PRESENT", obs::SLOT_PRESENT)?;
    m.add("SLOT_CARD_ID", obs::SLOT_CARD_ID)?;
    m.add("SLOT_IS_SPELL", obs::SLOT_IS_SPELL)?;
    m.add("SLOT_ATTACK", obs::SLOT_ATTACK)?;
    m.add("SLOT_HEALTH", obs::SLOT_HEALTH)?;
    m.add("SLOT_FIGHT_ATTACK", obs::SLOT_FIGHT_ATTACK)?;
    m.add("SLOT_FIGHT_HEALTH", obs::SLOT_FIGHT_HEALTH)?;
    m.add("SLOT_GOLDEN", obs::SLOT_GOLDEN)?;
    m.add("SLOT_TIER", obs::SLOT_TIER)?;
    m.add("SLOT_TRIBES", obs::SLOT_TRIBES)?;
    m.add("SLOT_BEAST", obs::SLOT_BEAST)?;
    m.add("SLOT_MECH", obs::SLOT_MECH)?;
    m.add("SLOT_DRAGON", obs::SLOT_DRAGON)?;
    m.add("SLOT_MURLOC", obs::SLOT_MURLOC)?;
    m.add("SLOT_DEMON", obs::SLOT_DEMON)?;
    m.add("SLOT_PIRATE", obs::SLOT_PIRATE)?;
    m.add("SLOT_ELEMENTAL", obs::SLOT_ELEMENTAL)?;
    m.add("SLOT_UNDEAD", obs::SLOT_UNDEAD)?;
    m.add("SLOT_QUILBOAR", obs::SLOT_QUILBOAR)?;
    m.add("SLOT_ABERRATION", obs::SLOT_ABERRATION)?;
    m.add("SLOT_KEYWORDS", obs::SLOT_KEYWORDS)?;
    m.add("SLOT_STEALTH", obs::SLOT_STEALTH)?;
    m.add("SLOT_POISONOUS", obs::SLOT_POISONOUS)?;
    m.add("SLOT_MAGNETIZATIONS", obs::SLOT_MAGNETIZATIONS)?;
    m.add("SLOT_VOLUMIZERS", obs::SLOT_VOLUMIZERS)?;
    m.add("SLOT_IMPROVEMENTS", obs::SLOT_IMPROVEMENTS)?;
    m.add("SLOT_EVENTS_LEFT", obs::SLOT_EVENTS_LEFT)?;
    m.add("SLOT_FROM_POOL", obs::SLOT_FROM_POOL)?;
    m.add("SLOT_WINNERS_BREAD", obs::SLOT_WINNERS_BREAD)?;
    m.add("SLOT_TAUGHT_SPELL", obs::SLOT_TAUGHT_SPELL)?;
    m.add("SLOT_LOCKED", obs::SLOT_LOCKED)?;
    m.add("SLOT_ACTIVATE_COST", obs::SLOT_ACTIVATE_COST)?;
    m.add("SLOT_EXTRA_MAGNETIZE", obs::SLOT_EXTRA_MAGNETIZE)?;
    m.add("SLOT_COST", obs::SLOT_COST)?;
    m.add("SLOT_GEM_ATTACK", obs::SLOT_GEM_ATTACK)?;
    m.add("SLOT_GEM_HEALTH", obs::SLOT_GEM_HEALTH)?;
    m.add("SLOT_TEMP_SHIELD", obs::SLOT_TEMP_SHIELD)?;
    m.add("SLOT_DOOMED", obs::SLOT_DOOMED)?;
    m.add("SLOT_BOTH_EFFECTS", obs::SLOT_BOTH_EFFECTS)?;
    m.add("SLOT_LINKED", obs::SLOT_LINKED)?;
    m.add("SLOT_LINK_GROUP", obs::SLOT_LINK_GROUP)?;
    m.add("SLOT_TAVERN_SPELL", obs::SLOT_TAVERN_SPELL)?;
    m.add("SLOT_NO_REWARD", obs::SLOT_NO_REWARD)?;
    m.add("SLOT_ATTACHED", obs::SLOT_ATTACHED)?;
    m.add("SLOT_ATTACHED_GOLDEN", obs::SLOT_ATTACHED_GOLDEN)?;
    m.add("ATTACHED_SLOTS", bgcore::minion::MAX_ATTACHED)?;
    m.add("SLOT_GIFTS", obs::SLOT_GIFTS)?;
    m.add("GIFT_SLOTS", bgcore::MAX_GIFTS)?;
    m.add("SLOT_HATS", obs::SLOT_HATS)?;
    m.add("SLOT_TICKETS", obs::SLOT_TICKETS)?;
    m.add("SLOT_FROZEN", obs::SLOT_FROZEN)?;
    m.add("SLOT_REANIMATE", obs::SLOT_REANIMATE)?;
    m.add("SCALAR_DARK_GIFT_USES", obs::SCALAR_DARK_GIFT_USES)?;
    m.add("SCALAR_DARK_GIFT_PRESSED", obs::SCALAR_DARK_GIFT_PRESSED)?;
    m.add("SCALAR_DARK_GIFT_READY", obs::SCALAR_DARK_GIFT_READY)?;
    m.add("SCALAR_DARK_OFFER", obs::SCALAR_DARK_OFFER)?;
    m.add("SCALAR_PARADOX", obs::SCALAR_PARADOX)?;
    m.add("SCALAR_FREE_MINION_LEFT", obs::SCALAR_FREE_MINION_LEFT)?;
    m.add("SCALAR_CONVICTION_COUNT", obs::SCALAR_CONVICTION_COUNT)?;
    m.add("SCALAR_CONVICTION_ATTACK", obs::SCALAR_CONVICTION_ATTACK)?;
    m.add("SCALAR_CONVICTION_HEALTH", obs::SCALAR_CONVICTION_HEALTH)?;
    m.add("SCALAR_LOCKED_CARD", obs::SCALAR_LOCKED_CARD)?;
    m.add("SCALAR_LOCKED_ATTACK", obs::SCALAR_LOCKED_ATTACK)?;
    m.add("SCALAR_LOCKED_HEALTH", obs::SCALAR_LOCKED_HEALTH)?;
    m.add("SCALAR_PENDING_WAGER", obs::SCALAR_PENDING_WAGER)?;
    m.add("SCALAR_WAGER_OFFER", obs::SCALAR_WAGER_OFFER)?;
    m.add("SCALAR_WAGER_PICK", obs::SCALAR_WAGER_PICK)?;
    m.add("SCALAR_PENDING_HOLMES", obs::SCALAR_PENDING_HOLMES)?;
    m.add("SCALAR_HOLMES_OFFER", obs::SCALAR_HOLMES_OFFER)?;
    m.add("SCALAR_HOLMES_EMPTY", obs::SCALAR_HOLMES_EMPTY)?;
    m.add("OPP_ALIVE", obs::OPP_ALIVE)?;
    m.add("OPP_HEALTH", obs::OPP_HEALTH)?;
    m.add("OPP_TIER", obs::OPP_TIER)?;
    m.add("OPP_SEEN", obs::OPP_SEEN)?;
    m.add("OPP_TURNS_SINCE", obs::OPP_TURNS_SINCE)?;
    m.add("OPP_NEXT", obs::OPP_NEXT)?;
    m.add("OPP_LAST", obs::OPP_LAST)?;
    m.add("OPP_PLACEMENT", obs::OPP_PLACEMENT)?;
    m.add("OPP_TRIBES", obs::OPP_TRIBES)?;
    m.add("OPP_DEITY", obs::OPP_DEITY)?;
    m.add("OPP_DEITY_CTHUN", obs::OPP_DEITY_CTHUN)?;
    m.add("OPP_DEITY_YSHAARJ", obs::OPP_DEITY_YSHAARJ)?;
    m.add("OPP_DEITY_ATTACK", obs::OPP_DEITY_ATTACK)?;
    m.add("OPP_DEITY_HEALTH", obs::OPP_DEITY_HEALTH)?;
    m.add("OPP_DEITY_GOLDEN", obs::OPP_DEITY_GOLDEN)?;
    m.add("OPP_GEM_ATTACK", obs::OPP_GEM_ATTACK)?;
    m.add("OPP_GEM_HEALTH", obs::OPP_GEM_HEALTH)?;
    m.add("OPP_SPELL_ATTACK", obs::OPP_SPELL_ATTACK)?;
    m.add("OPP_SPELL_HEALTH", obs::OPP_SPELL_HEALTH)?;
    m.add("ACTION_KINDS", ACTION_KINDS.to_vec())?;
    m.add("END_TURN", Action::EndTurn.index())?;
    m.add("HERO_POWER", Action::HeroPower.index())?;
    m.add("HERO_POWER2", Action::HeroPower2.index())?;
    m.add("SCALAR_ARMOR", obs::SCALAR_ARMOR)?;
    m.add("SCALAR_HERO", obs::SCALAR_HERO)?;
    m.add("SCALAR_POWER_HERO", obs::SCALAR_POWER_HERO)?;
    m.add("SCALAR_POWER_COST", obs::SCALAR_POWER_COST)?;
    m.add("SCALAR_POWER_USES_LEFT", obs::SCALAR_POWER_USES_LEFT)?;
    m.add("SCALAR_POWER_USED_GAME", obs::SCALAR_POWER_USED_GAME)?;
    m.add("SCALAR_POWER_UNLOCKED", obs::SCALAR_POWER_UNLOCKED)?;
    m.add("SCALAR_POWER_USABLE", obs::SCALAR_POWER_USABLE)?;
    m.add("SCALAR_POWER_PROGRESS", obs::SCALAR_POWER_PROGRESS)?;
    m.add("SCALAR_POWER_LEFT", obs::SCALAR_POWER_LEFT)?;
    m.add("POWER_FEATURES", obs::POWER_FEATURES)?;
    m.add("SCALAR_POWER2_HERO", obs::SCALAR_POWER2_HERO)?;
    m.add("SCALAR_POWER2_COST", obs::SCALAR_POWER2_COST)?;
    m.add("SCALAR_POWER2_USES_LEFT", obs::SCALAR_POWER2_USES_LEFT)?;
    m.add("SCALAR_POWER2_USED_GAME", obs::SCALAR_POWER2_USED_GAME)?;
    m.add("SCALAR_POWER2_UNLOCKED", obs::SCALAR_POWER2_UNLOCKED)?;
    m.add("SCALAR_POWER2_USABLE", obs::SCALAR_POWER2_USABLE)?;
    m.add("SCALAR_POWER2_PROGRESS", obs::SCALAR_POWER2_PROGRESS)?;
    m.add("SCALAR_POWER2_LEFT", obs::SCALAR_POWER2_LEFT)?;
    m.add("SCALAR_PENDING_HERO", obs::SCALAR_PENDING_HERO)?;
    m.add("SCALAR_PENDING_POWER", obs::SCALAR_PENDING_POWER)?;
    m.add("SCALAR_POWER_OFFER", obs::SCALAR_POWER_OFFER)?;
    m.add("SCALAR_HERO_OFFER", obs::SCALAR_HERO_OFFER)?;
    m.add("SCALAR_TRINKET", obs::SCALAR_TRINKET)?;
    m.add("SCALAR_TRINKET_COUNT", obs::SCALAR_TRINKET_COUNT)?;
    m.add("SCALAR_TRINKET_TRIBE", obs::SCALAR_TRINKET_TRIBE)?;
    m.add("SCALAR_PENDING_TRINKET", obs::SCALAR_PENDING_TRINKET)?;
    m.add("SCALAR_TRINKET_OFFER_SLOT", obs::SCALAR_TRINKET_OFFER_SLOT)?;
    m.add("SCALAR_TRINKET_OPTION", obs::SCALAR_TRINKET_OPTION)?;
    m.add("TRINKET_OPTION_FEATURES", obs::TRINKET_OPTION_FEATURES)?;
    m.add("TRINKET_CHOICES", bgcore::TRINKET_CHOICES)?;
    m.add("TRINKET_SLOTS", bgcore::TRINKET_SLOTS)?;
    m.add("OPP_HERO", obs::OPP_HERO)?;
    m.add("OPP_ARMOR", obs::OPP_ARMOR)?;
    m.add("OPP_TRINKET", obs::OPP_TRINKET)?;
    m.add("OPP_TOP_TRIBE", obs::OPP_TOP_TRIBE)?;
    m.add("OPP_TRIBE_MIXED", obs::OPP_TRIBE_MIXED)?;
    m.add("OPP_TRIBE_COUNT", obs::OPP_TRIBE_COUNT)?;
    m.add("OPP_FIGHTS", obs::OPP_FIGHTS)?;
    m.add("FIGHT_FEATURES", obs::FIGHT_FEATURES)?;
    m.add("FIGHTS_SHOWN", bgcore::FIGHTS_SHOWN)?;
    m.add("OPP_FIGHT_HAPPENED", obs::OPP_FIGHT_HAPPENED)?;
    m.add("OPP_FIGHT_OUTCOME", obs::OPP_FIGHT_OUTCOME)?;
    m.add("OPP_FIGHT_DAMAGE", obs::OPP_FIGHT_DAMAGE)?;
    m.add("OLD_BOARD_AGE", obs::OLD_BOARD_AGE)?;
    m.add("LEARNER", LEARNER)?;
    m.add("SNAPSHOT", SNAPSHOT)?;
    m.add("STANDARD", STANDARD)?;
    m.add("RUSH", RUSH)?;
    m.add("RANDOM", RANDOM)?;
    m.add("REAL", REAL)?;
    m.add("SEAT_KINDS", SEAT_KINDS)?;
    Ok(())
}
