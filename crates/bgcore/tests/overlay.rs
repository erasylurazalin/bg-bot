//! The comp overlay: active comp detection, the dominance lines (the Shell
//! and the Warden cycle without naming either), enabler picks at a Discover
//! or Choose One, the bonus array and the sell mask.

use std::sync::Arc;

use bgcore::cards::{ChooseOption, SpellEffect, Tribes};
use bgcore::comps::Comps;
use bgcore::game::PendingChooseOne;
use bgcore::overlay::{
    F_ACTIVE, F_ENABLER_OFFERED, F_ENABLER_TAKEN, F_LINE_FORCED, F_LINE_FOUND, OVERLAY_FLAGS,
    Overlay, ShellLine,
};
use bgcore::search::Position;
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
fn the_shell_line_is_found_without_naming_the_card() {
    let d = data();
    let o = Overlay::new(comps(&d), 2.0, &d);
    let mut g = sandbox(7);
    ready(&mut g);
    g.players[0].lost_last_combat = true;
    place(&mut g, 0, 0, "Ravaging Scorpid", false);
    place(&mut g, 0, 2, "Tortollan Blue Shell", false);
    mine(&mut g);
    let mut manual = g.clone();
    let gold = g.players[0].gold;

    let mut line = ShellLine::default();
    let mut bonus = [0.0f32; N_ACTIONS];
    let mut f = flags();
    let buy = o.decide(&g, 0, &mut line, &mut bonus, &mut f).expect("the buy");
    assert_eq!(buy, Action::Buy(0));
    assert_eq!(line.turn, g.turn);
    assert_eq!(f[F_LINE_FOUND], 1);
    assert_eq!(f[F_LINE_FORCED], 1);
    g.step(buy).unwrap();

    let mut f = flags();
    let play = o.decide(&g, 0, &mut line, &mut bonus, &mut f).expect("the play");
    assert_eq!(play, Action::Play { hand: 0, pos: 1 });
    assert_eq!(f[F_LINE_FOUND], 0, "the line started on the buy");
    assert_eq!(f[F_LINE_FORCED], 1);
    g.step(play).unwrap();

    let mut f = flags();
    let sell = o.decide(&g, 0, &mut line, &mut bonus, &mut f).expect("the sell");
    assert_eq!(sell, Action::Sell(1));
    assert_eq!(f[F_LINE_FORCED], 1);
    assert_eq!(line, ShellLine::default());
    g.step(sell).unwrap();

    manual.step(Action::Buy(0)).unwrap();
    manual.step(Action::Play { hand: 0, pos: 1 }).unwrap();
    manual.step(Action::Sell(1)).unwrap();
    assert_eq!(g.players[0], manual.players[0], "the line is the hand-played line");
    assert_eq!(g.pool, manual.pool);
    assert_eq!(g.players[0].gold, gold + 2, "+2 Gold after a loss");
    assert_eq!(g.players[0].board.len(), 1, "the one board minion is still there");
    assert!(g.players[0].hand.is_empty());
}

#[test]
fn a_card_that_sells_for_less_is_not_forced() {
    let d = data();
    let o = Overlay::new(comps(&d), 2.0, &d);
    let mut g = sandbox(11);
    ready(&mut g);
    // The Shell sells for 5 only after a loss; without one it costs 3 and
    // sells for less, so the line loses Gold and does not dominate.
    place(&mut g, 0, 2, "Tortollan Blue Shell", false);
    mine(&mut g);
    let mut line = ShellLine::default();
    let mut bonus = [0.0f32; N_ACTIONS];
    let mut f = flags();
    assert_eq!(o.decide(&g, 0, &mut line, &mut bonus, &mut f), None);
    assert_eq!(line, ShellLine::default());
    assert_eq!(f[F_LINE_FOUND], 0);
}

#[test]
fn a_full_board_has_no_dominating_line() {
    let d = data();
    let o = Overlay::new(comps(&d), 2.0, &d);
    let mut g = sandbox(13);
    ready(&mut g);
    g.players[0].lost_last_combat = true;
    for _ in 0..7 {
        place(&mut g, 0, 0, "Microbot", false);
    }
    place(&mut g, 0, 2, "Tortollan Blue Shell", false);
    mine(&mut g);
    // A line needs a free slot. Making one means selling a minion first, and
    // the sale on its own is a loss that dominance cannot call free value.
    let mut line = ShellLine::default();
    let mut bonus = [0.0f32; N_ACTIONS];
    let mut f = flags();
    assert_eq!(o.decide(&g, 0, &mut line, &mut bonus, &mut f), None);
    assert_eq!(line, ShellLine::default());
    assert_eq!(f[F_LINE_FOUND], 0);
}

#[test]
fn no_gold_means_no_line() {
    let d = data();
    let o = Overlay::new(comps(&d), 2.0, &d);
    let mut g = sandbox(17);
    ready(&mut g);
    g.players[0].lost_last_combat = true;
    g.players[0].gold = 2;
    place(&mut g, 0, 0, "Ravaging Scorpid", false);
    place(&mut g, 0, 2, "Tortollan Blue Shell", false);
    mine(&mut g);
    let mut line = ShellLine::default();
    let mut bonus = [0.0f32; N_ACTIONS];
    let mut f = flags();
    assert_eq!(o.decide(&g, 0, &mut line, &mut bonus, &mut f), None);
    assert_eq!(line, ShellLine::default());
}

/// Era's free Dragon cycle: Brann, Kalecgos and four Dragons on the board,
/// a Draconic Warden in the Tavern for 3 Gold.
fn warden_position(seed: u64) -> Game {
    let mut g = sandbox(seed);
    ready(&mut g);
    for name in ["Brann Bronzebeard", "Kalecgos, Arcane Aspect"] {
        place(&mut g, 0, 0, name, false);
    }
    for name in ["Bronze Warden", "Blue Whelp", "Tarecgosa", "Persistent Poet"] {
        place(&mut g, 0, 0, name, false);
    }
    place(&mut g, 0, 2, "Draconic Warden", false);
    mine(&mut g);
    g
}

#[test]
fn the_warden_cycle_is_found_without_naming_the_card() {
    let d = data();
    let o = Overlay::new(comps(&d), 2.0, &d);
    let mut g = warden_position(19);
    let before = Position::of(&g, 0);
    let gold = g.players[0].gold;
    let board = g.players[0].board.len();
    let mut manual = g.clone();
    let mut line = ShellLine::default();
    let mut bonus = [0.0f32; N_ACTIONS];
    let mut forced = Vec::new();
    for _ in 0..40 {
        let mut f = flags();
        let Some(a) = o.decide(&g, 0, &mut line, &mut bonus, &mut f) else { break };
        assert_eq!(f[F_LINE_FORCED], 1);
        forced.push(a);
        g.step(a).unwrap();
    }
    assert!(!forced.is_empty(), "the Warden cycle is free value");
    for a in &forced {
        manual.step(*a).unwrap();
    }
    assert_eq!(g.players[0], manual.players[0], "the line is the hand-played line");
    assert_eq!(g.pool, manual.pool);
    assert_eq!(g.players[0].gold, gold, "the cycle is free: {gold}");
    assert_eq!(g.players[0].board.len(), board, "the board is as wide as it was");
    let d = before.judge(&g);
    assert!(d.dominates, "blocked by {:?}", d.blocked);
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
    let mut line = ShellLine::default();
    let mut bonus = [0.0f32; N_ACTIONS];
    let mut f = flags();
    assert_eq!(o.decide(&g, 0, &mut line, &mut bonus, &mut f), Some(Action::Choose(1)));
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
    let mut line = ShellLine::default();
    let mut bonus = [0.0f32; N_ACTIONS];
    let mut f = flags();
    assert_eq!(o.decide(&g, 0, &mut line, &mut bonus, &mut f), None);
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
    let mut line = ShellLine::default();
    let mut bonus = [0.0f32; N_ACTIONS];
    let mut f = flags();
    assert_eq!(o.decide(&g, 0, &mut line, &mut bonus, &mut f), Some(Action::Choose(1)));
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
    let mut line = ShellLine::default();
    let mut bonus = [0.0f32; N_ACTIONS];
    let mut f = flags();
    assert_eq!(o.decide(&g, 0, &mut line, &mut bonus, &mut f), None);
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
    let mut line = ShellLine::default();
    let mut bonus = [0.0f32; N_ACTIONS];
    let mut f = flags();
    assert_eq!(o.decide(&g, 0, &mut line, &mut bonus, &mut f), None);
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
    let mut line = ShellLine::default();
    let mut bonus = [0.0f32; N_ACTIONS];
    let mut f = flags();
    assert_eq!(o.decide(&g, 0, &mut line, &mut bonus, &mut f), None);
    assert!(bonus.iter().all(|&b| b == 0.0));
    assert!(f.iter().all(|&v| v == 0));
}
