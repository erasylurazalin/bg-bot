//! The Timewarped minions Era's logs show on a board: Icky Imp, Scout,
//! Wargear, Mythrax, Kil'rek, Immortal, Nine Frogs and Alleycat.

use std::sync::Arc;

use bgcore::fight::{Counters, Watch, fight_with};
use bgcore::{
    Action, CardDb, CardId, Data, Game, Keywords, Minion, Rng, Timewarp, Tribe, Tribes,
};

fn db() -> CardDb {
    Data::bundled().cards
}

fn game(seed: u64) -> Game {
    Game::new_with_lobby(Arc::new(Data::bundled()), seed, Tribes::ALL)
}

fn id(db: &CardDb, name: &str) -> CardId {
    db.by_name(name).expect(name)
}

fn mk(db: &CardDb, name: &str, golden: bool) -> Minion {
    db.make(id(db, name), golden)
}

fn put_token(g: &mut Game, p: usize, name: &str, golden: bool) {
    let c = id(&g.data.cards, name);
    let m = g.players[p].make_card(&g.data.cards, c, golden, 0);
    g.players[p].board.push(m);
}

fn put_on_board(g: &mut Game, p: usize, name: &str, golden: bool) {
    let c = id(&g.data.cards, name);
    let slot = g.data.cards.get(c).pool_slot as usize;
    let copies = if golden { 3 } else { 1 };
    g.pool[slot] -= copies;
    let m = g.players[p].make_card(&g.data.cards, c, golden, copies);
    g.players[p].board.push(m);
}

fn set_shop(g: &mut Game, p: usize, names: &[&str]) {
    let old: Vec<Minion> = g.players[p].shop.drain(..).collect();
    for m in old {
        let slot = g.data.cards.get(m.card).pool_slot as usize;
        g.pool[slot] += m.pool_copies;
    }
    for n in names {
        let c = id(&g.data.cards, n);
        let slot = g.data.cards.get(c).pool_slot as usize;
        g.pool[slot] -= 1;
        let m = g.players[p].make_card(&g.data.cards, c, false, 1);
        g.players[p].shop.push(m);
    }
}

fn answer(g: &mut Game) -> bool {
    let Some(p) = g.current_player() else { return false };
    let pl = &g.players[p];
    let mut cands = [Action::EndTurn; 4];
    let n = if !pl.discover.is_empty()
        || !pl.power_offer.is_empty()
        || pl.pending_choose_one.is_some()
    {
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

fn act(g: &mut Game, p: usize, a: Action) {
    turn_of(g, p);
    g.step(a).unwrap_or_else(|_| panic!("{a:?} illegal for player {p}"));
}

fn end_all_turns(g: &mut Game) {
    let t = g.turn;
    while g.turn == t {
        if !answer(g) {
            g.step(Action::EndTurn).unwrap();
        }
    }
}

/// Player 0's next opponent is a harmless wall, so a board survives the fight.
fn wall_opponent(g: &mut Game) {
    let Some(bgcore::Opponent::Player(q)) = g.players[0].opponent else { panic!() };
    let q = q as usize;
    g.players[q].board.clear();
    put_token(g, q, "Microbot", false);
    g.players[q].board[0].attack = 0;
    g.players[q].board[0].health = 1000;
}

#[derive(Default)]
struct Summoned(Vec<(usize, Minion)>);

impl Watch for Summoned {
    fn summon(&mut self, side: usize, _pos: usize, m: &Minion) {
        self.0.push((side, *m));
    }
}

#[derive(Default)]
struct Attacks(Vec<(CardId, i32, i32)>);

impl Watch for Attacks {
    fn attack(&mut self, side: usize, attacker: &Minion, _defender: &Minion) {
        if side == 0 {
            self.0.push((attacker.card, attacker.attack, attacker.health));
        }
    }
}

#[test]
fn the_phase_5_cards_are_timewarped_tokens_with_double_golden_stats() {
    let db = db();
    let cards = [
        ("Timewarped Icky Imp", Timewarp::Major, 5),
        ("Timewarped Scout", Timewarp::Major, 5),
        ("Timewarped Wargear", Timewarp::Major, 5),
        ("Timewarped Mythrax", Timewarp::Major, 5),
        ("Timewarped Kil'rek", Timewarp::Minor, 3),
        ("Timewarped Immortal", Timewarp::Major, 5),
        ("Timewarped Nine Frogs", Timewarp::Major, 5),
        ("Timewarped Alleycat", Timewarp::Minor, 3),
    ];
    for (name, side, tier) in cards {
        let c = id(&db, name);
        let card = db.get(c);
        assert!(card.token, "{name} is a token");
        assert_eq!(card.timewarp, Some(side), "{name} side");
        assert_eq!(card.tier, tier, "{name} tier");
        assert_eq!(card.timewarp_cost, 1, "{name} is a 1-Chronum card");
        assert!(!card.lobby_ok(Tribes::ALL), "{name} is never a normal draw");
        assert!(c as usize >= db.pool_cards, "{name} takes no pool slot");
        let golden = db.make(c, true);
        assert_eq!(
            (golden.attack, golden.health),
            (card.attack * 2, card.health * 2),
            "{name} golden doubles"
        );
    }
    assert!(db.by_name("Timewarped Lei").is_none(), "Lei needs Buddies");
    // Summoner's Spellcraft is not built, but the card is here so the logged
    // 11_17 visit offer loads. It carries no Timewarp marker, so it never
    // enters a Candelabra or visit pool.
    let summoner = db.by_name("Timewarped Summoner").expect("the logged offer loads");
    assert_eq!(db.get(summoner).timewarp, None, "Naga cards carry no marker");
}

#[test]
fn icky_imp_summons_imps_with_the_dead_minions_maximum_stats() {
    let db = db();
    let check = |golden: bool, count: usize| {
        let mut imp = mk(&db, "Timewarped Icky Imp", golden);
        imp.attack = 7;
        imp.health = 20;
        imp.keywords.set(Keywords::TAUNT);
        let mut killer = mk(&db, "Microbot", false);
        killer.attack = 50;
        killer.health = 1000;
        let mut seen = Summoned::default();
        let _ = fight_with(
            &db,
            &[imp],
            &[killer],
            [Counters::default(); 2],
            6,
            &mut Rng::new(3),
            &mut seen,
        );
        let imps: Vec<Minion> = seen
            .0
            .iter()
            .filter(|(side, m)| *side == 0 && db.get(m.card).name == "Imp")
            .map(|(_, m)| *m)
            .collect();
        assert_eq!(imps.len(), count, "Imp count, golden {golden}");
        for m in imps {
            assert_eq!((m.attack, m.health), (7, 20), "the dead minion's maximum stats");
        }
    };
    check(false, 2);
    check(true, 4);
}

#[test]
fn scout_improves_each_turn_and_sells_for_that_many_tier_seven_minions() {
    let check = |golden: bool, count: usize| {
        let mut g = game(61);
        put_token(&mut g, 0, "Timewarped Scout", golden);
        wall_opponent(&mut g);
        end_all_turns(&mut g);
        end_all_turns(&mut g);
        let slot = g.players[0]
            .board
            .iter()
            .position(|m| g.data.cards.get(m.card).name == "Timewarped Scout")
            .expect("the Scout survived the walls");
        assert_eq!(g.players[0].board[slot].improvements, 2, "one step per turn");
        act(&mut g, 0, Action::Sell(slot as u8));
        let tier_seven = g.players[0]
            .hand
            .iter()
            .filter(|m| !g.data.cards.get(m.card).is_spell() && g.data.cards.get(m.card).tier == 7)
            .count();
        assert_eq!(tier_seven, count);
        g.check_invariants().unwrap();
    };
    check(false, 3);
    check(true, 6);
}

#[test]
fn wargear_doubles_the_target_after_it_magnetizes() {
    let check = |golden: bool, expected: (i32, i32)| {
        let mut g = game(62);
        put_on_board(&mut g, 0, "Cord Puller", false);
        g.players[0].board[0].attack = 5;
        g.players[0].board[0].health = 5;
        let c = id(&g.data.cards, "Timewarped Wargear");
        let m = g.players[0].make_card(&g.data.cards, c, golden, 0);
        g.magnetize_onto(0, 0, m);
        assert_eq!(
            (g.players[0].board[0].attack, g.players[0].board[0].health),
            expected
        );
        g.check_invariants().unwrap();
    };
    // 5/5 host plus a 5/5 Wargear is 10/10, then doubled.
    check(false, (20, 20));
    // 5/5 host plus a golden 10/10 Wargear is 15/15, then tripled.
    check(true, (45, 45));
}

#[test]
fn mythrax_gains_the_stats_of_three_different_types() {
    let run = |golden: bool| -> (i32, i32) {
        let db = db();
        let mut mythrax = mk(&db, "Timewarped Mythrax", golden);
        mythrax.attack = if golden { 16 } else { 8 };
        mythrax.health = if golden { 18 } else { 9 };
        let mut beast = mk(&db, "Buzzing Vermin", false);
        beast.attack = 2;
        beast.health = 3;
        let mut mech = mk(&db, "Cord Puller", false);
        mech.attack = 4;
        mech.health = 5;
        let mut demon = mk(&db, "Wrath Weaver", false);
        demon.attack = 6;
        demon.health = 7;
        let mut wall = mk(&db, "Microbot", false);
        wall.attack = 0;
        wall.health = 1000;
        let mut seen = Attacks::default();
        let _ = fight_with(
            &db,
            &[mythrax, beast, mech, demon],
            &[wall],
            [Counters::default(); 2],
            3,
            &mut Rng::new(4),
            &mut seen,
        );
        let mine = id(&db, "Timewarped Mythrax");
        let (_, a, h) = seen
            .0
            .iter()
            .find(|(c, _, _)| *c == mine)
            .copied()
            .expect("Mythrax attacked");
        (a, h)
    };
    assert_eq!(run(false), (8 + 2 + 4 + 6, 9 + 3 + 5 + 7));
    assert_eq!(run(true), (16 + 2 * (2 + 4 + 6), 18 + 2 * (3 + 5 + 7)));
}

#[test]
fn kilrek_gets_one_random_demon_and_two_when_golden() {
    let db = db();
    let check = |golden: bool, count: usize| {
        let mut kilrek = mk(&db, "Timewarped Kil'rek", golden);
        kilrek.attack = 0;
        kilrek.health = 1;
        kilrek.keywords.set(Keywords::TAUNT);
        let mut killer = mk(&db, "Microbot", false);
        killer.attack = 50;
        killer.health = 1000;
        let out = fight_with(
            &db,
            &[kilrek],
            &[killer],
            [Counters::default(); 2],
            6,
            &mut Rng::new(5),
            &mut (),
        );
        let gained = out.sides[0].cards_gained();
        assert_eq!(gained.len(), count, "Demon count, golden {golden}");
        for c in gained {
            assert!(db.get(*c).tribes.has(Tribe::Demon), "{} is a Demon", db.get(*c).name);
        }
    };
    check(false, 1);
    check(true, 2);
}

#[test]
fn immortal_gains_the_stats_of_its_neighbours() {
    let run = |golden: bool| -> (i32, i32) {
        let db = db();
        let mut left = mk(&db, "Buzzing Vermin", false);
        left.attack = 2;
        left.health = 3;
        let mut immortal = mk(&db, "Timewarped Immortal", golden);
        immortal.attack = if golden { 16 } else { 8 };
        immortal.health = if golden { 16 } else { 8 };
        let mut right = mk(&db, "Cord Puller", false);
        right.attack = 4;
        right.health = 5;
        let mut wall = mk(&db, "Microbot", false);
        wall.attack = 0;
        wall.health = 1000;
        let mut seen = Attacks::default();
        let _ = fight_with(
            &db,
            &[left, immortal, right],
            &[wall],
            [Counters::default(); 2],
            3,
            &mut Rng::new(6),
            &mut seen,
        );
        let card = id(&db, "Timewarped Immortal");
        let (_, a, h) = seen
            .0
            .iter()
            .find(|(c, _, _)| *c == card)
            .copied()
            .expect("the Immortal attacked");
        (a, h)
    };
    assert_eq!(run(false), (8 + 2 + 4, 8 + 3 + 5));
    assert_eq!(run(true), (16 + 2 * (2 + 4), 16 + 2 * (3 + 5)));
}

#[test]
fn nine_frogs_gets_a_tavern_spell_of_the_bought_tier_nine_times() {
    let check = |golden: bool, count: usize| {
        let mut g = game(63);
        put_token(&mut g, 0, "Timewarped Nine Frogs", golden);
        assert_eq!(g.players[0].board[0].improvements, 9);
        set_shop(&mut g, 0, &["Buzzing Vermin"]);
        g.players[0].gold = 10;
        act(&mut g, 0, Action::Buy(0));
        assert_eq!(g.players[0].board[0].improvements, 8, "one use spent");
        let spells = g.players[0]
            .hand
            .iter()
            .filter(|m| g.data.cards.get(m.card).is_spell())
            .collect::<Vec<_>>();
        assert_eq!(spells.len(), count);
        for m in spells {
            assert_eq!(g.data.cards.get(m.card).tier, 1, "the bought minion's Tier");
            assert!(g.data.cards.get(m.card).tavern_spell);
        }
        g.check_invariants().unwrap();
    };
    check(false, 1);
    check(true, 2);
}

#[test]
fn alleycat_summons_tabbycats_with_its_stats_at_the_end_of_the_turn() {
    let check = |golden: bool, count: usize| {
        let mut g = game(64);
        put_token(&mut g, 0, "Timewarped Alleycat", golden);
        g.players[0].board[0].attack = 9;
        g.players[0].board[0].health = 11;
        wall_opponent(&mut g);
        end_all_turns(&mut g);
        let alleycat = id(&g.data.cards, "Timewarped Alleycat");
        let tabby = id(&g.data.cards, "Tabbycat");
        let cats: Vec<Minion> = g.players[0]
            .board
            .iter()
            .filter(|m| m.card == tabby)
            .copied()
            .collect();
        assert_eq!(cats.len(), count, "Tabbycat count, golden {golden}");
        for m in cats {
            assert_eq!((m.attack, m.health), (9, 11), "the Alleycat's stats");
        }
        assert!(
            g.players[0].board.iter().any(|m| m.card == alleycat),
            "the Alleycat stays"
        );
        g.check_invariants().unwrap();
    };
    check(false, 1);
    check(true, 2);
}
