//! Tier 7: the engine side (the Tavern never offers it, the draw helpers) and
//! the seven tier 7 minions of the five-tribe lobby, with the real cards from
//! cards.toml: Polarizing Beatboxer, Champion of Sargeras, Obsidian Ravager,
//! Futurefin, Highkeeper Ra, The Last One Standing and Stalwart Kodo.

use std::sync::Arc;

use bgcore::bots::random_action;
use bgcore::fight::{Watch, fight_with};
use bgcore::{Action, CardDb, CardId, Counters, Data, Game, Keywords, Minion, Rng, Tribe};

const TIER7: [&str; 7] = [
    "Polarizing Beatboxer",
    "Champion of Sargeras",
    "Obsidian Ravager",
    "Futurefin",
    "Highkeeper Ra",
    "The Last One Standing",
    "Stalwart Kodo",
];

fn db() -> CardDb {
    Data::bundled().cards
}

fn lobby(ts: &[Tribe]) -> bgcore::Tribes {
    ts.iter().copied().fold(bgcore::Tribes::NONE, |m, t| m.with(t))
}

fn game(seed: u64) -> Game {
    Game::new_with_lobby(Arc::new(Data::bundled()), seed, lobby(&[Tribe::Beast, Tribe::Mech, Tribe::Dragon, Tribe::Murloc, Tribe::Demon]))
}

/// A lobby with every tier 7 minion's tribe, so the tier 7 tests can give
/// them all out (Highkeeper Ra is neutral, The Last One Standing is all).
fn tier7_lobby() -> bgcore::Tribes {
    lobby(&[Tribe::Beast, Tribe::Mech, Tribe::Dragon, Tribe::Murloc, Tribe::Demon])
}

fn mk(db: &CardDb, name: &str, golden: bool) -> Minion {
    db.make(db.by_name(name).expect(name), golden)
}

fn id(db: &CardDb, name: &str) -> CardId {
    db.by_name(name).expect(name)
}

/// A minion that never attacks and takes a long time to kill.
fn wall(db: &CardDb) -> Minion {
    let mut w = mk(db, "Microbot", false);
    w.attack = 0;
    w.health = 1000;
    w
}

#[derive(Debug, PartialEq, Eq, Clone)]
enum Ev {
    /// side, attacker name, attacker attack, defender name, defender health
    Attack(usize, String, i32, String, i32),
    /// side, name, attack, health at the moment it is summoned
    Summon(usize, String, i32, i32),
    /// side, name
    Death(usize, String),
}

struct Log {
    db: CardDb,
    ev: Vec<Ev>,
}

impl Log {
    fn new(db: &CardDb) -> Log {
        Log { db: db.clone(), ev: Vec::new() }
    }

    fn name(&self, m: &Minion) -> String {
        self.db.get(m.card).name.clone()
    }

    fn attacks(&self) -> Vec<&Ev> {
        self.ev.iter().filter(|e| matches!(e, Ev::Attack(..))).collect()
    }

    fn deaths(&self, side: usize, name: &str) -> usize {
        self.ev.iter().filter(|e| **e == Ev::Death(side, name.to_string())).count()
    }

    fn summons(&self, side: usize, name: &str) -> Vec<(i32, i32)> {
        self.ev
            .iter()
            .filter_map(|e| match e {
                Ev::Summon(s, n, a, h) if *s == side && n == name => Some((*a, *h)),
                _ => None,
            })
            .collect()
    }
}

impl Watch for Log {
    fn attack(&mut self, side: usize, a: &Minion, d: &Minion) {
        let e = Ev::Attack(side, self.name(a), a.attack, self.name(d), d.health);
        self.ev.push(e);
    }
    fn death(&mut self, side: usize, _pos: usize, m: &Minion) {
        let e = Ev::Death(side, self.name(m));
        self.ev.push(e);
    }
    fn summon(&mut self, side: usize, _pos: usize, m: &Minion) {
        let e = Ev::Summon(side, self.name(m), m.attack, m.health);
        self.ev.push(e);
    }
}

fn run(
    db: &CardDb,
    a: &[Minion],
    b: &[Minion],
    max_attacks: u32,
    seed: u64,
) -> (bgcore::Outcome, Log) {
    let mut log = Log::new(db);
    let out = fight_with(
        db,
        a,
        b,
        [Counters::default(); 2],
        max_attacks,
        &mut Rng::new(seed),
        &mut log,
    );
    (out, log)
}

// ---- game helpers ----

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

/// A card from the pool straight onto the board; golden takes three copies.
fn put_on_board(g: &mut Game, p: usize, name: &str, golden: bool) {
    let c = id(&g.data.cards, name);
    let slot = g.data.cards.get(c).pool_slot as usize;
    let copies = if golden { 3 } else { 1 };
    g.pool[slot] -= copies;
    let m = g.players[p].make_card(&g.data.cards, c, golden, copies);
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

fn stats(m: &Minion) -> (i32, i32) {
    (m.attack, m.health)
}

// ================= tier 7 in the engine =================

#[test]
fn the_seven_tier_7_minions_are_in_the_pool_with_tier_6_copies() {
    let data = Data::bundled();
    // a lobby with every tier 7 minion's tribe (Mech, Demon, Dragon, Murloc,
    // Beast; Highkeeper Ra is neutral and The Last One Standing is all)
    let lobby = tier7_lobby();
    let g = Game::new_with_lobby(Arc::new(Data::bundled()), 1, lobby);
    let tier6 = data.rules.copies(6);
    assert_eq!(data.rules.copies(7), tier6);
    for n in TIER7 {
        let c = id(&data.cards, n);
        assert_eq!(data.cards.get(c).tier, 7, "{n}");
        assert!((c as usize) < data.cards.pool_cards, "{n} is a pool minion");
        assert_eq!(g.pool[c as usize], tier6, "{n} starts with tier 6's copy count");
    }
    g.check_invariants().unwrap();
}

#[test]
fn the_tavern_never_shows_tier_7_and_you_cannot_level_to_it() {
    let mut g = game(5);
    g.players[0].tier = 6;
    g.players[0].gold = 10;
    turn_of(&mut g, 0);
    assert!(!g.is_legal(Action::Level), "no leveling past tier 6");
    for _ in 0..40 {
        if g.players[0].gold == 0 {
            g.players[0].gold = 10;
        }
        act(&mut g, 0, Action::Roll);
        for m in &g.players[0].shop {
            assert!(g.data.cards.get(m.card).tier <= 6, "{} in the Tavern", name(&g, m));
        }
    }
    g.check_invariants().unwrap();
}

#[test]
fn no_tier_7_minion_shows_up_in_games_played_at_random() {
    let data = Arc::new(Data::bundled());
    for seed in 0..30 {
        let mut g = Game::new(data.clone(), seed);
        let mut rng = Rng::new(seed ^ 99);
        while let Some(_p) = g.current_player() {
            g.step(random_action(&g, &mut rng)).unwrap();
            for pl in &g.players {
                // A tier 7 minion can reach a hand or a board only through a
                // card that names tier 7 (Greedy Conniver, Pagle's Fishing
                // Rod), never from the Tavern.
                for m in &pl.shop {
                    assert!(g.data.cards.get(m.card).tier <= 6, "seed {seed}: {}", name(&g, m));
                }
                // A tier 7 option is fine only when the whole Discover is
                // tier 7, which is a card that names tier 7 (Greedy
                // Conniver); every other draw stops at tier 6.
                let offered: Vec<u8> = pl.discover.iter().map(|&c| g.data.cards.get(c).tier).collect();
                for &tier in &offered {
                    assert!(
                        tier <= 6 || offered.iter().all(|&t| t == 7),
                        "seed {seed}: a Discover offered tier {tier} beside {offered:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn a_triple_at_tier_6_discovers_tier_6_not_tier_7() {
    for seed in 0..6 {
        let mut g = game(seed);
        g.players[0].tier = 6;
        g.players[0].gold = 10;
        put_in_hand(&mut g, 0, "Wolf Pup", false);
        put_in_hand(&mut g, 0, "Wolf Pup", false);
        set_shop(&mut g, 0, &["Wolf Pup", "Cord Puller", "Cord Puller"]);
        act(&mut g, 0, Action::Buy(0));
        act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
        // the reward spell's tier was fixed at play time: tier 6, not 7
        let p = &g.players[0];
        assert_eq!(p.hand[0].improvements, 6);
        act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
        let p = &g.players[0];
        assert_eq!(p.discover.len(), 3);
        assert!(p.discover.iter().all(|&c| g.data.cards.get(c).tier == 6), "seed {seed}");
        g.check_invariants().unwrap();
    }
}

#[test]
fn random_draws_up_to_your_tier_never_reach_tier_7() {
    let db = db();
    let mut rng = Rng::new(3);
    for _ in 0..3000 {
        for t in Tribe::EVERY {
            let c = db.random_pool_minion(t, 0, bgcore::Tribes::ALL, &mut rng).unwrap();
            assert!(db.get(c).tier <= 6);
        }
        let c = db.random_deathrattle_minion(0, bgcore::Tribes::ALL, &mut rng).unwrap();
        assert!(db.get(c).tier <= 6);
        let tribes = db.get(id(&db, "Wolf Pup")).tribes;
        let c = db.random_different_minion(tribes, 0, 0, bgcore::Tribes::ALL, &mut rng).unwrap();
        assert!(db.get(c).tier <= 6);
    }
    // And the tier 6 draw Highkeeper Ra uses is exactly tier 6.
    for _ in 0..500 {
        let c = db.random_tier_minion(6, bgcore::Tribes::ALL, &mut rng).unwrap();
        assert_eq!(db.get(c).tier, 6);
    }
}

#[test]
fn the_tier_7_draw_helpers() {
    let mut g = game(11);
    let before = g.pool.clone();
    // Get a Tier 7 minion: generated, so the pool doesn't change.
    for _ in 0..6 {
        assert!(g.give_tier7_minion(0));
    }
    let hand: Vec<Minion> = g.players[0].hand.to_vec();
    assert!(!hand.is_empty());
    assert!(hand.iter().all(|m| g.data.cards.get(m.card).tier == 7 && m.pool_copies == 0));
    assert_eq!(g.pool, before);
    g.check_invariants().unwrap();
    // a full hand takes nothing
    while !g.players[0].hand.is_full() {
        let m = g.players[0].make_card(&g.data.cards, id(&g.data.cards, "Microbot"), false, 0);
        g.players[0].hand.push(m);
    }
    assert!(!g.give_tier7_minion(0));

    // Discover a Tier 7 minion: three different ones from the pool; the pick
    // takes its copy and the other two go back.
    let mut g = game(12);
    g.discover_tier7_minion(0);
    let offered: Vec<CardId> = g.players[0].discover.to_vec();
    assert_eq!(offered.len(), 3);
    assert!(offered.iter().all(|&c| g.data.cards.get(c).tier == 7));
    let mut uniq = offered.clone();
    uniq.sort();
    uniq.dedup();
    assert_eq!(uniq.len(), 3);
    g.check_invariants().unwrap();
    let before = g.pool.clone();
    act(&mut g, 0, Action::Choose(1));
    assert_eq!(g.players[0].hand[0].card, offered[1]);
    assert_eq!(g.players[0].hand[0].pool_copies, 1);
    for &c in &offered {
        let back = if c == offered[1] { 0 } else { 1 };
        assert_eq!(g.pool[c as usize], before[c as usize] + back);
    }
    g.check_invariants().unwrap();
}

#[test]
fn robust_evolution_never_takes_a_minion_up_to_tier_7() {
    for seed in 0..12 {
        let mut g = game(seed);
        put_on_board(&mut g, 0, "Deathstrider", false);
        g.give(0, "Robust Evolution").unwrap();
        act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
        act(&mut g, 0, Action::Target(0));
        let evolved = g.players[0].board[0];
        assert_eq!(g.data.cards.get(evolved.card).tier, 6, "seed {seed}");
        g.check_invariants().unwrap();
    }
}

#[test]
fn games_with_tier_7_minions_in_play_keep_the_pool_exact() {
    let data = Arc::new(Data::bundled());
    for seed in 0..120 {
        let mut g = Game::new_with_lobby(data.clone(), seed, tier7_lobby());
        for p in 0..bgcore::PLAYERS {
            for (k, n) in TIER7.iter().enumerate() {
                if (k + p + seed as usize).is_multiple_of(2) {
                    g.give(p, n).unwrap();
                }
            }
        }
        let mut rng = Rng::new(seed ^ 0xabc);
        let mut steps = 0;
        while g.current_player().is_some() {
            g.step(random_action(&g, &mut rng)).unwrap();
            g.check_invariants().unwrap_or_else(|e| panic!("seed {seed} step {steps}: {e}"));
            steps += 1;
        }
    }
}

// ================= Rally timing =================

#[test]
fn every_rally_resolves_before_the_attack_lands() {
    let db = db();
    // Heroic Underdog gains its target's Attack before the hit: the attack
    // event already shows the gain.
    let mut target = wall(&db);
    target.attack = 5;
    let (_, log) = run(&db, &[mk(&db, "Heroic Underdog", false)], &[target], 1, 1);
    assert_eq!(
        log.attacks()[0],
        &Ev::Attack(0, "Heroic Underdog".into(), 1 + 5, "Microbot".into(), 1000)
    );
    // Flittering Bat's Foraging Bat is on the board before the attack.
    let (_, log) = run(&db, &[mk(&db, "Flittering Bat", false)], &[wall(&db)], 1, 1);
    let summon = log.ev.iter().position(|e| matches!(e, Ev::Summon(0, ..))).unwrap();
    let attack = log.ev.iter().position(|e| matches!(e, Ev::Attack(..))).unwrap();
    assert!(summon < attack, "the Rally summon comes first");
}

// ================= Obsidian Ravager =================

fn taunt(mut m: Minion, attack: i32, health: i32) -> Minion {
    m.attack = attack;
    m.health = health;
    m.keywords.clear(Keywords::DIVINE_SHIELD);
    m.keywords.set(Keywords::TAUNT);
    m
}

fn plain(mut m: Minion, attack: i32, health: i32) -> Minion {
    m.attack = attack;
    m.health = health;
    m.keywords.clear(Keywords::DIVINE_SHIELD);
    m.keywords.clear(Keywords::TAUNT);
    m
}

#[test]
fn ravager_hits_the_target_before_the_attack_lands_so_it_takes_its_attack_twice() {
    let db = db();
    let rav = mk(&db, "Obsidian Ravager", false);
    let enemies = [
        plain(wall(&db), 0, 100),
        taunt(wall(&db), 0, 14),
        plain(wall(&db), 0, 100),
    ];
    let (_, log) = run(&db, &[rav], &enemies, 1, 3);
    // The attack event comes after the Rally: the target is already at 14 - 7.
    assert_eq!(
        log.attacks()[0],
        &Ev::Attack(0, "Obsidian Ravager".into(), 7, "Microbot".into(), 7)
    );
    // 7 from the Rally and 7 from the attack kill a 14 health target.
    assert_eq!(log.deaths(1, "Microbot"), 1);
    // The attacker took the target's (zero) damage and lives.
    assert_eq!(log.deaths(0, "Obsidian Ravager"), 0);
}

#[test]
fn ravager_hits_one_random_neighbour_and_golden_hits_both() {
    let db = db();
    let enemies = || {
        [
            plain(wall(&db), 0, 7),
            taunt(wall(&db), 0, 100),
            plain(wall(&db), 0, 7),
        ]
    };
    let mut hit_left = false;
    let mut hit_right = false;
    for seed in 0..30 {
        let (_, log) = run(&db, &[mk(&db, "Obsidian Ravager", false)], &enemies(), 1, seed);
        let dead = log.deaths(1, "Microbot");
        assert_eq!(dead, 1, "exactly one neighbour takes the 7 (seed {seed})");
        // which one: the survivor still stands, so check by the board order
        // through a second fight where the left one is stronger
        let mut left_tough = enemies();
        left_tough[0].health = 1000;
        let (_, log2) = run(&db, &[mk(&db, "Obsidian Ravager", false)], &left_tough, 1, seed);
        if log2.deaths(1, "Microbot") == 1 {
            hit_right = true;
        } else {
            hit_left = true;
        }
    }
    assert!(hit_left && hit_right, "the neighbour is picked at random");
    for seed in 0..5 {
        let (_, log) = run(&db, &[mk(&db, "Obsidian Ravager", true)], &enemies(), 1, seed);
        // the golden Ravager has 14 Attack: the target takes 28 in all and
        // both neighbours die
        assert_eq!(log.deaths(1, "Microbot"), 2, "both neighbours (seed {seed})");
        assert_eq!(
            log.attacks()[0],
            &Ev::Attack(0, "Obsidian Ravager".into(), 14, "Microbot".into(), 100 - 14)
        );
    }
}

#[test]
fn ravager_next_to_the_edge_has_one_neighbour() {
    let db = db();
    let enemies = [taunt(wall(&db), 0, 100), plain(wall(&db), 0, 7)];
    for seed in 0..5 {
        let (_, log) = run(&db, &[mk(&db, "Obsidian Ravager", false)], &enemies, 1, seed);
        assert_eq!(log.deaths(1, "Microbot"), 1);
    }
}

#[test]
fn ravager_rally_pops_divine_shield_and_a_kill_makes_the_swing_fizzle() {
    let db = db();
    // A shielded target loses the shield to the Rally and then takes the hit.
    let mut shielded = taunt(wall(&db), 0, 7);
    shielded.keywords.set(Keywords::DIVINE_SHIELD);
    let (_, log) = run(&db, &[mk(&db, "Obsidian Ravager", false)], &[shielded, wall(&db)], 1, 1);
    assert_eq!(log.deaths(1, "Microbot"), 1, "the Rally pops the shield, the attack kills");
    assert_eq!(
        log.attacks()[0],
        &Ev::Attack(0, "Obsidian Ravager".into(), 7, "Microbot".into(), 7)
    );

    // A target the Rally kills never gets hit: no attack event, and it can't
    // hit back (this one would kill the Ravager).
    let killer = taunt(wall(&db), 50, 7);
    let board = [mk(&db, "Obsidian Ravager", false), wall(&db)];
    let (_, log) = run(&db, &board, &[killer], 1, 1);
    assert_eq!(log.deaths(1, "Microbot"), 1);
    assert_eq!(log.deaths(0, "Obsidian Ravager"), 0);
    assert!(log.attacks().is_empty(), "the swing fizzles");
}

#[test]
fn ravager_rally_with_venomous_kills_and_deathrattles_run() {
    let db = db();
    let mut rav = mk(&db, "Obsidian Ravager", false);
    rav.keywords.set(Keywords::VENOMOUS);
    let (_, log) = run(&db, &[rav], &[taunt(wall(&db), 0, 1000), wall(&db)], 1, 1);
    assert_eq!(log.deaths(1, "Microbot"), 1, "Venomous kills a 1000 health target");

    // A Buzzing Vermin the Rally kills summons its Beetle before the attack,
    // and the swing then fizzles.
    let vermin = mk(&db, "Buzzing Vermin", false);
    let board = [mk(&db, "Obsidian Ravager", false), wall(&db)];
    let (_, log) = run(&db, &board, &[vermin], 1, 1);
    assert_eq!(log.deaths(1, "Buzzing Vermin"), 1);
    assert_eq!(log.summons(1, "Beetle").len(), 1);
    assert!(log.attacks().is_empty());
}

// ================= Highkeeper Ra =================

#[test]
fn ra_battlecry_gets_a_tier_6_minion_golden_gets_two_full_hand_loses_it() {
    let mut g = game(31);
    put_in_hand(&mut g, 0, "Highkeeper Ra", false);
    let pool = g.pool.clone();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    let got: Vec<Minion> = g.players[0].hand.to_vec();
    assert_eq!(got.len(), 1);
    assert_eq!(g.data.cards.get(got[0].card).tier, 6);
    assert!(!got[0].golden);
    assert_eq!(g.pool, pool, "a generated card takes nothing from the pool");
    g.check_invariants().unwrap();

    let mut g = game(32);
    put_in_hand(&mut g, 0, "Highkeeper Ra", true);
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    // the golden play also puts a Triple Reward spell in hand (tier 0)
    let tier6 = g.players[0]
        .hand
        .iter()
        .filter(|m| g.data.cards.get(m.card).tier == 6)
        .count();
    assert_eq!(tier6, 2, "two tier 6 minions");
    g.check_invariants().unwrap();

    // Full hand: the extra card is lost.
    let mut g = game(33);
    put_in_hand(&mut g, 0, "Highkeeper Ra", true);
    for _ in 0..9 {
        let m = g.players[0].make_card(&g.data.cards, id(&g.data.cards, "Microbot"), false, 0);
        g.players[0].hand.push(m);
    }
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    assert_eq!(g.players[0].hand.len(), 10);
    // the one free slot goes to the Triple Reward spell (it comes before
    // anything the Battlecry adds), so both tier 6 minions are lost
    let rewards = g.players[0]
        .hand
        .iter()
        .filter(|m| g.data.cards.get(m.card).name == "Triple Reward")
        .count();
    assert_eq!(rewards, 1);
    let tier6 = g.players[0]
        .hand
        .iter()
        .filter(|m| g.data.cards.get(m.card).tier == 6)
        .count();
    assert_eq!(tier6, 0, "both found no room");
    g.check_invariants().unwrap();
}

#[test]
fn ra_deathrattle_and_every_rally_get_tier_6_minions() {
    let db = db();
    let tier = |out: &bgcore::Outcome| -> Vec<u8> {
        out.sides[0].cards_gained().iter().map(|&c| db.get(c).tier).collect()
    };
    // Rally: one card per attack, two for golden, and Windfury attacks twice.
    let (out, _) = run(&db, &[mk(&db, "Highkeeper Ra", false)], &[wall(&db)], 1, 1);
    assert_eq!(tier(&out), vec![6]);
    let (out, _) = run(&db, &[mk(&db, "Highkeeper Ra", true)], &[wall(&db)], 1, 1);
    assert_eq!(tier(&out), vec![6, 6]);
    let mut wf = mk(&db, "Highkeeper Ra", false);
    wf.keywords.set(Keywords::WINDFURY);
    let (out, _) = run(&db, &[wf], &[wall(&db)], 1, 1);
    assert_eq!(tier(&out), vec![6, 6], "one per swing");

    // Deathrattle: dies to the Taunt it can't beat, one card, two with Titus.
    let killer = |db: &CardDb| {
        let mut k = mk(db, "Microbot", false);
        k.attack = 50;
        k.health = 2000;
        k
    };
    let victim = |golden: bool| {
        let mut v = db.make(id(&db, "Highkeeper Ra"), golden);
        v.attack = 0; // never attacks, so no Rally cards
        v.health = 1;
        v.keywords.set(Keywords::TAUNT);
        v
    };
    // the enemy has one minion more, so it attacks first and hits the Taunt
    let fight = |board: &[Minion]| {
        let enemies: Vec<Minion> = (0..=board.len()).map(|_| killer(&db)).collect();
        fight_with(&db, board, &enemies, [Counters::default(); 2], 1, &mut Rng::new(7), &mut ())
    };
    assert_eq!(tier(&fight(&[victim(false)])), vec![6]);
    assert_eq!(tier(&fight(&[victim(true)])), vec![6, 6]);
    assert_eq!(tier(&fight(&[victim(false), mk(&db, "Titus Rivendare", false)])), vec![6, 6]);
}

// ================= Champion of Sargeras =================

#[test]
fn champion_battlecry_buffs_the_tavern_this_game() {
    let mut g = game(41);
    put_in_hand(&mut g, 0, "Champion of Sargeras", false);
    set_shop(&mut g, 0, &["Wolf Pup", "Cord Puller", "Cord Puller"]);
    let base: Vec<(i32, i32)> = g.players[0].shop.iter().map(stats).collect();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    let after: Vec<(i32, i32)> = g.players[0].shop.iter().map(stats).collect();
    for (b, a) in base.iter().zip(&after) {
        assert_eq!(*a, (b.0 + 8, b.1 + 8));
    }
    // Minions that show up later have it too: it lasts the game.
    act(&mut g, 0, Action::Roll);
    for m in &g.players[0].shop {
        let c = g.data.cards.get(m.card);
        if !c.is_spell() {
            assert_eq!(stats(m), (c.attack + 8, c.health + 8), "{}", c.name);
        }
    }
    g.check_invariants().unwrap();

    // Golden: +16/+16.
    let mut g = game(42);
    put_in_hand(&mut g, 0, "Champion of Sargeras", true);
    set_shop(&mut g, 0, &["Wolf Pup", "Cord Puller", "Cord Puller"]);
    let base: Vec<(i32, i32)> = g.players[0].shop.iter().map(stats).collect();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    for (b, m) in base.iter().zip(&g.players[0].shop) {
        assert_eq!(stats(m), (b.0 + 16, b.1 + 16));
    }
}

#[test]
fn champion_deathrattle_buffs_the_tavern_from_a_fight_golden_and_with_titus() {
    let db = db();
    let victim = |golden: bool| {
        let mut v = db.make(id(&db, "Champion of Sargeras"), golden);
        v.health = 1;
        v.keywords.set(Keywords::TAUNT);
        v
    };
    let mut killer = mk(&db, "Microbot", false);
    killer.attack = 50;
    killer.health = 2000;
    let enemies = [killer, killer];
    let gain = |board: &[Minion]| {
        fight_with(&db, board, &enemies, [Counters::default(); 2], 1, &mut Rng::new(7), &mut ())
            .sides[0]
            .tavern_buff_gain
    };
    assert_eq!(gain(&[victim(false)]), (8, 8));
    assert_eq!(gain(&[victim(true)]), (16, 16));
    assert_eq!(gain(&[victim(false), mk(&db, "Titus Rivendare", false)]), (16, 16));

    // And it reaches the Tavern after a real fight.
    let mut g = game(43);
    let Some(bgcore::Opponent::Player(q)) = g.players[0].opponent else { panic!() };
    put_on_board(&mut g, 0, "Champion of Sargeras", false);
    g.players[0].board[0].health = 1;
    put_on_board(&mut g, q as usize, "Cage Gnawer", false);
    g.players[q as usize].board[0].attack = 50;
    g.players[q as usize].board[0].health = 2000;
    while g.turn == 1 {
        g.step(Action::EndTurn).unwrap();
    }
    assert!(g.players[0].tavern_buffs.iter().any(|b| (b.attack, b.health) == (8, 8) && !b.this_turn));
}

// ================= Futurefin =================

fn end_turn_of(g: &mut Game, p: usize) {
    act(g, p, Action::EndTurn);
}

#[test]
fn futurefin_gives_its_stats_to_the_leftmost_minion_in_hand() {
    let mut g = game(51);
    put_on_board(&mut g, 0, "Futurefin", false);
    // a spell first, then two minions: the left-most MINION gets it
    g.give(0, "Sludge Corrosion").unwrap();
    g.give(0, "Wolf Pup").unwrap();
    g.give(0, "Cord Puller").unwrap();
    let pup = stats(&g.players[0].hand[1]);
    let bot = stats(&g.players[0].hand[2]);
    end_turn_of(&mut g, 0);
    let hand = &g.players[0].hand;
    assert_eq!(stats(&hand[1]), (pup.0 + 7, pup.1 + 13));
    assert_eq!(stats(&hand[2]), bot, "only the left-most one");
    // it's a copy: Futurefin keeps its stats
    assert_eq!(stats(&g.players[0].board[0]), (7, 13));
    g.check_invariants().unwrap();

    // Golden Futurefin (14/26) gives double: 28/52.
    let mut g = game(52);
    put_on_board(&mut g, 0, "Futurefin", true);
    g.give(0, "Wolf Pup").unwrap();
    let pup = stats(&g.players[0].hand[0]);
    end_turn_of(&mut g, 0);
    assert_eq!(stats(&g.players[0].hand[0]), (pup.0 + 28, pup.1 + 52));
    assert_eq!(stats(&g.players[0].board[0]), (14, 26));

    // Drakkari Enchanter doubles the end of turn.
    let mut g = game(53);
    put_on_board(&mut g, 0, "Futurefin", false);
    put_on_board(&mut g, 0, "Drakkari Enchanter", false);
    g.give(0, "Wolf Pup").unwrap();
    let pup = stats(&g.players[0].hand[0]);
    end_turn_of(&mut g, 0);
    assert_eq!(stats(&g.players[0].hand[0]), (pup.0 + 14, pup.1 + 26));

    // No minion in hand: nothing happens, spells are skipped.
    let mut g = game(54);
    put_on_board(&mut g, 0, "Futurefin", false);
    g.give(0, "Sludge Corrosion").unwrap();
    let spell = g.players[0].hand[0];
    end_turn_of(&mut g, 0);
    assert_eq!(g.players[0].hand[0], spell);
    g.check_invariants().unwrap();
}

// ================= The Last One Standing =================

fn los_board(db: &CardDb) -> Vec<Minion> {
    // slot 0 is the Last One Standing, the rest one minion per type; Foraging
    // Bat is a Beast, Microbot a Mech, Amber Guardian a Dragon, Magicfin
    // Mycologist a Murloc and Demon Fodder a Demon
    vec![
        mk(db, "The Last One Standing", false),
        mk(db, "Foraging Bat", false),
        mk(db, "Microbot", false),
        mk(db, "Amber Guardian", false),
        mk(db, "Magicfin Mycologist", false),
        mk(db, "Demon Fodder", false),
    ]
}

#[test]
fn last_one_standing_buffs_one_minion_of_each_type_once_and_can_pick_itself() {
    let db = db();
    // Pirate is a type too, and the Last One Standing is the only Pirate
    // here: it is always among the picks (as its own Pirate pick, or earlier
    // for another type), so it is buffed once and five or six minions are.
    // An all-type minion is a type of its own ("all"), so the Last One
    // Standing takes that pick and every other minion takes its own: all six
    // are buffed, once each (the log of a Misplaced Tea Set with a Gatekeeper
    // Amalgam on the board, 15_25_26-g1 turn 10 step 24, shows the same rule).
    for seed in 0..300 {
        let (out, _) = run(&db, &los_board(&db), &[wall(&db)], 1, seed);
        let perm = &out.sides[0].permanent_stats;
        let buffed: Vec<usize> = (0..6).filter(|&i| perm[i] != (0, 0)).collect();
        assert_eq!(buffed.len(), 6, "seed {seed}: {perm:?}");
        assert!(buffed.iter().all(|&i| perm[i] == (15, 15)), "seed {seed}: {perm:?}");
    }

    // Alone it is every type but one minion: one buff, not five.
    let (out, _) = run(&db, &[mk(&db, "The Last One Standing", false)], &[wall(&db)], 1, 1);
    assert_eq!(out.sides[0].permanent_stats[0], (15, 15));
    // Golden does the whole thing twice (with a fresh pick each time).
    let (out, _) = run(&db, &[mk(&db, "The Last One Standing", true)], &[wall(&db)], 1, 1);
    assert_eq!(out.sides[0].permanent_stats[0], (30, 30));
    let (out, _) = run(&db, &los_board_golden(&db), &[wall(&db)], 1, 2);
    let total: i32 = out.sides[0].permanent_stats.iter().map(|s| s.0).sum();
    assert!((10 * 15..=12 * 15).contains(&total), "two passes of five or six buffs: {total}");
}

fn los_board_golden(db: &CardDb) -> Vec<Minion> {
    let mut b = los_board(db);
    b[0] = mk(db, "The Last One Standing", true);
    b
}

#[test]
fn last_one_standing_triggers_before_the_attack_and_the_gain_is_kept() {
    let db = db();
    // Alone: the buff comes first, so the attack event already shows it.
    let (_, log) = run(&db, &[mk(&db, "The Last One Standing", false)], &[wall(&db)], 1, 1);
    assert_eq!(
        log.attacks()[0],
        &Ev::Attack(0, "The Last One Standing".into(), 30, "Microbot".into(), 1000)
    );

    // After a real fight the stats are on the board.
    let mut g = game(61);
    let Some(bgcore::Opponent::Player(q)) = g.players[0].opponent else { panic!() };
    put_on_board(&mut g, 0, "The Last One Standing", false);
    put_on_board(&mut g, q as usize, "Cage Gnawer", false);
    g.players[q as usize].board[0].attack = 1;
    g.players[q as usize].board[0].health = 2000;
    while g.turn == 1 {
        g.step(Action::EndTurn).unwrap();
    }
    let los = g.players[0].board[0];
    assert!(los.attack >= 30 && los.health >= 30, "kept: {:?}", stats(&los));
    g.check_invariants().unwrap();
}

#[test]
fn last_one_standing_rally_works_in_the_shop_through_sky_hatch_runaway() {
    let mut g = game(62);
    g.players[0].gold = 10;
    put_on_board(&mut g, 0, "The Last One Standing", false);
    put_on_board(&mut g, 0, "Sky-hatch Runaway", false);
    act(&mut g, 0, Action::Activate(1));
    // the Last One Standing is the only Rally minion, so it is picked at once
    assert!(g.players[0].pending_target.is_none());
    let b = &g.players[0].board;
    // the Runaway is a Dragon: it can only be picked for that one type
    let gained = (b[0].attack - 15) + (b[1].attack - 4);
    assert!(gained >= 15 && (gained % 15 == 0), "buffed in 15s: {gained}");
    assert!(b[0].attack - 15 <= 15, "no minion gets two buffs");
    assert!(b[1].attack - 4 <= 15, "no minion gets two buffs");
    g.check_invariants().unwrap();
}

// ================= Stalwart Kodo =================

/// Boom-in-a-Box deals 3 to everything else at the start of combat, which
/// kills 1/1 Buzzing Vermin and damages the Kodo.
fn kodo_board(db: &CardDb, golden: bool, vermin: usize) -> Vec<Minion> {
    let mut b = vec![mk(db, "Stalwart Kodo", golden), mk(db, "Boom-in-a-Box", false)];
    for _ in 0..vermin {
        let mut v = mk(db, "Buzzing Vermin", false);
        v.keywords.clear(Keywords::TAUNT);
        b.push(v);
    }
    b
}

fn walls(db: &CardDb, n: usize) -> Vec<Minion> {
    (0..n).map(|_| wall(db)).collect()
}

#[test]
fn kodo_gives_summons_its_attack_and_maximum_health() {
    let db = db();
    // Kodo is hit for 3 by Boom, so its Health is 29 but its maximum stays 32.
    let (_, log) = run(&db, &kodo_board(&db, false, 1), &walls(&db, 4), 1, 1);
    // Beetle is 2/2: +16 Attack, +32 maximum Health (not 29)
    assert_eq!(log.summons(0, "Beetle"), vec![(2 + 16, 2 + 32)]);

    // Golden Kodo (32/64) gives double: +64/+128.
    let (_, log) = run(&db, &kodo_board(&db, true, 1), &walls(&db, 4), 1, 1);
    assert_eq!(log.summons(0, "Beetle"), vec![(2 + 64, 2 + 128)]);
}

#[test]
fn kodo_stops_after_three_summons_per_combat() {
    let db = db();
    let (_, log) = run(&db, &kodo_board(&db, false, 5), &walls(&db, 8), 1, 1);
    let beetles = log.summons(0, "Beetle");
    assert_eq!(beetles.len(), 5);
    assert_eq!(&beetles[..3], &[(18, 34); 3], "the first three get the gift");
    assert_eq!(&beetles[3..], &[(2, 2); 2], "then Kodo is done");

    // Two Kodos have three gifts each, and each one gives its own stats.
    let mut board = vec![mk(&db, "Stalwart Kodo", false), mk(&db, "Stalwart Kodo", false)];
    board.push(mk(&db, "Boom-in-a-Box", false));
    for _ in 0..4 {
        let mut v = mk(&db, "Buzzing Vermin", false);
        v.keywords.clear(Keywords::TAUNT);
        board.push(v);
    }
    let (_, log) = run(&db, &board, &walls(&db, 9), 1, 1);
    let beetles = log.summons(0, "Beetle");
    assert_eq!(beetles.len(), 4);
    // two Kodos each give +16/+32 to each of the first three summons
    assert_eq!(beetles[0], (2 + 32, 2 + 64));
    assert_eq!(beetles[1], (2 + 32, 2 + 64));
    assert_eq!(beetles[2], (2 + 32, 2 + 64));
    assert_eq!(beetles[3], (2, 2), "both Kodos have used their three");
}

#[test]
fn kodo_counts_a_reborn_summon_too() {
    let db = db();
    let mut board = kodo_board(&db, false, 1);
    board[2].keywords.set(Keywords::REBORN);
    let (_, log) = run(&db, &board, &walls(&db, 4), 1, 1);
    // the Deathrattle Beetle and the Reborn Vermin (1 Health) both get it
    assert_eq!(log.summons(0, "Beetle"), vec![(18, 34)]);
    assert_eq!(log.summons(0, "Buzzing Vermin"), vec![(1 + 16, 1 + 32)]);
}

#[test]
fn kodo_does_not_buff_summons_on_the_other_side() {
    let db = db();
    let (_, log) = run(&db, &walls(&db, 1), &kodo_board(&db, false, 1), 1, 1);
    // side 1 has the Kodo, so its Beetle is the one that gets the gift
    assert_eq!(log.summons(1, "Beetle"), vec![(18, 34)]);
    assert!(log.summons(0, "Beetle").is_empty());
}

// ================= Polarizing Beatboxer =================

fn board_has_lullabot(m: &Minion, db: &CardDb) -> bool {
    m.attached_slice().iter().any(|a| db.get(a.card).name == "Lullabot")
}

/// The first tier 1 Magnetic is Lullabot (2/2, carries an end of turn
/// effect, so it shows up in `attached`).
fn play_lullabot_on(g: &mut Game, pos: u8) {
    g.give(0, "Lullabot").unwrap();
    let hand = (g.players[0].hand.len() - 1) as u8;
    act(g, 0, Action::Play { hand, pos });
}

#[test]
fn beatboxer_gets_a_copy_of_every_magnetic_that_goes_to_a_different_minion() {
    let mut g = game(71);
    put_on_board(&mut g, 0, "Polarizing Beatboxer", false);
    put_on_board(&mut g, 0, "Cord Puller", false);
    let lullabot = id(&g.data.cards, "Lullabot") as usize;
    let copies = g.pool[lullabot];
    play_lullabot_on(&mut g, 1);
    let b = &g.players[0].board;
    assert_eq!(b.len(), 2);
    assert_eq!(stats(&b[1]), (1 + 2, 1 + 2));
    // same stats, keywords, effects (the attachment) and it counts as a
    // Magnetization on the Beatboxer
    assert_eq!(stats(&b[0]), (5 + 2, 10 + 2));
    assert_eq!(b[0].magnetizations, 1);
    assert!(board_has_lullabot(&b[0], &g.data.cards));
    // Lullabot has no keywords of its own in the pool, so the host's don't change
    assert!(!b[0].keywords.has(Keywords::DIVINE_SHIELD));
    // no pool copy was taken for the copy: only the one played is consumed
    assert_eq!(g.pool[lullabot], copies - 1);
    assert_eq!(g.consumed[lullabot], 1);
    g.check_invariants().unwrap();
}

#[test]
fn beatboxer_does_not_copy_a_magnetic_played_onto_itself() {
    let mut g = game(72);
    put_on_board(&mut g, 0, "Polarizing Beatboxer", false);
    play_lullabot_on(&mut g, 0);
    let b = &g.players[0].board;
    assert_eq!(stats(&b[0]), (7, 12), "one Lullabot, not two");
    assert_eq!(b[0].magnetizations, 1);
    g.check_invariants().unwrap();
}

#[test]
fn golden_beatboxer_gets_the_copy_twice() {
    let mut g = game(73);
    put_on_board(&mut g, 0, "Polarizing Beatboxer", true);
    put_on_board(&mut g, 0, "Cord Puller", false);
    play_lullabot_on(&mut g, 1);
    let b = &g.players[0].board;
    assert_eq!(stats(&b[0]), (10 + 4, 20 + 4));
    assert_eq!(b[0].magnetizations, 2);
    assert_eq!(stats(&b[1]), (3, 3));
    g.check_invariants().unwrap();
}

#[test]
fn two_beatboxers_do_not_loop() {
    let mut g = game(74);
    put_on_board(&mut g, 0, "Polarizing Beatboxer", false);
    put_on_board(&mut g, 0, "Polarizing Beatboxer", false);
    put_on_board(&mut g, 0, "Cord Puller", false);
    play_lullabot_on(&mut g, 2);
    let b = &g.players[0].board;
    assert_eq!(stats(&b[2]), (3, 3));
    for i in 0..2 {
        assert_eq!(stats(&b[i]), (7, 12), "beatboxer {i} has exactly one copy");
        assert_eq!(b[i].magnetizations, 1);
    }
    g.check_invariants().unwrap();

    // Magnetizing straight to the first Beatboxer: the second copies
    // nothing. A Magnetization onto a Beatboxer leaves the others alone,
    // which is also what stops two Beatboxers looping: the log's Beatboxer
    // played at `Hearthstone_2026_10_29_22_20_04-g3` turn 14 step 16 takes
    // the Spark Snapper's Satellite (5/10 to 105/160) and the Beatboxer
    // already on the board stays where it is.
    let mut g = game(75);
    put_on_board(&mut g, 0, "Polarizing Beatboxer", false);
    put_on_board(&mut g, 0, "Polarizing Beatboxer", false);
    play_lullabot_on(&mut g, 0);
    let b = &g.players[0].board;
    assert_eq!(stats(&b[0]), (7, 12));
    assert_eq!(stats(&b[1]), (5, 10), "a Magnetization onto a Beatboxer sets none off");
    assert_eq!(b[0].magnetizations, 1);
    assert_eq!(b[1].magnetizations, 0);
    g.check_invariants().unwrap();
}

#[test]
fn a_beatboxer_played_takes_its_satellite_and_sets_none_off() {
    // Hearthstone_2026_10_29_22_20_04-g3 turn 14 step 16: a Beatboxer played
    // from hand takes Spark Snapper's 100/150 Satellite (5/10 to 105/160) and
    // the Beatboxer already on the board gets nothing.
    let mut g = game(77);
    put_on_board(&mut g, 0, "Spark Snapper", false);
    put_on_board(&mut g, 0, "Polarizing Beatboxer", false);
    put_in_hand(&mut g, 0, "Polarizing Beatboxer", false);
    act(&mut g, 0, Action::Play { hand: 0, pos: 2 });
    let b = &g.players[0].board;
    assert_eq!(stats(&b[2]), (5 + 2, 10 + 3), "the Satellite joined it");
    assert_eq!(stats(&b[1]), (5, 10), "the other Beatboxer got nothing");
    g.check_invariants().unwrap();
}

#[test]
fn a_satellite_joins_the_played_magnetic_before_it_merges() {
    // The raw log's PLAY block of a Magnetic played from hand runs the
    // "whenever you play a Mech" triggers against it while it is a minion of
    // its own, so Spark Snapper's Satellite joins it there and every copy of
    // it carries the Satellite: `Hearthstone_2026_10_29_22_20_04-g3` turn 13
    // step 10, a 2/4 Annoy-o-Module with an 84/126 Satellite on it into a Red
    // Volumizer, the host +86/+130 and the Beatboxer +170/+256.
    let mut g = game(78);
    put_on_board(&mut g, 0, "Spark Snapper", false);
    put_on_board(&mut g, 0, "Cord Puller", false);
    put_on_board(&mut g, 0, "Polarizing Beatboxer", false);
    g.give(0, "Annoy-o-Module").unwrap(); // 2/4 Magnetic
    act(&mut g, 0, Action::Play { hand: 0, pos: 1 });
    let b = &g.players[0].board;
    assert_eq!(stats(&b[1]), (1 + 4, 1 + 7), "the module and its 2/3 Satellite");
    assert_eq!(b[1].magnetizations, 1, "one Magnetization: the Satellite rode in");
    assert_eq!(stats(&b[2]), (5 + 6, 10 + 10), "the Beatboxer gets both");
    assert_eq!(b[2].magnetizations, 2);
    assert_eq!(stats(&b[0]), (6, 8), "the Snapper only improves itself");
    g.check_invariants().unwrap();
}

#[test]
fn beatboxer_with_mechagnome_interpreter_and_utility_drone() {
    let mut g = game(76);
    put_on_board(&mut g, 0, "Polarizing Beatboxer", false);
    put_on_board(&mut g, 0, "Cord Puller", false);
    put_on_board(&mut g, 0, "Mechagnome Interpreter", false);
    play_lullabot_on(&mut g, 1);
    let b = &g.players[0].board;
    // the Interpreter gives +3/+1 to the Mech that was Magnetized, and the
    // copy is a Magnetization too, so the Beatboxer gets it as well
    assert_eq!(stats(&b[1]), (1 + 2 + 3, 1 + 2 + 1));
    assert_eq!(stats(&b[0]), (5 + 2 + 3, 10 + 2 + 1), "a 5/3 copy, not buffed again (8/4)");
    assert_eq!(b[0].magnetizations, 1);
    g.check_invariants().unwrap();

    // Utility Drone counts Magnetizations: the Beatboxer's count includes the
    // copy, so at the end of the turn it grows by it.
    let mut g = game(77);
    put_on_board(&mut g, 0, "Polarizing Beatboxer", false);
    put_on_board(&mut g, 0, "Cord Puller", false);
    put_on_board(&mut g, 0, "Utility Drone", false);
    play_lullabot_on(&mut g, 1);
    assert_eq!(g.players[0].board[0].magnetizations, 1);
    let before = stats(&g.players[0].board[0]);
    let drone = g.data.cards.get(id(&g.data.cards, "Utility Drone"));
    let (da, dh) = match drone.end_of_turn {
        Some(bgcore::cards::EndOfTurn::BuffPerMagnetization { attack, health }) => (attack, health),
        other => panic!("Utility Drone changed: {other:?}"),
    };
    end_turn_of(&mut g, 0);
    let after = stats(&g.players[0].board[0]);
    // Lullabot grows its host by +0/+1 at the end of turn too
    assert_eq!(after, (before.0 + da, before.1 + dh + 1));
    g.check_invariants().unwrap();
}

#[test]
fn beatboxer_with_glambot_satellites() {
    for golden in [false, true] {
        let mut g = game(78);
        put_on_board(&mut g, 0, "Polarizing Beatboxer", false);
        put_on_board(&mut g, 0, "Cord Puller", false);
        put_on_board(&mut g, 0, "Glambot", golden);
        g.give(0, "Tavern Dish Banana").unwrap();
        act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
        act(&mut g, 0, Action::Target(1));
        let n = if golden { 2 } else { 1 };
        let b = &g.players[0].board;
        // Tavern Dish Banana +2/+2, then a 4/4 Satellite per Magnetization
        assert_eq!(stats(&b[1]), (1 + 2 + 4 * n, 1 + 2 + 4 * n), "golden {golden}");
        assert_eq!(stats(&b[0]), (5 + 4 * n, 10 + 4 * n), "golden {golden}");
        assert_eq!(b[0].magnetizations, n as u16);
        g.check_invariants().unwrap();
    }
}

#[test]
fn beatboxer_with_drone_duplicator_gets_each_extra_magnetization() {
    let mut g = game(79);
    put_on_board(&mut g, 0, "Polarizing Beatboxer", false);
    put_on_board(&mut g, 0, "Cord Puller", false);
    g.players[0].board[1].extra_magnetize = 1;
    let lullabot = id(&g.data.cards, "Lullabot") as usize;
    let copies = g.pool[lullabot];
    play_lullabot_on(&mut g, 1);
    let b = &g.players[0].board;
    // the Magnetization happens twice, without a second pool copy
    assert_eq!(stats(&b[1]), (1 + 4, 1 + 4));
    assert_eq!(b[1].magnetizations, 2);
    assert_eq!(stats(&b[0]), (5 + 4, 10 + 4));
    assert_eq!(b[0].magnetizations, 2);
    assert_eq!(g.pool[lullabot], copies - 1);
    assert_eq!(g.consumed[lullabot], 1);
    g.check_invariants().unwrap();
}

#[test]
fn beatboxer_copy_brings_nested_attachments() {
    let mut g = game(80);
    put_on_board(&mut g, 0, "Polarizing Beatboxer", false);
    put_on_board(&mut g, 0, "Cord Puller", false);
    // a Lullabot already carrying another Lullabot is played onto the Puller
    g.give(0, "Lullabot").unwrap();
    let inner = g.data.cards.make(id(&g.data.cards, "Lullabot"), false);
    let carrier = &mut g.players[0].hand[0];
    let db = g.data.cards.clone();
    carrier.magnetize(&db, inner);
    play_lullabot_on_hand0(&mut g, 1);
    let b = &g.players[0].board;
    assert_eq!(b[1].attached_slice().len(), 2);
    assert_eq!(b[0].attached_slice().len(), 2, "the whole stack is copied");
    assert_eq!(stats(&b[0]), (5 + 4, 10 + 4));
    assert_eq!(b[0].magnetizations, 2, "a Magnetic with a Magnetization counts both");
}

fn play_lullabot_on_hand0(g: &mut Game, pos: u8) {
    act(g, 0, Action::Play { hand: 0, pos });
}

#[test]
fn kodo_reborn_counts_as_damaged_not_as_max_one() {
    let db = db();
    // A Reborn Kodo with 1 Health dies to Boom and comes back with 1 Health
    // but a maximum of 32: the Beetle a Vermin leaves gets the full 32.
    let mut kodo = mk(&db, "Stalwart Kodo", false);
    kodo.health = 1;
    kodo.keywords.set(Keywords::REBORN);
    let mut vermin = mk(&db, "Buzzing Vermin", false);
    vermin.keywords.clear(Keywords::TAUNT);
    let board = [kodo, mk(&db, "Boom-in-a-Box", false), vermin];
    let (_, log) = run(&db, &board, &walls(&db, 4), 1, 1);
    assert_eq!(log.deaths(0, "Stalwart Kodo"), 1);
    assert_eq!(log.summons(0, "Beetle"), vec![(2 + 16, 2 + 32)]);

    // A Reborn minion that Kodo gives stats to is a recipient like any other.
    let mut board = kodo_board(&db, false, 1);
    board[2].keywords.set(Keywords::REBORN);
    let (_, log) = run(&db, &board, &walls(&db, 4), 1, 1);
    assert_eq!(log.summons(0, "Buzzing Vermin"), vec![(1 + 16, 1 + 32)]);
}

#[test]
fn beatboxer_sees_a_magnetization_to_a_minion_in_the_tavern() {
    let mut g = game(81);
    put_on_board(&mut g, 0, "Polarizing Beatboxer", false);
    put_on_board(&mut g, 0, "Glambot", false);
    set_shop(&mut g, 0, &["Cord Puller"]);
    g.give(0, "Tavern Dish Banana").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    act(&mut g, 0, Action::TargetShop(0));
    assert_eq!(stats(&g.players[0].shop[0]), (1 + 2 + 4, 1 + 2 + 4));
    let b = &g.players[0].board[0];
    assert_eq!(stats(b), (5 + 4, 10 + 4));
    assert_eq!(b.magnetizations, 1);
    g.check_invariants().unwrap();
}

#[test]
fn beatboxer_copies_spark_snappers_satellite_but_nothing_triggers_from_a_copy() {
    let mut g = game(82);
    put_on_board(&mut g, 0, "Polarizing Beatboxer", false);
    put_on_board(&mut g, 0, "Spark Snapper", false);
    put_in_hand(&mut g, 0, "Cord Puller", false);
    // Spark Snapper attaches a 2/3 (the Satellite token itself is 4/4)
    let sat = g.data.cards.make(id(&g.data.cards, "Satellite"), false);
    assert_eq!(stats(&sat), (4, 4));
    let sat = Minion { attack: 2, health: 3, ..sat };
    act(&mut g, 0, Action::Play { hand: 0, pos: 2 });
    let b = &g.players[0].board;
    // the played Puller and the Beatboxer each got one Satellite
    assert_eq!(stats(&b[2]), (1 + sat.attack, 1 + sat.health));
    assert_eq!(stats(&b[0]), (5 + sat.attack, 10 + sat.health));
    assert_eq!(b[0].magnetizations, 1);
    // Spark Snapper triggered once, by the Mech played from hand
    assert_eq!(b[1].improvements, 1);
    g.check_invariants().unwrap();
}

#[test]
fn beatboxer_copy_of_a_volumizer_raises_your_volumizers_again() {
    for (golden, times) in [(false, 2), (true, 3)] {
        let mut g = game(83);
        put_on_board(&mut g, 0, "Polarizing Beatboxer", golden);
        put_on_board(&mut g, 0, "Cord Puller", false);
        g.give(0, "Blue Volumizer").unwrap();
        let hand = g.players[0].hand[0];
        let (da, dh) = g.data.cards.get(hand.card).volumizer_buff.unwrap();
        act(&mut g, 0, Action::Play { hand: 0, pos: 1 });
        // the real one plus one bonus per copy
        assert_eq!(g.players[0].volumizer_bonus, (da * times, dh * times), "golden {golden}");
        g.check_invariants().unwrap();
    }
}
