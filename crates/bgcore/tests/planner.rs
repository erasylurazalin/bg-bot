//! The comp planner: the comp library, the standing Shell rule, the commit
//! and give-up rules, and determinism. The game numbers are in the brief's
//! report; these tests pin the bot's decisions.

use std::sync::Arc;

use bgcore::cards::Tribes;
use bgcore::comps::Comps;
use bgcore::planner::Planner;
use bgcore::{Action, Data, Game, Minion};

fn data() -> Arc<Data> {
    Arc::new(Data::bundled())
}

fn comps(d: &Data) -> Comps {
    Comps::from_toml(include_str!("../../../data/comps/comps.toml"), d).expect("the comp library")
}

/// Put a card in a zone, taking its pool copy so the accounting stays exact.
fn place(g: &mut Game, p: usize, zone: usize, name: &str, golden: bool) {
    let c = g.data.cards.by_name(name).unwrap_or_else(|| panic!("{name}"));
    let slot = g.data.cards.get(c).pool_slot as usize;
    let copies = if slot < g.pool.len() && !g.data.cards.get(c).token {
        g.pool[slot] -= 1;
        1
    } else {
        0
    };
    let m = g.players[p].make_card(&g.data.cards, c, golden, copies);
    match zone {
        0 => g.players[p].board.push(m),
        1 => g.players[p].hand.push(m),
        _ => g.players[p].shop.push(m),
    }
}

/// Empty the sandbox player's zones and give the pool every copy back.
fn ready(g: &mut Game) {
    for m in g.players[0].shop.drain(..).collect::<Vec<_>>() {
        let slot = g.data.cards.get(m.card).pool_slot as usize;
        g.pool[slot] += m.pool_copies;
    }
    for m in g.players[0].board.drain(..).collect::<Vec<_>>() {
        let slot = g.data.cards.get(m.card).pool_slot as usize;
        g.pool[slot] += m.pool_copies;
    }
    for m in g.players[0].hand.drain(..).collect::<Vec<_>>() {
        let slot = g.data.cards.get(m.card).pool_slot as usize;
        g.pool[slot] += m.pool_copies;
    }
    let pl = &mut g.players[0];
    pl.gold = 10;
    pl.done = false;
}

fn sandbox(seed: u64) -> Game {
    Game::new_sandbox(data(), seed, Some(Tribes::ALL), 0)
}

fn mine(g: &mut Game) {
    while g.current_player() != Some(0) {
        let a = if g.is_legal(Action::Freeze) { Action::Freeze } else { Action::EndTurn };
        g.step(a).unwrap();
    }
}

fn name(g: &Game, m: &Minion) -> String {
    g.data.cards.get(m.card).name.clone()
}

#[test]
fn the_comp_file_loads_both_drafts() {
    let d = data();
    let c = comps(&d);
    assert_eq!(c.comps.len(), 2);
    assert!(c.index_of("beetles").is_some());
    assert!(c.index_of("scam").is_some());
    for comp in &c.comps {
        assert!(comp.draft, "{} is marked draft", comp.name);
        assert!(!comp.core.is_empty(), "{} has a core", comp.name);
    }
    let beetles = c.index_of("beetles").map(|i| &c.comps[i]).unwrap();
    assert_eq!(beetles.give_up.as_ref().map(|g| g.to.as_str()), Some("scam"));
}

#[test]
fn an_unknown_card_name_fails_with_the_name() {
    let d = data();
    let text = r#"
[[comp]]
name = "broken"
core = [{ name = "No Such Card", weight = 1.0 }]
"#;
    let err = Comps::from_toml(text, &d).unwrap_err();
    assert!(err.contains("No Such Card"), "{err}");
    assert!(err.contains("broken"), "{err}");
}

#[test]
fn the_blue_shell_cycle_fires_after_a_loss() {
    let d = data();
    let c = comps(&d);
    let mut g = sandbox(7);
    ready(&mut g);
    g.players[0].lost_last_combat = true;
    place(&mut g, 0, 2, "Tortollan Blue Shell", false);
    // A beetle piece gives the planner a comp to ask (the Balinda rule).
    place(&mut g, 0, 0, "Forest Rover", false);
    mine(&mut g);
    let mut p = Planner::new(c, 11, &d);
    let a = p.action(&g, 0);
    assert_eq!(a, Action::CycleShop(0), "the Shell is bought, played and sold");
    assert_eq!(p.state.shell_cycle_turns, vec![1]);
    assert_eq!(p.state.shell_chance_turns, vec![1]);
}

#[test]
fn the_blue_shell_cycle_does_not_fire_without_the_loss() {
    let d = data();
    let c = comps(&d);
    let mut g = sandbox(11);
    ready(&mut g);
    place(&mut g, 0, 2, "Tortollan Blue Shell", false);
    place(&mut g, 0, 0, "Forest Rover", false);
    mine(&mut g);
    let mut p = Planner::new(c, 11, &d);
    let a = p.action(&g, 0);
    assert!(!matches!(a, Action::CycleShop(_)), "{a:?}");
    assert!(p.state.shell_cycle_turns.is_empty());
    assert!(p.state.shell_chance_turns.is_empty());
}

#[test]
fn a_full_board_sells_the_lowest_comp_weight_not_the_weakest() {
    let d = data();
    let c = comps(&d);
    let mut g = sandbox(13);
    ready(&mut g);
    g.players[0].lost_last_combat = true;
    place(&mut g, 0, 2, "Tortollan Blue Shell", false);
    // The core is positive weight; the Humming Bird is not_in_comp.
    for card in [
        "Buzzing Vermin",
        "Ravaging Scorpid",
        "Turquoise Skitterer",
        "Banana Slamma",
        "Holy Vanguard",
        "Humming Bird",
        "Forest Rover",
    ] {
        place(&mut g, 0, 0, card, false);
    }
    let bird = (0..g.players[0].board.len())
        .find(|&i| name(&g, &g.players[0].board[i]) == "Humming Bird")
        .expect("the bird is on the board");
    let weakest = (0..g.players[0].board.len())
        .min_by_key(|&i| {
            let m = &g.players[0].board[i];
            m.attack + m.health
        })
        .expect("a board");
    assert_ne!(bird, weakest, "the bird is not the weakest by stats");
    mine(&mut g);
    let mut p = Planner::new(c, 13, &d);
    let a = p.action(&g, 0);
    assert_eq!(a, Action::Sell(bird as u8), "sell the non-comp card, not the 1/1 core");
    assert_eq!(p.state.shell_chance_turns, vec![1], "the chance is recorded once");
}

#[test]
fn it_commits_to_beetles_and_switches_to_scam_on_give_up() {
    let d = data();
    let c = comps(&d);
    let mut g = sandbox(17);
    ready(&mut g);
    place(&mut g, 0, 0, "Banana Slamma", false);
    g.turn = 5;
    g.players[0].tier = 3;
    mine(&mut g);
    let mut p = Planner::new(c, 17, &d);
    let _ = p.action(&g, 0);
    assert_eq!(p.comp_name(), Some("beetles"), "Banana Slamma is a core signal");
    assert_eq!(p.state.committed_turn, Some(5));

    // Past the give-up turn, with only one weak core piece: drop to scam.
    ready(&mut g);
    place(&mut g, 0, 0, "Buzzing Vermin", false);
    g.turn = 11;
    g.players[0].tier = 6;
    g.players[0].done = false;
    let _ = p.action(&g, 0);
    assert_eq!(p.comp_name(), Some("scam"));
    assert_eq!(p.state.switched_turn, Some(11));
}

#[test]
fn the_same_seed_plays_the_same_actions() {
    let d = data();
    let c = comps(&d);
    let mut g = sandbox(23);
    ready(&mut g);
    place(&mut g, 0, 0, "Forest Rover", false);
    place(&mut g, 0, 2, "Buzzing Vermin", false);
    place(&mut g, 0, 2, "Turquoise Skitterer", false);
    mine(&mut g);
    let mut g1 = g.clone();
    let mut g2 = g.clone();
    let mut p1 = Planner::new(c.clone(), 99, &d);
    let mut p2 = Planner::new(c, 99, &d);
    for step in 0..6 {
        let a1 = p1.action(&g1, 0);
        let a2 = p2.action(&g2, 0);
        assert_eq!(a1, a2, "step {step}");
        if g1.is_legal(a1) {
            g1.step(a1).unwrap();
        }
        if g2.is_legal(a2) {
            g2.step(a2).unwrap();
        }
        if g1.over || g2.over {
            break;
        }
        mine(&mut g1);
        mine(&mut g2);
    }
    assert_eq!(g1.players[0], g2.players[0]);
    assert_eq!(g1.pool, g2.pool);
}
