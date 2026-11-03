//! The Beast and Mech minions of the 5-tribe lobby, with the real cards from
//! cards.toml: Rally and attack triggers, the Lobster build, Avenge,
//! Fishbait, Treasure Parrot, Drone Duplicator and the rest.

use std::sync::Arc;

use bgcore::fight::{Watch, fight_watched, fight_with};
use bgcore::{Tribe, Action, CardDb, Counters, Data, Game, Keywords, Minion, Rng};

fn db() -> CardDb {
    Data::bundled().cards
}

fn mk(db: &CardDb, name: &str, golden: bool) -> Minion {
    db.make(db.by_name(name).expect(name), golden)
}

/// A minion that never attacks and takes a long time to kill.
fn wall(db: &CardDb) -> Minion {
    let mut w = mk(db, "Microbot", false);
    w.attack = 0;
    w.health = 100;
    w
}

#[derive(Default)]
struct Log {
    /// (side, attacker name, attack, health, defender name) at the moment of each attack
    attacks: Vec<(usize, String, i32, i32, String)>,
    summons: Vec<(usize, usize, String, i32, i32)>,
    db: Option<CardDb>,
}

impl Watch for Log {
    fn attack(&mut self, side: usize, a: &Minion, d: &Minion) {
        let db = self.db.as_ref().unwrap();
        self.attacks.push((
            side,
            db.get(a.card).name.clone(),
            a.attack,
            a.health,
            db.get(d.card).name.clone(),
        ));
    }
    fn summon(&mut self, side: usize, pos: usize, m: &Minion) {
        let db = self.db.as_ref().unwrap();
        self.summons.push((side, pos, db.get(m.card).name.clone(), m.attack, m.health));
    }
}

fn run(
    db: &CardDb,
    a: &[Minion],
    b: &[Minion],
    counters: [Counters; 2],
    max_attacks: u32,
) -> (bgcore::Outcome, Log) {
    let mut log = Log { db: Some(db.clone()), ..Default::default() };
    let out = fight_with(db, a, b, counters, max_attacks, &mut Rng::new(1), &mut log);
    (out, log)
}

// ---- Rally and attack triggers in fights ----

#[test]
fn wolf_pup_rally_buffs_the_other_minions() {
    let db = db();
    let a = [mk(&db, "Wolf Pup", false), mk(&db, "Cord Puller", false)];
    let (_, log) = run(&db, &a, &[wall(&db)], [Counters::default(); 2], 2);
    // Pup attacks first, with its own stats; the Cord Puller gets +4/+1 before it attacks
    assert_eq!((log.attacks[0].1.as_str(), log.attacks[0].2, log.attacks[0].3), ("Wolf Pup", 3, 6));
    assert_eq!((log.attacks[1].1.as_str(), log.attacks[1].2, log.attacks[1].3), ("Cord Puller", 5, 2));

    // golden: +8/+2
    let a = [mk(&db, "Wolf Pup", true), mk(&db, "Cord Puller", false)];
    let (_, log) = run(&db, &a, &[wall(&db)], [Counters::default(); 2], 2);
    assert_eq!((log.attacks[0].2, log.attacks[0].3), (6, 12));
    assert_eq!((log.attacks[1].2, log.attacks[1].3), (9, 3));
}

#[test]
fn cage_gnawer_buffs_beasts_when_a_friendly_beast_attacks() {
    let db = db();
    let a = [
        mk(&db, "Buzzing Vermin", false),
        mk(&db, "Cage Gnawer", false),
        mk(&db, "Cord Puller", false),
    ];
    let (_, log) = run(&db, &a, &[wall(&db)], [Counters::default(); 2], 3);
    // the Vermin attacks: Beasts get +2/+1 first
    assert_eq!((log.attacks[0].1.as_str(), log.attacks[0].2, log.attacks[0].3), ("Buzzing Vermin", 3, 2));
    // the Gnawer's own attack counts too: 2/7, +2/+1 from the Vermin, +2/+1 from itself
    assert_eq!((log.attacks[1].1.as_str(), log.attacks[1].2, log.attacks[1].3), ("Cage Gnawer", 6, 9));
    // a Mech attacking triggers nothing
    assert_eq!((log.attacks[2].1.as_str(), log.attacks[2].2, log.attacks[2].3), ("Cord Puller", 1, 1));

    // golden: +4/+2
    let a = [mk(&db, "Buzzing Vermin", false), mk(&db, "Cage Gnawer", true)];
    let (_, log) = run(&db, &a, &[wall(&db)], [Counters::default(); 2], 1);
    assert_eq!((log.attacks[0].2, log.attacks[0].3), (1 + 4, 1 + 2));
}

#[test]
fn rally_summons_a_pool_minion_and_it_waits_its_turn() {
    let db = db();
    let (_, log) = run(&db, &[mk(&db, "Hoarding Hyena", false)], &[wall(&db)], [Counters::default(); 2], 1);
    assert_eq!((log.summons[0].1, log.summons[0].2.as_str(), log.summons[0].3, log.summons[0].4), (1, "Tasty Lobster", 2, 1));
    // golden: a Golden Tasty Lobster
    let (_, log) = run(&db, &[mk(&db, "Hoarding Hyena", true)], &[wall(&db)], [Counters::default(); 2], 1);
    assert_eq!((log.summons[0].2.as_str(), log.summons[0].3, log.summons[0].4), ("Tasty Lobster", 4, 2));
}

#[test]
fn a_lobster_the_hyena_summons_is_plain_and_a_dying_lobster_raises_the_count() {
    let db = db();
    // The improvement is to the Deathrattle, not to Lobster stats: even with a count of 3
    // the Lobster a Hyena summons is a plain 2/1.
    let counters = [Counters { improve: [3, 0], ..Default::default() }, Counters::default()];
    let (_, log) = run(&db, &[mk(&db, "Hoarding Hyena", false)], &[wall(&db)], counters, 1);
    assert_eq!((log.summons[0].2.as_str(), log.summons[0].3, log.summons[0].4), ("Tasty Lobster", 2, 1));
    // a Lobster that dies raises the count: here one killed by a strong enemy
    let mut killer = mk(&db, "Microbot", false);
    killer.attack = 50;
    killer.health = 50;
    let out = bgcore::fight(&db, &[mk(&db, "Tasty Lobster", false)], &[killer], 500, &mut Rng::new(1));
    assert_eq!(out.sides[0].improve_gain, [1, 0]);
    assert_eq!(out.sides[0].deathrattles_triggered, 1);
}

#[test]
fn the_lobster_count_is_live_in_a_fight_and_carries_into_the_next_one() {
    let db = db();
    // Two Lobsters die to the same strong enemy; the second one's Deathrattle is
    // stronger, so the lone Gnawer that is left grows by 2/1 then 4/2 (and the second
    // fight starts from the count the first one left).
    let mut killer = mk(&db, "Microbot", false);
    killer.attack = 50;
    killer.health = 50;
    let a = [mk(&db, "Tasty Lobster", false), mk(&db, "Tasty Lobster", false)];
    let out = bgcore::fight(&db, &a, &[killer], 500, &mut Rng::new(1));
    assert_eq!(out.sides[0].improve_gain, [2, 0]);
    assert_eq!(out.sides[0].deathrattles_triggered, 2);
}

#[test]
fn deathstrider_triggers_the_leftmost_deathrattle_after_a_rally_minion_attacks() {
    let db = db();
    let a = [
        mk(&db, "Tasty Lobster", false),
        mk(&db, "Hoarding Hyena", false),
        mk(&db, "Deathstrider", false),
    ];
    // Lobster attacks (no Rally), then the Hyena attacks (Rally): Deathstrider triggers the
    // Lobster's Deathrattle, which raises the Lobster count and buffs a random Beast.
    let (out, log) = run(&db, &a, &[wall(&db)], [Counters::default(); 2], 2);
    assert_eq!(out.sides[0].improve_gain, [1, 0]);
    assert_eq!(out.sides[0].deathrattles_triggered, 1);
    assert_eq!(log.attacks.len(), 2);

    // after the Lobster's own attack nothing triggers
    let (out, _) = run(&db, &a, &[wall(&db)], [Counters::default(); 2], 1);
    assert_eq!(out.sides[0].deathrattles_triggered, 0);

    // golden Deathstrider triggers it twice
    let a = [
        mk(&db, "Tasty Lobster", false),
        mk(&db, "Hoarding Hyena", false),
        mk(&db, "Deathstrider", true),
    ];
    let (out, _) = run(&db, &a, &[wall(&db)], [Counters::default(); 2], 2);
    assert_eq!(out.sides[0].improve_gain, [2, 0]);
    assert_eq!(out.sides[0].deathrattles_triggered, 2);

    // it is the left-most Deathrattle: here the Vermin's, which summons a Beetle
    let a = [
        mk(&db, "Buzzing Vermin", false),
        mk(&db, "Tasty Lobster", false),
        mk(&db, "Hoarding Hyena", false),
        mk(&db, "Deathstrider", false),
    ];
    let (out, log) = run(&db, &a, &[wall(&db)], [Counters::default(); 2], 3);
    let names: Vec<&str> = log.summons.iter().map(|s| s.2.as_str()).collect();
    assert_eq!(names, ["Tasty Lobster", "Beetle"]);
    assert_eq!(out.sides[0].improve_gain, [0, 0]);
    // the Beetle stands right of the Vermin
    assert_eq!(log.summons[1].1, 1);

    // a non-Rally minion attacking does not trigger it
    let a = [mk(&db, "Buzzing Vermin", false), mk(&db, "Deathstrider", false)];
    let (_, log) = run(&db, &a, &[wall(&db)], [Counters::default(); 2], 1);
    assert!(log.summons.is_empty());
}

#[test]
fn treasure_parrot_gets_a_golden_touch_once_after_35_damage() {
    let db = db();
    let touch = db.by_name("Golden Touch").unwrap();
    let out = bgcore::fight(&db, &[mk(&db, "Treasure Parrot", false)], &[wall(&db)], 500, &mut Rng::new(1));
    // 5 damage a swing, 100 Health: 20 swings, 100 damage in all, one Golden Touch
    assert_eq!(out.winner, Some(0));
    assert_eq!(out.sides[0].cards_gained(), &[touch]);
    assert_eq!(out.sides[0].permanent_improvements[0], 100);

    // the damage dealt so far is kept: at 30 it fires on the next swing, at 35 it is already used
    let mut parrot = mk(&db, "Treasure Parrot", false);
    parrot.improvements = 30;
    let out = bgcore::fight(&db, &[parrot], &[wall(&db)], 1, &mut Rng::new(1));
    assert_eq!(out.sides[0].cards_gained(), &[touch]);
    parrot.improvements = 35;
    let out = bgcore::fight(&db, &[parrot], &[wall(&db)], 3, &mut Rng::new(1));
    assert!(out.sides[0].cards_gained().is_empty());
    parrot.improvements = 29;
    let out = bgcore::fight(&db, &[parrot], &[wall(&db)], 1, &mut Rng::new(1));
    assert!(out.sides[0].cards_gained().is_empty());

    // golden: two Golden Touches
    let out = bgcore::fight(&db, &[mk(&db, "Treasure Parrot", true)], &[wall(&db)], 500, &mut Rng::new(1));
    assert_eq!(out.sides[0].cards_gained(), &[touch, touch]);

    // a Divine Shield takes the hit and the damage doesn't count
    let mut shielded = wall(&db);
    shielded.keywords.set(Keywords::DIVINE_SHIELD);
    shielded.health = 5;
    let mut parrot = mk(&db, "Treasure Parrot", false);
    parrot.improvements = 30;
    let out = bgcore::fight(&db, &[parrot], &[shielded], 500, &mut Rng::new(1));
    // first swing pops the shield (0 damage), the second deals 5 and reaches 35
    assert_eq!(out.sides[0].cards_gained(), &[touch]);
}

#[test]
fn relentless_deflector_taunts_while_it_has_divine_shield() {
    let db = db();
    let mut deflector = mk(&db, "Relentless Deflector", false);
    deflector.keywords.set(Keywords::DIVINE_SHIELD);
    let bystander = mk(&db, "Microbot", false);
    let mut fodder = mk(&db, "Microbot", false);
    fodder.health = 100;
    for seed in 0..60 {
        let mut log = Log { db: Some(db.clone()), ..Default::default() };
        fight_watched(&db, &[fodder, fodder, fodder], &[bystander, deflector], 500, &mut Rng::new(seed), &mut log);
        // three minions attack first: they must hit the Deflector while its shield is up
        assert_eq!(log.attacks[0].4, "Relentless Deflector", "seed {seed}");
    }
    // without the shield it is an ordinary target
    let plain = mk(&db, "Relentless Deflector", false);
    let mut hit_bystander = false;
    for seed in 0..60 {
        let mut log = Log { db: Some(db.clone()), ..Default::default() };
        fight_watched(&db, &[fodder, fodder, fodder], &[bystander, plain], 500, &mut Rng::new(seed), &mut log);
        hit_bystander |= log.attacks[0].4 == "Microbot";
    }
    assert!(hit_bystander);
}

// ---- The Tavern: Fishbait, Activate, Magnetization ----

#[test]
fn spark_snapper_satellites_grow_by_2_3_per_mech_played() {
    let mut g = game(205);
    put_on_board(&mut g, 0, "Spark Snapper");
    g.give(0, "Cord Puller").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 1 });
    // First Mech: a 2/3 Satellite, so the 1/1 host becomes 3/4.
    assert_eq!((g.players[0].board[1].attack, g.players[0].board[1].health), (3, 4));
    assert_eq!(g.players[0].board[0].improvements, 1);
    g.give(0, "Cord Puller").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 2 });
    // Second Mech: 4/6, so the 1/1 host becomes 5/7.
    assert_eq!((g.players[0].board[2].attack, g.players[0].board[2].health), (5, 7));
    assert_eq!(g.players[0].board[0].improvements, 2);
    g.check_invariants().unwrap();
}

#[test]
fn golden_spark_snapper_satellites_start_at_4_6_and_grow_by_4_6() {
    let mut g = game(206);
    let c = g.data.cards.by_name("Spark Snapper").unwrap();
    let slot = g.data.cards.get(c).pool_slot as usize;
    g.pool[slot] -= 3;
    let m = g.players[0].make_card(&g.data.cards, c, true, 3);
    g.players[0].board.push(m);
    g.give(0, "Cord Puller").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 1 });
    assert_eq!((g.players[0].board[1].attack, g.players[0].board[1].health), (5, 7));
    g.give(0, "Cord Puller").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 2 });
    assert_eq!((g.players[0].board[2].attack, g.players[0].board[2].health), (9, 13));
    assert_eq!(g.players[0].board[0].improvements, 2);
    g.check_invariants().unwrap();
}

fn lobby(ts: &[Tribe]) -> bgcore::Tribes {
    ts.iter().copied().fold(bgcore::Tribes::NONE, |m, t| m.with(t))
}

fn game(seed: u64) -> Game {
    Game::new_with_lobby(Arc::new(Data::bundled()), seed, lobby(&[Tribe::Beast, Tribe::Mech, Tribe::Murloc, Tribe::Pirate, Tribe::Undead]))
}

/// Answer whatever the current player has open (a Discover, a target, a
/// discard, or a trinket offer), so a helper that only wants the next turn
/// never stalls on a prompt. False when nothing is open.
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

fn put_on_board(g: &mut Game, p: usize, name: &str) {
    let c = g.data.cards.by_name(name).unwrap();
    let slot = g.data.cards.get(c).pool_slot as usize;
    g.pool[slot] -= 1;
    let mut m = g.players[p].make_card(&g.data.cards, c, false, 1);
    m.uses_left = m.turn_uses(&g.data.cards);
    g.players[p].board.push(m);
}

fn name(g: &Game, m: &Minion) -> String {
    g.data.cards.get(m.card).name.clone()
}

/// Replace a player's shop with the named cards, through the pool.
fn set_shop(g: &mut Game, p: usize, names: &[&str]) {
    let old: Vec<Minion> = g.players[p].shop.drain(..).collect();
    for m in old {
        if !g.data.cards.get(m.card).token {
            let slot = g.data.cards.get(m.card).pool_slot as usize;
            g.pool[slot] += m.pool_copies;
        }
    }
    for n in names {
        let c = g.data.cards.by_name(n).unwrap();
        let slot = g.data.cards.get(c).pool_slot as usize;
        g.pool[slot] -= 1;
        let m = g.players[p].make_card(&g.data.cards, c, false, 1);
        g.players[p].shop.push(m);
    }
}

fn stats(m: &Minion) -> (i32, i32) {
    (m.attack, m.health)
}

fn shop_names(g: &Game, p: usize) -> Vec<String> {
    g.players[p].shop.iter().map(|m| name(g, m)).collect()
}

#[test]
fn snarky_shark_refreshes_with_a_fishbait_the_leftmost_beast_eats() {
    let mut g = game(1);
    put_on_board(&mut g, 0, "Buzzing Vermin");
    put_on_board(&mut g, 0, "Snarky Shark");
    let gold = g.players[0].gold;
    act(&mut g, 0, Action::Sell(1));
    let pl = &g.players[0];
    assert_eq!(pl.gold, gold + 1);
    // the Tavern was refreshed (3 slots at Tier 1) and one slot was the Fishbait, now eaten
    assert_eq!(pl.shop.len(), 2);
    assert!(!shop_names(&g, 0).contains(&"Fishbait".to_string()));
    // the Vermin killed it: +5/+5
    assert_eq!(stats(&pl.board[0]), (6, 6));
    g.check_invariants().unwrap();
}

#[test]
fn snarky_shark_without_a_beast_leaves_a_buyable_fishbait() {
    let mut g = game(2);
    put_on_board(&mut g, 0, "Cord Puller");
    put_on_board(&mut g, 0, "Snarky Shark");
    act(&mut g, 0, Action::Sell(1));
    assert_eq!(g.players[0].shop.len(), 3);
    let slot = shop_names(&g, 0).iter().position(|n| n == "Fishbait").expect("a Fishbait in the Tavern");
    // always the left-most slot
    assert_eq!(slot, 0);
    let fishbait = g.players[0].shop[slot];
    assert_eq!(stats(&fishbait), (0, 1));
    assert!(g.data.cards.get(fishbait.card).tribes.has(bgcore::Tribe::Beast));
    // it can be bought like any minion
    act(&mut g, 0, Action::Buy(slot as u8));
    let pl = &g.players[0];
    assert_eq!(name(&g, pl.hand.last().unwrap()), "Fishbait");
    assert_eq!(pl.hand.last().unwrap().pool_copies, 0);
    g.check_invariants().unwrap();
}

#[test]
fn golden_fishbait_has_2_health_and_the_shark_may_fail_to_kill_it() {
    let mut g = game(3);
    put_on_board(&mut g, 0, "Buzzing Vermin");
    put_on_board(&mut g, 0, "Snarky Shark");
    g.players[0].board[1].golden = true;
    act(&mut g, 0, Action::Sell(1));
    // a 1 Attack Vermin leaves the Golden Fishbait at 1 Health, and gains nothing
    assert_eq!(g.players[0].shop.len(), 3);
    let slot = shop_names(&g, 0).iter().position(|n| n == "Fishbait").unwrap();
    assert_eq!(slot, 0);
    let f = g.players[0].shop[slot];
    assert!(f.golden);
    assert_eq!(stats(&f), (0, 1));
    assert_eq!(stats(&g.players[0].board[0]), (1, 1));
    g.check_invariants().unwrap();
}

#[test]
fn a_fishbait_attack_triggers_rally_and_attack_effects() {
    // Wolf Pup is the left-most Beast: its Rally gives the Cord Puller +4/+1, and it eats the bait
    let mut g = game(4);
    put_on_board(&mut g, 0, "Wolf Pup");
    put_on_board(&mut g, 0, "Cord Puller");
    put_on_board(&mut g, 0, "Snarky Shark");
    act(&mut g, 0, Action::Sell(2));
    assert_eq!(stats(&g.players[0].board[0]), (3 + 5, 6 + 5));
    assert_eq!(stats(&g.players[0].board[1]), (1 + 4, 1 + 1));
    g.check_invariants().unwrap();

    // Cage Gnawer buffs the Beasts when the Vermin attacks
    let mut g = game(5);
    put_on_board(&mut g, 0, "Buzzing Vermin");
    put_on_board(&mut g, 0, "Cage Gnawer");
    put_on_board(&mut g, 0, "Snarky Shark");
    act(&mut g, 0, Action::Sell(2));
    assert_eq!(stats(&g.players[0].board[0]), (1 + 2 + 5, 1 + 1 + 5));
    assert_eq!(stats(&g.players[0].board[1]), (2 + 2, 7 + 1));

    // a Rally that summons puts the summon on the board
    let mut g = game(6);
    put_on_board(&mut g, 0, "Flittering Bat");
    put_on_board(&mut g, 0, "Snarky Shark");
    act(&mut g, 0, Action::Sell(1));
    let names: Vec<String> = g.players[0].board.iter().map(|m| name(&g, m)).collect();
    assert_eq!(names, ["Flittering Bat", "Foraging Bat"]);
    g.check_invariants().unwrap();
}

#[test]
fn lurking_lionfish_replaces_the_tavern_card_you_pick() {
    let mut g = game(7);
    put_on_board(&mut g, 0, "Lurking Lionfish");
    g.players[0].gold = 5;
    let before = shop_names(&g, 0);
    let n = before.len();
    act(&mut g, 0, Action::Activate(0));
    assert_eq!(g.players[0].gold, 3);
    // the player picks the Tavern slot
    assert!(g.players[0].pending_shop_target.is_some());
    act(&mut g, 0, Action::TargetShop(1));
    let pl = &g.players[0];
    assert_eq!(pl.shop.len(), n - 1);
    // the Lionfish is the left-most Beast: 3/4 plus 5/5
    assert_eq!(stats(&pl.board[0]), (8, 9));
    // the card that was replaced is back in the pool
    assert_eq!(shop_names(&g, 0), [before[0].clone(), before[2].clone()]);
    g.check_invariants().unwrap();

    // not enough gold, or an empty Tavern, and Activate is not legal
    let mut g = game(8);
    put_on_board(&mut g, 0, "Lurking Lionfish");
    g.players[0].gold = 1;
    turn_of(&mut g, 0);
    assert!(!g.is_legal(Action::Activate(0)));
    g.players[0].gold = 5;
    g.players[0].shop.clear();
    assert!(!g.is_legal(Action::Activate(0)));
}

#[test]
fn drone_duplicator_makes_the_next_magnetization_happen_again() {
    let mut g = game(9);
    put_on_board(&mut g, 0, "Drone Duplicator");
    g.players[0].gold = 5;
    act(&mut g, 0, Action::Activate(0));
    assert_eq!(g.players[0].gold, 4);
    g.give(0, "Annoy-o-Module").unwrap(); // 2/4 Magnetic
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    let host = g.players[0].board[0];
    // 5/2 plus two 2/4s, and two Magnetizations
    assert_eq!(stats(&host), (5 + 4, 2 + 8));
    assert_eq!(host.magnetizations, 2);
    // only the next one: the second Annoy-o-Module merges once
    g.give(0, "Annoy-o-Module").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    assert_eq!(stats(&g.players[0].board[0]), (5 + 6, 2 + 12));
    assert_eq!(g.players[0].board[0].magnetizations, 3);
    g.check_invariants().unwrap();

    // golden: two extra times
    let mut g = game(10);
    put_on_board(&mut g, 0, "Drone Duplicator");
    g.players[0].board[0].golden = true;
    g.players[0].gold = 5;
    act(&mut g, 0, Action::Activate(0));
    g.give(0, "Annoy-o-Module").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    assert_eq!(g.players[0].board[0].magnetizations, 3);

    // it does not last past the turn
    let mut g = game(11);
    put_on_board(&mut g, 0, "Drone Duplicator");
    g.players[0].gold = 5;
    act(&mut g, 0, Action::Activate(0));
    assert_eq!(g.players[0].board[0].extra_magnetize, 1);
    act(&mut g, 0, Action::EndTurn);
    assert_eq!(g.players[0].board[0].extra_magnetize, 0);
}

#[test]
fn drone_presses_carry_into_the_triple_and_add_up() {
    // two Drones pressed (1 extra each), then the third bought: the golden
    // carries 1 + 1 = 2 extras, its own press adds 2 more (it starts with a
    // fresh Activate), so the next Magnetization happens 4 extra times,
    // 5 in total
    let mut g = game(9);
    put_on_board(&mut g, 0, "Drone Duplicator");
    put_on_board(&mut g, 0, "Drone Duplicator");
    g.players[0].gold = 20;
    act(&mut g, 0, Action::Activate(0));
    act(&mut g, 0, Action::Activate(1));
    set_shop(&mut g, 0, &["Drone Duplicator"]);
    act(&mut g, 0, Action::Buy(0)); // the third copy triples
    assert_eq!(g.players[0].board.len(), 0, "the copies went into the golden");
    let gold = g.players[0].hand.iter().position(|m| m.golden).unwrap();
    act(&mut g, 0, Action::Play { hand: gold as u8, pos: 0 });
    assert_eq!(g.players[0].board[0].extra_magnetize, 2, "the copies' extras carried");
    act(&mut g, 0, Action::Activate(0));
    assert_eq!(g.players[0].board[0].extra_magnetize, 4, "the golden's own press adds 2");
    g.give(0, "Annoy-o-Module").unwrap(); // 2/4 Magnetic
    let h = g.players[0].hand.len() as u8 - 1;
    act(&mut g, 0, Action::Play { hand: h as u8, pos: 0 });
    assert_eq!(g.players[0].board[0].magnetizations, 5, "1 + 4 extra");
    g.check_invariants().unwrap();
}

#[test]
fn a_drone_triple_keeps_only_its_copies_pressed_extras() {
    // both copies pressed, the golden not: 1 + 1 carried, 3 Magnetizations
    // in total
    let mut g = game(10);
    put_on_board(&mut g, 0, "Drone Duplicator");
    put_on_board(&mut g, 0, "Drone Duplicator");
    g.players[0].gold = 20;
    act(&mut g, 0, Action::Activate(0));
    act(&mut g, 0, Action::Activate(1));
    set_shop(&mut g, 0, &["Drone Duplicator"]);
    act(&mut g, 0, Action::Buy(0));
    let gold = g.players[0].hand.iter().position(|m| m.golden).unwrap();
    act(&mut g, 0, Action::Play { hand: gold as u8, pos: 0 });
    assert_eq!(g.players[0].board[0].extra_magnetize, 2);
    g.give(0, "Annoy-o-Module").unwrap();
    let h = g.players[0].hand.len() as u8 - 1;
    act(&mut g, 0, Action::Play { hand: h as u8, pos: 0 });
    assert_eq!(g.players[0].board[0].magnetizations, 3);
    g.check_invariants().unwrap();
}

#[test]
fn gearfin_gets_two_one_cost_tavern_spells_at_the_end_of_turn() {
    let mut g = game(12);
    put_on_board(&mut g, 0, "Gearfin");
    act(&mut g, 0, Action::EndTurn);
    let pl = &g.players[0];
    assert_eq!(pl.hand.len(), 2);
    assert!(pl.hand.iter().all(|m| g.data.cards.get(m.card).is_spell() && g.data.cards.get(m.card).cost == 1));
    // Gearfin is both a Mech and a Murloc
    let t = g.data.cards.get(pl.board[0].card).tribes;
    assert!(t.has(bgcore::Tribe::Mech) && t.has(bgcore::Tribe::Murloc));

    let mut g = game(13);
    put_on_board(&mut g, 0, "Gearfin");
    g.players[0].board[0].golden = true;
    act(&mut g, 0, Action::EndTurn);
    assert_eq!(g.players[0].hand.len(), 4);
}

#[test]
fn golden_touch_makes_a_tavern_minion_golden() {
    let mut g = game(14);
    g.give(0, "Golden Touch").unwrap();
    let before: Vec<(i32, i32)> = g.players[0].shop.iter().map(stats).collect();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    // Aureate Laureate is always Golden: only a Golden Touch counts.
    let golden: Vec<usize> = (0..g.players[0].shop.len())
        .filter(|&i| {
            g.players[0].shop[i].golden && !g.data.cards.get(g.players[0].shop[i].card).always_golden
        })
        .collect();
    assert_eq!(golden.len(), 1);
    let i = golden[0];
    assert_eq!(stats(&g.players[0].shop[i]), (before[i].0 * 2, before[i].1 * 2));
    g.check_invariants().unwrap();
}

#[test]
fn prosthetic_hand_magnetizes_to_a_mech_and_brings_reborn() {
    let mut g = game(15);
    put_on_board(&mut g, 0, "Cord Puller");
    g.give(0, "Prosthetic Hand").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    assert_eq!(g.players[0].board.len(), 1);
    let m = g.players[0].board[0];
    assert!(m.keywords.has(Keywords::REBORN));
    assert_eq!(stats(&m), (1 + 3, 1 + 1));
}

// ---- Combat results that reach the player ----

#[test]
fn treasure_parrot_hands_over_a_golden_touch_after_the_fight() {
    let mut g = game(16);
    let Some(bgcore::Opponent::Player(q)) = g.players[0].opponent else { panic!() };
    put_on_board(&mut g, 0, "Treasure Parrot");
    g.players[0].board[0].improvements = 30;
    put_on_board(&mut g, q as usize, "Cage Gnawer");
    g.players[q as usize].board[0].attack = 0;
    g.players[q as usize].board[0].health = 1000;
    while g.turn == 1 {
        let p = g.current_player().unwrap();
        g.step(Action::EndTurn).unwrap();
        let _ = p;
    }
    let pl = &g.players[0];
    assert!(pl.hand.iter().any(|m| name(&g, m) == "Golden Touch"));
    assert!(pl.board[0].improvements >= 35);
    g.check_invariants().unwrap();
}

#[test]
fn tasty_lobster_improvements_and_falling_sky_golem_reach_the_player() {
    let mut g = game(17);
    let Some(bgcore::Opponent::Player(q)) = g.players[0].opponent else { panic!() };
    put_on_board(&mut g, 0, "Tasty Lobster");
    put_on_board(&mut g, 0, "Falling Sky Golem");
    put_on_board(&mut g, q as usize, "Cage Gnawer");
    g.players[q as usize].board[0].attack = 50;
    g.players[q as usize].board[0].health = 1000;
    while g.turn == 1 {
        g.step(Action::EndTurn).unwrap();
    }
    let pl = &g.players[0];
    // the Lobster died once: the next Lobster Deathrattle is stronger, and the Golem counts it
    assert_eq!(pl.improve, [1, 0]);
    assert_eq!(pl.deathrattles_triggered, 1);
    let golem = pl.board.iter().find(|m| name(&g, m) == "Falling Sky Golem").unwrap();
    assert_eq!(stats(golem), (4 + 4, 2 + 2));
    // a new Lobster is plain, a new Golem comes with the bonus
    let lobster = g.data.cards.by_name("Tasty Lobster").unwrap();
    let m = g.players[0].make_card(&g.data.cards, lobster, false, 0);
    assert_eq!(stats(&m), (2, 1));
    let golem_id = g.data.cards.by_name("Falling Sky Golem").unwrap();
    let m = g.players[0].make_card(&g.data.cards, golem_id, true, 0);
    assert_eq!(stats(&m), (8 + 8, 4 + 4));
    g.check_invariants().unwrap();
}

#[test]
fn an_all_type_minion_is_a_mech_for_magnetic() {
    let mut g = game(310);
    put_on_board(&mut g, 0, "Gatekeeper Amalgam"); // every tribe, a Mech too
    g.give(0, "Lullabot").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    // the Magnetic merges into the all-type minion instead of placing
    assert_eq!(g.players[0].board.len(), 1);
    assert_eq!(g.players[0].board[0].magnetizations, 1);
    assert_eq!(stats(&g.players[0].board[0]), (6 + 2, 6 + 2));
    g.check_invariants().unwrap();
}

// ---- Card data ----

#[test]
fn rally_and_attack_triggers_only_take_effects_a_fight_can_run() {
    let base = |extra: &str| {
        format!(
            "[[minion]]\nname = \"A\"\ntier = 1\ntribe = \"beast\"\nstats = [1, 1]\n{extra}\n"
        )
    };
    let ok = base("rally = { buff = [1, 1], target = \"others\" }");
    assert!(CardDb::from_toml(&ok).is_ok());
    for bad in [
        "rally = { gold_next_turn = 1 }",
        "rally = { buff = [1, 1], target = \"chosen\" }",
        "after_friendly_attack = { effect = { damage_hero = 1 } }",
        "whenever_friendly_attack = { effect = { replace_with = \"B\" } }",
        "avenge = { every = 0, effect = { gain_keyword = \"taunt\" } }",
        "avenge = { every = 2, effect = { tavern_buff = [1, 1] } }",
        "deathrattle = { effect = { gold_next_turn = 1 } }",
    ] {
        let toml = format!("{}\n[[token]]\nname = \"B\"\ntier = 1\ntribe = \"beast\"\nstats = [0, 1]\n", base(bad));
        assert!(CardDb::from_toml(&toml).is_err(), "{bad} should be refused");
    }
    let bad_tribe = base("whenever_friendly_attack = { attacker_tribe = \"naga\", effect = { buff = [1, 1], target = \"all\" } }");
    assert!(CardDb::from_toml(&bad_tribe).unwrap_err().contains("unknown tribe"));
}

// ---- Goldrinn in the shop ----

fn end_all_turns(g: &mut Game) {
    let t = g.turn;
    while g.turn == t {
        if !answer(g) {
            g.step(Action::EndTurn).unwrap();
        }
    }
}

#[test]
fn goldrinn_in_the_shop_buffs_all_beasts_until_the_next_turn() {
    let mut g = game(21);
    let Some(bgcore::Opponent::Player(q)) = g.players[0].opponent else { panic!() };
    let q = q as usize;
    for n in ["Wolf Pup", "Goldrinn, the Great Wolf", "Deathstrider", "Snarky Shark", "Snarky Shark"] {
        put_on_board(&mut g, 0, n);
    }
    put_on_board(&mut g, q, "Cage Gnawer");
    g.players[q].board[0].attack = 0;
    g.players[q].board[0].health = 1000;
    let before: Vec<(i32, i32)> = g.players[0].board.iter().map(stats).collect();
    // each sold Shark makes the Wolf Pup eat a Fishbait (+5/+5 and a Rally that gives the
    // others +4/+1) and Deathstrider triggers Goldrinn's Deathrattle: +7/+7 each time
    act(&mut g, 0, Action::Sell(4));
    act(&mut g, 0, Action::Sell(3));
    assert_eq!(g.players[0].beast_bonus, (14, 14));
    let pup = stats(&g.players[0].board[0]);
    let goldrinn = stats(&g.players[0].board[1]);
    let strider = stats(&g.players[0].board[2]);
    assert_eq!(goldrinn, (before[1].0 + 8 + 14, before[1].1 + 2 + 14));
    assert_eq!(strider, (before[2].0 + 8 + 14, before[2].1 + 2 + 14));
    assert_eq!(pup, (before[0].0 + 10 + 14, before[0].1 + 10 + 14));
    // a Beast played afterwards has it too
    g.give(0, "Flittering Bat").unwrap();
    let hand = g.players[0].hand.len() as u8 - 1;
    let (ba, bh) = stats(&g.players[0].hand[hand as usize]);
    act(&mut g, 0, Action::Play { hand, pos: 3 });
    let bat = g.players[0].board.iter().position(|m| name(&g, m) == "Flittering Bat").unwrap();
    assert_eq!(stats(&g.players[0].board[bat]), (ba + 14, bh + 14));
    // a Mech does not
    g.give(0, "Cord Puller").unwrap();
    let hand = g.players[0].hand.len() as u8 - 1;
    let (ma, mh) = stats(&g.players[0].hand[hand as usize]);
    act(&mut g, 0, Action::Play { hand, pos: 0 });
    assert_eq!(stats(&g.players[0].board[0]), (ma, mh));

    // the fight sees the buffed Beasts, and the bonus is gone when the next turn starts
    let goldrinn_before_fight = stats(&g.players[0].board[2]);
    end_all_turns(&mut g);
    assert_eq!(g.players[0].beast_bonus, (0, 0));
    let gr = g.players[0].board.iter().find(|m| name(&g, m) == "Goldrinn, the Great Wolf").unwrap();
    assert_eq!(stats(gr), (goldrinn_before_fight.0 - 14, goldrinn_before_fight.1 - 14));
    let bat = g.players[0].board.iter().find(|m| name(&g, m) == "Flittering Bat").unwrap();
    assert_eq!(stats(bat), (ba, bh));
    g.check_invariants().unwrap();
}

#[test]
fn clearing_the_beast_bonus_never_kills_a_beast() {
    let mut g = game(22);
    put_on_board(&mut g, 0, "Buzzing Vermin");
    g.players[0].beast_bonus = (7, 7);
    g.players[0].board[0].health = 3;
    g.players[0].board[0].attack = 2;
    end_all_turns(&mut g);
    let v = g.players[0].board[0];
    assert!(v.health >= 1 && v.attack >= 0);
    assert_eq!(g.players[0].beast_bonus, (0, 0));
}

#[test]
fn beasts_summoned_in_a_fight_get_the_players_beast_bonus() {
    let db = db();
    let counters = [Counters { beast_attack: 14, beast_health: 14, ..Default::default() }, Counters::default()];
    let (_, log) = run(&db, &[mk(&db, "Hoarding Hyena", false)], &[wall(&db)], counters, 1);
    assert_eq!((log.summons[0].2.as_str(), log.summons[0].3, log.summons[0].4), ("Tasty Lobster", 16, 15));
}

#[test]
fn a_triple_does_not_keep_the_beast_bonus_twice() {
    let mut g = game(23);
    put_on_board(&mut g, 0, "Buzzing Vermin");
    put_on_board(&mut g, 0, "Buzzing Vermin");
    g.players[0].beast_bonus = (7, 7);
    for m in &mut g.players[0].board {
        m.attack += 7;
        m.health += 7;
    }
    // a third copy bought completes the triple
    let c = g.data.cards.by_name("Buzzing Vermin").unwrap();
    let third = g.players[0].make_card(&g.data.cards, c, false, 1);
    let old = std::mem::replace(&mut g.players[0].shop[0], third);
    let (old_slot, slot) = (
        g.data.cards.get(old.card).pool_slot as usize,
        g.data.cards.get(c).pool_slot as usize,
    );
    g.pool[old_slot] += 1;
    g.pool[slot] -= 1;
    g.players[0].gold = 10;
    act(&mut g, 0, Action::Buy(0));
    let gold = g.players[0].hand.iter().find(|m| m.golden).expect("a golden Vermin");
    let plain = g.data.cards.make(gold.card, true);
    assert_eq!(stats(gold), stats(&plain));
}

/// A Volumizer that merges into a host does not grow it by the pair it just
/// improved: "After you Magnetize this, improve your future Magnetizations"
/// is for the Magnetizations after this one, and the host takes the card's own
/// stats (triage group M1; log 2026_10_28_22_11_50-g5 turn 6 step 12:
/// Relentless Deflector 13/19 to 20/36 for a Red Volumizer's 4/8, and its
/// Volumizer in hand 3/7 to 4/8).
#[test]
fn a_volumizers_own_improvement_does_not_reach_its_host() {
    let mut g = game(346);
    put_on_board(&mut g, 0, "Cord Puller");
    let shop = g.data.cards.by_name("Red Volumizer").unwrap();
    let slot = g.data.cards.get(shop).pool_slot as usize;
    g.pool[slot] -= 1;
    let mut vol = g.players[0].make_card(&g.data.cards, shop, false, 1);
    vol.attack = 4;
    vol.health = 8;
    g.players[0].hand.push(vol);
    // A second Volumizer in hand: the improvement does reach that one.
    let green = g.data.cards.by_name("Green Volumizer").unwrap();
    let gslot = g.data.cards.get(green).pool_slot as usize;
    g.pool[gslot] -= 1;
    g.players[0].hand.push(g.players[0].make_card(&g.data.cards, green, false, 1));

    let before = stats(&g.players[0].board[0]);
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    assert_eq!(
        stats(&g.players[0].board[0]),
        (before.0 + 4, before.1 + 8),
        "the host takes the Volumizer's own 4/8 and not its +3/+0 improvement"
    );
    let in_hand = stats(&g.players[0].hand[0]);
    assert_eq!(in_hand, (6, 3), "the Green Volumizer still in hand grew by +3/+0");
    g.check_invariants().unwrap();
}
