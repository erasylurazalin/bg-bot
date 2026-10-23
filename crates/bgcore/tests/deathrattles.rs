//! Every Deathrattle in the card pool, wherever it can fire: dying in a
//! fight, with Titus and golden Titus, and triggered by Deathstrider in the
//! shop through a Fishbait attack by a Rally minion.

use std::sync::Arc;

use bgcore::fight::{Counters, fight_with};
use bgcore::{Action, CardDb, CardId, Data, Game, Keywords, Minion, Rng};

fn card(db: &CardDb, name: &str) -> CardId {
    db.by_name(name).unwrap_or_else(|| panic!("no card {name}"))
}

fn make(db: &CardDb, name: &str) -> Minion {
    db.make(card(db, name), false)
}

/// A minion that kills in one hit and never dies.
fn wall(db: &CardDb) -> Minion {
    let mut m = make(db, "Microbot");
    m.attack = 50;
    m.health = 2000;
    m
}

/// The card under test. It dies to one hit and is the enemy's only Taunt.
/// Divine Shield is cleared so the shield does not eat the killing blow:
/// the loop measures the Deathrattle, not the keyword.
fn victim(db: &CardDb, c: CardId, golden: bool) -> Minion {
    let mut m = db.make(c, golden);
    m.health = 1;
    m.keywords.set(Keywords::TAUNT);
    m.keywords.clear(Keywords::DIVINE_SHIELD);
    m
}

fn deathrattle_cards(db: &CardDb) -> Vec<CardId> {
    (0..db.cards.len())
        .map(|c| c as CardId)
        .filter(|&c| db.get(c).deathrattle.is_some())
        .collect()
}

/// One fight, one attack, and how many Deathrattles the test side fired.
///
/// The wall side always has one minion more than the test side, so it
/// attacks first and the Taunt victim is the target. One attack kills it,
/// so no later death can add to the count.
fn fired(db: &CardDb, board: &[Minion]) -> u16 {
    let enemies: Vec<Minion> = (0..=board.len()).map(|_| wall(db)).collect();
    let out = fight_with(
        db,
        board,
        &enemies,
        [Counters::default(); 2],
        1,
        &mut Rng::new(7),
        &mut (),
    );
    out.sides[0].deathrattles_triggered
}

fn turn_of(g: &mut Game, p: usize) {
    while g.current_player() != Some(p) {
        g.step(Action::Freeze).unwrap();
    }
}

#[test]
fn every_deathrattle_fires_once_twice_and_three_times() {
    let db = Data::bundled().cards;
    let cards = deathrattle_cards(&db);
    assert!(cards.len() >= 19, "the loop should cover every Deathrattle card");
    for &c in &cards {
        let name = db.get(c).name.clone();
        for golden in [false, true] {
            let how = if golden { "golden" } else { "normal" };
            let one = fired(&db, &[victim(&db, c, golden)]);
            assert_eq!(one, 1, "{name} ({how}) dying in a fight");
            // A Start of Combat that destroys its neighbours (Stitched
            // Salvager) would destroy Titus too: keep a filler between.
            let gap: Vec<Minion> = if db.get(c).start_of_combat.is_some() {
                vec![make(&db, "Microbot")]
            } else {
                vec![]
            };
            let with = |titus: Minion| {
                let mut board = vec![victim(&db, c, golden)];
                board.extend(gap.iter().copied());
                board.push(titus);
                board
            };
            let titus = fired(&db, &with(make(&db, "Titus Rivendare")));
            assert_eq!(titus, 2, "{name} ({how}) dying with Titus on board");
            let golden_titus =
                fired(&db, &with(db.make(card(&db, "Titus Rivendare"), true)));
            assert_eq!(golden_titus, 3, "{name} ({how}) dying with golden Titus");
        }
    }
}

#[test]
fn deathstrider_triggers_every_deathrattle_in_the_shop() {
    let data = Arc::new(Data::bundled());
    let cards = deathrattle_cards(&data.cards);
    assert!(cards.len() >= 19, "the loop should cover every Deathrattle card");
    for &c in &cards {
        let name = data.cards.get(c).name.clone();
        for golden in [false, true] {
            let how = if golden { "golden" } else { "normal" };
            let mut g = Game::new(data.clone(), 11);
            // Wolf Pup goes first: it is the left-most Beast, so a Snarky
            // Shark's Fishbait is attacked by a Rally minion. The card under
            // test is the only other Deathrattle, so Deathstrider picks it.
            g.players[0].board.clear();
            g.players[0].board.push(data.cards.make(card(&data.cards, "Wolf Pup"), false));
            g.players[0].board.push(data.cards.make(card(&data.cards, "Deathstrider"), false));
            g.players[0].board.push(data.cards.make(c, golden));
            g.players[0].board.push(data.cards.make(card(&data.cards, "Snarky Shark"), false));
            assert_eq!(g.players[0].deathrattles_triggered, 0);
            turn_of(&mut g, 0);
            g.step(Action::Sell(3)).unwrap();
            assert_eq!(
                g.players[0].deathrattles_triggered, 1,
                "{name} ({how}) triggered by Deathstrider in the shop"
            );
            g.check_invariants().unwrap();
        }
    }
}
