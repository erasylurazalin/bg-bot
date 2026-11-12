//! The 15 Phase 3 Timewarped minions, the rest of the export's new-code
//! cards.

use std::sync::Arc;

use bgcore::fight::{Watch, fight_with, fight_with_hands};
use bgcore::{
    Action, CardDb, CardId, Counters, Data, Game, Minion, Rng, Timewarp, Tribes,
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

fn play(g: &mut Game, p: usize, hand: u8, pos: u8) {
    act(g, p, Action::Play { hand, pos });
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

fn killer(db: &CardDb, attack: i32, health: i32) -> Minion {
    let mut m = mk(db, "Microbot", false);
    m.attack = attack;
    m.health = health;
    m
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
fn the_phase_3_marker_keeps_each_card_out_of_the_normal_draws() {
    let db = db();
    let cards = [
        ("Timewarped Centurion", 5),
        ("Timewarped Electron", 5),
        ("Timewarped Grease Bot", 5),
        ("Timewarped Hawkstrider", 5),
        ("Timewarped Interpreter", 5),
        ("Timewarped Lil' Quilboar", 5),
        ("Timewarped Mrrrglr", 5),
        ("Timewarped Nest Swarmer", 5),
        ("Timewarped Peggy", 5),
        ("Timewarped Raider", 5),
        ("Timewarped Squallfin", 5),
        ("Timewarped Sylvar", 5),
        ("Timewarped Theotar", 5),
        ("Timewarped Twirler", 5),
        ("Timewarped Shivarra", 5),
    ];
    for (n, tier) in cards {
        let c = id(&db, n);
        let card = db.get(c);
        assert!(card.token, "{n} is a token");
        assert_eq!(card.timewarp, Some(Timewarp::Major), "{n} side");
        assert_eq!(card.tier, tier, "{n} tier");
        assert!(!card.lobby_ok(Tribes::ALL), "{n} is never a normal draw");
        assert!(c as usize >= db.pool_cards, "{n} takes no pool slot");
    }
}

#[test]
fn centurion_copies_a_tavern_spell_three_times_a_turn() {
    let check = |golden: bool, copies: usize| {
        let mut g = game(11);
        put_token(&mut g, 0, "Timewarped Centurion", golden);
        for _ in 0..4 {
            g.give(0, "Tavern Coin").unwrap();
            let h = g.players[0].hand.len() as u8 - 1;
            play(&mut g, 0, h, 0);
        }
        assert_eq!(hand_of(&g, 0, "Tavern Coin"), copies);
        g.check_invariants().unwrap();
    };
    check(false, 3);
    check(true, 6);
}

#[test]
fn electron_magnetizes_satellites_to_all_mechs_twice_a_turn() {
    let check = |golden: bool, gain: i32| {
        let mut g = game(12);
        put_token(&mut g, 0, "Timewarped Electron", golden);
        put_on_board(&mut g, 0, "Cord Puller", false);
        put_on_board(&mut g, 0, "Wolf Pup", false);
        let before: Vec<(i32, i32)> = g.players[0].board.iter().map(stats).collect();
        for _ in 0..4 {
            g.give(0, "Tavern Coin").unwrap();
            let h = g.players[0].hand.len() as u8 - 1;
            play(&mut g, 0, h, 0);
        }
        assert_eq!(stats(&g.players[0].board[0]), (before[0].0 + gain, before[0].1 + gain));
        assert_eq!(stats(&g.players[0].board[1]), (before[1].0 + gain, before[1].1 + gain));
        assert_eq!(stats(&g.players[0].board[2]), before[2], "a non-Mech is untouched");
        g.check_invariants().unwrap();
    };
    check(false, 8);
    check(true, 16);
}

#[test]
fn grease_bot_buffs_everyone_after_a_shield_is_lost() {
    let db = db();
    let grease = id(&db, "Timewarped Grease Bot");
    let check = |golden: bool, expected: i32| {
        let mut g = db.make(grease, golden);
        g.health = 100;
        let mut shield = mk(&db, "Cord Puller", false);
        shield.health = 100;
        let enemy = killer(&db, 1, 1000);
        let out = fight_with(
            &db,
            &[g, shield],
            &[enemy],
            [Counters::default(); 2],
            4,
            &mut Rng::new(3),
            &mut (),
        );
        assert!(out.sides[0].permanent_stats[0].0 >= expected);
    };
    check(false, 3);
    check(true, 6);
}

#[test]
fn hawkstrider_triggers_every_friendly_deathrattle() {
    let db = db();
    let hawk = id(&db, "Timewarped Hawkstrider");
    let check = |golden: bool, count: usize| {
        let mut h = db.make(hawk, golden);
        h.health = 100;
        let rescue = mk(&db, "Rescue Bot", false);
        let enemy = killer(&db, 0, 1000);
        let out = fight_with(
            &db,
            &[h, rescue],
            &[enemy],
            [Counters::default(); 2],
            1,
            &mut Rng::new(5),
            &mut (),
        );
        let repair = id(&db, "Repair Job");
        assert_eq!(
            out.sides[0].cards_gained().iter().filter(|&&c| c == repair).count(),
            count
        );
    };
    check(false, 1);
    check(true, 2);
}

#[test]
fn interpreter_buffs_all_mechs_when_one_is_played() {
    let check = |golden: bool, step: i32| {
        let mut g = game(14);
        put_token(&mut g, 0, "Timewarped Interpreter", golden);
        put_on_board(&mut g, 0, "Cord Puller", false);
        put_on_board(&mut g, 0, "Wolf Pup", false);
        let before: Vec<(i32, i32)> = g.players[0].board.iter().map(stats).collect();
        g.give(0, "Cord Puller").unwrap();
        let h = g.players[0].hand.len() as u8 - 1;
        play(&mut g, 0, h, 3);
        for (i, b) in before.iter().enumerate().take(2) {
            assert_eq!(stats(&g.players[0].board[i]), (b.0 + step, b.1 + step));
        }
        assert_eq!(stats(&g.players[0].board[3]), (1 + step, 1 + step), "the played Mech");
        assert_eq!(stats(&g.players[0].board[2]), before[2], "the Wolf Pup is not a Mech");
        g.check_invariants().unwrap();
    };
    check(false, 3);
    check(true, 6);
}

#[test]
fn lil_quilboar_plays_gems_on_all_your_quilboar() {
    let check = |golden: bool, step: i32| {
        let mut g = game(15);
        put_token(&mut g, 0, "Timewarped Lil' Quilboar", golden);
        put_on_board(&mut g, 0, "Gem Rat", false);
        put_on_board(&mut g, 0, "Roadboar", false);
        let before: Vec<(i32, i32)> = g.players[0].board.iter().map(stats).collect();
        g.give(0, "Floating Candle").unwrap();
        let h = g.players[0].hand.len() as u8 - 1;
        play(&mut g, 0, h, 0);
        act(&mut g, 0, Action::Target(0));
        for (i, b) in before.iter().enumerate() {
            assert_eq!(stats(&g.players[0].board[i]), (b.0 + step, b.1 + step));
        }
        g.check_invariants().unwrap();
    };
    check(false, 3);
    check(true, 6);
}

#[test]
fn mrrrglr_gives_adjacent_murlocs_the_stats_in_hand() {
    let db = db();
    let mrrrglr = id(&db, "Timewarped Mrrrglr");
    let gearfin = id(&db, "Gearfin");
    let spitter = id(&db, "Bile Spitter");
    let check = |golden: bool, times: i32| {
        let mut host = db.make(mrrrglr, golden);
        host.health = 100;
        let mut left = db.make(gearfin, false);
        left.health = 100;
        let mut right = db.make(spitter, false);
        right.health = 100;
        let mut h1 = mk(&db, "Wolf Pup", false);
        h1.attack = 3;
        h1.health = 4;
        let mut h2 = mk(&db, "Cord Puller", false);
        h2.attack = 2;
        h2.health = 5;
        let enemy = killer(&db, 0, 1000);
        let mut seen = Attacks::default();
        fight_with_hands(
            &db,
            &[left, host, right],
            &[enemy],
            [&[h1, h2], &[]],
            [Counters::default(); 2],
            4,
            &mut Rng::new(7),
            &mut seen,
        );
        let expected = (left_attack(&db, gearfin) + 5 * times, 0);
        assert!(
            seen.0.iter().any(|&(c, a, _)| c == gearfin && a == expected.0),
            "Gearfin got the hand stats"
        );
        assert!(
            seen.0.iter().any(|&(c, a, _)| c == spitter && a == left_attack(&db, spitter) + 5 * times),
            "Bile Spitter got the hand stats"
        );
    };
    fn left_attack(db: &CardDb, c: CardId) -> i32 {
        db.get(c).attack
    }
    check(false, 1);
    check(true, 2);
}

#[test]
fn nest_swarmer_buffs_and_summons_a_beetle() {
    let db = db();
    let check = |golden: bool, buff: (i32, i32), beetles: usize| {
        let mut g = game(17);
        let nest = db.by_name("Timewarped Nest Swarmer").unwrap();
        let m = g.players[0].make_card(&g.data.cards, nest, golden, 0);
        g.players[0].hand.push(m);
        let h = g.players[0].hand.len() as u8 - 1;
        play(&mut g, 0, h, 0);
        assert_eq!(g.players[0].beetle_bonus, buff);
        let beetle = db.beetle_token.unwrap();
        assert_eq!(
            g.players[0].board.iter().filter(|m| m.card == beetle).count(),
            beetles
        );
        g.check_invariants().unwrap();
    };
    check(false, (2, 2), 1);
    check(true, (4, 4), 2);

    // The Deathrattle uses the same effect: Floating Candle triggers it.
    let mut g = game(18);
    put_token(&mut g, 0, "Timewarped Nest Swarmer", false);
    g.give(0, "Floating Candle").unwrap();
    let h = g.players[0].hand.len() as u8 - 1;
    play(&mut g, 0, h, 0);
    act(&mut g, 0, Action::Target(0));
    assert_eq!(g.players[0].beetle_bonus, (2, 2));
    let beetle = db.beetle_token.unwrap();
    assert_eq!(g.players[0].board.iter().filter(|m| m.card == beetle).count(), 1);
}

#[test]
fn peggy_buffs_your_pirates_when_a_card_enters_hand() {
    let check = |golden: bool, step: i32| {
        let mut g = game(19);
        put_token(&mut g, 0, "Timewarped Peggy", golden);
        put_on_board(&mut g, 0, "Southsea Busker", false);
        put_on_board(&mut g, 0, "Wolf Pup", false);
        let before: Vec<(i32, i32)> = g.players[0].board.iter().map(stats).collect();
        g.give(0, "Tavern Coin").unwrap();
        assert_eq!(stats(&g.players[0].board[0]), (before[0].0 + step, before[0].1 + step));
        assert_eq!(stats(&g.players[0].board[1]), (before[1].0 + step, before[1].1 + step));
        assert_eq!(stats(&g.players[0].board[2]), before[2], "the Wolf Pup is not a Pirate");
        g.check_invariants().unwrap();
    };
    check(false, 1);
    check(true, 2);
}

#[test]
fn raider_buffs_pirates_after_a_tier_four_card() {
    let check = |golden: bool, step: (i32, i32)| {
        let mut g = game(20);
        put_token(&mut g, 0, "Timewarped Raider", golden);
        put_on_board(&mut g, 0, "Southsea Busker", false);
        put_on_board(&mut g, 0, "Wolf Pup", false);
        let before: Vec<(i32, i32)> = g.players[0].board.iter().map(stats).collect();
        g.give(0, "Ghastcoiler").unwrap();
        let h = g.players[0].hand.len() as u8 - 1;
        play(&mut g, 0, h, 3);
        assert_eq!(stats(&g.players[0].board[0]), (before[0].0 + step.0, before[0].1 + step.1));
        assert_eq!(stats(&g.players[0].board[1]), (before[1].0 + step.0, before[1].1 + step.1));
        assert_eq!(stats(&g.players[0].board[2]), before[2], "the Wolf Pup is not a Pirate");
        g.check_invariants().unwrap();
    };
    check(false, (3, 2));
    check(true, (6, 4));

    // Tier 3 does not fire it.
    let mut g = game(21);
    put_token(&mut g, 0, "Timewarped Raider", false);
    let before = stats(&g.players[0].board[0]);
    g.give(0, "Waveling").unwrap();
    let h = g.players[0].hand.len() as u8 - 1;
    play(&mut g, 0, h, 1);
    assert_eq!(stats(&g.players[0].board[0]), before);
}

#[test]
fn squallfin_buffs_board_and_hand_after_a_murloc() {
    let check = |golden: bool, step: (i32, i32)| {
        let mut g = game(22);
        put_token(&mut g, 0, "Timewarped Squallfin", golden);
        put_on_board(&mut g, 0, "Gearfin", false);
        g.give(0, "Wolf Pup").unwrap();
        let hand_before = stats(&g.players[0].hand[0]);
        let board_before: Vec<(i32, i32)> = g.players[0].board.iter().map(stats).collect();
        g.give(0, "Bile Spitter").unwrap();
        let h = g.players[0].hand.len() as u8 - 1;
        play(&mut g, 0, h, 2);
        for (b, m) in board_before.iter().zip(&g.players[0].board) {
            assert_eq!(stats(m), (b.0 + step.0, b.1 + step.1));
        }
        let played = &g.players[0].board[2];
        assert_eq!(stats(played), (1 + step.0, 10 + step.1), "the played Murloc");
        let hand = &g.players[0].hand[0];
        assert_eq!(stats(hand), (hand_before.0 + step.0, hand_before.1 + step.1));
        g.check_invariants().unwrap();
    };
    check(false, (2, 2));
    check(true, (4, 4));
}

#[test]
fn sylvar_buffs_adjacent_minions_per_golden() {
    let check = |golden: bool, step: i32| {
        let mut g = game(23);
        put_on_board(&mut g, 0, "Cord Puller", false);
        put_token(&mut g, 0, "Timewarped Sylvar", golden);
        put_on_board(&mut g, 0, "Wolf Pup", false);
        put_token(&mut g, 0, "Microbot", false);
        let before: Vec<(i32, i32)> = g.players[0].board.iter().map(stats).collect();
        wall_opponent(&mut g);
        end_all_turns(&mut g);
        assert_eq!(stats(&g.players[0].board[0]), (before[0].0 + step, before[0].1 + step));
        assert_eq!(stats(&g.players[0].board[2]), (before[2].0 + step, before[2].1 + step));
        assert_eq!(stats(&g.players[0].board[3]), before[3], "not adjacent");
        g.check_invariants().unwrap();
    };
    check(false, 8);
    check(true, 32);
}

#[test]
fn theotar_buffs_a_minion_of_each_type_after_a_neutral_play() {
    let check = |golden: bool, step: i32| {
        let mut g = game(24);
        put_token(&mut g, 0, "Timewarped Theotar", golden);
        g.give(0, "Timewarped Arm").unwrap();
        let h = g.players[0].hand.len() as u8 - 1;
        let before = stats(&g.players[0].board[0]);
        play(&mut g, 0, h, 1);
        assert_eq!(
            stats(&g.players[0].board[0]),
            (before.0 + step, before.1 + step),
            "the all-type Theotar is one of the types"
        );
        g.check_invariants().unwrap();
    };
    check(false, 6);
    check(true, 12);
}

#[test]
fn twirler_casts_blood_gem_barrage_after_a_gem() {
    let check = |golden: bool, barrage: (i32, i32)| {
        let mut g = game(25);
        put_token(&mut g, 0, "Timewarped Twirler", golden);
        let before = stats(&g.players[0].board[0]);
        g.give(0, "Blood Gem").unwrap();
        let h = g.players[0].hand.len() as u8 - 1;
        play(&mut g, 0, h, 0);
        act(&mut g, 0, Action::Target(0));
        assert_eq!(g.players[0].barrage, barrage);
        assert_eq!(stats(&g.players[0].board[0]), (before.0 + 1, before.1 + 1));
        g.check_invariants().unwrap();
    };
    check(false, (2, 2));
    check(true, (4, 4));
}

#[test]
fn shivarra_gains_the_stats_of_a_consumed_minion() {
    let check = |golden: bool, step: i32| {
        let mut g = game(26);
        put_on_board(&mut g, 0, "Wrath Weaver", false);
        put_token(&mut g, 0, "Timewarped Shivarra", golden);
        put_on_board(&mut g, 0, "Soulkeeping Jailer", false);
        set_shop(&mut g, 0, &["Cord Puller"]);
        g.players[0].gold = 10;
        let before = stats(&g.players[0].board[1]);
        act(&mut g, 0, Action::Activate(2));
        assert_eq!(stats(&g.players[0].board[1]), (before.0 + step, before.1 + step));
        g.check_invariants().unwrap();
    };
    check(false, 1);
    check(true, 2);
}
