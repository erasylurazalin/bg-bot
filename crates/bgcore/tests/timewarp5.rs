//! Phase 5: the Timeworn Candelabra, both sides, and the seven logged
//! Candelabra Discovers.

use std::sync::Arc;

use bgcore::cards::Timewarp;
use bgcore::trinkets::Slot;
use bgcore::{Action, CardDb, CardId, Data, Game, Rng, Tribes};

fn db() -> CardDb {
    Data::bundled().cards
}

fn game(seed: u64) -> Game {
    Game::new_with_lobby(Arc::new(Data::bundled()), seed, Tribes::ALL)
}

fn answer(g: &mut Game) -> bool {
    let Some(p) = g.current_player() else { return false };
    let pl = &g.players[p];
    let mut cands = [Action::EndTurn; 4];
    let n = if !pl.discover.is_empty() || pl.pending_choose_one.is_some() {
        cands[0] = Action::Choose(0);
        1
    } else if pl.pending_discard.is_some() {
        cands[0] = Action::Discard(0);
        1
    } else if pl.pending_shop_target.is_some() {
        cands[0] = Action::TargetShop(0);
        1
    } else if pl.pending_target.is_some() {
        cands[0] = Action::Target(0);
        cands[1] = Action::TargetShop(0);
        2
    } else if !pl.trinket_offer.is_empty() {
        cands[0] = Action::ChooseTrinket(0);
        1
    } else {
        return false;
    };
    for &a in &cands[..n] {
        if g.is_legal(a) {
            g.step(a).ok();
            return true;
        }
    }
    false
}

fn turn_of(g: &mut Game, p: usize) {
    while g.current_player() != Some(p) {
        if !answer(g) {
            g.step(Action::Freeze).unwrap();
        }
    }
}

/// Pick the trinket with this card_id through the real offer path.
fn take(g: &mut Game, card_id: &str) -> u8 {
    let i = g.data.trinkets.by_card_id(card_id).expect(card_id);
    let slot = g.data.trinkets.get(i).slot;
    turn_of(g, 0);
    g.players[0].trinket_offer.clear();
    g.players[0].trinket_offer_slot = match slot {
        Slot::Lesser => 1,
        Slot::Greater => 2,
    };
    let _ = g.players[0].trinket_offer.try_push((i, 0));
    g.step(Action::ChooseTrinket(0)).unwrap();
    i
}

#[test]
fn both_candelabras_are_built_for_their_slots() {
    let d = Data::bundled();
    let trinkets = &d.trinkets;
    let lesser = trinkets.by_card_id("BG35_MagicItem_823").expect("lesser");
    let greater = trinkets.by_card_id("BG35_MagicItem_823t").expect("greater");
    assert_eq!(trinkets.get(lesser).slot, Slot::Lesser);
    assert_eq!(trinkets.get(greater).slot, Slot::Greater);
    assert_eq!(trinkets.get(lesser).name, "Timeworn Candelabra");
    assert_eq!(trinkets.get(greater).name, "Timeworn Candelabra");
    assert!(trinkets.get(lesser).tribes.is_empty());
    assert!(trinkets.get(greater).tribes.is_empty());
    use bgcore::trinkets::Act;
    assert!(matches!(
        trinkets.get(lesser).on_pick[0],
        Act::DiscoverTimewarp(Timewarp::Minor)
    ));
    assert!(matches!(
        trinkets.get(greater).on_pick[0],
        Act::DiscoverTimewarp(Timewarp::Major)
    ));
}

#[test]
fn the_built_lists_are_the_marked_tokens_and_ignore_the_lobby() {
    let db = db();
    let mut minor = 0;
    let mut major = 0;
    let mut minor_two = 0;
    let mut major_two = 0;
    for c in 0..db.cards.len() {
        if db.cards[c].timewarp.is_some() {
            assert!(db.cards[c].token, "{} is a token", db.cards[c].name);
            assert!(!db.cards[c].lobby_ok(Tribes::ALL));
            assert!(c >= db.pool_cards);
            match db.cards[c].timewarp {
                Some(Timewarp::Minor) => {
                    minor += 1;
                    minor_two += usize::from(db.cards[c].timewarp_cost == 2 && !db.cards[c].spell);
                }
                Some(Timewarp::Major) => {
                    major += 1;
                    major_two += usize::from(db.cards[c].timewarp_cost == 2 && !db.cards[c].spell);
                }
                None => {}
            }
        }
    }
    assert_eq!((minor, major), (23, 52), "75 built cards");
    // The Candelabra's own list is the 2-cost minions; the 1-cost cards and
    // the visit's spells belong to the full Timewarped shop the logs show.
    assert_eq!((minor_two, major_two), (19, 39), "58 Candelabra cards");

    // The draw has no lobby argument at all. Hunt every card on both sides.
    let mut rng = Rng::new(600);
    let mut seen_minor = std::collections::BTreeSet::new();
    let mut seen_major = std::collections::BTreeSet::new();
    for _ in 0..4000 {
        if let Some(c) = db.random_timewarp(Timewarp::Minor, &[], &mut rng) {
            seen_minor.insert(c);
        }
        if let Some(c) = db.random_timewarp(Timewarp::Major, &[], &mut rng) {
            seen_major.insert(c);
        }
    }
    assert_eq!(seen_minor.len(), 19);
    assert_eq!(seen_major.len(), 39);
    for c in seen_minor.into_iter().chain(seen_major) {
        assert!(db.get(c).token);
        assert!(db.get(c).timewarp.is_some());
        assert_eq!(db.get(c).timewarp_cost, 2, "the Candelabra list is 2-cost");
    }
}

#[test]
fn the_seven_logged_discovers_are_offered_except_the_unbuilt_ones() {
    let db = db();
    let logged = [
        ("Timewarped Lei", Timewarp::Minor),
        ("Timewarped Sporebat", Timewarp::Minor),
        ("Timewarped Pagle", Timewarp::Minor),
        ("Timewarped Elise", Timewarp::Minor),
        ("Timewarped Embalmer", Timewarp::Minor),
        ("Timewarped Pillager", Timewarp::Minor),
        ("Timewarped Skipper", Timewarp::Minor),
        ("Timewarped Copter", Timewarp::Minor),
        ("Timewarped Sellemental", Timewarp::Minor),
        ("Timewarped Caretaker", Timewarp::Major),
        ("Timewarped Anub'arak", Timewarp::Major),
        ("Timewarped Ghoul-acabra", Timewarp::Major),
        ("Timewarped Chameleon", Timewarp::Major),
        ("Timewarped Nalaa", Timewarp::Major),
        ("Timewarped Promo-Drake", Timewarp::Major),
        ("Timewarped Painter", Timewarp::Major),
        ("Timewarped Substrate", Timewarp::Major),
        ("Timewarped Geist", Timewarp::Major),
    ];
    let mut missing = Vec::new();
    for (name, side) in logged {
        match db.by_name(name) {
            Some(c) => assert_eq!(db.get(c).timewarp, Some(side), "{name} side"),
            None => missing.push(name),
        }
    }
    assert_eq!(missing, ["Timewarped Lei"]);
    for name in ["Timewarped Festergut", "Timewarped Commander", "Timewarped Seer"] {
        assert!(db.by_name(name).is_none(), "{name} is not built");
    }
}

#[test]
fn the_lesser_candelabra_offers_three_minor_tokens_without_touching_the_pool() {
    let mut g = game(601);
    let pool_before = g.pool.clone();
    take(&mut g, "BG35_MagicItem_823");
    assert_eq!(g.players[0].discover.len(), 3);
    let opts: Vec<CardId> = g.players[0].discover.iter().copied().collect();
    for &c in &opts {
        assert_eq!(g.data.cards.get(c).timewarp, Some(Timewarp::Minor));
        assert!(g.data.cards.get(c).token);
    }
    assert_eq!(g.pool, pool_before, "the offers hold no pool copies");
    let picked = opts[0];
    turn_of(&mut g, 0);
    g.step(Action::Choose(0)).unwrap();
    assert_eq!(g.pool, pool_before, "the pick moves no pool copy");
    assert!(g.players[0].hand.iter().any(|m| m.card == picked));
    g.check_invariants().unwrap();
}

#[test]
fn the_greater_candelabra_offers_three_major_tokens() {
    let mut g = game(602);
    let pool_before = g.pool.clone();
    take(&mut g, "BG35_MagicItem_823t");
    assert_eq!(g.players[0].discover.len(), 3);
    for &c in &g.players[0].discover {
        assert_eq!(g.data.cards.get(c).timewarp, Some(Timewarp::Major));
        assert!(g.data.cards.get(c).token);
    }
    assert_eq!(g.pool, pool_before);
    turn_of(&mut g, 0);
    assert_eq!(g.pool, pool_before);
    g.check_invariants().unwrap();
}

#[test]
fn the_candelabra_lists_include_the_neutral_lobby_tagged_cards() {
    let db = db();
    // Whelp Smuggler and Nomi are neutral with an associated lobby; the
    // logged draw offered Chameleon in a lobby without Beasts. The list is
    // built from the marker, not `lobby_ok`.
    let whelp = db.by_name("Timewarped Whelp Smuggler").unwrap();
    let nomi = db.by_name("Timewarped Nomi").unwrap();
    let chameleon = db.by_name("Timewarped Chameleon").unwrap();
    assert!(db.get(whelp).lobby.is_some());
    assert!(db.get(nomi).lobby.is_some());
    assert_eq!(db.get(chameleon).timewarp, Some(Timewarp::Major));
    let mut rng = Rng::new(603);
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..1000 {
        if let Some(c) = db.random_timewarp(Timewarp::Major, &[], &mut rng) {
            seen.insert(c);
        }
    }
    assert!(seen.contains(&nomi), "Nomi is on the Major list");
    assert!(seen.contains(&chameleon), "Chameleon is on the Major list");
    let mut seen_minor = std::collections::BTreeSet::new();
    for _ in 0..1000 {
        if let Some(c) = db.random_timewarp(Timewarp::Minor, &[], &mut rng) {
            seen_minor.insert(c);
        }
    }
    assert!(seen_minor.contains(&whelp));
}
