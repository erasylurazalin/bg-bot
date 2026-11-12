//! The 15 Phase 2 Timewarped minions that need new code, from the Timeworn
//! Candelabra's Minor and Major 2-cost lists.

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

fn put_on_board(g: &mut Game, p: usize, name: &str, golden: bool) {
    let c = id(&g.data.cards, name);
    let slot = g.data.cards.get(c).pool_slot as usize;
    let copies = if golden { 3 } else { 1 };
    g.pool[slot] -= copies;
    let m = g.players[p].make_card(&g.data.cards, c, golden, copies);
    g.players[p].board.push(m);
}

fn put_token(g: &mut Game, p: usize, name: &str, golden: bool) {
    let c = id(&g.data.cards, name);
    let m = g.players[p].make_card(&g.data.cards, c, golden, 0);
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

fn weak(db: &CardDb) -> Minion {
    let mut m = mk(db, "Microbot", false);
    m.attack = 0;
    m.health = 1;
    m
}

fn killer(db: &CardDb, attack: i32, health: i32) -> Minion {
    let mut m = mk(db, "Microbot", false);
    m.attack = attack;
    m.health = health;
    m
}

#[derive(Default)]
struct Attacks(usize);

impl Watch for Attacks {
    fn attack(&mut self, side: usize, _attacker: &Minion, _defender: &Minion) {
        if side == 1 {
            self.0 += 1;
        }
    }
}

#[test]
fn the_phase_2_marker_keeps_each_card_out_of_the_normal_draws() {
    let db = db();
    let cards = [
        ("Timewarped Arm", Timewarp::Minor, 3),
        ("Timewarped Elegist", Timewarp::Minor, 3),
        ("Timewarped Geomancer", Timewarp::Minor, 3),
        ("Timewarped Jelly Belly", Timewarp::Minor, 3),
        ("Timewarped Low-Flier", Timewarp::Minor, 3),
        ("Timewarped Pagle", Timewarp::Minor, 3),
        ("Timewarped Piper", Timewarp::Minor, 3),
        ("Timewarped Recycler", Timewarp::Minor, 3),
        ("Timewarped Guard", Timewarp::Minor, 3),
        ("Timewarped Bonker", Timewarp::Major, 5),
        ("Timewarped Sellemental", Timewarp::Minor, 3),
        ("Timewarped Skipper", Timewarp::Minor, 3),
        ("Timewarped Sporebat", Timewarp::Minor, 3),
        ("Timewarped Behemoth", Timewarp::Major, 5),
        ("Timewarped Caretaker", Timewarp::Major, 5),
    ];
    for (n, side, tier) in cards {
        let c = id(&db, n);
        let card = db.get(c);
        assert!(card.token, "{n} is a token");
        assert_eq!(card.timewarp, Some(side), "{n} side");
        assert_eq!(card.tier, tier, "{n} tier");
        assert!(!card.lobby_ok(Tribes::ALL), "{n} is never a normal draw");
        assert!(c as usize >= db.pool_cards, "{n} takes no pool slot");
    }
}

#[test]
fn arm_gives_the_attacked_minion_attack_for_good() {
    let db = db();
    let arm = id(&db, "Timewarped Arm");
    let check = |golden: bool, per_hit: i32| {
        let mut a = db.make(arm, golden);
        a.health = 10000;
        let enemy = killer(&db, 1, 10000);
        let mut seen = Attacks::default();
        let out = fight_with(
            &db,
            &[a],
            &[enemy],
            [Counters::default(); 2],
            20,
            &mut Rng::new(3),
            &mut seen,
        );
        assert_eq!(out.sides[0].permanent_stats[0].0, per_hit * seen.0 as i32);
    };
    check(false, 8);
    check(true, 16);
}

#[test]
fn elegist_buffs_the_board_and_hand_at_the_end_of_the_turn() {
    let check = |golden: bool, expected: (i32, i32)| {
        let mut g = game(11);
        put_token(&mut g, 0, "Timewarped Elegist", golden);
        put_on_board(&mut g, 0, "Cord Puller", false);
        g.give(0, "Wolf Pup").unwrap();
        let hand_before = stats(&g.players[0].hand[0]);
        let board_before: Vec<(i32, i32)> = g.players[0].board.iter().map(stats).collect();
        wall_opponent(&mut g);
        end_all_turns(&mut g);
        for (b, m) in board_before.iter().zip(&g.players[0].board) {
            assert_eq!(stats(m), (b.0 + expected.0, b.1 + expected.1));
        }
        let hand = &g.players[0].hand[0];
        assert_eq!(
            stats(hand),
            (hand_before.0 + expected.0, hand_before.1 + expected.1)
        );
        g.check_invariants().unwrap();
    };
    check(false, (2, 1));
    check(true, (4, 2));
}

#[test]
fn geomancer_avenge_gets_a_gem_and_raises_the_gem_value() {
    let db = db();
    let geomancer = id(&db, "Timewarped Geomancer");
    let check = |golden: bool, expected: (i32, i32)| {
        let mut g = db.make(geomancer, golden);
        g.health = 1000;
        let board = [weak(&db), weak(&db), weak(&db), weak(&db), weak(&db), g];
        let enemy = killer(&db, 50, 10000);
        let out = fight_with(
            &db,
            &board,
            &[enemy],
            [Counters::default(); 2],
            500,
            &mut Rng::new(5),
            &mut (),
        );
        assert_eq!(out.sides[0].gem_gain, expected);
        let gem = db.blood_gem.unwrap();
        assert_eq!(out.sides[0].cards_gained().iter().filter(|&&c| c == gem).count(), 1);
    };
    check(false, (1, 1));
    check(true, (2, 2));
}

#[test]
fn jelly_belly_buffs_everyone_after_a_friendly_reborn() {
    let db = db();
    let jelly = id(&db, "Timewarped Jelly Belly");
    let check = |golden: bool, expected: i32| {
        let mut p = db.make(id(&db, "Timewarped Pillager"), false);
        p.health = 1;
        let mut j = db.make(jelly, golden);
        j.health = 1000;
        let enemy = killer(&db, 50, 10000);
        let out = fight_with(
            &db,
            &[p, j],
            &[enemy],
            [Counters::default(); 2],
            10,
            &mut Rng::new(7),
            &mut (),
        );
        assert!(out.sides[0].permanent_stats[1].0 >= expected, "the Jelly Belly grew");
        assert!(
            out.sides[0].permanent_stats[0].0 >= expected,
            "the reborn minion kept it too"
        );
    };
    check(false, 2);
    check(true, 4);
}

#[test]
fn low_flier_buffs_below_its_attack_then_below_its_health() {
    let check = |golden: bool, step: i32| {
        let mut g = game(13);
        put_token(&mut g, 0, "Timewarped Low-Flier", golden);
        put_token(&mut g, 0, "Microbot", false);
        put_on_board(&mut g, 0, "Cord Puller", false);
        put_on_board(&mut g, 0, "Wolf Pup", false);
        g.players[0].board[1].attack = 1;
        g.players[0].board[1].health = 20;
        g.players[0].board[2].attack = 20;
        g.players[0].board[2].health = 1;
        g.players[0].board[3].attack = 1;
        g.players[0].board[3].health = 1;
        let host = stats(&g.players[0].board[0]);
        wall_opponent(&mut g);
        end_all_turns(&mut g);
        assert_eq!(stats(&g.players[0].board[1]), (1 + step, 20));
        assert_eq!(stats(&g.players[0].board[2]), (20, 1 + step));
        assert_eq!(stats(&g.players[0].board[3]), (1 + step, 1 + step));
        assert_eq!(stats(&g.players[0].board[0]), host, "the host is not below itself");
        g.check_invariants().unwrap();
    };
    check(false, 2);
    check(true, 4);
}

#[test]
fn pagle_gives_one_triple_reward_a_combat_and_golden_two() {
    let db = db();
    let pagle = id(&db, "Timewarped Pagle");
    let check = |golden: bool, count: usize| {
        let mut p = db.make(pagle, golden);
        p.attack = 50;
        p.health = 100;
        let enemies = [weak(&db), weak(&db)];
        let out = fight_with(
            &db,
            &[p],
            &enemies,
            [Counters::default(); 2],
            100,
            &mut Rng::new(9),
            &mut (),
        );
        let reward = db.triple_reward.unwrap();
        assert_eq!(
            out.sides[0].cards_gained().iter().filter(|&&c| c == reward).count(),
            count
        );
    };
    check(false, 1);
    check(true, 2);
}

#[test]
fn piper_raises_the_gem_attack_at_most_three_times_a_combat() {
    let db = db();
    let piper = id(&db, "Timewarped Piper");
    let check = |golden: bool, expected: i32| {
        let mut p = db.make(piper, golden);
        p.health = 1000;
        let enemy = killer(&db, 1, 10000);
        let out = fight_with(
            &db,
            &[p],
            &[enemy],
            [Counters::default(); 2],
            30,
            &mut Rng::new(11),
            &mut (),
        );
        assert_eq!(out.sides[0].gem_gain.0, expected);
    };
    check(false, 3);
    check(true, 6);
}

#[test]
fn recycler_avenge_raises_maximum_gold() {
    let db = db();
    let recycler = id(&db, "Timewarped Recycler");
    let check = |golden: bool, expected: u8| {
        let mut r = db.make(recycler, golden);
        r.health = 1000;
        let board = [weak(&db), weak(&db), weak(&db), weak(&db), r];
        let enemy = killer(&db, 50, 10000);
        let out = fight_with(
            &db,
            &board,
            &[enemy],
            [Counters::default(); 2],
            100,
            &mut Rng::new(13),
            &mut (),
        );
        assert_eq!(out.sides[0].max_gold_gain, expected);
        let mut g = game(14);
        put_token(&mut g, 0, "Timewarped Recycler", golden);
        g.apply_side_outcome(0, &out.sides[0], true);
        assert_eq!(g.players[0].extra_max_gold, expected);
    };
    check(false, 1);
    check(true, 2);
}

#[test]
fn guard_rally_gives_divine_shield_permanently() {
    let db = db();
    let guard = id(&db, "Timewarped Guard");
    let check = |golden: bool, count: u32| {
        let mut g = db.make(guard, golden);
        g.health = 100;
        let board = [g, weak(&db), weak(&db), weak(&db)];
        let enemy = killer(&db, 0, 1000);
        let out = fight_with(
            &db,
            &board,
            &[enemy],
            [Counters::default(); 2],
            1,
            &mut Rng::new(15),
            &mut (),
        );
        let shielded = out.sides[0].permanent_keywords[1..]
            .iter()
            .filter(|&&k| k & Keywords::DIVINE_SHIELD != 0)
            .count();
        assert_eq!(shielded, count as usize);
    };
    check(false, 1);
    check(true, 2);
}

#[test]
fn bonker_rally_plays_permanent_gems_on_the_others() {
    let db = db();
    let bonker = id(&db, "Timewarped Bonker");
    let check = |golden: bool, expected: (i32, i32)| {
        let mut b = db.make(bonker, golden);
        b.health = 100;
        let board = [b, weak(&db), weak(&db)];
        let enemy = killer(&db, 0, 1000);
        let out = fight_with(
            &db,
            &board,
            &[enemy],
            [Counters::default(); 2],
            1,
            &mut Rng::new(17),
            &mut (),
        );
        assert_eq!(out.sides[0].permanent_gems[1], expected);
        assert_eq!(out.sides[0].permanent_gems[2], expected);
        assert_eq!(out.sides[0].permanent_gems[0], (0, 0));
    };
    check(false, (4, 4));
    check(true, (8, 8));
}

#[test]
fn sellemental_gets_generated_copies_at_the_end_of_the_turn() {
    let check = |golden: bool, count: usize| {
        let mut g = game_with(
            19,
            &[Tribe::Beast, Tribe::Mech, Tribe::Murloc, Tribe::Demon, Tribe::Pirate],
        );
        let sellemental = id(&g.data.cards, "Sellemental");
        let slot = g.data.cards.get(sellemental).pool_slot as usize;
        let pool_before = g.pool[slot];
        assert_eq!(pool_before, 0, "no Elemental in this lobby");
        put_token(&mut g, 0, "Timewarped Sellemental", golden);
        wall_opponent(&mut g);
        end_all_turns(&mut g);
        assert_eq!(hand_of(&g, 0, "Sellemental"), count);
        for m in &g.players[0].hand {
            if name(&g, m) == "Sellemental" {
                assert_eq!(m.pool_copies, 0, "a generated copy");
            }
        }
        assert_eq!(g.pool[slot], pool_before, "the pool is untouched");
        g.check_invariants().unwrap();
    };
    check(false, 1);
    check(true, 2);
}

#[test]
fn skipper_gives_random_tier_one_minions_after_a_tier_two_sale() {
    let check = |golden: bool, count: usize| {
        let mut g = game(21);
        put_token(&mut g, 0, "Timewarped Skipper", golden);
        set_shop(&mut g, 0, &["Soul Rewinder"]);
        g.players[0].gold = 10;
        g.players[0].tier = 6;
        act(&mut g, 0, Action::Buy(0));
        act(&mut g, 0, Action::Play { hand: 0, pos: 1 });
        act(&mut g, 0, Action::Sell(1));
        let tier_one = g.players[0]
            .hand
            .iter()
            .filter(|m| {
                let c = g.data.cards.get(m.card);
                !c.is_spell() && c.tier == 1
            })
            .count();
        assert_eq!(tier_one, count);
        g.check_invariants().unwrap();
    };
    check(false, 1);
    check(true, 2);
}

#[test]
fn skipper_ignores_a_sale_that_is_not_tier_two() {
    let mut g = game(22);
    put_token(&mut g, 0, "Timewarped Skipper", false);
    set_shop(&mut g, 0, &["Southsea Busker"]);
    g.players[0].gold = 10;
    g.players[0].tier = 6;
    act(&mut g, 0, Action::Buy(0));
    act(&mut g, 0, Action::Play { hand: 0, pos: 1 });
    act(&mut g, 0, Action::Sell(1));
    let tier_one = g.players[0]
        .hand
        .iter()
        .filter(|m| {
            let c = g.data.cards.get(m.card);
            !c.is_spell() && c.tier == 1
        })
        .count();
    assert_eq!(tier_one, 0, "Tier 1 is not Tier 2");
    g.check_invariants().unwrap();
}

#[test]
fn sporebat_deathrattle_gives_random_spells_costing_two_or_more() {
    let db = db();
    let sporebat = id(&db, "Timewarped Sporebat");
    let check = |golden: bool, count: usize| {
        let mut s = db.make(sporebat, golden);
        s.health = 1;
        s.keywords.set(Keywords::TAUNT);
        let enemy = killer(&db, 50, 100);
        let mut counters = [Counters::default(); 2];
        counters[0].tier = 6;
        let out = fight_with(
            &db,
            &[s],
            &[enemy],
            counters,
            100,
            &mut Rng::new(23),
            &mut (),
        );
        let gained = out.sides[0].cards_gained();
        assert_eq!(gained.len(), count);
        for &c in gained {
            let card = db.get(c);
            assert!(card.is_spell());
            assert!(card.cost >= 2, "{} costs {}", card.name, card.cost);
        }
    };
    check(false, 1);
    check(true, 2);
}

#[test]
fn behemoth_gains_the_stats_of_a_bought_elemental() {
    let check = |golden: bool, expected: (i32, i32)| {
        let mut g = game(25);
        put_token(&mut g, 0, "Timewarped Behemoth", golden);
        set_shop(&mut g, 0, &["Sellemental"]);
        g.players[0].gold = 10;
        let before = stats(&g.players[0].board[0]);
        act(&mut g, 0, Action::Buy(0));
        assert_eq!(
            stats(&g.players[0].board[0]),
            (before.0 + expected.0, before.1 + expected.1)
        );
        g.check_invariants().unwrap();
    };
    check(false, (3, 3));
    check(true, (6, 6));

    // A non-Elemental does not fire it.
    let mut g = game(26);
    put_token(&mut g, 0, "Timewarped Behemoth", false);
    set_shop(&mut g, 0, &["Cord Puller"]);
    g.players[0].gold = 10;
    let before = stats(&g.players[0].board[0]);
    act(&mut g, 0, Action::Buy(0));
    assert_eq!(stats(&g.players[0].board[0]), before);
}

#[test]
fn caretaker_overflow_raises_the_undead_bonus() {
    let db = db();
    let caretaker = id(&db, "Timewarped Caretaker");
    let check = |golden: bool, expected: i32| {
        let mut c = db.make(caretaker, golden);
        c.health = 1;
        c.keywords.set(Keywords::TAUNT);
        let mut board = vec![c];
        for _ in 0..6 {
            let mut m = db.make(id(&db, "Skeleton"), false);
            m.attack = 0;
            m.health = 100;
            board.push(m);
        }
        let enemy = killer(&db, 1, 1000);
        let out = fight_with(
            &db,
            &board,
            &[enemy],
            [Counters::default(); 2],
            2,
            &mut Rng::new(27),
            &mut (),
        );
        assert_eq!(out.sides[0].undead_gain.0, expected);
    };
    check(false, 4);
    check(true, 9);
}
