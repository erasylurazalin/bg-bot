use bgcore::fight::{Watch, fight_watched, fight_with};
use bgcore::{CardDb, Counters, Data, Keywords, Minion, Rng, fight};

fn db() -> CardDb {
    CardDb::from_toml(include_str!("data/v01-cards.toml")).unwrap()
}

fn s2_db() -> CardDb {
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

#[test]
fn start_of_combat_humming_bird_and_boom_in_a_box() {
    let db = s2_db();
    // Humming Bird gives friendly Beasts +1 Attack (golden +2) for the combat,
    // including Beasts summoned later.
    let a = board(&db, &["Buzzing Vermin", "Humming Bird"]);
    let mut b = board(&db, &["Microbot"]);
    b[0].attack = 1;
    b[0].health = 20;
    let mut log = Log { db: Some(db.clone()), ..Default::default() };
    fight_watched(&db, &a, &b, 500, &mut Rng::new(1), &mut log);
    assert_eq!(log.summons.len(), 1);
    // Beetle is 2/2 base + 1 Attack from Humming Bird = 3/2
    assert_eq!((log.summons[0].2.attack, log.summons[0].2.health), (3, 2));

    // Boom-in-a-Box deals 3 damage to all other minions at Start of Combat
    // (golden twice, popping shield then dealing 3).
    let mut boom = db.make(db.by_name("Boom-in-a-Box").unwrap(), true);
    boom.attack = 0;
    let mut cord = db.make(db.by_name("Cord Puller").unwrap(), false);
    cord.health = 3; // Divine Shield + 3 HP: wave 1 pops shield, wave 2 kills it
    let out = fight(&db, &[boom], &[cord], 500, &mut Rng::new(2));
    // Cord Puller died in Start of Combat and summoned a 1/1 Microbot, which kills 0-attack Boom.
    assert_eq!(out.winner, Some(1));
    assert_eq!(out.survivor_tiers, 1);
}

#[test]
fn rally_flittering_bat_and_headhunter_gryphon() {
    let db = s2_db();
    let bat = db.make(db.by_name("Flittering Bat").unwrap(), true);
    let mut dummy = db.make(db.by_name("Microbot").unwrap(), false);
    dummy.attack = 0;
    dummy.health = 5;
    let mut log = Log { db: Some(db.clone()), ..Default::default() };
    let out = fight_watched(&db, &[bat, dummy], &[dummy], 500, &mut Rng::new(1), &mut log);
    assert_eq!(out.winner, Some(0));
    // Golden Flittering Bat summons two 1/1 Foraging Bats on its first attack
    assert!(log.summons.len() >= 2);
    assert_eq!((log.summons[0].2.attack, log.summons[0].2.health), (1, 1));
    assert_eq!((log.summons[1].2.attack, log.summons[1].2.health), (1, 1));

    // Headhunter Gryphon gives a random Beast on attack (golden gives 2)
    let gryphon = db.make(db.by_name("Headhunter Gryphon").unwrap(), false);
    let gold_gryphon = db.make(db.by_name("Headhunter Gryphon").unwrap(), true);
    let mut target = db.make(db.by_name("Microbot").unwrap(), false);
    target.attack = 100;
    target.health = 1;
    let out1 = fight(&db, &[gryphon], &[target], 500, &mut Rng::new(7));
    assert_eq!(out1.sides[0].cards_gained().len(), 1);
    let out2 = fight(&db, &[gold_gryphon], &[target], 500, &mut Rng::new(7));
    assert_eq!(out2.sides[0].cards_gained().len(), 2);
}

#[test]
fn beast_summon_scaling_leviathan_banana_slamma() {
    let db = s2_db();
    let vermin = db.make(db.by_name("Buzzing Vermin").unwrap(), false);
    let leviathan = db.make(db.by_name("Lurking Leviathan").unwrap(), false);
    let slamma = db.make(db.by_name("Banana Slamma").unwrap(), false);
    let gold_slamma = db.make(db.by_name("Banana Slamma").unwrap(), true);
    let mut wall = db.make(db.by_name("Microbot").unwrap(), false);
    wall.attack = 10;
    wall.health = 100;

    // Beetle (2/2) + Lurking Leviathan (+3 Attack) = 5/2, then Banana Slamma
    // doubles Attack to 10/2.
    let mut log = Log { db: Some(db.clone()), ..Default::default() };
    fight_watched(&db, &[vermin, leviathan, slamma], &[wall], 500, &mut Rng::new(1), &mut log);
    assert_eq!((log.summons[0].2.attack, log.summons[0].2.health), (10, 2));

    // With golden Banana Slamma in place of plain, 5/2 triples Attack to 15/2.
    let mut log2 = Log { db: Some(db.clone()), ..Default::default() };
    fight_watched(&db, &[vermin, leviathan, gold_slamma], &[wall], 500, &mut Rng::new(1), &mut log2);
    assert_eq!((log2.summons[0].2.attack, log2.summons[0].2.health), (15, 2));
}

#[test]
fn titus_rivendare_and_sewer_lord() {
    let db = s2_db();
    let titus = db.make(db.by_name("Titus Rivendare").unwrap(), false);
    let gold_titus = db.make(db.by_name("Titus Rivendare").unwrap(), true);
    let mut killer = db.make(db.by_name("Microbot").unwrap(), false);
    killer.attack = 10;
    killer.health = 100;

    // Buzzing Vermin (1 Beetle) with plain Titus summons 2 Beetles; with golden Titus, 3 Beetles.
    let vermin = db.make(db.by_name("Buzzing Vermin").unwrap(), false);
    let mut log_t1 = Log { db: Some(db.clone()), ..Default::default() };
    fight_watched(&db, &[vermin, titus], &[killer], 500, &mut Rng::new(1), &mut log_t1);
    assert_eq!(log_t1.summons.len(), 2);

    let mut log_t2 = Log { db: Some(db.clone()), ..Default::default() };
    fight_watched(&db, &[vermin, gold_titus], &[killer], 500, &mut Rng::new(1), &mut log_t2);
    assert_eq!(log_t2.summons.len(), 3);

    // Sewer Lord summons 2 Sewer Rats, each of which summons a 2/3 Half-Shell with Taunt.
    let sewer_lord = db.make(db.by_name("Sewer Lord").unwrap(), false);
    let mut log_sl = Log { db: Some(db.clone()), ..Default::default() };
    fight_watched(&db, &[sewer_lord], &[killer], 500, &mut Rng::new(1), &mut log_sl);
    let spawned: Vec<String> =
        log_sl.summons.iter().map(|s| db.get(s.2.card).name.clone()).collect();
    assert_eq!(spawned, ["Sewer Rat", "Sewer Rat", "Half-Shell", "Half-Shell"]);
}

#[test]
fn turquoise_skitterer_and_ravaging_scorpid_buff_beetles_in_combat() {
    let db = s2_db();
    // Existing Beetle on board (2/2) + Turquoise Skitterer dying (+5/+5, then summons a Beetle):
    // both the existing Beetle and the newly summoned Beetle have +5/+5 (7/7), and the side
    // outcome records +5/+5 Beetle gain.
    let mut skitterer = db.make(db.by_name("Turquoise Skitterer").unwrap(), false);
    skitterer.keywords.set(Keywords::TAUNT);
    let beetle = db.make(db.by_name("Beetle").unwrap(), false);
    let mut killer = db.make(db.by_name("Microbot").unwrap(), false);
    killer.attack = 5;
    killer.health = 5;
    let mut log = Log { db: Some(db.clone()), ..Default::default() };
    let out = fight_with(
        &db,
        &[skitterer, beetle],
        &[killer],
        [Counters::default(); 2],
        500,
        &mut Rng::new(1),
        &mut log,
    );
    assert_eq!(out.winner, Some(0));
    assert_eq!((out.sides[0].beetle_attack_gain, out.sides[0].beetle_health_gain), (5, 5));
    assert_eq!((log.summons[0].2.attack, log.summons[0].2.health), (7, 7));
}

#[test]
fn rescue_bot_and_combat_spells_in_fight() {
    let db = s2_db();
    let repair_job = db.by_name("Repair Job").unwrap();

    // Rescue Bot gives 1 Repair Job on death (golden gives 2).
    let rescue = db.make(db.by_name("Rescue Bot").unwrap(), false);
    let gold_rescue = db.make(db.by_name("Rescue Bot").unwrap(), true);
    let mut killer = db.make(db.by_name("Microbot").unwrap(), false);
    killer.attack = 10;
    killer.health = 10;
    let out1 = fight(&db, &[rescue], &[killer], 500, &mut Rng::new(1));
    assert_eq!(out1.sides[0].cards_gained(), &[repair_job]);
    let out2 = fight(&db, &[gold_rescue], &[killer], 500, &mut Rng::new(1));
    assert_eq!(out2.sides[0].cards_gained(), &[repair_job, repair_job]);

    // Brood of Nozdormu doubles left-most minion's Attack; Upper Hand sets enemy Health to 1.
    let mut a = db.make(db.by_name("Microbot").unwrap(), false); // 3/10 -> 6/10 with Brood
    a.attack = 3;
    a.health = 10;
    let mut b = db.make(db.by_name("Microbot").unwrap(), false);
    b.attack = 5;
    b.health = 100; // set to 1 by Upper Hand
    let c0 = Counters { brood_of_nozdormu: 1, upper_hand: 1, ..Default::default() };
    let out3 = fight_with(&db, &[a], &[b], [c0, Counters::default()], 500, &mut Rng::new(1), &mut ());
    assert_eq!(out3.winner, Some(0));
}

