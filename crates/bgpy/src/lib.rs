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
use std::sync::Arc;

use bgcore::bots::{random_action, rush_action, standard_action};
use bgcore::obs::{self, OBS_SIZE};
use bgcore::real_boards::{RealBoardSpec, RealBoards, RealCounters, RealDeitySpec, RealMinionSpec};
use bgcore::{ACTION_KINDS, Action, Data, Game, Keywords, MAX_TIER, N_ACTIONS, PLAYERS, Rng, TRIBES};
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
            // Rapid Reanimation mark and the no-reward mark are the replay's
            // (they come from the action files).
            reanimate: false,
            no_reward: false,
            dark_gift_states: Vec::new(),
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
        Env { game, kinds, rng, reported: [false; PLAYERS] }
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
    #[new]
    #[pyo3(signature = (n, seed, lineup, cards_toml=None, rules_toml=None, heroes_toml=None, real_boards=None))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        n: usize,
        seed: u64,
        lineup: [u8; SEAT_KINDS],
        cards_toml: Option<&str>,
        rules_toml: Option<&str>,
        heroes_toml: Option<&str>,
        real_boards: Option<PyBoards>,
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
            let specs: Vec<RealBoardSpec> = raw
                .into_iter()
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
                    // The rest of the file's board (the game it came from, its
                    // side and result, Health, armor, power, trinkets, quest
                    // rewards) is for the fight check, not for a seat.
                    ..RealBoardSpec::default()
                })
                .collect();
            // Names that match no card are dropped here; train/real_boards.py
            // reports them (it knows both stages of the mapping), so this
            // stays quiet: every eval VecEnv would repeat the line.
            let (boards, _) = RealBoards::build(&specs, &data.cards, &data.gifts);
            Arc::new(boards)
        });
        let lineup = check_lineup(lineup, boards.is_some())?;
        let seed = seed.wrapping_mul(1 << 32);
        let mut envs: Vec<Env> =
            (0..n as u64).map(|i| Env::new(&data, seed + i, &lineup, boards.as_ref())).collect();
        envs.par_iter_mut().for_each(Env::advance);
        Ok(VecEnv { data, envs, lineup, boards, next_seed: seed + n as u64 })
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
    m.add("N_HEROES", n_heroes)?;
    m.add("N_POWERS", n_powers)?;
    m.add("N_TRINKETS", n_trinkets)?;
    m.add("OBS_SIZE", OBS_SIZE)?;
    m.add("N_ACTIONS", N_ACTIONS)?;
    m.add("N_CARDS", n_cards)?;
    m.add("PLAYERS", PLAYERS)?;
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
    m.add("LEARNER", LEARNER)?;
    m.add("SNAPSHOT", SNAPSHOT)?;
    m.add("STANDARD", STANDARD)?;
    m.add("RUSH", RUSH)?;
    m.add("RANDOM", RANDOM)?;
    m.add("REAL", REAL)?;
    m.add("SEAT_KINDS", SEAT_KINDS)?;
    Ok(())
}
