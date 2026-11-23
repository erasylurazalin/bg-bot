//! Murozond and Morchie: the two Timewarp visits, from the logs of the
//! current patch (`11_04_15_53_32-g5`, `11_17_00_28_04-g3` and
//! `11_14_00_45_13-g8`). The older games (`10_24_17_59_26-g2` and
//! `10_28_22_11_50-g4`) are patch 254477; their visit shape is the same, but
//! the engine models the current patch only.

use std::sync::Arc;

use bgcore::cards::Timewarp;
use bgcore::heroes::PowerEffect;
use bgcore::{Action, Data, Game, Tribes};

fn data() -> Arc<Data> {
    Arc::new(Data::bundled())
}

fn hero_game(seed: u64, hero: &str) -> Game {
    let d = data();
    let h = d.heroes.by_name(hero).expect(hero);
    Game::new_sandbox_with_hero(d, seed, Some(Tribes::ALL), 0, h)
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

/// Take the first Timewarp offer the Chronum can pay for, when one is open.
fn take_offer(g: &mut Game) -> bool {
    let Some(p) = g.current_player() else { return false };
    let pl = &g.players[p];
    if pl.timewarp_visit.is_none() {
        return false;
    }
    for s in 0..pl.timewarp_offers.len() as u8 {
        let a = Action::Buy(s);
        if g.is_legal(a) {
            g.step(a).unwrap();
            return true;
        }
    }
    false
}

/// End every turn until `turn` starts, keeping everyone alive.
fn to_turn(g: &mut Game, turn: u32) {
    while g.turn < turn && !g.over {
        for pl in &mut g.players {
            pl.health = pl.health.max(30);
        }
        let t = g.turn;
        while g.turn == t && !g.over {
            if !answer(g) && !take_offer(g) {
                g.step(Action::EndTurn).unwrap();
            }
        }
    }
}

#[test]
fn both_heroes_are_built_with_their_visit_powers() {
    let d = data();
    for (name, card, armor, side, turn) in [
        ("Murozond, Unbounded", "BG34_HERO_000", 12, Timewarp::Major, 8),
        ("Morchie", "BG34_HERO_004", 8, Timewarp::Minor, 5),
    ] {
        let h = d.heroes.by_name(name).expect(name);
        assert_eq!(d.heroes.get(h).card_id, card);
        assert_eq!(d.heroes.get(h).armor, armor);
        let power = d.heroes.hero_power(h);
        assert!(power.passive, "{name} has no button");
        assert_eq!(
            power.effect,
            PowerEffect::TimewarpVisit { turn, side },
            "{name}'s power"
        );
    }
}

#[test]
fn morchie_opens_the_minor_visit_on_turn_five() {
    let mut g = hero_game(700, "Morchie");
    for turn in 1..5 {
        to_turn(&mut g, turn);
        let progress = g.players[0].power.expect("a power").progress;
        assert_eq!(progress, 5 - turn, "turn {turn}'s turns left");
    }
    to_turn(&mut g, 5);
    {
        let pl = &g.players[0];
        assert_eq!(pl.power.expect("a power").progress, 1, "the log keeps 1");
        assert_eq!(pl.timewarp_visit.as_ref().expect("a visit").side, Timewarp::Minor);
        assert_eq!(pl.gold, 2, "2 Chronum where Gold lives");
        assert_eq!(pl.timewarp_offers.len(), 5, "five offers");
        for &c in &pl.timewarp_offers {
            let card = g.data.cards.get(c);
            assert_eq!(card.timewarp, Some(Timewarp::Minor), "{}", card.name);
            assert!((1..=2).contains(&card.timewarp_cost), "{}", card.name);
        }
    }
    // Spend the Chronum, then the normal Tavern and Gold come back.
    while g.players[0].timewarp_visit.is_some() {
        assert!(take_offer(&mut g), "an offer is affordable");
    }
    assert!(g.players[0].gold > 2, "the turn's Gold is back");
    assert!(!g.players[0].shop.is_empty(), "the normal Tavern is back");
    g.check_invariants().unwrap();
}

#[test]
fn murozond_opens_the_major_visit_on_turn_eight() {
    let mut g = hero_game(701, "Murozond, Unbounded");
    to_turn(&mut g, 7);
    assert_eq!(g.players[0].power.expect("a power").progress, 1);
    assert!(g.players[0].timewarp_visit.is_none(), "no visit before turn 8");
    to_turn(&mut g, 8);
    {
        let pl = &g.players[0];
        assert_eq!(pl.timewarp_visit.as_ref().expect("a visit").side, Timewarp::Major);
        assert_eq!(pl.gold, 2);
        assert_eq!(pl.timewarp_offers.len(), 5, "five offers");
        for &c in &pl.timewarp_offers {
            let card = g.data.cards.get(c);
            assert_eq!(card.timewarp, Some(Timewarp::Major), "{}", card.name);
            assert!((1..=2).contains(&card.timewarp_cost), "{}", card.name);
        }
    }
    while g.players[0].timewarp_visit.is_some() {
        assert!(take_offer(&mut g), "an offer is affordable");
    }
    assert!(g.players[0].gold > 2);
    g.check_invariants().unwrap();
}
