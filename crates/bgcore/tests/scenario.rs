//! Scenario starts (`docs/briefs/comp-starts.md`): a late game with a real
//! board, a comp's core cards in hand, the tier and Health the plan asks for,
//! exact pool accounting, and determinism.

use std::sync::Arc;

use bgcore::bots::random_action;
use bgcore::comps::Comps;
use bgcore::real_boards::{RealBoardSpec, RealBoards, RealMinionSpec};
use bgcore::scenario::{ScenarioPlan, ScenarioSeat, deadline_tier};
use bgcore::{Data, Game, PLAYERS, Rng, Tribes};

fn data() -> Arc<Data> {
    Arc::new(Data::bundled())
}

fn comps(d: &Data) -> Comps {
    Comps::from_toml(include_str!("../../../data/comps/comps.toml"), d).expect("the comp library")
}

fn minion(name: &str, attack: i32, health: i32) -> RealMinionSpec {
    RealMinionSpec {
        name: name.to_string(),
        attack,
        health,
        golden: false,
        keywords: Vec::new(),
        numbers: Vec::new(),
        dark_gifts: Vec::new(),
        reanimate: false,
        no_reward: false,
        dark_gift_states: Vec::new(),
        vanguard_aura: [0, 0],
    }
}

/// One board of turn 7, so the draw has no choice.
fn one_board() -> Arc<RealBoards> {
    let spec = RealBoardSpec {
        turn: 7,
        tier: 5,
        minions: vec![minion("Amber Guardian", 3, 2), minion("Bonker", 2, 7)],
        ..RealBoardSpec::default()
    };
    let (boards, unmatched) = RealBoards::build(&[spec], &data().cards, &data().gifts);
    assert!(unmatched.is_empty(), "unmatched: {unmatched:?}");
    Arc::new(boards)
}

fn plan(comp_name: &str) -> (ScenarioPlan, Vec<bgcore::CardId>) {
    let d = data();
    let lib = comps(&d);
    let comp = lib.index_of(comp_name).expect("the comp");
    let mut core: Vec<_> = lib.comps[comp].core.iter().collect();
    core.sort_by(|a, b| b.weight.total_cmp(&a.weight));
    let core: Vec<_> = core.iter().map(|c| c.id).take(bgcore::HAND).collect();
    let tier = deadline_tier(7).max(
        lib.comps[comp]
            .core
            .iter()
            .map(|c| d.cards.get(c.id).tier)
            .max()
            .unwrap(),
    );
    let mut seats: [Option<ScenarioSeat>; PLAYERS] = std::array::from_fn(|_| None);
    seats[0] = Some(ScenarioSeat { comp, core: core.clone(), tier, health: 27 });
    (ScenarioPlan { turn: 7, lobby: Tribes::ALL, seats }, core)
}

fn assert_same(a: &Game, b: &Game) {
    assert_eq!(a.turn, b.turn);
    assert_eq!(a.current_player(), b.current_player());
    assert_eq!(a.pool, b.pool);
    for p in 0..PLAYERS {
        let (x, y) = (&a.players[p], &b.players[p]);
        assert_eq!(x.health, y.health, "player {p} Health");
        assert_eq!(x.tier, y.tier, "player {p} tier");
        assert_eq!(x.board.len(), y.board.len(), "player {p} board");
        for (m, n) in x.board.iter().zip(&y.board) {
            assert_eq!((m.card, m.attack, m.health), (n.card, n.attack, n.health));
        }
        assert_eq!(x.hand.len(), y.hand.len(), "player {p} hand");
        for (m, n) in x.hand.iter().zip(&y.hand) {
            assert_eq!((m.card, m.attack, m.health), (n.card, n.attack, n.health));
        }
    }
}

#[test]
fn a_scenario_start_has_the_body_hand_tier_and_pool() {
    let d = data();
    let (plan, core) = plan("Beasts - Tasty Lobstah");
    let g = Game::new_scenario(d.clone(), 42, &[], one_board(), &plan);

    assert_eq!(g.turn, 7, "the game starts on the drawn turn");
    assert!(g.scenario_game);
    let pl = &g.players[0];
    let body: Vec<_> = pl.board.iter().map(|m| m.card).collect();
    let amber = d.cards.by_name("Amber Guardian").unwrap();
    let bonker = d.cards.by_name("Bonker").unwrap();
    assert_eq!(body, vec![amber, bonker], "the drawn real board is the body");
    // The body is a real board's: it holds no pool copies.
    assert!(pl.board.iter().all(|m| m.pool_copies == 0));
    let hand: Vec<_> = pl.hand.iter().map(|m| m.card).collect();
    assert_eq!(hand, core, "every core card is in hand, highest weight first");
    assert!(pl.hand.iter().all(|m| m.pool_copies == 1));
    assert_eq!(pl.tier, 6, "the highest core card's tier wins over the curve's 4");
    assert_eq!(pl.health, 27, "the plan's Health, not a start-of-turn heal");
    assert_eq!(pl.gold, d.rules.gold(7), "the normal Gold for turn 7");
    // A scenario game hands out no Trinkets and no Dark Gifts.
    assert!(pl.trinkets.iter().all(|t| t.of == 0));
    assert!(pl.trinket_offer.is_empty());
    assert!(!g.is_legal(bgcore::Action::DarkGift));
    g.check_invariants().expect("exact pool accounting");
}

#[test]
fn two_scenario_games_with_one_seed_are_the_same() {
    let d = data();
    let (plan, _) = plan("Beasts - Tasty Lobstah");
    let boards = one_board();
    let mut a = Game::new_scenario(d.clone(), 73, &[], boards.clone(), &plan);
    let mut b = Game::new_scenario(d.clone(), 73, &[], boards, &plan);
    assert_same(&a, &b);
    let mut ra = Rng::new(99);
    let mut rb = Rng::new(99);
    for _ in 0..200 {
        assert_same(&a, &b);
        let aa = random_action(&a, &mut ra);
        let ba = random_action(&b, &mut rb);
        assert_eq!(aa, ba);
        a.step(aa).expect("the first copy's action is legal");
        b.step(ba).expect("the second copy's action is legal");
        a.check_invariants().expect("first copy invariants");
        b.check_invariants().expect("second copy invariants");
        if a.over {
            break;
        }
    }
    assert_eq!(a.over, b.over);
    assert_eq!(a.placements(), b.placements());
}

#[test]
fn a_normal_game_does_not_read_the_scenario_path() {
    let d = data();
    let mut a = Game::new_random_heroes(d.clone(), 11);
    let mut b = Game::new_random_heroes(d.clone(), 11);
    assert!(!a.scenario_game && !b.scenario_game);
    let mut ra = Rng::new(7);
    let mut rb = Rng::new(7);
    for _ in 0..200 {
        assert_same(&a, &b);
        let aa = random_action(&a, &mut ra);
        let ba = random_action(&b, &mut rb);
        a.step(aa).expect("legal");
        b.step(ba).expect("legal");
        if a.over {
            break;
        }
    }
    assert_eq!(a.placements(), b.placements());
}
