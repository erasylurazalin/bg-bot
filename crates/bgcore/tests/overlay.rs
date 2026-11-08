//! The comp overlay: active comp detection, the Shell rules, enabler picks at
//! a Discover or Choose One, the bonus array and the sell mask.

use std::sync::Arc;

use bgcore::cards::{ChooseOption, SpellEffect, Tribes};
use bgcore::comps::Comps;
use bgcore::game::PendingChooseOne;
use bgcore::overlay::{
    F_ACTIVE, F_ENABLER_OFFERED, F_ENABLER_TAKEN, F_FORCED_SELL, F_SHELL_LEGAL, F_SHELL_TAKEN,
    OVERLAY_FLAGS, Overlay,
};
use bgcore::{Action, Data, Game, N_ACTIONS};

fn data() -> Arc<Data> {
    Arc::new(Data::bundled())
}

fn comps(d: &Data) -> Comps {
    Comps::from_toml(include_str!("../../../data/comps/comps.toml"), d).expect("the comp library")
}

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

fn flags() -> [i32; OVERLAY_FLAGS] {
    [0; OVERLAY_FLAGS]
}

#[test]
fn the_beetle_comp_has_its_overlay_cards() {
    let d = data();
    let c = comps(&d);
    let beetles = &c.comps[c.index_of("beetles").unwrap()];
    assert_eq!(beetles.enabler.len(), 1);
    assert!(beetles.is_enabler(beetles.enabler[0]));
    assert!(beetles.payoff.iter().any(|&id| beetles.is_payoff(id)));
    assert_eq!(beetles.filler.len(), 4);
    let scam = &c.comps[c.index_of("scam").unwrap()];
    assert!(scam.enabler.is_empty(), "scam has no trigger in this round");
}

#[test]
fn the_shell_is_forced_after_a_loss_with_room() {
    let d = data();
    let o = Overlay::new(comps(&d), 2.0, &d);
    let mut g = sandbox(7);
    ready(&mut g);
    g.players[0].lost_last_combat = true;
    place(&mut g, 0, 0, "Ravaging Scorpid", false);
    place(&mut g, 0, 2, "Tortollan Blue Shell", false);
    mine(&mut g);
    let mut bonus = [0.0f32; N_ACTIONS];
    let mut f = flags();
    assert_eq!(o.decide(&g, 0, &mut bonus, &mut f), Some(Action::CycleShop(0)));
    assert_eq!(f[F_SHELL_LEGAL], 1);
    assert_eq!(f[F_SHELL_TAKEN], 1);
    assert_eq!(f[F_ACTIVE], 1);
}

#[test]
fn the_shell_is_not_forced_without_a_loss() {
    let d = data();
    let o = Overlay::new(comps(&d), 2.0, &d);
    let mut g = sandbox(11);
    ready(&mut g);
    place(&mut g, 0, 0, "Ravaging Scorpid", false);
    place(&mut g, 0, 2, "Tortollan Blue Shell", false);
    mine(&mut g);
    let mut bonus = [0.0f32; N_ACTIONS];
    let mut f = flags();
    assert_eq!(o.decide(&g, 0, &mut bonus, &mut f), None);
    assert_eq!(f[F_SHELL_LEGAL], 0);
}

#[test]
fn a_full_board_shell_sells_the_non_comp_card() {
    let d = data();
    let o = Overlay::new(comps(&d), 2.0, &d);
    let mut g = sandbox(13);
    ready(&mut g);
    g.players[0].lost_last_combat = true;
    place(&mut g, 0, 0, "Ravaging Scorpid", false);
    place(&mut g, 0, 0, "Banana Slamma", false);
    for _ in 0..5 {
        place(&mut g, 0, 0, "Microbot", false);
    }
    place(&mut g, 0, 2, "Tortollan Blue Shell", false);
    mine(&mut g);
    let mut bonus = [0.0f32; N_ACTIONS];
    let mut f = flags();
    let a = o.decide(&g, 0, &mut bonus, &mut f).expect("a forced sale");
    let Action::Sell(slot) = a else { panic!("not a sale: {a:?}") };
    assert_eq!(g.data.cards.get(g.players[0].board[slot as usize].card).name, "Microbot");
    assert_eq!(f[F_SHELL_LEGAL], 1);
    assert_eq!(f[F_FORCED_SELL], 1);
    assert_eq!(f[F_SHELL_TAKEN], 0);
}

#[test]
fn a_full_board_shell_is_skipped_without_an_active_comp() {
    let d = data();
    let o = Overlay::new(comps(&d), 2.0, &d);
    let mut g = sandbox(17);
    ready(&mut g);
    g.players[0].lost_last_combat = true;
    for _ in 0..7 {
        place(&mut g, 0, 0, "Microbot", false);
    }
    place(&mut g, 0, 2, "Tortollan Blue Shell", false);
    mine(&mut g);
    let mut bonus = [0.0f32; N_ACTIONS];
    let mut f = flags();
    assert_eq!(o.decide(&g, 0, &mut bonus, &mut f), None);
    assert_eq!(f[F_SHELL_LEGAL], 0);
}

#[test]
fn an_enabler_at_a_discover_is_taken() {
    let d = data();
    let o = Overlay::new(comps(&d), 2.0, &d);
    let mut g = sandbox(19);
    ready(&mut g);
    let banana = g.data.cards.by_name("Banana Slamma").unwrap();
    let scorp = g.data.cards.by_name("Ravaging Scorpid").unwrap();
    g.players[0].discover.push(banana);
    g.players[0].discover.push(scorp);
    mine(&mut g);
    let mut bonus = [0.0f32; N_ACTIONS];
    let mut f = flags();
    assert_eq!(o.decide(&g, 0, &mut bonus, &mut f), Some(Action::Choose(1)));
    assert_eq!(f[F_ENABLER_OFFERED], 1);
    assert_eq!(f[F_ENABLER_TAKEN], 1);
    assert_eq!(f[F_ACTIVE], 0, "not active until the card is taken");
}

#[test]
fn an_enabler_is_not_taken_outside_the_lobby() {
    let d = data();
    let o = Overlay::new(comps(&d), 2.0, &d);
    let mut g = Game::new_sandbox(data(), 23, Some(Tribes::of(bgcore::Tribe::Mech)), 0);
    ready(&mut g);
    let scorp = g.data.cards.by_name("Ravaging Scorpid").unwrap();
    g.players[0].discover.push(scorp);
    mine(&mut g);
    let mut bonus = [0.0f32; N_ACTIONS];
    let mut f = flags();
    assert_eq!(o.decide(&g, 0, &mut bonus, &mut f), None);
    assert_eq!(f[F_ENABLER_OFFERED], 0);
}

#[test]
fn a_choose_one_direct_card_enabler_is_taken() {
    let d = data();
    let o = Overlay::new(comps(&d), 2.0, &d);
    let mut g = sandbox(29);
    ready(&mut g);
    let scorp = g.data.cards.by_name("Ravaging Scorpid").unwrap();
    let banana = g.data.cards.by_name("Banana Slamma").unwrap();
    g.players[0].pending_choose_one = Some(PendingChooseOne {
        source: None,
        golden: false,
        options: [
            ChooseOption::SpellEffect(SpellEffect::GetCard { card: banana, count: 1 }),
            ChooseOption::SpellEffect(SpellEffect::GetCard { card: scorp, count: 1 }),
        ],
        tavern: false,
        repeats: 1,
    });
    mine(&mut g);
    let mut bonus = [0.0f32; N_ACTIONS];
    let mut f = flags();
    assert_eq!(o.decide(&g, 0, &mut bonus, &mut f), Some(Action::Choose(1)));
    assert_eq!(f[F_ENABLER_TAKEN], 1);
}

#[test]
fn payoff_and_filler_get_the_bonus() {
    let d = data();
    let o = Overlay::new(comps(&d), 4.0, &d);
    let mut g = sandbox(31);
    ready(&mut g);
    place(&mut g, 0, 0, "Ravaging Scorpid", false);
    place(&mut g, 0, 0, "Banana Slamma", false);
    place(&mut g, 0, 0, "Turquoise Skitterer", false);
    place(&mut g, 0, 1, "Banana Slamma", false);
    place(&mut g, 0, 1, "Turquoise Skitterer", false);
    place(&mut g, 0, 2, "Banana Slamma", false);
    place(&mut g, 0, 2, "Turquoise Skitterer", false);
    mine(&mut g);
    let mut bonus = [0.0f32; N_ACTIONS];
    let mut f = flags();
    assert_eq!(o.decide(&g, 0, &mut bonus, &mut f), None);
    assert_eq!(bonus[Action::Buy(0).index()], 4.0, "payoff buy");
    assert_eq!(bonus[Action::Buy(1).index()], 2.0, "filler buy");
    assert_eq!(bonus[Action::Play { hand: 0, pos: 2 }.index()], 4.0, "payoff play");
    assert_eq!(bonus[Action::Play { hand: 1, pos: 2 }.index()], 2.0, "filler play");
    assert!(bonus[Action::Sell(0).index()].is_infinite(), "the enabler cannot be sold");
    assert!(bonus[Action::Sell(1).index()].is_infinite(), "a payoff cannot be sold");
    assert_eq!(bonus[Action::Sell(2).index()], 0.0, "a filler can be sold");
}

#[test]
fn a_full_board_gets_a_bonus_for_selling_a_non_comp_card() {
    let d = data();
    let o = Overlay::new(comps(&d), 2.0, &d);
    let mut g = sandbox(37);
    ready(&mut g);
    place(&mut g, 0, 0, "Ravaging Scorpid", false);
    for _ in 0..6 {
        place(&mut g, 0, 0, "Microbot", false);
    }
    place(&mut g, 0, 1, "Turquoise Skitterer", false);
    mine(&mut g);
    let mut bonus = [0.0f32; N_ACTIONS];
    let mut f = flags();
    assert_eq!(o.decide(&g, 0, &mut bonus, &mut f), None);
    assert_eq!(bonus[Action::Sell(0).index()], f32::NEG_INFINITY);
    assert_eq!(bonus[Action::Sell(1).index()], 2.0, "a non-comp card is worth selling");
}

#[test]
fn no_active_comp_changes_nothing() {
    let d = data();
    let o = Overlay::new(comps(&d), 8.0, &d);
    let mut g = sandbox(41);
    ready(&mut g);
    place(&mut g, 0, 0, "Microbot", false);
    place(&mut g, 0, 2, "Banana Slamma", false);
    place(&mut g, 0, 2, "Turquoise Skitterer", false);
    mine(&mut g);
    let mut bonus = [0.0f32; N_ACTIONS];
    let mut f = flags();
    assert_eq!(o.decide(&g, 0, &mut bonus, &mut f), None);
    assert!(bonus.iter().all(|&b| b == 0.0));
    assert!(f.iter().all(|&v| v == 0));
}
