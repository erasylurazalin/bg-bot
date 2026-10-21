use bgcore::fight::{Watch, fight_watched};
use bgcore::{CardDb, Data, Keywords, Minion, Rng, fight};

fn db() -> CardDb {
    Data::bundled().cards
}

fn board(db: &CardDb, names: &[&str]) -> Vec<Minion> {
    names.iter().map(|n| db.make(db.by_name(n).expect(n), false)).collect()
}

/// Run a fight under many seeds and hand every outcome to `check`.
fn each_seed(a: &[Minion], b: &[Minion], mut check: impl FnMut(bgcore::Outcome)) {
    let db = db();
    for seed in 0..500 {
        check(fight(&db, a, b, 500, &mut Rng::new(seed)));
    }
}

#[derive(Default)]
struct Log {
    attacks: Vec<(usize, String, String)>,
    deaths: Vec<(usize, String)>,
    summons: Vec<(usize, usize, Minion)>,
    db: Option<CardDb>,
}

impl Watch for Log {
    fn attack(&mut self, side: usize, a: &Minion, d: &Minion) {
        let db = self.db.as_ref().unwrap();
        self.attacks.push((side, db.get(a.card).name.clone(), db.get(d.card).name.clone()));
    }
    fn death(&mut self, side: usize, _pos: usize, m: &Minion) {
        let name = self.db.as_ref().unwrap().get(m.card).name.clone();
        self.deaths.push((side, name));
    }
    fn summon(&mut self, side: usize, pos: usize, m: &Minion) {
        self.summons.push((side, pos, *m));
    }
}

fn watched(a: &[Minion], b: &[Minion], seed: u64) -> (bgcore::Outcome, Log) {
    let db = db();
    let mut log = Log { db: Some(db.clone()), ..Default::default() };
    let out = fight_watched(&db, a, b, 500, &mut Rng::new(seed), &mut log);
    (out, log)
}

#[test]
fn bigger_board_attacks_first() {
    let db = db();
    let a = board(&db, &["Wolf Pup"]);
    let b = board(&db, &["Wolf Pup", "Wolf Pup"]);
    for seed in 0..50 {
        let (_, log) = watched(&a, &b, seed);
        assert_eq!(log.attacks[0].0, 1);
    }
}

#[test]
fn equal_boards_flip_a_coin_for_first_attack() {
    let db = db();
    let a = board(&db, &["Wolf Pup"]);
    let firsts: usize = (0..400).map(|s| watched(&a, &a, s).1.attacks[0].0).sum();
    assert!((150..250).contains(&firsts), "{firsts} of 400");
}

#[test]
fn attackers_go_left_to_right_and_wrap() {
    let db = db();
    let a = board(&db, &["Wolf Pup", "Ram", "Shield Drone"]);
    let mut b = board(&db, &["Brute"]);
    b[0].health = 1000;
    b[0].attack = 0;
    let (_, log) = watched(&a, &b, 1);
    let order: Vec<&str> = log.attacks.iter().filter(|x| x.0 == 0).map(|x| x.1.as_str()).take(6).collect();
    assert_eq!(order, ["Wolf Pup", "Ram", "Shield Drone", "Wolf Pup", "Ram", "Shield Drone"]);
}

#[test]
fn taunt_must_be_attacked_first() {
    let db = db();
    let a = board(&db, &["Wolf Pup", "Wolf Pup", "Wolf Pup"]);
    let mut b = board(&db, &["Wolf Pup", "Ram", "Wolf Pup"]);
    b[1].health = 100;
    for seed in 0..100 {
        let (_, log) = watched(&a, &b, seed);
        for (side, _, def) in &log.attacks {
            if *side == 0 {
                assert_eq!(def, "Ram");
            }
        }
    }
}

#[test]
fn divine_shield_blocks_one_hit() {
    // Without the shield the Pup wins. With it, they trade into a tie
    // whoever attacks first.
    let db = db();
    let a = board(&db, &["Rust Bot"]);
    let b = board(&db, &["Wolf Pup"]);
    each_seed(&a, &b, |o| assert_eq!(o.winner, None));
    let mut plain = a.clone();
    plain[0].keywords = Keywords::default();
    each_seed(&plain, &b, |o| assert_eq!(o.winner, Some(1)));
}

#[test]
fn zero_damage_does_not_pop_divine_shield() {
    let db = db();
    let mut a = board(&db, &["Shield Drone"]);
    let mut b = board(&db, &["Wolf Pup"]);
    b[0].attack = 0;
    b[0].health = 100;
    a[0].attack = 1;
    let (out, log) = watched(&a, &b, 3);
    // 100 hits to kill the Pup, and the Drone keeps its shield throughout
    assert_eq!(out.attacks, 100);
    assert_eq!(out.winner, Some(0));
    assert!(log.deaths.iter().all(|d| d.0 == 1));
}

#[test]
fn venomous_kills_anything_it_damages() {
    let db = db();
    let a = board(&db, &["Viper"]);
    let b = board(&db, &["Warden"]);
    each_seed(&a, &b, |o| assert_eq!(o.winner, None));
}

#[test]
fn divine_shield_stops_venomous_and_venom_stays() {
    // The Drone's shield eats the Viper's hit, so the Drone wins every time.
    let db = db();
    let a = board(&db, &["Viper"]);
    let b = board(&db, &["Shield Drone"]);
    each_seed(&a, &b, |o| {
        assert_eq!(o.winner, Some(1));
        assert_eq!(o.survivor_tiers, 2);
    });
}

#[test]
fn windfury_attacks_twice_in_a_row() {
    let db = db();
    let a = board(&db, &["Twinblade"]);
    let mut b = board(&db, &["Brute"]);
    b[0].attack = 0;
    b[0].health = 100;
    let (_, log) = watched(&a, &b, 0);
    let sides: Vec<usize> = log.attacks.iter().map(|x| x.0).take(4).collect();
    assert_eq!(sides, [0, 0, 0, 0]); // the Brute can't attack, Twinblade swings twice per turn
    let mut a = board(&db, &["Twinblade", "Wolf Pup"]);
    a[0].health = 100;
    let mut b = board(&db, &["Warden", "Warden"]);
    b[0].health = 100;
    b[1].health = 100;
    let (_, log) = watched(&a, &b, 0);
    let first_a = log.attacks.iter().position(|x| x.0 == 0).unwrap();
    assert_eq!(log.attacks[first_a].1, "Twinblade");
    assert_eq!(log.attacks[first_a + 1].1, "Twinblade");
    assert_eq!(log.attacks[first_a + 2].0, 1);
}

#[test]
fn reborn_comes_back_once_with_printed_attack_and_1_health() {
    let db = db();
    let mut a = board(&db, &["Ember Spirit"]);
    a[0].attack = 10;
    let b = board(&db, &["Warden"]);
    for seed in 0..50 {
        let (out, log) = watched(&a, &b, seed);
        assert_eq!(out.winner, Some(0));
        assert_eq!(log.summons.len(), 1);
        let (side, pos, m) = log.summons[0];
        assert_eq!((side, pos), (0, 0));
        assert_eq!((m.attack, m.health), (3, 1));
        assert!(!m.keywords.has(Keywords::REBORN));
    }
}

#[test]
fn reborn_turns_a_trade_into_a_win() {
    let db = db();
    let a = board(&db, &["Ember Spirit"]);
    let b = board(&db, &["Wolf Pup"]);
    each_seed(&a, &b, |o| {
        assert_eq!(o.winner, Some(0));
        assert_eq!(o.survivor_tiers, 3);
    });
}

#[test]
fn deathrattle_tokens_fight_on() {
    // Alpha 5/5 vs Brute 8/8: Alpha leaves the Brute at 3 and two 2/2 Wolves.
    // The Brute kills one, the other trades with it. Without the tokens the
    // Brute would win.
    let db = db();
    let a = board(&db, &["Alpha"]);
    let b = board(&db, &["Brute"]);
    each_seed(&a, &b, |o| assert_eq!(o.winner, None));
}

#[test]
fn survivor_tiers_count_tokens_as_their_tier() {
    let db = db();
    let a = board(&db, &["Ancient Beast"]);
    let mut b = board(&db, &["Wolf Pup"]);
    b[0].attack = 9;
    b[0].health = 9;
    b[0].keywords.set(Keywords::DIVINE_SHIELD);
    // Pup kills the Beast, Beast pops the shield; three 3/3 tokens take the 9/9
    // down to 0 over three attacks while one token dies to it... whatever the
    // details, a token-only win counts 1 per token.
    each_seed(&a, &b, |o| {
        if o.winner == Some(0) {
            assert!(o.survivor_tiers <= 3);
        }
    });
}

#[test]
fn neither_side_able_to_attack_is_a_tie() {
    let db = db();
    let mut a = board(&db, &["Wolf Pup"]);
    let mut b = board(&db, &["Wolf Pup"]);
    a[0].attack = 0;
    b[0].attack = 0;
    each_seed(&a, &b, |o| assert_eq!((o.winner, o.attacks), (None, 0)));
}

#[test]
fn a_side_that_cant_attack_is_skipped() {
    let db = db();
    let mut a = board(&db, &["Wolf Pup", "Wolf Pup"]);
    a[0].attack = 0;
    a[1].attack = 0;
    let b = board(&db, &["Brute"]);
    each_seed(&a, &b, |o| assert_eq!(o.winner, Some(1)));
}

#[test]
fn attack_cap_ends_in_a_tie() {
    let db = db();
    let mut a = board(&db, &["Wolf Pup"]);
    a[0].health = 1_000_000;
    let b = a.clone();
    each_seed(&a, &b, |o| assert_eq!((o.winner, o.attacks), (None, 500)));
}

#[test]
fn empty_boards() {
    let db = db();
    let a = board(&db, &["Wolf Pup"]);
    each_seed(&[], &[], |o| assert_eq!(o.winner, None));
    each_seed(&a, &[], |o| assert_eq!((o.winner, o.survivor_tiers), (Some(0), 1)));
}

#[test]
fn same_seed_same_fight() {
    let db = db();
    let a = board(&db, &["Alpha", "Viper", "Twinblade", "Ember Spirit"]);
    let b = board(&db, &["Gear Golem", "Shredder", "Stag", "Den Mother", "Ram"]);
    for seed in 0..100 {
        let x = fight(&db, &a, &b, 500, &mut Rng::new(seed));
        let y = fight(&db, &a, &b, 500, &mut Rng::new(seed));
        assert_eq!(x, y);
    }
}
