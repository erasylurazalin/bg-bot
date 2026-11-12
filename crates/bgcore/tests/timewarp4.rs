//! The Phase 4 Timewarped minions, whose rules are Era's guesses in
//! docs/notes/UNSURE.md. One test per guessed rule.

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
struct Seen {
    attacks: Vec<(CardId, i32, i32)>,
    summons: Vec<CardId>,
}

impl Watch for Seen {
    fn attack(&mut self, side: usize, attacker: &Minion, _defender: &Minion) {
        if side == 0 {
            self.attacks.push((attacker.card, attacker.attack, attacker.health));
        }
    }
    fn summon(&mut self, side: usize, _pos: usize, m: &Minion) {
        if side == 0 {
            self.summons.push(m.card);
        }
    }
}

#[test]
fn the_phase_4_marker_keeps_each_card_out_of_the_normal_draws() {
    let db = db();
    let cards = [
        ("Timewarped Elise", Timewarp::Minor, 3),
        ("Timewarped Embalmer", Timewarp::Minor, 3),
        ("Timewarped Tender", Timewarp::Minor, 3),
        ("Timewarped Whelp Smuggler", Timewarp::Minor, 3),
        ("Timewarped Amalgam", Timewarp::Major, 5),
        ("Timewarped Chameleon", Timewarp::Major, 5),
        ("Timewarped Deios", Timewarp::Major, 5),
        ("Timewarped Expeditioner", Timewarp::Major, 5),
        ("Timewarped Ghoul-acabra", Timewarp::Major, 5),
        ("Timewarped Painter", Timewarp::Major, 5),
        ("Timewarped Promo-Drake", Timewarp::Major, 5),
        ("Timewarped Jungle King", Timewarp::Major, 5),
        ("Timewarped Kil'jaeden", Timewarp::Major, 5),
        ("Timewarped Magnanimoose", Timewarp::Major, 5),
        ("Timewarped Swirler", Timewarp::Major, 5),
        ("Timewarped Whirl-O-Tron", Timewarp::Major, 5),
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
fn elise_counts_five_refreshes_then_makes_the_top_tavern_minions_golden() {
    let check = |golden: bool, want: usize| {
        let mut g = game(31);
        put_token(&mut g, 0, "Timewarped Elise", golden);
        g.players[0].gold = 100;
        for _ in 0..5 {
            act(&mut g, 0, Action::Roll);
        }
        assert_eq!(g.players[0].board[0].improvements, 5, "the count resets");
        let minions: Vec<(usize, u8)> = g.players[0]
            .shop
            .iter()
            .enumerate()
            .filter(|(_, m)| !g.data.cards.get(m.card).is_spell())
            .map(|(i, m)| (i, g.data.cards.get(m.card).tier))
            .collect();
        let best = minions.iter().map(|&(_, t)| t).max().unwrap_or(0);
        let gold: Vec<usize> = g.players[0]
            .shop
            .iter()
            .enumerate()
            .filter(|(_, m)| m.golden)
            .map(|(i, _)| i)
            .collect();
        assert_eq!(gold.len(), want, "the top {} are golden", want);
        for i in &gold {
            assert_eq!(g.data.cards.get(g.players[0].shop[*i].card).tier, best);
        }
        g.check_invariants().unwrap();
    };
    check(false, 1);
    check(true, 2);
    // Four Refreshes leave one on the clock.
    let mut g = game(32);
    put_token(&mut g, 0, "Timewarped Elise", false);
    g.players[0].gold = 100;
    for _ in 0..4 {
        act(&mut g, 0, Action::Roll);
    }
    assert_eq!(g.players[0].board[0].improvements, 1);
    assert!(g.players[0].shop.iter().all(|m| !m.golden));
}

#[test]
fn embalmer_gives_a_summoned_minion_reborn() {
    let mut g = game(33);
    put_token(&mut g, 0, "Timewarped Embalmer", false);
    put_on_board(&mut g, 0, "Cord Puller", false);
    g.give(0, "Floating Candle").unwrap();
    let h = g.players[0].hand.len() as u8 - 1;
    play(&mut g, 0, h, 0);
    act(&mut g, 0, Action::Target(1));
    let microbot = id(&g.data.cards, "Microbot");
    let summoned = g.players[0]
        .board
        .iter()
        .find(|m| m.card == microbot)
        .expect("the Deathrattle summoned a Microbot");
    assert!(summoned.keywords.has(Keywords::REBORN));
    assert_eq!(g.players[0].board[0].uses_left, 0, "the one use is spent");
    g.check_invariants().unwrap();
}

#[test]
fn tender_gets_two_stat_spells_at_the_end_of_the_turn() {
    let check = |golden: bool, count: usize| {
        let mut g = game(34);
        put_token(&mut g, 0, "Timewarped Tender", golden);
        wall_opponent(&mut g);
        end_all_turns(&mut g);
        let spells = g.players[0]
            .hand
            .iter()
            .filter(|m| g.data.cards.get(m.card).is_spell())
            .collect::<Vec<_>>();
        assert_eq!(spells.len(), count);
        for m in spells {
            assert!(g.data.cards.get(m.card).gives_stats(), "{}", g.data.cards.get(m.card).name);
        }
        g.check_invariants().unwrap();
    };
    check(false, 2);
    check(true, 4);
}

#[test]
fn whelp_smuggler_adds_health_on_an_attack_gain() {
    let check = |golden: bool, health: i32| {
        let mut g = game(35);
        put_token(&mut g, 0, "Timewarped Whelp Smuggler", golden);
        put_on_board(&mut g, 0, "Wolf Pup", false);
        let before = stats(&g.players[0].board[1]);
        g.give(0, "Bananas").unwrap();
        let h = g.players[0].hand.len() as u8 - 1;
        play(&mut g, 0, h, 0);
        act(&mut g, 0, Action::Target(1));
        assert_eq!(stats(&g.players[0].board[1]), (before.0 + 1, before.1 + 1 + health));
        g.check_invariants().unwrap();
    };
    check(false, 3);
    check(true, 6);
}

#[test]
fn amalgam_buffs_the_tavern_by_the_played_minions_types() {
    let check = |golden: bool, step: i32| {
        let mut g = game(36);
        put_token(&mut g, 0, "Timewarped Amalgam", golden);
        set_shop(&mut g, 0, &["Wolf Pup", "Cord Puller"]);
        let before: Vec<(i32, i32)> = g.players[0].shop.iter().map(stats).collect();
        g.give(0, "Wolf Pup").unwrap();
        let h = g.players[0].hand.len() as u8 - 1;
        play(&mut g, 0, h, 1);
        assert_eq!(
            stats(&g.players[0].shop[0]),
            (before[0].0 + step, before[0].1 + step)
        );
        assert_eq!(stats(&g.players[0].shop[1]), before[1], "the Mech is not a Beast");
        g.check_invariants().unwrap();
    };
    check(false, 4);
    check(true, 8);
}

#[test]
fn chameleon_becomes_a_copy_of_its_left_minion() {
    let db = db();
    let chameleon = id(&db, "Timewarped Chameleon");
    let repair = id(&db, "Repair Job");
    let check = |golden: bool, rewards: usize| {
        let mut c = db.make(chameleon, golden);
        c.health = 1;
        let r = mk(&db, "Rescue Bot", false);
        let enemy = killer(&db, 50, 1000);
        let out = fight_with(
            &db,
            &[r, c],
            &[enemy],
            [Counters::default(); 2],
            20,
            &mut Rng::new(37),
            &mut (),
        );
        assert_eq!(
            out.sides[0].cards_gained().iter().filter(|&&x| x == repair).count(),
            rewards
        );
    };
    check(false, 2);
    check(true, 3);
}

#[test]
fn deios_makes_deathrattles_trigger_twice() {
    let db = db();
    let deios = id(&db, "Timewarped Deios");
    let repair = id(&db, "Repair Job");
    let check = |golden: bool, count: usize| {
        let mut d = db.make(deios, golden);
        d.health = 100;
        let mut r = mk(&db, "Rescue Bot", false);
        r.health = 1;
        r.keywords.set(Keywords::TAUNT);
        let enemy = killer(&db, 50, 1000);
        let out = fight_with(
            &db,
            &[r, d],
            &[enemy],
            [Counters::default(); 2],
            10,
            &mut Rng::new(38),
            &mut (),
        );
        assert_eq!(
            out.sides[0].cards_gained().iter().filter(|&&x| x == repair).count(),
            count
        );
    };
    check(false, 2);
    check(true, 3);
}

#[test]
fn expeditions_gained_stats_reach_the_two_leftmost_hand_minions() {
    let check = |golden: bool, step: i32| {
        let mut g = game(39);
        put_token(&mut g, 0, "Timewarped Expeditioner", golden);
        put_on_board(&mut g, 0, "Cord Puller", false);
        put_on_board(&mut g, 0, "Wolf Pup", false);
        g.give(0, "Waveling").unwrap();
        g.give(0, "Southsea Busker").unwrap();
        let before: Vec<(i32, i32)> = g.players[0].hand.iter().map(stats).collect();
        g.give(0, "Bananas").unwrap();
        let h = g.players[0].hand.len() as u8 - 1;
        play(&mut g, 0, h, 0);
        act(&mut g, 0, Action::Target(0));
        assert_eq!(
            stats(&g.players[0].hand[0]),
            (before[0].0 + step, before[0].1 + step)
        );
        assert_eq!(
            stats(&g.players[0].hand[1]),
            (before[1].0 + step, before[1].1 + step)
        );
        g.check_invariants().unwrap();
    };
    check(false, 1);
    check(true, 2);
}

#[test]
fn ghoul_acabra_buffs_after_a_deathrattle() {
    let check = |golden: bool, attack: i32, health: i32| {
        let mut g = game(40);
        put_token(&mut g, 0, "Timewarped Ghoul-acabra", golden);
        put_on_board(&mut g, 0, "Cord Puller", false);
        let before: Vec<(i32, i32)> = g.players[0].board.iter().map(stats).collect();
        g.give(0, "Floating Candle").unwrap();
        let h = g.players[0].hand.len() as u8 - 1;
        play(&mut g, 0, h, 0);
        act(&mut g, 0, Action::Target(1));
        for (i, b) in before.iter().enumerate() {
            assert_eq!(
                stats(&g.players[0].board[i]),
                (b.0 + attack, b.1 + health),
                "{}",
                g.data.cards.get(g.players[0].board[i].card).name
            );
        }
        g.check_invariants().unwrap();
    };
    check(false, 3, 2);
    check(true, 6, 4);
}

#[test]
fn painter_improves_after_a_low_tier_card_and_buffs_its_neighbours() {
    let check = |golden: bool, first: i32, second: i32| {
        let mut g = game(41);
        put_on_board(&mut g, 0, "Cord Puller", false);
        put_token(&mut g, 0, "Timewarped Painter", golden);
        put_on_board(&mut g, 0, "Wolf Pup", false);
        wall_opponent(&mut g);
        end_all_turns(&mut g);
        let middle: Vec<(i32, i32)> = [0, 2]
            .iter()
            .map(|&i| stats(&g.players[0].board[i]))
            .collect();
        g.give(0, "Waveling").unwrap();
        let h = g.players[0].hand.len() as u8 - 1;
        play(&mut g, 0, h, 3);
        wall_opponent(&mut g);
        end_all_turns(&mut g);
        assert_eq!(stats(&g.players[0].board[0]), (middle[0].0 + first, middle[0].1 + second));
        assert_eq!(stats(&g.players[0].board[2]), (middle[1].0 + first, middle[1].1 + second));
        g.check_invariants().unwrap();
    };
    check(false, 6, 4);
    check(true, 12, 8);
}

#[test]
fn promo_drake_buffs_at_start_and_improves_at_end_of_turn() {
    let db = db();
    let promo = id(&db, "Timewarped Promo-Drake");
    let check = |golden: bool, amount: i32| {
        let mut p = db.make(promo, golden);
        p.improvements = 1;
        p.health = 100;
        let mut target = mk(&db, "Cord Puller", false);
        target.attack = 1;
        target.health = 100;
        let enemy = killer(&db, 0, 1000);
        let mut seen = Seen::default();
        fight_with(
            &db,
            &[p, target],
            &[enemy],
            [Counters::default(); 2],
            4,
            &mut Rng::new(43),
            &mut seen,
        );
        assert!(seen.attacks.iter().any(|&(c, a, _)| c == target.card && a == 1 + amount));
    };
    check(false, 10);
    check(true, 20);
}

#[test]
fn jungle_king_improves_after_a_spell() {
    let check = |golden: bool, first: (i32, i32), second: (i32, i32)| {
        let mut g = game(44);
        put_token(&mut g, 0, "Timewarped Jungle King", golden);
        g.give(0, "Wolf Pup").unwrap();
        let h = g.players[0].hand.len() as u8 - 1;
        let base = stats(&g.players[0].hand[0]);
        play(&mut g, 0, h, 1);
        assert_eq!(
            stats(&g.players[0].board[1]),
            (base.0 + first.0, base.1 + first.1)
        );
        g.give(0, "Tavern Coin").unwrap();
        let h = g.players[0].hand.len() as u8 - 1;
        play(&mut g, 0, h, 0);
        g.give(0, "Wolf Pup").unwrap();
        let h = g.players[0].hand.len() as u8 - 1;
        let base2 = stats(&g.players[0].hand[0]);
        play(&mut g, 0, h, 2);
        assert_eq!(
            stats(&g.players[0].board[2]),
            (base2.0 + second.0, base2.1 + second.1)
        );
        g.check_invariants().unwrap();
    };
    check(false, (4, 3), (8, 6));
    check(true, (8, 6), (16, 12));
}

#[test]
fn kiljaeden_adds_two_tavern_demons_that_grow() {
    let mut g = game(45);
    put_token(&mut g, 0, "Timewarped Kil'jaeden", false);
    g.players[0].tier = 6;
    g.players[0].gold = 100;
    act(&mut g, 0, Action::Roll);
    let demon = |m: &Minion| g.data.cards.tribes_of(m).has(Tribe::Demon);
    let buffed = g.players[0]
        .shop
        .iter()
        .filter(|m| demon(m))
        .filter(|m| m.attack >= g.data.cards.get(m.card).attack + 7)
        .count();
    assert!(buffed >= 2, "two extra Demons had +7/+7");
    wall_opponent(&mut g);
    end_all_turns(&mut g);
    assert_eq!(g.players[0].board[0].improvements, 1, "it grew a turn");
}

#[test]
fn magnanimoose_summons_and_gets_an_opponent_minion() {
    let db = db();
    let moose = id(&db, "Timewarped Magnanimoose");
    let check = |golden: bool, count: usize| {
        let mut m = db.make(moose, golden);
        m.health = 1;
        m.keywords.set(Keywords::TAUNT);
        let mut enemy = mk(&db, "Bile Spitter", false);
        enemy.health = 100;
        let mut seen = Seen::default();
        let out = fight_with(
            &db,
            &[m],
            &[enemy],
            [Counters::default(); 2],
            2,
            &mut Rng::new(46),
            &mut seen,
        );
        assert_eq!(
            out.sides[0].cards_gained().iter().filter(|&&c| c == enemy.card).count(),
            count
        );
        assert_eq!(seen.summons.iter().filter(|&&c| c == enemy.card).count(), count);
    };
    check(false, 1);
    check(true, 2);
}

#[test]
fn swirler_adds_stats_to_an_elementals_gift() {
    let check = |golden: bool, step: i32| {
        let mut g = game(47);
        let swirler = id(&g.data.cards, "Timewarped Swirler");
        let m = g.players[0].make_card(&g.data.cards, swirler, golden, 0);
        g.players[0].hand.push(m);
        play(&mut g, 0, 0, 0);
        set_shop(&mut g, 0, &["Waveling"]);
        let before = stats(&g.players[0].shop[0]);
        g.give(0, "Dune Dweller").unwrap();
        let h = g.players[0].hand.len() as u8 - 1;
        play(&mut g, 0, h, 1);
        assert_eq!(
            stats(&g.players[0].shop[0]),
            (before.0 + 1 + step, before.1 + 1 + step)
        );
        g.check_invariants().unwrap();
    };
    check(false, 3);
    check(true, 6);
}

#[test]
fn whirl_o_tron_copies_the_two_left_deathrattles() {
    let db = db();
    let whirl = id(&db, "Timewarped Whirl-O-Tron");
    let repair = id(&db, "Repair Job");
    let check = |golden: bool, count: usize| {
        let mut w = db.make(whirl, golden);
        w.attack = 0;
        w.health = 1;
        w.keywords.set(Keywords::TAUNT);
        let mut r = mk(&db, "Rescue Bot", false);
        r.attack = 0;
        r.health = 100;
        let enemy = killer(&db, 50, 1000);
        let out = fight_with(
            &db,
            &[r, w],
            &[enemy],
            [Counters::default(); 2],
            2,
            &mut Rng::new(48),
            &mut (),
        );
        assert_eq!(
            out.sides[0].cards_gained().iter().filter(|&&x| x == repair).count(),
            count
        );
    };
    check(false, 1);
    check(true, 2);
}
