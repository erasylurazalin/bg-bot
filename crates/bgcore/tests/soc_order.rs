//! Start of Combat order, from Era's logs and Firestone
//! (docs/notes/soc-order-2026-11-23.md): phases, the per-phase coin, the
//! first attacker and the Wingmen lock.
//!
//! Each test names the report change it covers. The log line numbers in the
//! report are raw Power.log lines; the cards below are the ones those fights
//! use where a real card can stand in for the effect.

use bgcore::fight::{Watch, fight_with, fight_with_hands};
use bgcore::trinkets::{FightAct, FightEntry, Pick, When};
use bgcore::{CardDb, CardId, Counters, Data, Minion, Rng};

fn cards() -> CardDb {
    Data::bundled().cards
}

fn board(db: &CardDb, names: &[&str]) -> Vec<Minion> {
    names.iter().map(|n| db.make(db.by_name(n).expect(n), false)).collect()
}

fn wall(db: &CardDb, attack: i32, health: i32) -> Minion {
    let mut m = db.make(db.by_name("Microbot").unwrap(), false);
    m.attack = attack;
    m.health = health;
    m
}

#[derive(Clone, Debug, PartialEq)]
enum Event {
    Attack(usize, String, i32, i32),
    Death(usize, String, i32),
    Summon(usize, String),
    Soc(usize, String),
    Recount(usize),
}

#[derive(Default)]
struct Log {
    events: Vec<Event>,
    db: Option<CardDb>,
}

impl Log {
    fn new(db: &CardDb) -> Log {
        Log { events: Vec::new(), db: Some(db.clone()) }
    }

    fn first_summon_side(&self) -> Option<usize> {
        self.events.iter().find_map(|e| match e {
            Event::Summon(side, _) => Some(*side),
            _ => None,
        })
    }

    fn first_attack_side(&self) -> Option<usize> {
        self.events.iter().find_map(|e| match e {
            Event::Attack(side, _, _, _) => Some(*side),
            _ => None,
        })
    }

}

impl Watch for Log {
    fn attack(&mut self, side: usize, a: &Minion, _d: &Minion) {
        let name = self.db.as_ref().unwrap().get(a.card).name.clone();
        self.events.push(Event::Attack(side, name, a.attack, a.health));
    }
    fn death(&mut self, side: usize, _pos: usize, m: &Minion) {
        let name = self.db.as_ref().unwrap().get(m.card).name.clone();
        self.events.push(Event::Death(side, name, m.attack));
    }
    fn summon(&mut self, side: usize, _pos: usize, m: &Minion) {
        let name = self.db.as_ref().unwrap().get(m.card).name.clone();
        self.events.push(Event::Summon(side, name));
    }
    fn start_of_combat(&mut self, side: usize, card: CardId) {
        let name = self.db.as_ref().unwrap().get(card).name.clone();
        self.events.push(Event::Soc(side, name));
    }
    fn recount(&mut self, attacker: usize) {
        self.events.push(Event::Recount(attacker));
    }
}

fn trinket_summon(db: &CardDb, name: &str) -> Counters {
    let mut c = Counters::default();
    let card = db.by_name(name).unwrap();
    c.trinkets.push(
        &FightEntry::new(When::Soc, FightAct::Summon { card, count: 1, keywords: 0 }),
        0,
        0,
    );
    c
}

/// Change 1: trinkets, then pre-combat hero powers, then Wingmen, then the
/// other hero powers, then spells, then the hand, then the board.
#[test]
fn phase_order_trinkets_pre_combat_wingmen_other_spells_hand_board() {
    let db = cards();

    // Trinket (+2 Attack), ALL Will Burn (+2) and Wingmen (+2) all land on
    // the same lone minion before it attacks. Only the new order gives 3 + 6.
    // The report has a trinket before a pre-combat power in 14 fights, e.g.
    // 2026-11-13_02_12_34 line 448549 then 448709.
    let mut a = board(&db, &["Microbot"]);
    a[0].attack = 3;
    a[0].health = 20;
    let b = vec![wall(&db, 0, 1000)];
    let mut c = [Counters::default(); 2];
    c[0].trinkets.push(
        &FightEntry::new(
            When::Soc,
            FightAct::Stats { pick: Pick::All, attack: 2, health: 0, grow: 0 },
        ),
        0,
        0,
    );
    c[0].all_will_burn = 1;
    c[0].wingmen = 1;
    let mut log = Log::new(&db);
    fight_with(&db, &a, &b, c, 1, &mut Rng::new(3), &mut log);
    let Event::Attack(side, name, attack, _) = log.events[0].clone() else {
        panic!("the first event is Wingmen's attack: {:?}", log.events);
    };
    assert_eq!((side, name.as_str(), attack), (0, "Microbot", 9));

    // ALL Will Burn (+2) then Wingmen (+2) make 7. The 10-Health enemy takes
    // 7 from Wingmen, then Lightning Invocation's 3, and dies in Start of
    // Combat. The report's Wingmen before Lightning case is
    // 2026-11-17_18_04_17 line 1115517 then 1116722.
    let mut a = board(&db, &["Microbot"]);
    a[0].attack = 3;
    a[0].health = 10;
    let mut b = board(&db, &["Microbot"]);
    b[0].attack = 0;
    b[0].health = 10;
    let mut c = [Counters::default(); 2];
    c[0].all_will_burn = 1;
    c[0].wingmen = 1;
    c[0].soc_lightning = 1;
    c[0].soc_lightning_damage = 3;
    let out = fight_with(&db, &a, &b, c, 5, &mut Rng::new(4), &mut ());
    assert_eq!(out.attacks, 0, "the enemy died during Start of Combat");

    // Wingmen's attack lands before Boon of Beetles summons. In the report's
    // 2026-11-17_18_04_17 fight Lightning is followed by Upper Hand and then
    // Boon (lines 1116722, 1116850, 1116894).
    let a = vec![wall(&db, 0, 50)];
    let b = vec![wall(&db, 0, 1000)];
    let mut c = [Counters::default(); 2];
    c[0].wingmen = 1;
    c[0].boon_of_beetles = 1;
    let mut log = Log::new(&db);
    fight_with(&db, &a, &b, c, 1, &mut Rng::new(5), &mut log);
    assert!(
        matches!(log.events.first(), Some(Event::Attack(0, _, _, _))),
        "Wingmen first, Boon later: {:?}",
        log.events
    );
    assert!(
        log.events.iter().any(|e| matches!(e, Event::Summon(0, n) if n == "Beetle")),
        "Boon still fires in the spell phase: {:?}",
        log.events
    );

    // Spells (Boon) before the hand (Flighty Scout) before the board
    // (Diremuck Forager). The report's hand case is 2026-11-21_02_14_56
    // lines 776967 and 777010.
    let mut a = board(&db, &["Microbot", "Diremuck Forager"]);
    a[0].health = 50;
    let b = vec![wall(&db, 0, 200)];
    let hand = vec![
        db.make(db.by_name("Flighty Scout").unwrap(), false),
        db.make(db.by_name("Bream Counter").unwrap(), false),
    ];
    let mut c = [Counters::default(); 2];
    c[0].boon_of_beetles = 1;
    let mut log = Log::new(&db);
    fight_with_hands(&db, &a, &b, [&hand, &[]], c, 0, &mut Rng::new(6), &mut log);
    let summons: Vec<&str> = log
        .events
        .iter()
        .filter_map(|e| match e {
            Event::Summon(0, n) => Some(n.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(summons, ["Beetle", "Flighty Scout", "Bream Counter"]);
}

/// Change 2: deaths and summons-when-space resolve after the pre-combat hero
/// powers, before Wingmen. Teron destroys its marked minion and brings the
/// exact copy back on the same side of Wingmen's attack.
#[test]
fn pre_combat_deaths_and_summons_resolve_before_wingmen() {
    let db = cards();
    let mut marked = db.make(db.by_name("Microbot").unwrap(), false);
    marked.reanimate = true;
    marked.attack = 3;
    marked.health = 5;
    let mut other = db.make(db.by_name("Microbot").unwrap(), false);
    other.attack = 1;
    other.health = 20;
    let a = vec![marked, other];
    let b = vec![wall(&db, 0, 1000)];
    let mut c = [Counters::default(); 2];
    c[0].rapid_reanimation = 1;
    c[0].wingmen = 1;
    let mut log = Log::new(&db);
    fight_with(&db, &a, &b, c, 1, &mut Rng::new(7), &mut log);

    let death = log
        .events
        .iter()
        .position(|e| matches!(e, Event::Death(0, n, 3) if n == "Microbot"))
        .expect("Teron destroys the marked minion");
    let copy = log
        .events
        .iter()
        .position(|e| matches!(e, Event::Summon(0, n) if n == "Microbot"))
        .expect("the exact copy comes back");
    let attack = log
        .events
        .iter()
        .position(|e| matches!(e, Event::Attack(0, _, _, _)))
        .expect("Wingmen attacks");
    assert!(death < attack, "the death is before Wingmen: {:?}", log.events);
    assert!(copy < attack, "the resummon is before Wingmen: {:?}", log.events);
}

/// Change 3: each phase coin-flips which player goes first. The report's
/// both-sided cases show both orders: trinkets 3 Era and 2 opponent,
/// pre-combat powers 2 opponent, other powers 1/1, spells 1 Era and 2
/// opponent.
#[test]
fn every_phase_coin_flips_the_first_player() {
    let db = cards();

    let mut trinket_first = [false; 2];
    let mut pre_first = [false; 2];
    let mut other_first = [false; 2];
    let mut spell_first = [false; 2];
    let mut hand_first = [false; 2];
    for seed in 0..64 {
        let a = vec![wall(&db, 0, 1000)];
        let b = vec![wall(&db, 0, 1000)];

        // Trinkets: one Beetle each.
        let mut c = [Counters::default(); 2];
        c[0] = trinket_summon(&db, "Beetle");
        c[1] = trinket_summon(&db, "Beetle");
        let mut log = Log::new(&db);
        fight_with(&db, &a, &b, c, 0, &mut Rng::new(seed), &mut log);
        if let Some(side) = log.first_summon_side() {
            trinket_first[side] = true;
        }

        // Pre-combat hero powers: one Embrace Your Rage summon each.
        let mut c = [Counters::default(); 2];
        for side in &mut c {
            side.tier = 1;
            side.embrace_rage = 1;
        }
        let mut log = Log::new(&db);
        fight_with(&db, &a, &b, c, 0, &mut Rng::new(seed), &mut log);
        if let Some(side) = log.first_summon_side() {
            pre_first[side] = true;
        }

        // Other hero powers: Lightning Invocation each kills the other
        // player's 1-Health minion. The first death is on the victim's side,
        // so it names the caster of the first Invocation.
        let a1 = vec![wall(&db, 0, 1)];
        let b1 = vec![wall(&db, 0, 1)];
        let mut c = [Counters::default(); 2];
        for side in &mut c {
            side.soc_lightning = 1;
            side.soc_lightning_damage = 1;
        }
        let mut log = Log::new(&db);
        fight_with(&db, &a1, &b1, c, 0, &mut Rng::new(seed), &mut log);
        if let Some(Event::Death(side, _, _)) = log.events.first() {
            other_first[1 - *side] = true;
        }

        // Spells: one Boon of Beetles each.
        let mut c = [Counters::default(); 2];
        for side in &mut c {
            side.boon_of_beetles = 1;
        }
        let mut log = Log::new(&db);
        fight_with(&db, &a, &b, c, 0, &mut Rng::new(seed), &mut log);
        if let Some(side) = log.first_summon_side() {
            spell_first[side] = true;
        }

        // Hand: one Flighty Scout each.
        let hand = vec![db.make(db.by_name("Flighty Scout").unwrap(), false)];
        let mut log = Log::new(&db);
        fight_with_hands(
            &db,
            &a,
            &b,
            [&hand, &hand],
            [Counters::default(); 2],
            0,
            &mut Rng::new(seed),
            &mut log,
        );
        if let Some(side) = log.first_summon_side() {
            hand_first[side] = true;
        }
    }
    assert_eq!(trinket_first, [true, true], "trinket coin");
    assert_eq!(pre_first, [true, true], "pre-combat hero power coin");
    assert_eq!(other_first, [true, true], "other hero power coin");
    assert_eq!(spell_first, [true, true], "spell coin");
    assert_eq!(hand_first, [true, true], "hand coin");
}

/// Change 4: the board queue starts on the side that would attack first at
/// that moment (the larger board), not a fresh coin. Side 0 has one more
/// minion, so its Diremuck Forager always takes the first board turn.
#[test]
fn board_pass_starts_on_the_first_attacker() {
    let db = cards();
    let hand = vec![db.make(db.by_name("Bream Counter").unwrap(), false)];
    for seed in 0..48 {
        let a = board(&db, &["Diremuck Forager", "Microbot", "Microbot"]);
        let b = board(&db, &["Diremuck Forager", "Microbot"]);
        let mut log = Log::new(&db);
        fight_with_hands(
            &db,
            &a,
            &b,
            [&hand, &hand],
            [Counters::default(); 2],
            0,
            &mut Rng::new(seed),
            &mut log,
        );
        assert_eq!(
            log.first_summon_side(),
            Some(0),
            "seed {seed}: three minions against two starts the board phase"
        );
    }
}

/// Change 5: a board effect that changes a count changes the fight's first
/// attacker; the last recount is the one the fight uses. Stitched Salvager
/// takes side 0 from three minions to two, so side 1 attacks first.
#[test]
fn a_board_count_change_is_the_last_recount() {
    let db = cards();
    for seed in 0..32 {
        let a = board(&db, &["Microbot", "Stitched Salvager", "Microbot"]);
        let b = board(&db, &["Microbot", "Microbot", "Microbot"]);
        let mut log = Log::new(&db);
        fight_with(&db, &a, &b, [Counters::default(); 2], 1, &mut Rng::new(seed), &mut log);
        assert_eq!(
            log.first_attack_side(),
            Some(1),
            "seed {seed}: side 0 is smaller after Salvager, so side 1 attacks first"
        );
        assert!(
            log.events.iter().rev().any(|e| matches!(e, Event::Recount(1))),
            "seed {seed}: the count after the board effect is the last recount"
        );
    }
}

/// Change 6: the first attacker is counted from the boards as they entered
/// combat. Side 0 has four minions to three and Wingmen, then two Stitched
/// Salvagers take it down to two. The logs lean to the lock
/// (2026-11-20_01_10_46-g6 t8 and 2026-10-28_19_47_59-g3 t11), and Firestone
/// cites Mitchell and two replays at soc-illidan-hero-power.ts:114-123.
#[test]
fn wingmen_locks_the_entry_first_attacker() {
    let db = cards();
    for seed in 0..24 {
        let mut a =
            board(&db, &["Microbot", "Stitched Salvager", "Microbot", "Stitched Salvager"]);
        for m in &mut a {
            m.health = 1000;
        }
        let b = vec![wall(&db, 1, 1000), wall(&db, 1, 1000), wall(&db, 1, 1000)];
        let mut c = [Counters::default(); 2];
        c[0].wingmen = 1;
        let mut log = Log::new(&db);
        fight_with(&db, &a, &b, c, 3, &mut Rng::new(seed), &mut log);
        let attacks: Vec<usize> = log
            .events
            .iter()
            .filter_map(|e| match e {
                Event::Attack(side, _, _, _) => Some(*side),
                _ => None,
            })
            .collect();
        // The first two are Wingmen's own attacks (left then right). The
        // next one is the fight's first swing.
        assert!(attacks.len() > 2, "seed {seed}: a fight happened: {attacks:?}");
        assert_eq!(attacks[2], 0, "seed {seed}: Wingmen's side keeps the first swing");
    }
}

/// Change 7: the board queue is snapshotted before the hand phase, so a
/// Flighty Scout copy summoned from hand does not also take a board turn.
#[test]
fn a_scout_from_hand_does_not_take_a_board_turn() {
    let db = cards();
    let b = vec![wall(&db, 0, 1000)];
    let hand = vec![db.make(db.by_name("Flighty Scout").unwrap(), false)];
    let mut log = Log::new(&db);
    fight_with_hands(
        &db,
        &[],
        &b,
        [&hand, &[]],
        [Counters::default(); 2],
        1,
        &mut Rng::new(8),
        &mut log,
    );
    assert!(
        log.events.iter().any(|e| matches!(e, Event::Summon(0, n) if n == "Flighty Scout")),
        "the hand copy is summoned: {:?}",
        log.events
    );
    assert_eq!(
        log.events.iter().find_map(|e| match e {
            Event::Soc(_, name) if name == "Flighty Scout" => Some(name.as_str()),
            _ => None,
        }),
        None,
        "the copy is not in the board queue: {:?}",
        log.events
    );
}
