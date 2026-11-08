//! The cycle action: buy or play, sell, and sell what the line generated,
//! as one action, matching the same line played one ordinary action at a
//! time.

use std::sync::Arc;

use bgcore::cards::Tribes;
use bgcore::{Action, CardDb, Data, Game, Minion, N_ACTIONS};

fn data() -> Arc<Data> {
    Arc::new(Data::bundled())
}

fn db() -> CardDb {
    Data::bundled().cards
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

fn legal(g: &Game, a: Action) -> bool {
    let mut mask = [false; N_ACTIONS];
    g.legal_mask(&mut mask);
    mask[a.index()]
}

fn name(g: &Game, m: &Minion) -> String {
    g.data.cards.get(m.card).name.clone()
}

#[test]
fn shell_cycle_is_buy_play_sell() {
    let mut g = sandbox(7);
    ready(&mut g);
    g.players[0].lost_last_combat = true;
    place(&mut g, 0, 2, "Tortollan Blue Shell", false);
    place(&mut g, 0, 0, "Microbot", false);
    mine(&mut g);
    let gold = g.players[0].gold;
    assert!(legal(&g, Action::CycleShop(0)), "the Shell can be cycled");

    let mut hand = g.clone();
    hand.step(Action::Buy(0)).unwrap();
    hand.step(Action::Play { hand: 0, pos: 1 }).unwrap();
    hand.step(Action::Sell(1)).unwrap();

    g.step(Action::CycleShop(0)).unwrap();
    assert_eq!(g.players[0].gold, gold + 2, "the Shell is +2 Gold after a loss");
    assert_eq!(g.players[0].board.len(), 1);
    assert!(g.players[0].hand.is_empty());
    assert_eq!(g.players[0], hand.players[0], "the cycle is the same line");
    assert_eq!(g.pool, hand.pool, "every pool copy came back the same way");
}

#[test]
fn hand_cycle_is_play_and_sell() {
    let mut g = sandbox(11);
    ready(&mut g);
    place(&mut g, 0, 0, "Microbot", false);
    place(&mut g, 0, 1, "Microbot", false);
    mine(&mut g);

    let mut hand = g.clone();
    hand.step(Action::Play { hand: 0, pos: 1 }).unwrap();
    hand.step(Action::Sell(1)).unwrap();

    assert!(legal(&g, Action::CycleHand(0)), "a hand minion can be cycled");
    g.step(Action::CycleHand(0)).unwrap();
    assert_eq!(g.players[0], hand.players[0]);
    assert_eq!(g.pool, hand.pool);
}

/// Era's free Dragon cycle: Brann, Kalecgos and four Dragons, a Draconic
/// Warden in the Tavern for 3 Gold. The cycle sells the Warden and both
/// Chromadrakes Brann gives.
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
fn warden_cycle_matches_the_hand_played_line() {
    let mut g = warden_position(13);
    let gold = g.players[0].gold;
    assert!(legal(&g, Action::CycleShop(0)));

    let mut hand = g.clone();
    hand.step(Action::Buy(0)).unwrap();
    hand.step(Action::Play { hand: 0, pos: 6 }).unwrap();
    let drakes: Vec<u8> = (0..hand.players[0].hand.len() as u8).collect();
    assert_eq!(drakes.len(), 2, "Brann gives two Chromadrakes");
    assert!(
        hand.players[0].hand.iter().all(|m| db().chromadrakes.contains(&m.card)),
        "both are Chromadrakes"
    );
    hand.step(Action::Sell(6)).unwrap();
    for _ in 0..2 {
        hand.step(Action::Play { hand: 0, pos: 6 }).unwrap();
        hand.step(Action::Sell(6)).unwrap();
    }

    g.step(Action::CycleShop(0)).unwrap();
    assert_eq!(g.players[0].gold, gold, "the cycle is free");
    assert_eq!(g.players[0].board.len(), 6, "the board is as wide as it was");
    assert!(
        g.players[0].hand.iter().all(|m| db().get(m.card).is_spell()),
        "only the spells the Chromadrakes gave are left: {:?}",
        g.players[0].hand.iter().map(|m| name(&g, m)).collect::<Vec<_>>()
    );
    assert_eq!(g.players[0], hand.players[0], "the cycle is the same line");
    assert_eq!(g.pool, hand.pool, "every pool copy came back the same way");
}

#[test]
fn cycle_legality_is_cheap_and_strict() {
    // A full board has no room for the played card.
    let mut g = sandbox(17);
    ready(&mut g);
    g.players[0].lost_last_combat = true;
    place(&mut g, 0, 2, "Tortollan Blue Shell", false);
    for _ in 0..7 {
        place(&mut g, 0, 0, "Microbot", false);
    }
    place(&mut g, 0, 1, "Microbot", false);
    mine(&mut g);
    assert!(!legal(&g, Action::CycleShop(0)), "a full board cannot cycle");
    assert!(!legal(&g, Action::CycleHand(0)), "a full board cannot cycle a hand card");

    // A full hand has no room for the bought card.
    let mut g = sandbox(18);
    ready(&mut g);
    g.players[0].lost_last_combat = true;
    place(&mut g, 0, 2, "Tortollan Blue Shell", false);
    for _ in 0..bgcore::HAND {
        place(&mut g, 0, 1, "Microbot", false);
    }
    g.players[0].gold = 10;
    mine(&mut g);
    assert!(!legal(&g, Action::CycleShop(0)), "a full hand cannot buy");

    // Without the Gold for the buy.
    let mut g = sandbox(19);
    ready(&mut g);
    g.players[0].lost_last_combat = true;
    place(&mut g, 0, 2, "Tortollan Blue Shell", false);
    g.players[0].gold = 2;
    mine(&mut g);
    assert!(!legal(&g, Action::CycleShop(0)), "no Gold, no buy");

    // A spell cannot be sold.
    let mut g = sandbox(23);
    ready(&mut g);
    place(&mut g, 0, 2, "Blood Gem", false);
    mine(&mut g);
    assert!(!legal(&g, Action::CycleShop(0)), "a Tavern spell is not a cycle");
    let mut g = sandbox(29);
    ready(&mut g);
    place(&mut g, 0, 1, "Blood Gem", false);
    mine(&mut g);
    assert!(!legal(&g, Action::CycleHand(0)), "a hand spell is not a cycle");

    // The card's own play opens a Choose One: it is a cycle that pauses on
    // the choice now.
    let mut g = sandbox(31);
    ready(&mut g);
    place(&mut g, 0, 2, "Fearless Foodie", false);
    mine(&mut g);
    assert!(g.data.cards.play_opens_choice(g.players[0].shop[0].card));
    assert!(legal(&g, Action::CycleShop(0)), "a Choose One can be cycled");
    let mut g = sandbox(37);
    ready(&mut g);
    place(&mut g, 0, 1, "Fearless Foodie", false);
    mine(&mut g);
    assert!(legal(&g, Action::CycleHand(0)), "a Choose One can be cycled");

    // Faceless Manipulator is the one unsafe play: it becomes the copied
    // card, so there is no played card left to sell.
    let mut g = sandbox(38);
    ready(&mut g);
    place(&mut g, 0, 2, "Faceless Manipulator", false);
    mine(&mut g);
    assert!(g.data.cards.cycle_play_unsafe(g.players[0].shop[0].card));
    assert!(!legal(&g, Action::CycleShop(0)), "Faceless has no played card to sell");
    let mut g = sandbox(39);
    ready(&mut g);
    place(&mut g, 0, 1, "Faceless Manipulator", false);
    mine(&mut g);
    assert!(!legal(&g, Action::CycleHand(0)), "Faceless has no played card to sell");

    // And a plain minion is fine.
    let mut g = sandbox(41);
    ready(&mut g);
    place(&mut g, 0, 1, "Microbot", false);
    mine(&mut g);
    assert!(legal(&g, Action::CycleHand(0)));
}



#[test]
fn foodie_choice_cycle_matches_the_hand_line() {
    let mut g = sandbox(43);
    ready(&mut g);
    place(&mut g, 0, 0, "Felboar", false);
    place(&mut g, 0, 2, "Fearless Foodie", false);
    mine(&mut g);
    assert!(legal(&g, Action::CycleShop(0)));

    let mut hand = g.clone();
    hand.step(Action::Buy(0)).unwrap();
    hand.step(Action::Play { hand: 0, pos: 1 }).unwrap();
    hand.step(Action::Choose(1)).unwrap();
    hand.step(Action::Sell(1)).unwrap();

    g.step(Action::CycleShop(0)).unwrap();
    assert!(g.players[0].pending_cycle.is_some(), "the Choose One pauses the cycle");
    assert!(g.choice_waiting(0));
    assert!(!legal(&g, Action::EndTurn), "the open choice blocks End Turn");
    assert!(g.step(Action::EndTurn).is_err());
    g.step(Action::Choose(1)).unwrap();
    assert!(g.players[0].pending_cycle.is_none(), "the marker clears");
    assert_eq!(g.players[0], hand.players[0], "the cycle is the same line");
    assert_eq!(g.pool, hand.pool, "every pool copy came back the same way");
}

#[test]
fn discover_cycle_keeps_the_discovered_spell() {
    let mut g = sandbox(47);
    ready(&mut g);
    place(&mut g, 0, 2, "Rodeo Performer", false);
    mine(&mut g);
    assert!(legal(&g, Action::CycleShop(0)));

    let mut hand = g.clone();
    hand.step(Action::Buy(0)).unwrap();
    hand.step(Action::Play { hand: 0, pos: 0 }).unwrap();
    hand.step(Action::Choose(0)).unwrap();
    hand.step(Action::Sell(0)).unwrap();

    g.step(Action::CycleShop(0)).unwrap();
    assert!(g.players[0].pending_cycle.is_some(), "the Discover pauses the cycle");
    assert!(g.choice_waiting(0));
    g.step(Action::Choose(0)).unwrap();
    assert!(g.players[0].pending_cycle.is_none());
    assert_eq!(g.players[0], hand.players[0], "the cycle is the same line");
    assert_eq!(g.pool, hand.pool);
    assert_eq!(g.players[0].hand.len(), 1, "the Discovered spell stays");
    assert!(db().get(g.players[0].hand[0].card).is_spell());
}

#[test]
fn a_generated_choice_minion_pauses_and_finishes() {
    for seed in 50..400 {
        let mut g = sandbox(seed);
        ready(&mut g);
        place(&mut g, 0, 2, "Highkeeper Ra", false);
        mine(&mut g);
        let mut probe = g.clone();
        probe.step(Action::Buy(0)).unwrap();
        probe.step(Action::Play { hand: 0, pos: 0 }).unwrap();
        let Some(_) = probe
            .players[0]
            .hand
            .iter()
            .find(|m| db().get(m.card).name == "Veteran Brigand")
        else {
            continue;
        };
        g.step(Action::CycleShop(0)).unwrap();
        assert!(g.players[0].pending_cycle.is_some(), "the generated Choose One pauses");
        assert!(g.choice_waiting(0));
        g.step(Action::Choose(0)).unwrap();
        assert!(g.players[0].pending_cycle.is_none());
        assert_eq!(g.players[0].gold, 9, "Ra and Brigand bought and sold");
        assert!(g.players[0].board.is_empty());
        return;
    }
    panic!("no seed in 50..400 made Highkeeper Ra give Veteran Brigand");
}

#[test]
fn a_triple_on_the_buy_finishes_through_the_golden_choice() {
    let mut g = sandbox(53);
    ready(&mut g);
    place(&mut g, 0, 1, "Fearless Foodie", false);
    place(&mut g, 0, 1, "Fearless Foodie", false);
    place(&mut g, 0, 2, "Fearless Foodie", false);
    mine(&mut g);
    assert!(legal(&g, Action::CycleShop(0)));

    let gold = g.players[0].gold;
    g.step(Action::CycleShop(0)).unwrap();
    // The golden Foodie's Choose One pauses first; its eight Gems are the
    // payoff. The Triple Reward spell the buy generated is then played as
    // part of the line, and its Discover pauses too.
    assert!(g.players[0].pending_cycle.is_some(), "the golden Foodie's choice pauses");
    assert!(g.choice_waiting(0));
    g.step(Action::Choose(1)).unwrap();
    assert!(g.players[0].pending_cycle.is_some(), "the Triple Reward Discover pauses");
    assert!(g.choice_waiting(0));
    g.step(Action::Choose(0)).unwrap();
    assert!(g.players[0].pending_cycle.is_none());
    assert_eq!(g.players[0].gold, gold - 2, "buy 3, sell the golden Foodie for 1");
    assert_eq!(
        g.players[0].hand.iter().filter(|m| db().get(m.card).blood_gem).count(),
        8,
        "the golden Foodie's eight Gems stay"
    );
    let reward: Vec<_> = g.players[0]
        .hand
        .iter()
        .filter(|m| !db().get(m.card).blood_gem)
        .map(|m| m.card)
        .collect();
    assert_eq!(reward.len(), 1, "one Triple Reward card stays: {reward:?}");
    assert!(!db().get(reward[0]).is_spell());
    assert!(!g.players[0].hand.iter().any(|m| db().get(m.card).name == "Fearless Foodie"));
}

