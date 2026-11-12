//! Two arbitrary boards fought by the fight engine, for the tracker.
//!
//! The boards are JSON board objects in the shape `bgcore::real_boards`
//! parses (one entry of a board file's `boards` array), so a board from
//! `boards/real` and a board a tracker builds from a log go through the same
//! loader. Hero powers, trinkets, quest rewards, hands, counters and Deities
//! come along in that same shape.

use std::sync::Arc;

use bgcore::fight::fight_with_hands;
use bgcore::fight_check::{PreparedFight, prepare};
use bgcore::real_boards::{CardIdMap, RealBoardSpec, RealFightSpec, parse_board_files};
use bgcore::{Data, MAX_TIER, PLAYERS, Rng};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyDict;

/// One board text (the object a board file's `boards` array holds) through
/// `real_boards`'s own parser: no second parser, and every key it reads works.
fn board_spec_text(ids: &CardIdMap, text: &str) -> Result<RealBoardSpec, String> {
    // `parse_board_files` reads a whole file. A file with one board and no
    // game name is enough: the source and game are not read by a fight.
    let wrapped = format!(
        "{{\"source\":\"board\",\"game\":1,\"boards\":[{text}]}}"
    );
    let files = vec![("board".to_string(), wrapped)];
    let (mut specs, _unknown) = parse_board_files(&files, ids)?;
    if specs.len() != 1 {
        return Err("a board must be one JSON object in a board file's shape".to_string());
    }
    Ok(specs.remove(0))
}

fn board_spec(ids: &CardIdMap, text: &str) -> PyResult<RealBoardSpec> {
    board_spec_text(ids, text).map_err(PyValueError::new_err)
}

/// A board's Tavern tier for the damage sum: the file's own when it says one,
/// else the highest tier among its minions, as `RealBoard::build_one` does.
fn board_tier(spec: &RealBoardSpec, data: &Data) -> u8 {
    if spec.tier > 0 {
        return spec.tier.min(MAX_TIER);
    }
    spec.minions
        .iter()
        .filter_map(|m| data.cards.by_name(&m.name))
        .map(|id| data.cards.get(id).tier)
        .max()
        .unwrap_or(1)
        .clamp(1, MAX_TIER)
}

/// The board's winning blow: the winner's Tavern tier plus its surviving
/// minions' tiers, capped by the rules of the turn (`Game::combat`'s sum).
fn damage(raw: i32, data: &Data, turn: u32) -> i32 {
    data.rules
        .damage_cap(turn, PLAYERS)
        .map_or(raw, |cap| raw.min(cap))
}

/// What a batch of fights did: wins, ties and losses for `me`, and the damage
/// sum of every win (`dealt`) and loss (`taken`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Tally {
    wins: u32,
    ties: u32,
    losses: u32,
    dealt: i64,
    taken: i64,
}

impl Tally {
    fn add_win(&mut self, amount: i32) {
        self.wins += 1;
        self.dealt += amount as i64;
    }

    fn add_loss(&mut self, amount: i32) {
        self.losses += 1;
        self.taken += amount as i64;
    }
}

/// Fight `prepared` `n` times from `seed`, counting from `me`'s side.
fn fight_tally(
    data: &Data,
    prepared: &PreparedFight,
    turn: u32,
    me_tier: u8,
    opp_tier: u8,
    n: u32,
    seed: u64,
) -> Tally {
    let [me, opp] = &prepared.sides;
    let mut rng = Rng::new(seed);
    let mut tally = Tally::default();
    for _ in 0..n {
        let out = fight_with_hands(
            &data.cards,
            &me.board,
            &opp.board,
            [&me.hand, &opp.hand],
            [me.counters, opp.counters],
            data.rules.max_attacks,
            &mut rng,
            &mut (),
        );
        match out.winner {
            Some(0) => tally.add_win(damage(
                me_tier as i32 + out.survivor_tiers,
                data,
                turn,
            )),
            None => tally.ties += 1,
            Some(_) => tally.add_loss(damage(
                opp_tier as i32 + out.survivor_tiers,
                data,
                turn,
            )),
        }
    }
    tally
}

/// Two boards, seated once and then fought as many times as asked.
#[pyclass]
pub(crate) struct BoardFight {
    data: Arc<Data>,
    prepared: PreparedFight,
    turn: u32,
    me_tier: u8,
    opp_tier: u8,
}

#[pymethods]
impl BoardFight {
    /// `me` and `opp` are JSON board objects in a board file's shape,
    /// including `turn`, `tier` and `minions` (a `tier` of 0 takes the
    /// highest minion tier). `ref_cards` is the card id mapping
    /// `ref/bg-cards.json`: the current directory first, then its parent.
    #[new]
    #[pyo3(signature = (me, opp, ref_cards=None))]
    fn new(me: &str, opp: &str, ref_cards: Option<&str>) -> PyResult<Self> {
        let data = Arc::new(Data::bundled());
        let ids = match ref_cards {
            Some(path) => {
                let text = std::fs::read_to_string(path)
                    .map_err(|e| PyValueError::new_err(format!("{path}: {e}")))?;
                CardIdMap::parse(&text).map_err(PyValueError::new_err)?
            }
            None => crate::ref_card_ids()?,
        };
        let me = board_spec(&ids, me)?;
        let opp = board_spec(&ids, opp)?;
        let turn = me.turn;
        let me_tier = board_tier(&me, &data);
        let opp_tier = board_tier(&opp, &data);
        let fight = RealFightSpec { me, opp, next: None };
        let prepared = prepare(&data, &fight);
        Ok(BoardFight { data, prepared, turn, me_tier, opp_tier })
    }

    /// What the engine does not build on either side, one line each: a
    /// minion, hero power, trinket or Dark Gift it does not know, a quest
    /// reward, or a hand card the log did not name.
    fn missing(&self) -> Vec<String> {
        self.prepared.missing_lines()
    }

    fn complete(&self) -> bool {
        self.prepared.is_complete()
    }

    /// Fight the two boards `n` times with `seed`. Returns the shares of wins,
    /// ties and losses for `me`, the mean damage `me` deals when it wins, the
    /// mean damage `me` takes when it loses, and the same two means over every
    /// run. A tie deals no damage.
    fn fight(&self, py: Python<'_>, n: u32, seed: u64) -> PyResult<Py<PyDict>> {
        let t = fight_tally(
            &self.data,
            &self.prepared,
            self.turn,
            self.me_tier,
            self.opp_tier,
            n,
            seed,
        );
        let runs = n.max(1) as f64;
        let out = PyDict::new(py);
        out.set_item("n", n)?;
        out.set_item("wins", t.wins)?;
        out.set_item("ties", t.ties)?;
        out.set_item("losses", t.losses)?;
        out.set_item("win", round4(t.wins as f64 / runs))?;
        out.set_item("tie", round4(t.ties as f64 / runs))?;
        out.set_item("loss", round4(t.losses as f64 / runs))?;
        // What a win deals and what a loss takes, each over the fights it
        // happened in, and the same sums over every fight (zero on a tie).
        out.set_item(
            "damage_dealt",
            if t.wins == 0 { 0.0 } else { round4(t.dealt as f64 / t.wins as f64) },
        )?;
        out.set_item(
            "damage_taken",
            if t.losses == 0 { 0.0 } else { round4(t.taken as f64 / t.losses as f64) },
        )?;
        out.set_item("damage_dealt_mean", round4(t.dealt as f64 / runs))?;
        out.set_item("damage_taken_mean", round4(t.taken as f64 / runs))?;
        out.set_item("complete", self.prepared.is_complete())?;
        out.set_item("missing", self.prepared.missing_lines())?;
        out.set_item("turn", self.turn)?;
        Ok(out.unbind())
    }

    /// The names of every card the engine builds, for the tracker's coverage
    /// report: a pool card whose name is not here is one the engine has no
    /// card data for.
    #[staticmethod]
    fn built_card_names() -> Vec<String> {
        Data::bundled().cards.cards.iter().map(|c| c.name.clone()).collect()
    }
}

fn round4(x: f64) -> f64 {
    (x * 10_000.0).round() / 10_000.0
}


#[cfg(test)]
mod tests {
    use super::*;

    fn card_ids() -> CardIdMap {
        let path = format!("{}/../../ref/bg-cards.json", env!("CARGO_MANIFEST_DIR"));
        let text = std::fs::read_to_string(&path).expect("the ref card file");
        CardIdMap::parse(&text).expect("the ref card file parses")
    }

    #[test]
    fn a_known_board_pair_gives_a_fixed_result_for_a_fixed_seed() {
        let data = Data::bundled();
        let ids = card_ids();
        // Two 5/5 Campers against one 1/1 Bat: the Campers win every fight
        // and both are left standing, so a win deals tier 1 + two tier-1
        // survivors.
        let me = board_spec_text(
            &ids,
            r#"{"turn":1,"tier":1,"minions":[
                {"card_id":"BG33_886","attack":5,"health":5},
                {"card_id":"BG33_886","attack":5,"health":5}]}"#,
        )
        .unwrap();
        let opp = board_spec_text(
            &ids,
            r#"{"turn":1,"tier":1,"minions":[
                {"card_id":"BG36_200","attack":1,"health":1}]}"#,
        )
        .unwrap();
        let prepared = prepare(
            &data,
            &RealFightSpec { me: me.clone(), opp: opp.clone(), next: None },
        );
        assert!(prepared.is_complete());
        let tally = fight_tally(
            &data,
            &prepared,
            1,
            board_tier(&me, &data),
            board_tier(&opp, &data),
            200,
            42,
        );
        assert_eq!(tally, Tally { wins: 200, ties: 0, losses: 0, dealt: 600, taken: 0 });
    }
}
