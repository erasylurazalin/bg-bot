use bgcore::fight::{Watch, fight_watched, fight_with};
use bgcore::{CardDb, Counters, Data, Keywords, Minion, Rng, fight};

fn s2_db() -> CardDb {
    Data::bundled().cards
}

fn board(db: &CardDb, names: &[&str]) -> Vec<Minion> {
    names.iter().map(|n| db.make(db.by_name(n).expect(n), false)).collect()
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

    // Beetle (2/2) + Lurking Leviathan (+3 Attack) = 5/2. Leviathan's buff is
    // not an aura, so Banana Slamma multiplies it: 5 * 2 = 10/2. Power.log
    // 2026_11_08_03_26_06 line 763025 is the same rule with a bigger number:
    // a golden Sewer Rat at 165 (printed 6 plus +159) went to 495.
    let mut log = Log { db: Some(db.clone()), ..Default::default() };
    fight_watched(&db, &[vermin, leviathan, slamma], &[wall], 500, &mut Rng::new(1), &mut log);
    assert_eq!((log.summons[0].2.attack, log.summons[0].2.health), (10, 2));

    // A golden Banana Slamma multiplies it by three: 15/2.
    let mut log2 = Log { db: Some(db.clone()), ..Default::default() };
    fight_watched(&db, &[vermin, leviathan, gold_slamma], &[wall], 500, &mut Rng::new(1), &mut log2);
    assert_eq!((log2.summons[0].2.attack, log2.summons[0].2.health), (15, 2));
}

#[test]
fn banana_slamma_multiplies_non_aura_buffs_and_keeps_auras() {
    let db = s2_db();
    let vermin = db.make(db.by_name("Buzzing Vermin").unwrap(), false);
    let leviathan = db.make(db.by_name("Lurking Leviathan").unwrap(), false);
    let humming = db.make(db.by_name("Humming Bird").unwrap(), false);
    let slamma = db.make(db.by_name("Banana Slamma").unwrap(), false);
    let mut wall = db.make(db.by_name("Microbot").unwrap(), false);
    wall.attack = 10;
    wall.health = 100;

    // Start of Combat: Humming Bird gives friendly Beasts +1 Attack (an
    // aura). The Beetle is summoned at 2 printed + 1 aura + 3 non-aura
    // (Leviathan) = 6/2. Banana Slamma doubles the whole 6 and re-applies the
    // +1 aura: 6 * 2 + 1 = 13/2, not the 16 an "all buffs on top" rule gives.
    let mut log = Log { db: Some(db.clone()), ..Default::default() };
    fight_watched(
        &db,
        &[vermin, humming, leviathan, slamma],
        &[wall],
        500,
        &mut Rng::new(1),
        &mut log,
    );
    assert_eq!((log.summons[0].2.attack, log.summons[0].2.health), (13, 2));
}

#[test]
fn leviathan_beast_buffs_step_up_by_3_and_six_when_golden() {
    let db = s2_db();
    let mut wall = db.make(db.by_name("Microbot").unwrap(), false);
    wall.attack = 10;
    wall.health = 5000;

    // Two Vermins die in turn; each Deathrattle summons a Beetle. The first
    // gets +3 Attack, the second +6.
    let a = vec![
        db.make(db.by_name("Buzzing Vermin").unwrap(), false),
        db.make(db.by_name("Buzzing Vermin").unwrap(), false),
        db.make(db.by_name("Lurking Leviathan").unwrap(), false),
    ];
    let mut log = Log { db: Some(db.clone()), ..Default::default() };
    fight_watched(&db, &a, &[wall], 500, &mut Rng::new(1), &mut log);
    let mut attacks: Vec<i32> = log
        .summons
        .iter()
        .filter(|(_, _, m)| db.get(m.card).name == "Beetle")
        .map(|(_, _, m)| m.attack)
        .collect();
    attacks.sort();
    assert_eq!(attacks, vec![5, 8]);

    // A golden Leviathan steps by +6: 8 and 14.
    let a = vec![
        db.make(db.by_name("Buzzing Vermin").unwrap(), false),
        db.make(db.by_name("Buzzing Vermin").unwrap(), false),
        db.make(db.by_name("Lurking Leviathan").unwrap(), true),
    ];
    let mut log = Log { db: Some(db.clone()), ..Default::default() };
    fight_watched(&db, &a, &[wall], 500, &mut Rng::new(1), &mut log);
    let mut attacks: Vec<i32> = log
        .summons
        .iter()
        .filter(|(_, _, m)| db.get(m.card).name == "Beetle")
        .map(|(_, _, m)| m.attack)
        .collect();
    attacks.sort();
    assert_eq!(attacks, vec![8, 14]);
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
    // Existing Beetle on board (2/2) + Turquoise Skitterer dying: it summons
    // its Beetle first, then gives all Beetles +5/+5, the order in Power.log
    // 2026_11_11_02_43_24 line 152571. The Watch hook therefore sees the new
    // Beetle at 2/2; the +5/+5 lands on it and on the old Beetle afterwards,
    // and the side outcome records +5/+5 Beetle gain.
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
    assert_eq!((log.summons[0].2.attack, log.summons[0].2.health), (2, 2));
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

