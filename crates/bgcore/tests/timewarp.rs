//! The 12 data-only Timewarped minions from the Timeworn Candelabra's Minor
//! and Major 2-cost lists. They are tokens with a `timewarp` marker, so no
//! normal Tavern, Discover or random-minion draw reaches them.

use std::sync::Arc;

use bgcore::fight::{Watch, fight_with};
use bgcore::{
    Action, CardDb, CardId, Counters, Data, Game, Keywords, Minion, Rng, Timewarp, Tribe, Tribes,
};

fn db() -> CardDb {
    Data::bundled().cards
}

fn game(seed: u64) -> Game {
    Game::new_with_lobby(Arc::new(Data::bundled()), seed, Tribes::ALL)
}

fn game_with(seed: u64, tribes: &[Tribe]) -> Game {
    let lobby = tribes.iter().copied().fold(Tribes::NONE, |m, t| m.with(t));
    Game::new_with_lobby(Arc::new(Data::bundled()), seed, lobby)
}

fn id(db: &CardDb, name: &str) -> CardId {
    db.by_name(name).expect(name)
}

fn mk(db: &CardDb, name: &str, golden: bool) -> Minion {
    db.make(id(db, name), golden)
}

fn stats(m: &Minion) -> (i32, i32) {
    (m.attack, m.health)
}

/// A pool minion straight onto the board; golden takes three copies.
fn put_on_board(g: &mut Game, p: usize, name: &str, golden: bool) {
    let c = id(&g.data.cards, name);
    let slot = g.data.cards.get(c).pool_slot as usize;
    let copies = if golden { 3 } else { 1 };
    g.pool[slot] -= copies;
    let m = g.players[p].make_card(&g.data.cards, c, golden, copies);
    g.players[p].board.push(m);
}

/// A Timewarped token onto the board. It has no pool copy.
fn put_token(g: &mut Game, p: usize, name: &str, golden: bool) {
    let c = id(&g.data.cards, name);
    let m = g.players[p].make_card(&g.data.cards, c, golden, 0);
    g.players[p].board.push(m);
}

fn put_in_hand(g: &mut Game, p: usize, name: &str, golden: bool) {
    let c = id(&g.data.cards, name);
    let slot = g.data.cards.get(c).pool_slot as usize;
    let copies = if golden { 3 } else { 1 };
    g.pool[slot] -= copies;
    let m = g.players[p].make_card(&g.data.cards, c, golden, copies);
    g.players[p].hand.push(m);
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

fn name(g: &Game, m: &Minion) -> String {
    g.data.cards.get(m.card).name.clone()
}

fn hand_of(g: &Game, p: usize, card: &str) -> usize {
    let c = id(&g.data.cards, card);
    g.players[p].hand.iter().filter(|m| m.card == c).count()
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

/// Everyone ends their turn until the next turn starts.
fn end_all_turns(g: &mut Game) {
    let t = g.turn;
    while g.turn == t {
        if !answer(g) {
            g.step(Action::EndTurn).unwrap();
        }
    }
}

fn play(g: &mut Game, p: usize, hand: u8, pos: u8) {
    act(g, p, Action::Play { hand, pos });
}

/// Floating Candle triggers the first board minion's Deathrattle in the shop.
fn trigger_first_deathrattle(g: &mut Game, name: &str, golden: bool) {
    put_token(g, 0, name, golden);
    g.give(0, "Floating Candle").unwrap();
    let h = g.players[0].hand.len() as u8 - 1;
    act(g, 0, Action::Play { hand: h, pos: 0 });
    act(g, 0, Action::Target(0));
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
fn the_timewarp_marker_keeps_each_card_out_of_the_normal_draws() {
    let db = db();
    let cards = [
        ("Timewarped Copter", Timewarp::Minor, 3),
        ("Timewarped Jazzer", Timewarp::Minor, 3),
        ("Timewarped Pillager", Timewarp::Minor, 3),
        ("Timewarped Duskmaw", Timewarp::Major, 5),
        ("Timewarped Geist", Timewarp::Major, 5),
        ("Timewarped Goldrinn", Timewarp::Major, 5),
        ("Timewarped Hooktail", Timewarp::Major, 5),
        ("Timewarped Imp-filtrator", Timewarp::Major, 5),
        ("Timewarped Anub'arak", Timewarp::Major, 5),
        ("Timewarped Nomi", Timewarp::Major, 5),
        ("Timewarped Nalaa", Timewarp::Major, 5),
        ("Timewarped Substrate", Timewarp::Major, 5),
    ];
    for (name, side, tier) in cards {
        let c = id(&db, name);
        let card = db.get(c);
        assert!(card.token, "{name} is a token");
        assert_eq!(card.timewarp, Some(side), "{name} side");
        assert_eq!(card.tier, tier, "{name} tier");
        assert!(!card.lobby_ok(Tribes::ALL), "{name} is never a normal draw");
        assert!(
            c as usize >= db.pool_cards,
            "{name} takes no pool slot"
        );
        let golden = db.make(c, true);
        assert_eq!(
            (golden.attack, golden.health),
            (card.attack * 2, card.health * 2),
            "{name} golden doubles"
        );
    }
}

#[test]
fn copter_avenge_gives_a_random_mech_and_golden_gives_two() {
    let db = db();
    let copter = id(&db, "Timewarped Copter");
    let weak = || {
        let mut m = mk(&db, "Microbot", false);
        m.attack = 0;
        m.health = 1;
        m
    };
    let mut killer = mk(&db, "Microbot", false);
    killer.attack = 50;
    killer.health = 1000;
    let gained = |golden: bool| {
        let mut c = db.make(copter, golden);
        c.attack = 0;
        c.health = 10000;
        let board = [weak(), weak(), weak(), c];
        let out = fight_with(
            &db,
            &board,
            &[killer],
            [Counters::default(); 2],
            500,
            &mut Rng::new(3),
            &mut (),
        );
        out.sides[0].cards_gained().to_vec()
    };
    let one = gained(false);
    assert_eq!(one.len(), 1, "Avenge (3) fires once");
    assert!(db.get(one[0]).tribes.has(Tribe::Mech), "a random Mech");
    let two = gained(true);
    assert_eq!(two.len(), 2, "golden gives two");
    assert!(two.iter().all(|&c| db.get(c).tribes.has(Tribe::Mech)));
}

#[test]
fn jazzer_deathrattle_raises_the_blood_gem_health() {
    let mut g = game(11);
    let before = g.players[0].gem_health;
    trigger_first_deathrattle(&mut g, "Timewarped Jazzer", false);
    assert_eq!(g.players[0].gem_health, before + 2);
    g.check_invariants().unwrap();

    let mut g = game(12);
    let before = g.players[0].gem_health;
    trigger_first_deathrattle(&mut g, "Timewarped Jazzer", true);
    assert_eq!(g.players[0].gem_health, before + 4, "golden doubles");
}

#[test]
fn pillager_deathrattle_gives_a_tavern_coin_and_golden_gives_two() {
    let mut g = game(13);
    trigger_first_deathrattle(&mut g, "Timewarped Pillager", false);
    assert_eq!(hand_of(&g, 0, "Tavern Coin"), 1);
    g.check_invariants().unwrap();

    let mut g = game(14);
    trigger_first_deathrattle(&mut g, "Timewarped Pillager", true);
    assert_eq!(hand_of(&g, 0, "Tavern Coin"), 2, "golden gets two");
}

#[test]
fn duskmaw_avenge_buffs_all_dragons() {
    let db = db();
    let duskmaw = id(&db, "Timewarped Duskmaw");
    let dragon = id(&db, "Bronze Timewalker");
    let seen = |golden: bool| {
        let mut weak = mk(&db, "Microbot", false);
        weak.attack = 0;
        weak.health = 1;
        let mut d = db.make(duskmaw, golden);
        d.attack = 0;
        d.health = 100;
        let mut drag = db.make(dragon, false);
        drag.attack = 1;
        drag.health = 100;
        let mut enemy = mk(&db, "Microbot", false);
        enemy.attack = 1;
        enemy.health = 100000;
        let mut seen = Attacks::default();
        fight_with(
            &db,
            &[weak, d, drag],
            &[enemy],
            [Counters::default(); 2],
            500,
            &mut Rng::new(5),
            &mut seen,
        );
        seen
    };
    let plain = seen(false);
    assert!(
        plain
            .0
            .iter()
            .any(|&(c, a, _)| c == dragon && a == 7),
        "Avenge (1) gave the Dragon +6 Attack"
    );
    let golden = seen(true);
    assert!(
        golden
            .0
            .iter()
            .any(|&(c, a, _)| c == dragon && a == 13),
        "golden doubles to +12 Attack"
    );
}

#[test]
fn geist_deathrattle_raises_the_tavern_spell_bonus() {
    let mut g = game(15);
    trigger_first_deathrattle(&mut g, "Timewarped Geist", false);
    assert_eq!(g.players[0].spell_bonus, (2, 2));
    g.check_invariants().unwrap();

    let mut g = game(16);
    trigger_first_deathrattle(&mut g, "Timewarped Geist", true);
    assert_eq!(g.players[0].spell_bonus, (4, 4), "golden doubles");
}

#[test]
fn goldrinn_deathrattle_buffs_beasts_and_golden_doubles() {
    let db = db();
    let goldrinn = id(&db, "Timewarped Goldrinn");
    let gain = |golden: bool| {
        let mut m = db.make(goldrinn, golden);
        m.health = 1;
        m.keywords.set(Keywords::TAUNT);
        let mut killer = mk(&db, "Microbot", false);
        killer.attack = 50;
        killer.health = 100;
        let out = fight_with(
            &db,
            &[m],
            &[killer],
            [Counters::default(); 2],
            500,
            &mut Rng::new(7),
            &mut (),
        );
        out.sides[0].beast_bonus_gain
    };
    assert_eq!(gain(false), (4, 4));
    assert_eq!(gain(true), (8, 8), "golden doubles");
}

#[test]
fn hooktail_buffs_all_on_a_tavern_spell_and_golden_does_it_twice() {
    let check = |golden: bool, expected: (i32, i32)| {
        let mut g = game(17);
        put_token(&mut g, 0, "Timewarped Hooktail", golden);
        put_on_board(&mut g, 0, "Wolf Pup", false);
        let before: Vec<(i32, i32)> = g.players[0].board.iter().map(stats).collect();
        g.give(0, "Tavern Coin").unwrap();
        let h = g.players[0].hand.len() as u8 - 1;
        play(&mut g, 0, h, 0);
        for (b, m) in before.iter().zip(&g.players[0].board) {
            assert_eq!(stats(m), (b.0 + expected.0, b.1 + expected.1));
        }
        g.check_invariants().unwrap();
    };
    check(false, (2, 2));
    check(true, (4, 4));
}

#[test]
fn imp_filtrator_buffs_the_tavern_after_eight_gold() {
    let check = |golden: bool, expected: i32| {
        let mut g = game(18);
        put_token(&mut g, 0, "Timewarped Imp-filtrator", golden);
        set_shop(&mut g, 0, &["Wolf Pup", "Cord Puller"]);
        let before: Vec<(i32, i32)> = g.players[0].shop.iter().map(stats).collect();
        g.players[0].gold = 9;
        g.players[0].upgrade_cost = 8;
        act(&mut g, 0, Action::Level);
        for (b, m) in before.iter().zip(&g.players[0].shop) {
            assert_eq!(
                stats(m),
                if g.data.cards.get(m.card).is_spell() {
                    *b
                } else {
                    (b.0 + expected, b.1 + expected)
                }
            );
        }
        // The buff lasts the game, so a later Refresh has it too.
        act(&mut g, 0, Action::Roll);
        for m in &g.players[0].shop {
            let c = g.data.cards.get(m.card);
            if !c.is_spell() {
                assert_eq!(stats(m), (c.attack + expected, c.health + expected));
            }
        }
        g.check_invariants().unwrap();
    };
    check(false, 3);
    check(true, 6);
}

#[test]
fn anubarak_buffs_undead_after_another_undead_is_played() {
    let check = |golden: bool, expected: u32| {
        let mut g = game(19);
        put_token(&mut g, 0, "Timewarped Anub'arak", golden);
        put_in_hand(&mut g, 0, "Cadaver Caretaker", false);
        let h = g.players[0].hand.len() as u8 - 1;
        play(&mut g, 0, h, 1);
        assert_eq!(g.players[0].undead_bonus, (expected as i32, 0));
        g.check_invariants().unwrap();
    };
    check(false, 3);
    check(true, 6);
}

#[test]
fn anubarak_does_not_count_its_own_play() {
    let mut g = game(23);
    g.give(0, "Timewarped Anub'arak").unwrap();
    let h = g.players[0].hand.len() as u8 - 1;
    play(&mut g, 0, h, 0);
    assert_eq!(g.players[0].undead_bonus, (0, 0));
    g.check_invariants().unwrap();
}

#[test]
fn nomi_buffs_the_tavern_after_an_elemental_is_played() {
    let check = |golden: bool, expected: (i32, i32)| {
        let mut g = game(20);
        put_token(&mut g, 0, "Timewarped Nomi", golden);
        set_shop(&mut g, 0, &["Wolf Pup", "Cord Puller"]);
        let before: Vec<(i32, i32)> = g.players[0].shop.iter().map(stats).collect();
        put_in_hand(&mut g, 0, "Waveling", false);
        let h = g.players[0].hand.len() as u8 - 1;
        play(&mut g, 0, h, 1);
        for (b, m) in before.iter().zip(&g.players[0].shop) {
            assert_eq!(stats(m), (b.0 + expected.0, b.1 + expected.1));
        }
        g.check_invariants().unwrap();
    };
    check(false, (4, 3));
    check(true, (8, 6));
}

#[test]
fn nalaa_gives_a_friendly_minion_of_each_type_stats_on_any_spell() {
    let check = |golden: bool, expected: (i32, i32)| {
        let mut g = game(21);
        put_token(&mut g, 0, "Timewarped Nalaa", golden);
        put_on_board(&mut g, 0, "Wolf Pup", false);
        put_on_board(&mut g, 0, "Cord Puller", false);
        put_on_board(&mut g, 0, "Wrath Weaver", false);
        let before: Vec<(i32, i32)> = g.players[0].board.iter().map(stats).collect();
        g.give(0, "Tavern Coin").unwrap();
        let h = g.players[0].hand.len() as u8 - 1;
        play(&mut g, 0, h, 0);
        for (i, m) in g.players[0].board.iter().enumerate().skip(1).take(3) {
            assert_eq!(
                stats(m),
                (before[i].0 + expected.0, before[i].1 + expected.1),
                "{}",
                name(&g, m)
            );
        }
        assert_eq!(stats(&g.players[0].board[0]), before[0], "Nalaa is neutral");
        g.check_invariants().unwrap();
    };
    check(false, (4, 3));
    check(true, (8, 6));
}

#[test]
fn substrate_gets_a_temperature_shift_at_the_end_of_the_turn() {
    let check = |golden: bool, count: usize| {
        let mut g = game_with(
            22,
            &[Tribe::Beast, Tribe::Mech, Tribe::Murloc, Tribe::Demon, Tribe::Pirate],
        );
        put_token(&mut g, 0, "Timewarped Substrate", golden);
        assert!(g.players[0].board[0]
            .keywords
            .has(Keywords::DIVINE_SHIELD));
        end_all_turns(&mut g);
        assert_eq!(hand_of(&g, 0, "Temperature Shift"), count);
        g.check_invariants().unwrap();
    };
    check(false, 1);
    check(true, 2);
}
