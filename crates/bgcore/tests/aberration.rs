//! The Aberration tribe, with the real cards from cards.toml: the Deity (one
//! god per lobby, shop and fight gains, the awakening, C'Thun's split,
//! Y'Shaarj's return, waiting for space), the discard triggers, "if you
//! discard this" spells, linked cards, Mysterious K'Thir, The Shadow of
//! Doubt, the hidden numbers and De-volition-ist.

use std::sync::Arc;

use bgcore::fight::{Deity, Watch, fight_with};
use bgcore::{Tribe, Action, CardDb, CardId, Counters, Data, Game, Keywords, Minion, Rng};

fn db() -> CardDb {
    Data::bundled().cards
}

fn lobby(ts: &[Tribe]) -> bgcore::Tribes {
    ts.iter().copied().fold(bgcore::Tribes::NONE, |m, t| m.with(t))
}

fn game(seed: u64) -> Game {
    Game::new_with_lobby(Arc::new(Data::bundled()), seed, lobby(&[Tribe::Aberration, Tribe::Beast, Tribe::Mech, Tribe::Dragon, Tribe::Quilboar]))
}

fn mk(db: &CardDb, name: &str, golden: bool) -> Minion {
    db.make(db.by_name(name).expect(name), golden)
}

fn id(db: &CardDb, name: &str) -> CardId {
    db.by_name(name).expect(name)
}

fn stats(m: &Minion) -> (i32, i32) {
    (m.attack, m.health)
}

/// A minion that never attacks and takes a long time to kill.
fn wall(db: &CardDb) -> Minion {
    let mut m = mk(db, "Microbot", false);
    m.attack = 0;
    m.health = 2000;
    m
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

fn end_all_turns(g: &mut Game) {
    let t = g.turn;
    while g.turn == t {
        if !answer(g) {
            g.step(Action::EndTurn).unwrap();
        }
    }
}

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

fn play(g: &mut Game, p: usize, hand: u8, pos: u8) {
    act(g, p, Action::Play { hand, pos });
}

/// Answer a target, shop target or choice a played card left open (the
/// linked spells are random Tavern spells).
fn settle_prompts(g: &mut Game, p: usize) {
    loop {
        turn_of(g, p);
        let pl = &g.players[p];
        if pl.pending_target.is_some() {
            act(g, p, Action::Target(0));
        } else if pl.pending_shop_target.is_some() {
            act(g, p, Action::TargetShop(0));
        } else if pl.pending_choose_one.is_some() || !pl.discover.is_empty() {
            act(g, p, Action::Choose(0));
        } else {
            return;
        }
    }
}

fn with_health(mut m: Minion, health: i32) -> Minion {
    m.health = health;
    m
}

fn taunt(mut m: Minion) -> Minion {
    m.keywords.set(Keywords::TAUNT);
    m
}

/// A token minion that never swings or retaliates, with the health given.
fn sleeper(db: &CardDb, card: &str, health: i32) -> Minion {
    let mut m = mk(db, card, false);
    m.attack = 0;
    m.health = health;
    m
}

fn taunt_sleeper(db: &CardDb, card: &str, health: i32) -> Minion {
    taunt(sleeper(db, card, health))
}

fn deity_of(db: &CardDb, god: &str, attack: i32, health: i32, golden: bool) -> Deity {
    Deity { card: id(db, god), attack, health, golden }
}

#[derive(Clone, Debug)]
enum Ev {
    Attack { side: usize, card: CardId, minion: Minion },
    Death { side: usize, card: CardId },
    Summon { side: usize, pos: usize, card: CardId, minion: Minion },
}

#[derive(Default)]
struct Log(Vec<Ev>);

impl Watch for Log {
    fn attack(&mut self, side: usize, attacker: &Minion, _defender: &Minion) {
        self.0.push(Ev::Attack { side, card: attacker.card, minion: *attacker });
    }
    fn death(&mut self, side: usize, _pos: usize, m: &Minion) {
        self.0.push(Ev::Death { side, card: m.card });
    }
    fn summon(&mut self, side: usize, pos: usize, m: &Minion) {
        self.0.push(Ev::Summon { side, pos, card: m.card, minion: *m });
    }
}

fn run(
    db: &CardDb,
    a: &[Minion],
    b: &[Minion],
    counters: [Counters; 2],
    max_attacks: u32,
    seed: u64,
) -> (bgcore::Outcome, Log) {
    let mut log = Log::default();
    let out = fight_with(db, a, b, counters, max_attacks, &mut Rng::new(seed), &mut log);
    (out, log)
}

/// Summons of one card on one side, with where they landed.
fn summons(log: &Log, side: usize, card: CardId) -> Vec<(usize, Minion)> {
    log.0.iter()
        .filter_map(|e| match e {
            Ev::Summon { side: s, pos, card: c, minion } if *s == side && *c == card => {
                Some((*pos, *minion))
            }
            _ => None,
        })
        .collect()
}

/// Deaths on one side before the first event index `until`.
fn deaths_before(log: &Log, side: usize, until: usize) -> usize {
    log.0[..until]
        .iter()
        .filter(|e| matches!(e, Ev::Death { side: s, .. } if *s == side))
        .count()
}

// ================= The Deity =================

#[test]
fn one_god_per_lobby_starting_one_one() {
    let (cthun, yshaarj) = (id(&db(), "C'Thun"), id(&db(), "Y'Shaarj"));
    let mut seen = [false; 2];
    for seed in 0..80u64 {
        let g = game(seed);
        let d = g.players[0].deity.expect("every lobby has a Deity");
        assert_eq!((d.attack, d.health), (1, 1), "seed {seed}");
        assert!(!d.golden);
        assert!(d.card == cthun || d.card == yshaarj, "seed {seed}");
        // one god for the whole lobby, the same stats for everyone
        for pl in &g.players[1..] {
            assert_eq!(pl.deity, Some(d), "seed {seed}: the lobby shares one god");
        }
        seen[usize::from(d.card == yshaarj)] = true;
    }
    assert!(seen[0] && seen[1], "both gods should come up: {seen:?}");
}

#[test]
fn joyous_grows_the_deity_in_the_shop_and_it_stays() {
    let mut g = game(3);
    put_in_hand(&mut g, 0, "Joyous", false);
    play(&mut g, 0, 0, 0);
    assert_eq!(g.players[0].deity.map(|d| (d.attack, d.health)), Some((3, 2)));
    end_all_turns(&mut g);
    turn_of(&mut g, 0);
    assert_eq!(
        g.players[0].deity.map(|d| (d.attack, d.health)),
        Some((3, 2)),
        "the Deity keeps its gains across turns"
    );
    g.check_invariants().unwrap();
}

#[test]
fn drifting_sacrifice_grows_the_deity_in_a_fight_and_it_keeps() {
    let mut g = game(11);
    let Some(bgcore::Opponent::Player(q)) = g.players[0].opponent else { panic!() };
    put_on_board(&mut g, 0, "Drifting Sacrifice", false);
    let mut bot = mk(&g.data.cards, "Microbot", false);
    bot.attack = 3;
    bot.health = 100;
    g.players[q as usize].board.push(bot);
    end_all_turns(&mut g);
    // it dies, comes back through Reborn and dies again: two Deathrattles
    assert_eq!(
        g.players[0].deity.map(|d| (d.attack, d.health)),
        Some((5, 3)),
        "both Drifting Sacrifice deaths feed the Deity"
    );
    g.check_invariants().unwrap();
}

#[test]
fn the_deity_awakens_at_the_fourth_aberration_death_once_per_fight() {
    let db = db();
    let mut a = Vec::new();
    for _ in 0..4 {
        a.push(mk(&db, "Aberrant Tentacle", false));
    }
    a.push(wall(&db));
    let counters = [
        Counters { deity: Some(deity_of(&db, "Y'Shaarj", 4, 6, false)), ..Counters::default() },
        Counters::default(),
    ];
    let (_, log) = run(&db, &a, &[mk(&db, "Microbot", false)], counters, 60, 5);
    let god = id(&db, "Y'Shaarj");
    let arrivals = summons(&log, 0, god);
    assert_eq!(arrivals.len(), 1, "exactly one awakening per fight");
    assert_eq!(stats(&arrivals[0].1), (4, 6), "it wakes with the Deity's current stats");
    let i = log.0.iter().position(|e| matches!(e, Ev::Summon { side: 0, card: c, .. } if *c == god)).unwrap();
    assert_eq!(deaths_before(&log, 0, i), 4, "it wakes right at the fourth death");
    // more Aberration deaths follow (the god's own Deathrattle brings two
    // Tentacles back, and they die again): still one awakening, asserted above
}

#[test]
fn the_awakening_waits_for_board_space() {
    let db = db();
    // four Underrot Spawns die at once and each deathrattle refills the
    // board, so the god has nowhere to go until a Tentacle dies after them
    let mut a = Vec::new();
    for _ in 0..4 {
        a.push(mk(&db, "Underrot Spawn", false));
    }
    a.push(mk(&db, "Boom-in-a-Box", true));
    a.push(with_health(wall(&db), 100));
    a.push(with_health(wall(&db), 100));
    let counters = [
        Counters { deity: Some(deity_of(&db, "Y'Shaarj", 2, 3, false)), ..Counters::default() },
        Counters::default(),
    ];
    let mut killer = mk(&db, "Microbot", false);
    killer.attack = 3;
    killer.health = 1000;
    let (_, log) = run(&db, &a, &[killer], counters, 60, 5);
    let god = id(&db, "Y'Shaarj");
    let arrivals = summons(&log, 0, god);
    assert_eq!(arrivals.len(), 1, "the god arrives once space opens");
    assert_eq!(stats(&arrivals[0].1), (2, 3));
    let i = log.0.iter().position(|e| matches!(e, Ev::Summon { side: 0, card: c, .. } if *c == god)).unwrap();
    assert_eq!(
        deaths_before(&log, 0, i),
        5,
        "four Spawns plus one Tentacle: the board was full at the fourth death"
    );
}

#[test]
fn cthun_splits_its_stats_with_the_remainder_to_random_minions() {
    let db = db();
    for golden in [false, true] {
        let mut a = Vec::new();
        // two different sleepers, so each one's gain is measurable on its own
        for card in ["Microbot", "Foraging Bat"] {
            let mut w = sleeper(&db, card, 10);
            w.attack = 1;
            a.push(w);
        }
        for _ in 0..4 {
            a.push(mk(&db, "Aberrant Tentacle", false));
        }
        a.push(mk(&db, "Boom-in-a-Box", false));
        let counters = [
            Counters { deity: Some(deity_of(&db, "C'Thun", 9, 5, golden)), ..Counters::default() },
            Counters::default(),
        ];
        let (_, log) = run(&db, &a, &[wall(&db)], counters, 20, 7);
        let god = id(&db, "C'Thun");
        let arrivals = summons(&log, 0, god);
        assert_eq!(arrivals.len(), 1, "golden {golden}: one awakening");
        // every surviving minion swings at the enemy wall, so its attack
        // snapshot is its stats after the split
        let gain = |card: &str, base: i32| -> (i32, i32) {
            let c = id(&db, card);
            let hits: Vec<&Minion> = log.0.iter()
                .filter_map(|e| match e {
                    Ev::Attack { side: 0, card: x, minion } if *x == c => Some(minion),
                    _ => None,
                })
                .collect();
            assert!(!hits.is_empty(), "golden {golden}: {card} never attacked");
            let last = hits[hits.len() - 1];
            (last.attack, last.health - base)
        };
        let (k, mul) = if golden { (6, 2) } else { (3, 1) };
        let (w1, w1g) = gain("Microbot", 7);
        assert_eq!(w1, 1 + k, "golden {golden}: equal share of the attack");
        let (w2, w2g) = gain("Foraging Bat", 7);
        assert_eq!(w2, 1 + k);
        let (b, bg) = gain("Boom-in-a-Box", 10);
        assert_eq!(b, 5 + k);
        // the health total 5 (golden 10) is shared out, one each, the
        // remainder +1 to different minions
        assert_eq!(w1g + w2g + bg, 5 * mul, "golden {golden}: the health is split exactly");
        assert!(w1g >= 1 && w2g >= 1 && bg >= 1, "golden {golden}: everyone gets a share");
        assert_eq!(
            [w1g, w2g, bg].iter().filter(|&&g| g == 5 * mul / 3 + 1).count(),
            (5 * mul % 3) as usize,
            "golden {golden}: the remainder goes +1 to different minions"
        );
        // the god itself keeps its stats and its shape
        let god_hits: Vec<&Minion> = log.0.iter()
            .filter_map(|e| match e {
                Ev::Attack { side: 0, card: x, minion } if *x == god => Some(minion),
                _ => None,
            })
            .collect();
        assert!(!god_hits.is_empty());
        assert!(god_hits.iter().all(|m| stats(m) == (9, 5)), "golden {golden}: the god keeps its own");
    }
}

#[test]
fn yshaarj_brings_back_the_first_two_dead_with_their_maximum_stats() {
    let db = db();
    let a = vec![
        mk(&db, "Fetid Corroder", false),
        with_health(mk(&db, "Abyssal Envoy", false), 6),
        mk(&db, "Joyous", false),
        mk(&db, "Zoatroid", false),
        mk(&db, "Boom-in-a-Box", true),
    ];
    let counters = [
        Counters { deity: Some(deity_of(&db, "Y'Shaarj", 4, 6, false)), ..Counters::default() },
        Counters::default(),
    ];
    let mut killer = mk(&db, "Microbot", false);
    killer.attack = 100;
    killer.health = 1000;
    let (_, log) = run(&db, &a, &[killer], counters, 24, 5);
    let (god, fetid, envoy) = (id(&db, "Y'Shaarj"), id(&db, "Fetid Corroder"), id(&db, "Abyssal Envoy"));
    // the golden Boom kills all four Aberrations at Start of Combat: they die
    // in board order, so Fetid Corroder and Abyssal Envoy are the first two
    let arrivals = summons(&log, 0, god);
    assert_eq!(arrivals.len(), 1, "one awakening");
    assert_eq!(stats(&arrivals[0].1), (4, 6));
    let events: Vec<&Ev> = log.0.iter().collect();
    let god_death = events.iter().position(|e| matches!(e, Ev::Death { side: 0, card: c } if *c == god))
        .expect("the killer destroys the god");
    let back = |card: CardId| {
        events[god_death..].iter().any(|e| matches!(e, Ev::Summon { side: 0, card: c, .. } if *c == card))
    };
    assert!(back(fetid), "the first to die comes back");
    assert!(back(envoy), "the second to die comes back");
    let s = summons(&log, 0, fetid);
    assert_eq!(stats(&s[0].1), (3, 3), "full stats");
    let s = summons(&log, 0, envoy);
    assert_eq!(stats(&s[0].1), (2, 6), "maximum health, not the health it died at");
    // Joyous and Zoatroid died third and fourth: they stay dead
    assert!(summons(&log, 0, id(&db, "Joyous")).is_empty());
    assert!(summons(&log, 0, id(&db, "Zoatroid")).is_empty());
}

#[test]
fn a_golden_yshaarj_brings_back_four() {
    let db = db();
    let a = vec![
        mk(&db, "Fetid Corroder", false),
        with_health(mk(&db, "Abyssal Envoy", false), 6),
        mk(&db, "Joyous", false),
        mk(&db, "Zoatroid", false),
        mk(&db, "Boom-in-a-Box", true),
    ];
    let counters = [
        Counters { deity: Some(deity_of(&db, "Y'Shaarj", 4, 6, true)), ..Counters::default() },
        Counters::default(),
    ];
    let mut killer = mk(&db, "Microbot", false);
    killer.attack = 100;
    killer.health = 1000;
    let (_, log) = run(&db, &a, &[killer], counters, 24, 5);
    let god = id(&db, "Y'Shaarj");
    let events: Vec<&Ev> = log.0.iter().collect();
    let god_death = events.iter().position(|e| matches!(e, Ev::Death { side: 0, card: c } if *c == god))
        .expect("the killer destroys the god");
    let back = ["Fetid Corroder", "Abyssal Envoy", "Joyous", "Zoatroid"].iter()
        .filter(|&&n| {
            let c = id(&db, n);
            events[god_death..].iter().any(|e| matches!(e, Ev::Summon { side: 0, card: x, .. } if *x == c))
        })
        .count();
    assert_eq!(back, 4, "a golden Deity brings back all four");
}

// ================= Discard triggers =================

#[test]
fn every_discard_trigger_fires_on_one_discard() {
    let mut g = game(4);
    put_on_board(&mut g, 0, "Cutthroat K'Thir", false);
    put_on_board(&mut g, 0, "Mindbender Ghur'sha", false);
    put_on_board(&mut g, 0, "Harbinger Aph'lass", false);
    put_on_board(&mut g, 0, "Brain Rotter", false);
    put_on_board(&mut g, 0, "Mangled Bandit", false);
    g.give(0, "Blood Gem").unwrap();
    g.give(0, "Blood Gem").unwrap();
    act(&mut g, 0, Action::Activate(3));
    act(&mut g, 0, Action::Discard(0));
    let pl = &g.players[0];
    assert_eq!(stats(&pl.board[0]), (12, 12), "K'Thir: its own +4/+4 and Ghur'sha's");
    assert_eq!(stats(&pl.board[1]), (3, 9), "Ghur'sha does not buff itself");
    assert_eq!(stats(&pl.board[2]), (7, 10), "Aph'lass only gets Ghur'sha's buff");
    assert_eq!(stats(&pl.board[3]), (7, 8), "Brain Rotter only gets Ghur'sha's buff");
    assert_eq!(stats(&pl.board[4]), (7, 7));
    // K'Thir +4/+4, Aph'lass +2/+1 (not improved yet), the Brain Rotter reward +2/+2
    assert_eq!(pl.deity.map(|d| (d.attack, d.health)), Some((9, 8)));
    assert_eq!(pl.discarded, 1);

    // a second discard: Aph'lass has improved once, so it gives +4/+2 now
    act(&mut g, 0, Action::Activate(4));
    act(&mut g, 0, Action::Discard(0));
    let pl = &g.players[0];
    assert_eq!(stats(&pl.board[0]), (20, 20), "4 + 4 + 4 twice more");
    assert_eq!(stats(&pl.board[2]), (11, 14));
    assert_eq!(pl.deity.map(|d| (d.attack, d.health)), Some((17, 14)));
    assert_eq!(pl.board[2].improvements, 2, "Aph'lass improved once per discard");
    assert_eq!(pl.discarded, 2);
    assert_eq!(gems_in_hand(&g, 0), 3, "the Bandit reward");
    g.check_invariants().unwrap();
}

fn gems_in_hand(g: &Game, p: usize) -> usize {
    let gem = id(&g.data.cards, "Blood Gem");
    g.players[p].hand.iter().filter(|m| m.card == gem).count()
}

#[test]
fn sludge_corrosion_discarded_casts_twice_and_counts_twice() {
    let mut g = game(5);
    put_on_board(&mut g, 0, "Vicious Mindslasher", false);
    put_on_board(&mut g, 0, "Sha of Fear", false);
    put_on_board(&mut g, 0, "Mangled Bandit", false);
    g.give(0, "Sludge Corrosion").unwrap();
    act(&mut g, 0, Action::Activate(2));
    act(&mut g, 0, Action::Discard(0));
    let pl = &g.players[0];
    // each cast: Mindslasher +1/+2, everyone +3/+3 (Sha of Fear) and +1/+1
    // (the spell itself); the deity gets both gods' parts
    assert_eq!(stats(&pl.board[0]), (1 + 2 * 5, 2 + 2 * 6), "Mindslasher");
    assert_eq!(stats(&pl.board[1]), (10 + 2 * 4, 13 + 2 * 4), "Sha of Fear");
    assert_eq!(stats(&pl.board[2]), (3 + 2 * 4, 3 + 2 * 4), "Mangled Bandit");
    assert_eq!(pl.deity.map(|d| (d.attack, d.health)), Some((9, 11)));
    assert_eq!(pl.spells_cast, 2, "two real Tavern spell casts");
    assert_eq!(pl.discarded, 1);
    g.check_invariants().unwrap();
}

#[test]
fn an_activates_improvement_is_in_place_for_the_card_it_discards() {
    // N'raqi Frostcaller: "Discard a card for your Tavern spells to give an
    // extra +1/+1 this game". The improvement lands before the discarded
    // card's own casts, so a discarded Sludge Corrosion gives +2/+2 a cast and
    // not +1/+1 (`Hearthstone_2026_10_28_22_11_50-g6` turn 9 step 35 and turn
    // 12 step 7, each +2/+2 a minion short without this order).
    let mut g = game(19);
    put_on_board(&mut g, 0, "N'raqi Frostcaller", false);
    g.give(0, "Sludge Corrosion").unwrap();
    act(&mut g, 0, Action::Activate(0));
    act(&mut g, 0, Action::Discard(0));
    let pl = &g.players[0];
    assert_eq!(pl.spell_bonus, (1, 1), "the Activate's own +1/+1");
    // two casts, each +1/+1 plus the bonus it already reads
    assert_eq!(stats(&pl.board[0]), (6 + 4, 3 + 4), "the Frostcaller's own +2/+2 a cast");
    assert_eq!(pl.spells_cast, 2);
    g.check_invariants().unwrap();
}

#[test]
fn corrupted_coin_discarded_raises_maximum_gold_and_never_casts() {
    let mut g = game(6);
    put_on_board(&mut g, 0, "Mangled Bandit", false);
    g.give(0, "Corrupted Coin").unwrap();
    let gold = g.players[0].gold;
    act(&mut g, 0, Action::Activate(0));
    act(&mut g, 0, Action::Discard(0));
    let pl = &g.players[0];
    assert_eq!(pl.extra_max_gold, 2, "maximum Gold up by 2");
    assert_eq!(pl.gold, gold, "not cast: no 2 Gold, and the Activate costs 0");
    assert_eq!(pl.spells_cast, 0, "not cast: no spell counter");
    assert_eq!(stats(&pl.board[0]), (3, 3), "not cast: no board buff either");
    assert_eq!(gems_in_hand(&g, 0), 3);
    g.check_invariants().unwrap();
}

#[test]
fn energizing_chamber_discarded_casts_twice_on_the_deity() {
    let mut g = game(17);
    put_on_board(&mut g, 0, "Mangled Bandit", false);
    g.give(0, "Energizing Chamber").unwrap();
    act(&mut g, 0, Action::Activate(0));
    act(&mut g, 0, Action::Discard(0));
    let pl = &g.players[0];
    assert_eq!(pl.deity.map(|d| (d.attack, d.health)), Some((15, 15)), "+7/+7 twice");
    assert_eq!(pl.spells_cast, 2, "two real casts");
    g.check_invariants().unwrap();
}

// ================= Linked cards =================

#[test]
fn linked_spells_discard_each_other_across_turns() {
    let mut g = game(7);
    put_on_board(&mut g, 0, "Wandering Willbreaker", false);
    // the sleeper soaks up any targeted spell; K'Thir sits behind it
    g.players[0].board.push(sleeper(&g.data.cards, "Microbot", 50));
    put_on_board(&mut g, 0, "Cutthroat K'Thir", false);
    act(&mut g, 0, Action::Sell(0));
    let pl = &g.players[0];
    assert_eq!(pl.hand.len(), 2, "two random Tavern spells");
    let group = pl.hand[0].linked & 0x3fff;
    assert!(group != 0, "the two spells share a link id");
    assert!(pl.hand.iter().all(|m| m.linked & 0x3fff == group && m.linked & 0x8000 == 0));
    // the link lasts across turns
    end_all_turns(&mut g);
    turn_of(&mut g, 0);
    play(&mut g, 0, 0, 0);
    settle_prompts(&mut g, 0);
    let pl = &g.players[0];
    assert!(pl.hand.iter().all(|m| m.linked == 0), "playing one discards the other");
    assert_eq!(pl.discarded, 1, "a real discard");
    assert_eq!(stats(&pl.board[1]), (8, 8), "K'Thir saw the linked discard");
    assert_eq!(pl.deity.map(|d| (d.attack, d.health)), Some((5, 5)));
    g.check_invariants().unwrap();
}

#[test]
fn a_golden_willbreaker_links_four_and_two_plays_clear_the_rest() {
    let mut g = game(8);
    put_on_board(&mut g, 0, "Wandering Willbreaker", true);
    g.players[0].board.push(sleeper(&g.data.cards, "Microbot", 50));
    put_on_board(&mut g, 0, "Cutthroat K'Thir", false);
    act(&mut g, 0, Action::Sell(0));
    let group = g.players[0].hand[0].linked & 0x3fff;
    assert_eq!(g.players[0].hand.len(), 4, "golden links four");
    assert!(g.players[0].hand.iter().all(|m| m.linked & 0x3fff == group && m.linked & 0x8000 != 0));
    // playing one only marks the group: nothing is discarded yet
    play(&mut g, 0, 0, 0);
    settle_prompts(&mut g, 0);
    let linked_left = g.players[0].hand.iter().filter(|m| m.linked != 0).count();
    assert_eq!(linked_left, 3, "the first play discards nothing");
    assert_eq!(g.players[0].discarded, 0);
    // the second play discards the other two
    let h = g.players[0].hand.iter().position(|m| m.linked != 0).unwrap() as u8;
    play(&mut g, 0, h, 0);
    settle_prompts(&mut g, 0);
    let pl = &g.players[0];
    assert!(pl.hand.iter().all(|m| m.linked == 0));
    assert_eq!(pl.discarded, 2);
    assert_eq!(stats(&pl.board[1]), (12, 12), "K'Thir saw both discards");
    g.check_invariants().unwrap();
}

#[test]
fn faceless_operative_links_two_aberrations() {
    let mut g = game(9);
    put_on_board(&mut g, 0, "Faceless Operative", false);
    act(&mut g, 0, Action::Sell(0));
    let pl = &g.players[0];
    assert_eq!(pl.hand.len(), 2, "two random Aberrations");
    assert!(pl.hand.iter().all(|m| {
        let tribes = g.data.cards.get(m.card).tribes;
        tribes.has(bgcore::cards::Tribe::Aberration) && m.linked != 0
    }));
    play(&mut g, 0, 0, 0);
    let pl = &g.players[0];
    assert_eq!(pl.board.len(), 1, "the played Aberration lands");
    assert!(pl.hand.is_empty(), "playing it discards the other");
    assert_eq!(pl.discarded, 1);
    g.check_invariants().unwrap();
}

// ================= Mysterious K'Thir =================

#[test]
fn mysterious_kthir_discards_only_tavern_spells_and_grows_per_discard() {
    let mut g = game(12);
    put_on_board(&mut g, 0, "Mysterious K'Thir", false);
    let mut filler = mk(&g.data.cards, "Microbot", false);
    filler.attack = 0;
    filler.health = 50;
    g.players[0].board.push(filler);
    g.give(0, "Blood Gem").unwrap();
    g.give(0, "Sludge Corrosion").unwrap();
    g.give(0, "Triple Reward").unwrap();
    g.give(0, "Corrupted Coin").unwrap();
    turn_of(&mut g, 0);
    g.step(Action::EndTurn).unwrap();
    let pl = &g.players[0];
    // hand order was Gem, Sludge, Reward, Coin: only the two Tavern spells go
    assert_eq!(pl.hand.len(), 2, "the Blood Gem and the Triple Reward stay");
    assert_eq!(name(&g, &pl.hand[0]), "Blood Gem");
    assert_eq!(name(&g, &pl.hand[1]), "Triple Reward");
    // two discards: +7/+7 each, plus Sludge Corrosion casting twice (+1/+1 a cast)
    assert_eq!(stats(&pl.board[0]), (7 + 14 + 2, 7 + 14 + 2), "K'Thir");
    assert_eq!(stats(&pl.board[1]), (2, 52), "the filler only sees the two Sludge casts");
    assert_eq!(pl.extra_max_gold, 2, "the Coin was a real discard too");
    assert_eq!(pl.spells_cast, 2);
    assert_eq!(pl.discarded, 2);
    g.check_invariants().unwrap();
}

#[test]
fn mysterious_kthir_passes_over_a_lockbox() {
    // A Lockbox is a spell in hand but not a Tavern spell, so K'Thir's "discard
    // your 3 left-most Tavern spells" skips it and takes the Sludge Corrosion
    // behind it (`Hearthstone_2026_10_28_22_11_50-g6` turn 8 step 41: the
    // log's hand keeps the Lockbox and loses the Sludge Corrosions).
    let mut g = game(20);
    put_on_board(&mut g, 0, "Mysterious K'Thir", false);
    g.players[0].board.push(wall(&g.data.cards));
    g.give(0, "Lockbox").unwrap();
    g.give(0, "Sludge Corrosion").unwrap();
    turn_of(&mut g, 0);
    g.step(Action::EndTurn).unwrap();
    let pl = &g.players[0];
    assert_eq!(pl.hand.len(), 1, "only the Sludge Corrosion went");
    assert_eq!(name(&g, &pl.hand[0]), "Lockbox");
    // one discard (+7/+7) and the two casts the discarded Sludge makes
    assert_eq!(stats(&pl.board[0]), (7 + 7 + 2, 7 + 7 + 2), "K'Thir");
    assert_eq!(pl.discarded, 1);
    assert_eq!(pl.spells_cast, 2, "Sludge Corrosion casts twice");
    g.check_invariants().unwrap();
}

// ================= The Shadow of Doubt =================

#[test]
fn the_shadow_of_doubt_sees_every_hand_insert() {
    let mut g = game(13);
    put_on_board(&mut g, 0, "The Shadow of Doubt", false);
    put_on_board(&mut g, 0, "Mangled Bandit", false);
    let deity = |g: &Game| g.players[0].deity.map(|d| (d.attack, d.health)).unwrap();
    assert_eq!(deity(&g), (1, 1));
    // a bought minion
    set_shop(&mut g, 0, &["Wolf Pup"]);
    act(&mut g, 0, Action::Buy(0));
    assert_eq!(deity(&g), (5, 6), "a buy");
    // Blood Gems from the Bandit's reward (the discard itself adds nothing)
    g.give(0, "Blood Gem").unwrap();
    assert_eq!(deity(&g), (9, 11), "the given Gem");
    act(&mut g, 0, Action::Activate(1));
    let gem = g.players[0].hand.iter().position(|m| name(&g, m) == "Blood Gem").unwrap() as u8;
    act(&mut g, 0, Action::Discard(gem));
    assert_eq!(deity(&g), (21, 26), "three reward Gems");
    assert_eq!(gems_in_hand(&g, 0), 3);
    // a Triple Reward from playing a golden minion (its Battlecry is +4/+2)
    put_in_hand(&mut g, 0, "Joyous", true);
    let h = g.players[0].hand.iter().position(|m| name(&g, m) == "Joyous").unwrap() as u8;
    play(&mut g, 0, h, 2);
    assert_eq!(deity(&g), (29, 33), "the golden's +4/+2 and the Triple Reward's +4/+5");
    assert!(g.players[0].hand.iter().any(|m| name(&g, m) == "Triple Reward"));
    // a Discover pick
    g.discover_tier7_minion(0);
    act(&mut g, 0, Action::Choose(0));
    assert_eq!(deity(&g), (33, 38));
    g.check_invariants().unwrap();
}

// ================= Hidden numbers =================

#[test]
fn parasitic_fleshling_scales_with_every_card_discarded_this_game() {
    let mut g = game(14);
    let mut leftmost = mk(&g.data.cards, "Microbot", false);
    leftmost.attack = 0;
    leftmost.health = 50;
    g.players[0].board.push(leftmost);
    put_on_board(&mut g, 0, "Parasitic Fleshling", false);
    put_on_board(&mut g, 0, "Mangled Bandit", false);
    g.give(0, "Blood Gem").unwrap();
    act(&mut g, 0, Action::Activate(2));
    act(&mut g, 0, Action::Discard(0));
    turn_of(&mut g, 0);
    g.step(Action::EndTurn).unwrap();
    // one discard this game: +2/+2 base and +2/+2 more, doubled nothing
    assert_eq!(stats(&g.players[0].board[0]), (4, 54), "(1 + 1 discard) x +2/+2");
    end_all_turns(&mut g);
    turn_of(&mut g, 0);
    g.give(0, "Blood Gem").unwrap();
    act(&mut g, 0, Action::Activate(2));
    act(&mut g, 0, Action::Discard(0));
    turn_of(&mut g, 0);
    g.step(Action::EndTurn).unwrap();
    assert_eq!(stats(&g.players[0].board[0]), (10, 60), "(1 + 2 discards) x +2/+2 more");
    g.check_invariants().unwrap();
}

#[test]
fn faceless_converter_scales_with_tavern_spells_cast_this_game() {
    let db = db();
    let counters = [
        Counters {
            deity: Some(deity_of(&db, "C'Thun", 1, 1, false)),
            spells_cast: 3,
            ..Counters::default()
        },
        Counters::default(),
    ];
    let mut killer = mk(&db, "Microbot", false);
    killer.attack = 50;
    killer.health = 1000;
    let (out, _) = run(&db, &[mk(&db, "Faceless Converter", false)], &[killer], counters, 8, 5);
    // +2/+1 base and +2/+1 per Tavern spell: (2 + 6, 1 + 3)
    assert_eq!(out.sides[0].deity_gain, (8, 4));
}

#[test]
fn dark_puppeteers_deathrattle_raises_the_spell_bonus_for_good() {
    let db = db();
    let mut killer = mk(&db, "Microbot", false);
    killer.attack = 50;
    killer.health = 1000;
    let (out, _) = run(&db, &[mk(&db, "Dark Puppeteer", false)], &[killer], [Counters::default(); 2], 8, 5);
    assert_eq!(out.sides[0].spell_bonus_gain, (0, 4));
}

#[test]
fn mindbending_recruiter_activates_for_zero_gold() {
    let mut g = game(15);
    put_on_board(&mut g, 0, "Mindbending Recruiter", false);
    g.give(0, "Blood Gem").unwrap();
    g.players[0].gold = 10;
    act(&mut g, 0, Action::Activate(0));
    act(&mut g, 0, Action::Discard(0));
    let pl = &g.players[0];
    assert_eq!(pl.gold, 10, "the Activate costs 0");
    assert_eq!(pl.hand.len(), 1);
    let got = name(&g, &pl.hand[0]);
    assert!(got == "Joyous" || got == "Zoatroid", "a tier 1 Aberration, got {got}");
    g.check_invariants().unwrap();
}

// ================= De-volition-ist =================

#[test]
fn de_volitionist_breaks_health_ties_at_random() {
    let db = db();
    // Foraging Bat over Half-Shell: Half-Shell has Taunt in the data
    let (tie1, tie2) = (sleeper(&db, "Microbot", 4), sleeper(&db, "Foraging Bat", 4));
    let fodder = taunt_sleeper(&db, "Sewer Rat", 1);
    // one swing only: it goes into the Taunt fodder, leaving the two
    // 4-Health sleepers tied for the after-attack trigger
    let (_, log) = run(
        &db,
        &[mk(&db, "De-volition-ist", false)],
        &[tie1, tie2, fodder],
        [Counters::default(); 2],
        1,
        5,
    );
    let deaths: Vec<CardId> = log.0.iter()
        .filter_map(|e| match e {
            Ev::Death { side: 1, card } => Some(*card),
            _ => None,
        })
        .collect();
    assert!(deaths.contains(&id(&db, "Sewer Rat")), "the swing kills the fodder");
    let tie_deaths = deaths.iter().filter(|&&c| c == id(&db, "Microbot") || c == id(&db, "Foraging Bat")).count();
    assert_eq!(tie_deaths, 1, "exactly one of the tied minions dies");
    assert!(log.0.iter().all(|e| !matches!(e, Ev::Death { side: 0, .. })), "nothing of ours dies");
}

#[test]
fn de_volitionist_fires_even_when_it_dies_attacking() {
    let db = db();
    let mut killer = mk(&db, "Microbot", false);
    killer.attack = 50;
    killer.health = 100;
    let (_, log) = run(
        &db,
        &[mk(&db, "De-volition-ist", false), wall(&db)],
        &[killer],
        [Counters::default(); 2],
        4,
        5,
    );
    assert!(log.0.iter().any(|e| matches!(e, Ev::Death { side: 0, card } if *card == id(&db, "De-volition-ist"))));
    // the killer swings twice more after the exchange: both snapshots show
    // 100 - 4 (the swing it took) - 4 (the after-attack trigger)
    let hits: Vec<&Minion> = log.0.iter()
        .filter_map(|e| match e {
            Ev::Attack { side: 1, minion, .. } => Some(minion),
            _ => None,
        })
        .collect();
    assert!(hits.len() >= 2, "the killer attacks again");
    assert!(hits[1..].iter().all(|m| m.health == 92), "the trigger fired after death");
}

#[test]
fn a_golden_de_volitionist_deals_double() {
    let db = db();
    let (tie1, tie2) = (sleeper(&db, "Microbot", 16), sleeper(&db, "Foraging Bat", 16));
    let fodder = taunt_sleeper(&db, "Sewer Rat", 1);
    let (_, log) = run(
        &db,
        &[mk(&db, "De-volition-ist", true)],
        &[tie1, tie2, fodder],
        [Counters::default(); 2],
        1,
        5,
    );
    let deaths: Vec<CardId> = log.0.iter()
        .filter_map(|e| match e {
            Ev::Death { side: 1, card } => Some(*card),
            _ => None,
        })
        .collect();
    // golden: 8 attack, so 16 damage, enough to kill a tied minion outright
    assert_eq!(deaths.iter().filter(|&&c| c == id(&db, "Microbot") || c == id(&db, "Foraging Bat")).count(), 1);
}

// ================= Tokens and the shop =================

#[test]
fn zoatroid_sells_for_a_tentacle() {
    let mut g = game(16);
    put_on_board(&mut g, 0, "Zoatroid", false);
    act(&mut g, 0, Action::Sell(0));
    let pl = &g.players[0];
    assert_eq!(pl.hand.len(), 1);
    assert_eq!(name(&g, &pl.hand[0]), "Aberrant Tentacle");
    assert_eq!(stats(&pl.hand[0]), (0, 2));
    assert!(pl.hand[0].keywords.has(Keywords::TAUNT));
    assert!(g.data.cards.get(pl.hand[0].card).tribes.has(bgcore::cards::Tribe::Aberration));
    g.check_invariants().unwrap();
}
