//! The Undead tribe, with the real cards from cards.toml: the Undead bonus
//! "wherever they are", Reborn in fights and in the shop, the Snazzy Phantom
//! chain, Dead Bellringer, Butchering, Maw Caster, Tomb Turning, Mummifier,
//! Plaguerunner, Deathly Striker, Stitched Salvager, Lichling Hoarder, Drustfallen
//! Butcher, Eternal Knight and its friends, Barrier Banshee, Prosthetic Hand.

use std::sync::Arc;

use bgcore::fight::{Watch, fight_with};
use bgcore::{Action, CardDb, CardId, Counters, Data, Game, Keywords, Minion, Rng, Tribe};

fn db() -> CardDb {
    Data::bundled().cards
}

fn lobby(ts: &[Tribe]) -> bgcore::Tribes {
    ts.iter().copied().fold(bgcore::Tribes::NONE, |m, t| m.with(t))
}

fn game(seed: u64) -> Game {
    Game::new_with_lobby(Arc::new(Data::bundled()), seed, lobby(&[Tribe::Beast, Tribe::Mech, Tribe::Dragon, Tribe::Murloc, Tribe::Undead]))
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

/// A minion that kills almost anything and never dies.
fn killer(db: &CardDb) -> Minion {
    let mut m = mk(db, "Microbot", false);
    m.attack = 50;
    m.health = 2000;
    m
}

/// A minion that never attacks and takes a long time to kill.
fn wall(db: &CardDb) -> Minion {
    let mut m = mk(db, "Microbot", false);
    m.attack = 0;
    m.health = 2000;
    m
}

/// Dies to one hit, never attacks, and is hit before anything without Taunt.
fn fodder(db: &CardDb) -> Minion {
    let mut m = mk(db, "Microbot", false);
    m.attack = 0;
    m.health = 1;
    m.keywords.set(Keywords::TAUNT);
    m
}

/// The `i`th of several different fodder minions (so three of them in hand
/// and on the board never make a triple).
fn fodder_n(db: &CardDb, i: usize) -> Minion {
    let names = ["Microbot", "Foraging Bat", "Beetle", "Half-Shell", "Sewer Rat"];
    let mut m = mk(db, names[i % names.len()], false);
    m.attack = 0;
    m.health = 1;
    m.keywords.set(Keywords::TAUNT);
    m
}

fn with_health(mut m: Minion, health: i32) -> Minion {
    m.health = health;
    m
}

fn taunt(mut m: Minion) -> Minion {
    m.keywords.set(Keywords::TAUNT);
    m
}

#[derive(Clone, Debug, PartialEq)]
enum Ev {
    Attack { side: usize, card: CardId, minion: Minion },
    Death { side: usize, card: CardId },
    Summon { side: usize, card: CardId, minion: Minion },
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
    fn summon(&mut self, side: usize, _pos: usize, m: &Minion) {
        self.0.push(Ev::Summon { side, card: m.card, minion: *m });
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

fn summons(log: &Log, side: usize) -> Vec<(CardId, Minion)> {
    log.0
        .iter()
        .filter_map(|e| match e {
            Ev::Summon { side: s, card, minion } if *s == side => Some((*card, *minion)),
            _ => None,
        })
        .collect()
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

fn legal(g: &mut Game, p: usize, a: Action) -> bool {
    turn_of(g, p);
    g.is_legal(a)
}

fn end_all_turns(g: &mut Game) {
    let t = g.turn;
    while g.turn == t {
        if !answer(g) {
            g.step(Action::EndTurn).unwrap();
        }
    }
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

fn board_names(g: &Game, p: usize) -> Vec<String> {
    g.players[p].board.iter().map(|m| name(g, m)).collect()
}

fn play(g: &mut Game, p: usize, hand: u8, pos: u8) {
    act(g, p, Action::Play { hand, pos });
}

/// Give a generated card (a spell or a token) and play it from hand.
fn cast(g: &mut Game, spell: &str) {
    g.give(0, spell).unwrap();
    let h = g.players[0].hand.len() as u8 - 1;
    play(g, 0, h, 0);
}

fn pos_of(g: &Game, p: usize, card: &str) -> usize {
    let c = id(&g.data.cards, card);
    g.players[p].board.iter().position(|m| m.card == c).expect(card)
}

// ================= The Undead bonus, wherever they are =================

#[test]
fn undead_bonus_reaches_the_board_the_hand_the_tavern_and_later_undead() {
    let mut g = game(1);
    put_on_board(&mut g, 0, "Risen Rider", false); // 2/1
    put_in_hand(&mut g, 0, "Mummifier", false); // 5/2
    put_in_hand(&mut g, 0, "Nerubian Deathswarmer", false); // 1/4
    set_shop(&mut g, 0, &["Cadaver Caretaker", "Wolf Pup"]); // 3/3, a Beast
    let pup = stats(&g.players[0].shop[1]);
    // Deathswarmer is played at position 1: +1 Attack to every Undead.
    play(&mut g, 0, 1, 1);
    assert_eq!(g.players[0].undead_bonus, (1, 0));
    assert_eq!(stats(&g.players[0].board[0]), (3, 1), "Risen Rider on the board");
    assert_eq!(stats(&g.players[0].board[1]), (2, 4), "Deathswarmer itself");
    assert_eq!(stats(&g.players[0].hand[0]), (6, 2), "Mummifier in the hand");
    assert_eq!(stats(&g.players[0].shop[0]), (4, 3), "Cadaver Caretaker in the Tavern");
    assert_eq!(stats(&g.players[0].shop[1]), pup, "a Beast is not Undead");
    // Undead that show up later carry it: a generated one, a drawn one.
    g.give(0, "Harmless Bonehead").unwrap();
    assert_eq!(stats(g.players[0].hand.last().unwrap()), (2, 1));
    let c = id(&g.data.cards, "Eternal Knight");
    let m = g.players[0].make_card(&g.data.cards, c, false, 0);
    assert_eq!(stats(&m), (5, 2));
    g.check_invariants().unwrap();

    // Golden Deathswarmer gives +2.
    let mut g = game(2);
    put_on_board(&mut g, 0, "Risen Rider", false);
    put_in_hand(&mut g, 0, "Nerubian Deathswarmer", true);
    play(&mut g, 0, 0, 1);
    // the Triple Reward went to the hand as a spell; no Discover to answer
    assert_eq!(g.players[0].undead_bonus, (2, 0));
    assert_eq!(stats(&g.players[0].board[0]), (4, 1));
}

#[test]
fn undead_made_in_a_fight_carry_the_bonus() {
    let db = db();
    let counters = Counters { undead_attack: 3, undead_health: 1, ..Counters::default() };
    // Cadaver Caretaker's three Skeletons: 1/1 plus the bonus.
    let victim = with_health(taunt(mk(&db, "Cadaver Caretaker", false)), 1);
    let (_, log) = run(&db, &[victim], &[killer(&db), killer(&db)], [counters, Counters::default()], 1, 1);
    let skeletons = summons(&log, 0);
    assert_eq!(skeletons.len(), 3);
    for (c, m) in skeletons {
        assert_eq!(c, id(&db, "Skeleton"));
        assert_eq!(stats(&m), (4, 2));
    }
    // A Reborn Undead comes back as its plain card plus the bonus, with 1
    // Health plus the Health bonus: Risen Rider is 2/1, back as 5/2.
    let rider = with_health(mk(&db, "Risen Rider", false), 1);
    let (_, log) = run(&db, &[rider], &[killer(&db), killer(&db)], [counters, Counters::default()], 1, 1);
    let back = summons(&log, 0);
    assert_eq!(back.len(), 1);
    assert_eq!(stats(&back[0].1), (5, 2));
    assert!(!back[0].1.keywords.has(Keywords::REBORN));
    // A non-Undead is not touched.
    let (_, log) = run(
        &db,
        &[with_health(taunt(mk(&db, "Tasty Lobster", false)), 1), mk(&db, "Microbot", false)],
        &[killer(&db), killer(&db), killer(&db)],
        [counters, Counters::default()],
        1,
        1,
    );
    assert!(summons(&log, 0).is_empty());
}

#[test]
fn a_hand_undead_summoned_in_a_fight_keeps_the_bonus_it_had() {
    // Hand cards already carry the bonus (baked): the fight must not add it
    // a second time to a Reborn-free Undead copied back from the hand.
    let db = db();
    let counters = Counters { undead_attack: 4, ..Counters::default() };
    let striker = with_health(taunt(mk(&db, "Deathly Striker", false)), 1);
    let (out, _) = run(&db, &[striker], &[killer(&db), killer(&db)], [counters, Counters::default()], 1, 1);
    assert_eq!(out.sides[0].undead_gain, (0, 0));
}

// ================= Reborn =================

#[test]
fn reborn_in_a_fight_is_a_full_death_then_a_plain_minion_with_one_health() {
    let db = db();
    // Scarlet Skull with big buffs next to a Risen Rider: its Deathrattle
    // fires (the Rider gets +1/+2) and it comes back as the plain 2/1 card
    // without Reborn, in the same slot.
    let mut skull = mk(&db, "Scarlet Skull", false);
    skull.attack = 30;
    skull.health = 1;
    skull.keywords.set(Keywords::TAUNT);
    let rider = mk(&db, "Risen Rider", false);
    let (_, log) = run(&db, &[skull, rider], &[killer(&db), killer(&db), killer(&db)], [Counters::default(); 2], 1, 3);
    let deaths: Vec<_> = log.0.iter().filter(|e| matches!(e, Ev::Death { side: 0, .. })).collect();
    assert_eq!(deaths.len(), 1);
    let back = summons(&log, 0);
    assert_eq!(back.len(), 1);
    assert_eq!(back[0].0, id(&db, "Scarlet Skull"));
    assert_eq!(stats(&back[0].1), (2, 1));
    assert!(!back[0].1.keywords.has(Keywords::REBORN));
}

#[test]
fn a_reborn_minion_dies_for_real_the_second_time() {
    let db = db();
    let rider = with_health(mk(&db, "Risen Rider", false), 1);
    let (out, log) = run(&db, &[rider], &[killer(&db), killer(&db)], [Counters::default(); 2], 10, 3);
    let deaths = log.0.iter().filter(|e| matches!(e, Ev::Death { side: 0, .. })).count();
    assert_eq!(deaths, 2);
    assert_eq!(out.winner, Some(1));
}

#[test]
fn reborn_counts_for_eternal_knight_and_avenge() {
    let db = db();
    // A Reborn Knight dies twice and so counts twice; each living Knight
    // grows by +4/+2 per death.
    let mut knight = mk(&db, "Eternal Knight", false);
    knight.keywords.set(Keywords::REBORN);
    knight.keywords.set(Keywords::TAUNT);
    knight.health = 1;
    let (out, _) = run(&db, &[knight], &[killer(&db), killer(&db)], [Counters::default(); 2], 6, 3);
    assert_eq!(out.sides[0].knight_deaths, 2);
}

// ================= Dead Bellringer, Reborn in the shop =================

fn set_gold(g: &mut Game, p: usize, gold: u8) {
    g.players[p].gold = gold;
}

#[test]
fn bellringer_reborn_destroys_in_the_shop_with_deathrattles_and_one_health_that_stays() {
    let mut g = game(5);
    g.players[0].undead_bonus = (0, 2);
    put_on_board(&mut g, 0, "Dead Bellringer", false); // 3/8
    put_on_board(&mut g, 0, "Cadaver Caretaker", false); // 3/5 with the bonus
    set_gold(&mut g, 0, 5);
    assert_eq!(stats(&g.players[0].board[1]), (3, 5));
    act(&mut g, 0, Action::Activate(0));
    // the only other Undead: it is picked for you
    assert!(g.players[0].pending_target.is_none());
    // Caretaker died (three Skeletons in its slot, which triple into a
    // Golden one in the hand), then came back plain with 1 Health plus the
    // Health bonus; Bellringer got +4/+4.
    assert_eq!(board_names(&g, 0), ["Dead Bellringer", "Cadaver Caretaker"]);
    assert_eq!(stats(&g.players[0].board[0]), (7, 12));
    assert_eq!(stats(&g.players[0].board[1]), (3, 3), "plain 3/3 with 1 Health plus +0/+2");
    let hand: Vec<String> = g.players[0]
        .hand
        .iter()
        .map(|m| g.data.cards.get(m.card).name.clone())
        .collect();
    assert_eq!(hand, ["Skeleton"], "the three Skeletons tripled");
    assert!(!g.players[0].board[1].keywords.has(Keywords::REBORN));
    assert_eq!(g.players[0].gold, 4);
    g.check_invariants().unwrap();
    // it never heals: the 3 Health goes into the next turns and fights
    end_all_turns(&mut g);
    assert_eq!(stats(&g.players[0].board[1]), (3, 3));
    g.check_invariants().unwrap();

    // Bellringer is once per turn and golden gains +8/+8.
    let mut g = game(6);
    put_on_board(&mut g, 0, "Dead Bellringer", true);
    put_on_board(&mut g, 0, "Harmless Bonehead", false);
    put_on_board(&mut g, 0, "Harmless Bonehead", false);
    set_gold(&mut g, 0, 5);
    act(&mut g, 0, Action::Activate(0));
    act(&mut g, 0, Action::Target(1));
    assert_eq!(stats(&g.players[0].board[0]), (6 + 8, 12 + 8));
    assert!(!legal(&mut g, 0, Action::Activate(0)), "once per turn");
}

#[test]
fn bellringer_needs_a_different_undead_and_never_picks_a_copy() {
    let mut g = game(7);
    put_on_board(&mut g, 0, "Dead Bellringer", false);
    put_on_board(&mut g, 0, "Dead Bellringer", false);
    set_gold(&mut g, 0, 5);
    assert!(!legal(&mut g, 0, Action::Activate(0)), "only a copy of itself to point at");
    put_on_board(&mut g, 0, "Wolf Pup", false);
    assert!(!legal(&mut g, 0, Action::Activate(0)), "a Beast is not an Undead");
    put_on_board(&mut g, 0, "Risen Rider", false);
    put_on_board(&mut g, 0, "Harmless Bonehead", false);
    assert!(legal(&mut g, 0, Action::Activate(0)));
    act(&mut g, 0, Action::Activate(0));
    assert!(legal(&mut g, 0, Action::Target(3)));
    assert!(legal(&mut g, 0, Action::Target(4)));
    assert!(!legal(&mut g, 0, Action::Target(1)), "the other Bellringer is a copy");
    assert!(!legal(&mut g, 0, Action::Target(0)), "itself");
    assert!(!legal(&mut g, 0, Action::Target(2)), "a Beast");
}

#[test]
fn mummifier_never_gives_reborn_to_another_mummifier() {
    // A Mummifier dies in the shop: "a different friendly Undead" is not the
    // other Mummifier, whoever the random pick lands on.
    for seed in 0..12 {
        let mut g = game(100 + seed);
        put_on_board(&mut g, 0, "Dead Bellringer", false);
        put_on_board(&mut g, 0, "Mummifier", false);
        put_on_board(&mut g, 0, "Mummifier", false);
        put_on_board(&mut g, 0, "Harmless Bonehead", false);
        set_gold(&mut g, 0, 5);
        act(&mut g, 0, Action::Activate(0));
        act(&mut g, 0, Action::Target(1));
        // the first Mummifier came back plain (its Reborn used); the second
        // one never got Reborn
        let second = pos_of(&g, 0, "Mummifier");
        let mummifiers: Vec<&Minion> =
            g.players[0].board.iter().filter(|m| m.card == id(&g.data.cards, "Mummifier")).collect();
        assert_eq!(mummifiers.len(), 2);
        assert!(mummifiers.iter().all(|m| !m.keywords.has(Keywords::REBORN)), "seed {seed}");
        let _ = second;
        // exactly one of Bellringer and Bonehead got Reborn
        let reborn = g.players[0]
            .board
            .iter()
            .filter(|m| m.keywords.has(Keywords::REBORN))
            .count();
        assert_eq!(reborn, 1, "seed {seed}");
        g.check_invariants().unwrap();
    }
}

#[test]
fn mummifier_with_nobody_to_give_it_to_is_wasted() {
    let db = db();
    let m = with_health(taunt(mk(&db, "Mummifier", false)), 1);
    let (out, log) = run(&db, &[m], &[killer(&db), killer(&db)], [Counters::default(); 2], 1, 1);
    assert_eq!(out.winner, Some(1));
    assert!(summons(&log, 0).is_empty());
    // Golden gives two different Undead Reborn.
    let g = taunt(with_health(mk(&db, "Mummifier", true), 1));
    let bone = with_health(mk(&db, "Harmless Bonehead", false), 1);
    let bone2 = with_health(mk(&db, "Cadaver Caretaker", false), 1);
    let enemies = [killer(&db), killer(&db), killer(&db), killer(&db)];
    let (_, log) = run(&db, &[g, bone, bone2], &enemies, [Counters::default(); 2], 40, 5);
    // both are killed eventually and both come back through Reborn
    let reborn_cards: Vec<CardId> = log
        .0
        .iter()
        .filter_map(|e| match e {
            Ev::Summon { card, minion, .. }
                if *card == id(&db, "Harmless Bonehead") || *card == id(&db, "Cadaver Caretaker") =>
            {
                // a Reborn minion has 1 Health, not the full number
                (minion.health == 1).then_some(*card)
            }
            _ => None,
        })
        .collect();
    assert!(reborn_cards.contains(&id(&db, "Harmless Bonehead")));
    assert!(reborn_cards.contains(&id(&db, "Cadaver Caretaker")));
}

// ================= Butchering =================

#[test]
fn butchering_destroys_an_undead_and_gives_the_undead_attack_everywhere() {
    let mut g = game(8);
    put_on_board(&mut g, 0, "Harmless Bonehead", false); // 1/1, two Skeletons
    put_on_board(&mut g, 0, "Wolf Pup", false);
    put_on_board(&mut g, 0, "Risen Rider", false); // 2/1
    put_in_hand(&mut g, 0, "Mummifier", false); // 5/2
    set_shop(&mut g, 0, &["Cadaver Caretaker"]);
    cast(&mut g, "Butchering");
    act(&mut g, 0, Action::Target(0));
    assert_eq!(g.players[0].undead_bonus, (8, 0));
    // Bonehead is gone and its Skeletons stand where it stood, with the +8.
    assert_eq!(board_names(&g, 0), ["Skeleton", "Skeleton", "Wolf Pup", "Risen Rider"]);
    assert_eq!(stats(&g.players[0].board[0]), (9, 1));
    assert_eq!(stats(&g.players[0].board[3]), (10, 1));
    assert_eq!(stats(&g.players[0].hand[0]), (13, 2), "in the hand");
    assert_eq!(stats(&g.players[0].shop[0]), (11, 3), "in the Tavern");
    g.check_invariants().unwrap();

    // With a Health spell bonus it gives +8/+N.
    let mut g = game(9);
    g.players[0].spell_bonus = (0, 2);
    put_on_board(&mut g, 0, "Harmless Bonehead", false);
    put_on_board(&mut g, 0, "Risen Rider", false);
    cast(&mut g, "Butchering");
    act(&mut g, 0, Action::Target(1));
    assert_eq!(g.players[0].undead_bonus, (8, 2));
    // the Rider was destroyed and came back through Reborn with 1 Health
    // plus the bonus
    let rider = g.players[0].board.iter().find(|m| m.card == id(&g.data.cards, "Risen Rider")).unwrap();
    assert_eq!(stats(rider), (10, 3));
    assert!(!rider.keywords.has(Keywords::REBORN));

    // It can't be cast without a friendly Undead.
    let mut g = game(10);
    put_on_board(&mut g, 0, "Wolf Pup", false);
    g.give(0, "Butchering").unwrap();
    assert!(!legal(&mut g, 0, Action::Play { hand: 0, pos: 0 }));
    put_on_board(&mut g, 0, "Risen Rider", false);
    assert!(legal(&mut g, 0, Action::Play { hand: 0, pos: 0 }));
}

#[test]
fn butchering_counts_the_spell_power_the_undeads_own_death_gives() {
    // docs/rules/design.md "Butchering and spell power" (Era, 2026-11-02): the +8 Attack
    // is a Tavern spell number, and the Undead dies first, so a death that
    // gives the Tavern spells more (Friendly Geist's +1 Attack) is inside it.
    let mut g = game(12);
    put_on_board(&mut g, 0, "Friendly Geist", false); // 6/3, deathrattle: +1 spell Attack
    put_on_board(&mut g, 0, "Risen Rider", false); // 2/1
    cast(&mut g, "Butchering");
    act(&mut g, 0, Action::Target(0));
    assert_eq!(g.players[0].spell_bonus, (1, 0), "the Geist's deathrattle");
    assert_eq!(g.players[0].undead_bonus, (9, 0), "+8 plus the spell power it just gave");
    assert_eq!(stats(&g.players[0].board[0]), (11, 1), "the Rider has the +9");
    g.check_invariants().unwrap();
}

#[test]
fn butchering_cast_twice_by_balinda_counts_twice_for_forsaken_weaver() {
    let mut g = game(11);
    put_on_board(&mut g, 0, "Forsaken Weaver", false);
    put_on_board(&mut g, 0, "Balinda Stonehearth", false);
    put_on_board(&mut g, 0, "Risen Rider", false);
    // a spell Balinda casts twice: Weaver reacts to both casts
    g.give(0, "Tavern Dish Banana").unwrap();
    play(&mut g, 0, 0, 0);
    act(&mut g, 0, Action::Target(1));
    assert_eq!(g.players[0].undead_bonus, (6, 0));
    // a golden Weaver gives +6 per cast
    let mut g = game(12);
    put_on_board(&mut g, 0, "Forsaken Weaver", true);
    cast(&mut g, "Tavern Coin");
    assert_eq!(g.players[0].undead_bonus, (6, 0));
    assert_eq!(stats(&g.players[0].board[0]), (6 + 6, 16));
}

// ================= Maw Caster =================

#[test]
fn maw_caster_destroys_a_chosen_undead_and_discovers_an_undead() {
    let mut g = game(13);
    g.players[0].tier = 3;
    put_on_board(&mut g, 0, "Harmless Bonehead", false);
    put_in_hand(&mut g, 0, "Maw Caster", false);
    play(&mut g, 0, 0, 1);
    // Bonehead was destroyed (its Skeletons are there) and a Discover opens
    assert_eq!(board_names(&g, 0), ["Skeleton", "Skeleton", "Maw Caster"]);
    let options = g.players[0].discover.clone();
    assert_eq!(options.len(), 3);
    for c in &options {
        let card = g.data.cards.get(*c);
        assert!(card.tribes.has(Tribe::Undead), "{} is not an Undead", card.name);
        assert!(card.tier <= 3);
    }
    act(&mut g, 0, Action::Choose(0));
    assert_eq!(g.players[0].hand.len(), 1);
    assert!(g.data.cards.get(g.players[0].hand[0].card).tribes.has(Tribe::Undead));
    g.check_invariants().unwrap();

    // Nobody else Undead on the board: nothing happens, no Discover.
    let mut g = game(14);
    put_on_board(&mut g, 0, "Wolf Pup", false);
    put_in_hand(&mut g, 0, "Maw Caster", false);
    play(&mut g, 0, 0, 1);
    assert!(g.players[0].discover.is_empty());
    assert_eq!(board_names(&g, 0), ["Wolf Pup", "Maw Caster"]);
}

// ================= Tomb Turning =================

#[test]
fn tomb_turning_discovers_an_undead_that_dies_if_played_this_turn() {
    let mut g = game(15);
    g.players[0].tier = 4;
    cast(&mut g, "Tomb Turning");
    let options = g.players[0].discover.clone();
    assert_eq!(options.len(), 3);
    for c in &options {
        let card = g.data.cards.get(*c);
        assert!(card.tribes.has(Tribe::Undead));
        assert!(card.tier <= 4);
    }
    act(&mut g, 0, Action::Choose(0));
    assert!(g.players[0].hand[0].doomed, "marked in hand");
    g.check_invariants().unwrap();

    // Played this turn it dies right after its Battlecry (Deathrattle and
    // Reborn run): a Bonehead leaves its two Skeletons.
    let mut g = game(16);
    put_in_hand(&mut g, 0, "Harmless Bonehead", false);
    g.players[0].hand[0].doomed = true;
    play(&mut g, 0, 0, 0);
    assert_eq!(board_names(&g, 0), ["Skeleton", "Skeleton"]);
    g.check_invariants().unwrap();

    // A Reborn one comes back once: Risen Rider.
    let mut g = game(17);
    put_in_hand(&mut g, 0, "Risen Rider", false);
    g.players[0].hand[0].doomed = true;
    play(&mut g, 0, 0, 0);
    assert_eq!(board_names(&g, 0), ["Risen Rider"]);
    assert_eq!(g.players[0].board[0].health, 1);
    assert!(!g.players[0].board[0].keywords.has(Keywords::REBORN));
    assert!(!g.players[0].board[0].doomed);

    // Kept in hand past the end of the turn it is a normal minion.
    let mut g = game(18);
    put_in_hand(&mut g, 0, "Harmless Bonehead", false);
    g.players[0].hand[0].doomed = true;
    end_all_turns(&mut g);
    assert!(!g.players[0].hand[0].doomed);
    play(&mut g, 0, 0, 0);
    assert_eq!(board_names(&g, 0), ["Harmless Bonehead"]);
}

#[test]
fn a_tomb_turning_maw_caster_discovers_before_it_dies() {
    let mut g = game(19);
    g.players[0].tier = 3;
    put_on_board(&mut g, 0, "Harmless Bonehead", false);
    put_in_hand(&mut g, 0, "Maw Caster", false);
    g.players[0].hand[0].doomed = true;
    play(&mut g, 0, 0, 1);
    // still waiting on the Discover: the Maw Caster is alive
    assert!(!g.players[0].discover.is_empty());
    assert!(board_names(&g, 0).contains(&"Maw Caster".to_string()));
    act(&mut g, 0, Action::Choose(1));
    assert_eq!(board_names(&g, 0), ["Skeleton", "Skeleton"]);
    assert_eq!(g.players[0].hand.len(), 1);
    g.check_invariants().unwrap();
}

// ================= Snazzy Phantom, Barrier Banshee =================

#[test]
fn the_snazzy_phantom_chain_end_to_end() {
    // The Undead bonus is already +10 Attack. Dead Bellringer destroys the
    // Rider: it comes back as 12/1 and Phantom, the right-most Undead,
    // gets +12/+12.
    let mut g = game(20);
    g.players[0].undead_bonus = (10, 0);
    put_on_board(&mut g, 0, "Risen Rider", false);
    put_on_board(&mut g, 0, "Dead Bellringer", false);
    put_on_board(&mut g, 0, "Snazzy Phantom", false);
    assert_eq!(stats(&g.players[0].board[0]), (12, 1));
    assert_eq!(stats(&g.players[0].board[2]), (16, 8));
    set_gold(&mut g, 0, 5);
    act(&mut g, 0, Action::Activate(1));
    act(&mut g, 0, Action::Target(0));
    assert_eq!(stats(&g.players[0].board[0]), (12, 1), "Reborn: 12 Attack, 1 Health");
    assert_eq!(stats(&g.players[0].board[2]), (16 + 12, 8 + 12), "Phantom gives the Attack it came back with");
    assert_eq!(stats(&g.players[0].board[1]), (13 + 4, 6 + 4), "Bellringer's own +4/+4");
    g.check_invariants().unwrap();

    // And with Butchering: the Undead dies first (docs/rules/design.md "Butchering and
    // spell power": its death's Tavern spell power is inside the +8), so the
    // Reborn copy comes back before the bonus lands, and the Phantom gains
    // the Attack it came back with before the +8 does.
    let mut g = game(21);
    put_on_board(&mut g, 0, "Risen Rider", false);
    put_on_board(&mut g, 0, "Snazzy Phantom", false);
    cast(&mut g, "Butchering");
    act(&mut g, 0, Action::Target(0));
    // The Rider comes back as 2/1, Phantom (6 + 2 = 8 Attack) gains 2/2 for
    // it, and then the +8 lands on both: 2 + 8 = 10, 8 + 8 = 16 Attack.
    assert_eq!(stats(&g.players[0].board[0]), (10, 1));
    assert_eq!(stats(&g.players[0].board[1]), (16, 10));
    g.players[0].board[0].keywords.set(Keywords::REBORN);
    cast(&mut g, "Butchering");
    act(&mut g, 0, Action::Target(0));
    // +8 more (16 in all): the Rider comes back with 10 Attack, the Phantom
    // gains that 10/10 and then the 8: 16 + 10 + 8 = 34 Attack, 10 + 10 = 20.
    assert_eq!(g.players[0].undead_bonus, (16, 0));
    assert_eq!(stats(&g.players[0].board[0]), (18, 1));
    assert_eq!(stats(&g.players[0].board[1]), (34, 20));
    g.check_invariants().unwrap();
}

#[test]
fn banshee_and_phantom_ignore_their_own_reborn() {
    let db = db();
    // Alone and Reborn, in a fight: it comes back as the plain card, with no
    // Banshee gain (it attacks as 8/1, not 16/9 with Divine Shield).
    let mut banshee = with_health(mk(&db, "Barrier Banshee", false), 1);
    banshee.keywords.set(Keywords::REBORN);
    let enemies = [killer(&db), wall(&db)];
    let (_, log) = run(&db, &[banshee], &enemies, [Counters::default(); 2], 3, 1);
    let attacks: Vec<&Minion> = log
        .0
        .iter()
        .filter_map(|e| match e {
            Ev::Attack { side: 0, minion, .. } => Some(minion),
            _ => None,
        })
        .collect();
    assert!(!attacks.is_empty());
    let after = attacks.last().unwrap();
    assert_eq!((after.attack, after.health), (8, 1));
    assert!(!after.keywords.has(Keywords::DIVINE_SHIELD));

    let mut phantom = with_health(mk(&db, "Snazzy Phantom", false), 1);
    phantom.keywords.set(Keywords::REBORN);
    let (_, log) = run(&db, &[phantom], &enemies, [Counters::default(); 2], 3, 1);
    let last = log
        .0
        .iter()
        .filter_map(|e| match e {
            Ev::Attack { side: 0, minion, .. } => Some(*minion),
            _ => None,
        })
        .next_back()
        .unwrap();
    assert_eq!((last.attack, last.health), (6, 1));
}

#[test]
fn banshee_gains_when_another_minion_is_reborn_and_phantom_too() {
    let db = db();
    let mut banshee = mk(&db, "Barrier Banshee", false);
    banshee.attack = 1;
    banshee.health = 1000;
    let rider = with_health(taunt(mk(&db, "Risen Rider", false)), 1);
    let (_, log) = run(&db, &[banshee, rider], &[killer(&db)], [Counters::default(); 2], 6, 1);
    let best = log
        .0
        .iter()
        .filter_map(|e| match e {
            Ev::Attack { side: 0, card, minion } if *card == id(&db, "Barrier Banshee") => Some(*minion),
            _ => None,
        })
        .next_back()
        .unwrap();
    assert_eq!(best.attack, 1 + 8, "+8/+8 after the Rider's Reborn");
    assert!(best.keywords.has(Keywords::DIVINE_SHIELD) || best.health > 900);

    // Golden Phantom doubles: +2X to the right-most Undead.
    let mut phantom = mk(&db, "Snazzy Phantom", true);
    phantom.attack = 1;
    phantom.health = 1000;
    let rider = with_health(taunt(mk(&db, "Risen Rider", false)), 1);
    let counters = Counters { undead_attack: 5, ..Counters::default() };
    let (_, log) = run(&db, &[rider, phantom], &[killer(&db)], [counters, Counters::default()], 6, 1);
    // Rider returns as 2 + 5 = 7 Attack; golden Phantom gives 14/14
    let best = log
        .0
        .iter()
        .filter_map(|e| match e {
            Ev::Attack { side: 0, card, minion } if *card == id(&db, "Snazzy Phantom") => Some(*minion),
            _ => None,
        })
        .next_back()
        .unwrap();
    assert_eq!(best.attack, 1 + 14);
}

// ================= Plaguerunner, Geist =================

#[test]
fn plaguerunner_gives_two_in_a_fight_and_four_outside_it() {
    let db = db();
    let run_dies = |golden: bool| {
        let victim = with_health(taunt(mk(&db, "Plaguerunner", golden)), 1);
        let (out, _) = run(&db, &[victim], &[killer(&db), killer(&db)], [Counters::default(); 2], 1, 1);
        out.sides[0].undead_gain
    };
    assert_eq!(run_dies(false), (2, 0));
    assert_eq!(run_dies(true), (4, 0));

    // In the shop, destroyed by Dead Bellringer: +4 (golden +8), and the
    // Plaguerunner comes back with the bonus too (4 + 4 Attack).
    let mut g = game(22);
    put_on_board(&mut g, 0, "Dead Bellringer", false);
    put_on_board(&mut g, 0, "Plaguerunner", false);
    set_gold(&mut g, 0, 5);
    act(&mut g, 0, Action::Activate(0));
    assert_eq!(g.players[0].undead_bonus, (4, 0));
    let pr = g.players[0].board.iter().find(|m| m.card == id(&g.data.cards, "Plaguerunner")).unwrap();
    assert_eq!(stats(pr), (4 + 4, 1));
    assert_eq!(stats(&g.players[0].board[0]), (3 + 4 + 4, 6 + 4), "Bellringer too");
    g.check_invariants().unwrap();

    let mut g = game(23);
    put_on_board(&mut g, 0, "Dead Bellringer", false);
    put_on_board(&mut g, 0, "Plaguerunner", true);
    set_gold(&mut g, 0, 5);
    act(&mut g, 0, Action::Activate(0));
    assert_eq!(g.players[0].undead_bonus, (8, 0));

    // After a real fight the bonus reaches the board, the hand and the Tavern.
    let mut g = game(24);
    let Some(bgcore::Opponent::Player(q)) = g.players[0].opponent else { panic!() };
    put_on_board(&mut g, 0, "Plaguerunner", false);
    g.players[0].board[0].health = 1;
    put_on_board(&mut g, 0, "Risen Rider", false);
    put_in_hand(&mut g, 0, "Mummifier", false);
    set_shop(&mut g, 0, &["Cadaver Caretaker"]);
    put_on_board(&mut g, q as usize, "Cage Gnawer", false);
    g.players[q as usize].board[0].attack = 50;
    g.players[q as usize].board[0].health = 2000;
    end_all_turns(&mut g);
    assert_eq!(g.players[0].undead_bonus.0, 2);
    // (a new turn rolled a new Tavern: only the hand and the board are checked)
    assert_eq!(stats(&g.players[0].hand[0]), (5 + 2, 2));
    let rider = g.players[0].board.iter().find(|m| m.card == id(&g.data.cards, "Risen Rider")).unwrap();
    assert_eq!(rider.attack, 2 + 2);
    g.check_invariants().unwrap();
}

#[test]
fn friendly_geist_adds_to_the_tavern_spell_bonus() {
    let db = db();
    for (golden, bonus) in [(false, 1), (true, 2)] {
        let victim = with_health(taunt(mk(&db, "Friendly Geist", golden)), 1);
        let (out, _) = run(&db, &[victim], &[killer(&db), killer(&db)], [Counters::default(); 2], 1, 1);
        assert_eq!(out.sides[0].spell_bonus_gain, (bonus, 0));
    }
}

// ================= Skeletons and Hands =================

#[test]
fn skeleton_and_hand_summons_and_their_golden_counts() {
    let db = db();
    let count = |name: &str, golden: bool, token: &str| {
        let victim = with_health(taunt(mk(&db, name, golden)), 1);
        let (_, log) = run(&db, &[victim], &[killer(&db), killer(&db)], [Counters::default(); 2], 1, 1);
        summons(&log, 0).iter().filter(|(c, _)| *c == id(&db, token)).count()
    };
    assert_eq!(count("Harmless Bonehead", false, "Skeleton"), 2);
    assert_eq!(count("Harmless Bonehead", true, "Skeleton"), 4);
    assert_eq!(count("Cadaver Caretaker", false, "Skeleton"), 3);
    assert_eq!(count("Cadaver Caretaker", true, "Skeleton"), 6);
    assert_eq!(count("Handless Forsaken", false, "Helping Hand"), 1);
    assert_eq!(count("Handless Forsaken", true, "Helping Hand"), 2);
    let hand = db.make(id(&db, "Helping Hand"), false);
    assert_eq!(stats(&hand), (2, 1));
    assert!(hand.keywords.has(Keywords::REBORN));
}

// ================= Deathly Striker =================

#[test]
fn deathly_striker_summons_what_it_got_this_fight_and_it_is_gone_afterwards() {
    let db = db();
    let counters = Counters { tier: 6, ..Counters::default() };
    let mut striker = mk(&db, "Deathly Striker", false);
    striker.attack = 0;
    striker.health = 8;
    let mut board = vec![striker];
    for _ in 0..4 {
        board.push(fodder(&db));
    }
    let enemies = [killer(&db), wall(&db)];
    // The four Microbots die first (Taunt), the fourth death is the Avenge:
    // a random Undead is got. The Striker is still alive after 4 attacks.
    let (out, _) = run(&db, &board, &enemies, [counters, Counters::default()], 4, 1);
    assert_eq!(out.sides[0].cards_gained().len(), 1);
    let got = out.sides[0].cards_gained()[0];
    assert!(db.get(got).tribes.has(Tribe::Undead));
    assert!(db.get(got).tier <= 6);
    // The fifth attack kills the Striker: it summons the Undead from hand
    // for this fight only, so it is not kept.
    let (out, log) = run(&db, &board, &enemies, [counters, Counters::default()], 5, 1);
    assert_eq!(out.sides[0].cards_gained().len(), 0, "summoned, so gone from the hand");
    let summoned = summons(&log, 0);
    assert!(!summoned.is_empty());
    assert!(db.get(summoned[0].0).tribes.has(Tribe::Undead));
    // Golden Striker gets two.
    let mut gold = mk(&db, "Deathly Striker", true);
    gold.attack = 0;
    gold.health = 8;
    let mut board = vec![gold];
    for _ in 0..4 {
        board.push(fodder(&db));
    }
    let (out, _) = run(&db, &board, &enemies, [counters, Counters::default()], 4, 1);
    assert_eq!(out.sides[0].cards_gained().len(), 2);
}

// ================= Stitched Salvager =================

#[test]
fn stitched_salvager_destroys_its_left_neighbour_and_summons_an_exact_copy_when_it_dies() {
    let db = db();
    // The victim carries buffs, Taunt and a Divine Shield: the copy has all
    // of them, with full Health.
    let mut victim = mk(&db, "Cadaver Caretaker", false);
    victim.attack = 20;
    victim.health = 22;
    victim.keywords.set(Keywords::DIVINE_SHIELD);
    let salvager = taunt(with_health(mk(&db, "Stitched Salvager", false), 10));
    let enemies = [killer(&db), wall(&db), wall(&db)];
    let (_, log) = run(&db, &[victim, salvager], &enemies, [Counters::default(); 2], 1, 1);
    let caretaker = id(&db, "Cadaver Caretaker");
    // Start of Combat: the Caretaker dies (Deathrattle: three Skeletons)
    let deaths: Vec<CardId> = log
        .0
        .iter()
        .filter_map(|e| match e {
            Ev::Death { side: 0, card } => Some(*card),
            _ => None,
        })
        .collect();
    assert_eq!(deaths[0], caretaker);
    let summoned = summons(&log, 0);
    assert_eq!(summoned.iter().filter(|(c, _)| *c == id(&db, "Skeleton")).count(), 3);
    // then the Salvager dies to the first attack: an exact copy appears
    let copy = summoned.iter().find(|(c, _)| *c == caretaker).expect("the copy").1;
    assert_eq!(stats(&copy), (20, 22));
    assert!(copy.keywords.has(Keywords::DIVINE_SHIELD));
}

#[test]
fn salvager_victims_own_reborn_runs_and_the_copy_keeps_reborn() {
    let db = db();
    let mut rider = mk(&db, "Risen Rider", false);
    rider.attack = 30;
    let salvager = taunt(with_health(mk(&db, "Stitched Salvager", false), 10));
    let enemies = [killer(&db), wall(&db), wall(&db)];
    let (_, log) = run(&db, &[rider, salvager], &enemies, [Counters::default(); 2], 1, 1);
    let rider_id = id(&db, "Risen Rider");
    let riders: Vec<Minion> =
        summons(&log, 0).into_iter().filter(|(c, _)| *c == rider_id).map(|(_, m)| m).collect();
    assert_eq!(riders.len(), 2);
    // Reborn at the Start of Combat: the plain card with 1 Health
    assert_eq!(stats(&riders[0]), (2, 1));
    assert!(!riders[0].keywords.has(Keywords::REBORN));
    // the copy when the Salvager died: buffs and Reborn, full Health
    assert_eq!(riders[1].attack, 30);
    assert!(riders[1].keywords.has(Keywords::REBORN));
}

#[test]
fn salvager_never_copies_another_salvager_and_golden_takes_both_neighbours() {
    let db = db();
    let left = taunt(with_health(mk(&db, "Stitched Salvager", false), 10));
    let right = taunt(with_health(mk(&db, "Stitched Salvager", false), 10));
    let enemies = [killer(&db), wall(&db), wall(&db)];
    let (_, log) = run(&db, &[left, right], &enemies, [Counters::default(); 2], 1, 1);
    let salvager = id(&db, "Stitched Salvager");
    assert!(
        summons(&log, 0).iter().all(|(c, _)| *c != salvager),
        "a Salvager is destroyed but never copied"
    );

    let a = mk(&db, "Harmless Bonehead", false);
    let b = mk(&db, "Cadaver Caretaker", false);
    let gold = taunt(with_health(mk(&db, "Stitched Salvager", true), 10));
    let (_, log) = run(&db, &[a, gold, b], &[killer(&db), wall(&db), wall(&db), wall(&db)], [Counters::default(); 2], 1, 1);
    let deaths: Vec<CardId> = log
        .0
        .iter()
        .filter_map(|e| match e {
            Ev::Death { side: 0, card } => Some(*card),
            _ => None,
        })
        .collect();
    assert!(deaths.contains(&id(&db, "Harmless Bonehead")));
    assert!(deaths.contains(&id(&db, "Cadaver Caretaker")));
    // both copies come back when the golden Salvager dies
    let copies: Vec<CardId> = summons(&log, 0).into_iter().map(|(c, _)| c).collect();
    assert!(copies.iter().filter(|&&c| c == id(&db, "Harmless Bonehead")).count() >= 1);
    assert!(copies.iter().filter(|&&c| c == id(&db, "Cadaver Caretaker")).count() >= 1);
}

// ================= Lichling Hoarder, Drustfallen Butcher =================

/// A game where player 0's board fights player q's `killer` once.
fn fight_with_boards(g: &mut Game, board: Vec<Minion>) {
    let Some(bgcore::Opponent::Player(q)) = g.players[0].opponent else { panic!() };
    g.players[0].board.clear();
    g.players[0].board.extend(board);
    let db = g.data.cards.clone();
    g.players[q as usize].board.clear();
    let k = killer(&db);
    g.players[q as usize].board.push(k);
    end_all_turns(g);
}

#[test]
fn lichling_hoarder_gets_a_plain_copy_of_a_minion_that_started_on_the_board() {
    let db = db();
    for seed in 0..10 {
        let mut g = game(200 + seed);
        let mut gold = mk(&db, "Risen Rider", true);
        gold.attack = 0;
        gold.health = 500;
        let mut hoarder = mk(&db, "Lichling Hoarder", false);
        hoarder.attack = 0;
        hoarder.health = 2000;
        let board = vec![hoarder, gold, fodder_n(&db, 0), fodder_n(&db, 1), fodder_n(&db, 2)];
        fight_with_boards(&mut g, board);
        assert_eq!(g.players[0].hand.len(), 1, "seed {seed}");
        let m = g.players[0].hand[0];
        // a plain copy of the base card, nothing carried over
        let plain = g.players[0].make_card(&g.data.cards, m.card, false, 0);
        assert_eq!(m, plain, "seed {seed}");
        assert!(!m.golden);
        assert_eq!(m.pool_copies, 0, "generated: no pool copy");
        g.check_invariants().unwrap();
    }
    // The hand is full: it is lost.
    let mut g = game(210);
    for _ in 0..10 {
        g.give(0, "Skeleton").unwrap();
    }
    let mut hoarder = mk(&db, "Lichling Hoarder", false);
    hoarder.attack = 0;
    hoarder.health = 2000;
    fight_with_boards(&mut g, vec![hoarder, fodder_n(&db, 0), fodder_n(&db, 1), fodder_n(&db, 2)]);
    assert_eq!(g.players[0].hand.len(), 10);
    g.check_invariants().unwrap();
    // Golden gets two copies.
    let counters = Counters::default();
    let mut gold = mk(&db, "Lichling Hoarder", true);
    gold.attack = 0;
    gold.health = 4000;
    let (out, _) = run(
        &db,
        &[gold, fodder(&db), fodder(&db), fodder(&db)],
        &[killer(&db), wall(&db)],
        [counters, counters],
        3,
        1,
    );
    assert_eq!(out.sides[0].cards_gained().len(), 2);
}

#[test]
fn drustfallen_butcher_gets_a_butchering_after_the_fight() {
    let db = db();
    let mut g = game(220);
    let mut butcher = mk(&db, "Drustfallen Butcher", false);
    butcher.attack = 0;
    butcher.health = 2000;
    let mut board = vec![butcher];
    for _ in 0..4 {
        board.push(fodder(&db));
    }
    fight_with_boards(&mut g, board);
    assert_eq!(g.players[0].hand.len(), 1);
    assert_eq!(name(&g, &g.players[0].hand[0]), "Butchering");
    g.check_invariants().unwrap();
    // golden: two
    let mut gold = mk(&db, "Drustfallen Butcher", true);
    gold.attack = 0;
    gold.health = 4000;
    let (out, _) = run(
        &db,
        &[gold, fodder(&db), fodder(&db), fodder(&db), fodder(&db)],
        &[killer(&db), wall(&db)],
        [Counters::default(); 2],
        4,
        1,
    );
    assert_eq!(out.sides[0].cards_gained().len(), 2);
    // a full hand loses it
    let mut g = game(221);
    for _ in 0..10 {
        g.give(0, "Skeleton").unwrap();
    }
    let mut butcher = mk(&db, "Drustfallen Butcher", false);
    butcher.attack = 0;
    butcher.health = 2000;
    let mut board = vec![butcher];
    for _ in 0..4 {
        board.push(fodder(&db));
    }
    fight_with_boards(&mut g, board);
    assert_eq!(g.players[0].hand.len(), 10);
}

// ================= Eternal Knight, Summoner, Tycoon =================

#[test]
fn eternal_knights_count_every_death_wherever_they_are() {
    let db = db();
    // In a fight: three Knights, one dies, the other two grow by +4/+2.
    let mut a = mk(&db, "Eternal Knight", false);
    a.health = 1;
    a.keywords.set(Keywords::TAUNT);
    let b = mk(&db, "Eternal Knight", false);
    let c = mk(&db, "Eternal Knight", true);
    let (out, _) = run(&db, &[a, b, c], &[killer(&db), killer(&db), killer(&db), killer(&db)], [Counters::default(); 2], 1, 1);
    assert_eq!(out.sides[0].knight_deaths, 1);

    // In the game: the count goes up after a fight (a death in the fight),
    // and a Knight on the board, in the hand and in the Tavern grows; the
    // golden one grows by +8/+4.
    let mut g = game(230);
    put_on_board(&mut g, 0, "Eternal Knight", false);
    g.players[0].board[0].health = 1;
    put_on_board(&mut g, 0, "Eternal Knight", false);
    g.players[0].board[1].keywords.set(Keywords::TAUNT);
    g.players[0].board[1].health = 5000;
    put_in_hand(&mut g, 0, "Eternal Knight", false);
    put_in_hand(&mut g, 0, "Eternal Knight", true);
    let Some(bgcore::Opponent::Player(q)) = g.players[0].opponent else { panic!() };
    put_on_board(&mut g, q as usize, "Cage Gnawer", false);
    g.players[q as usize].board[0].attack = 10;
    g.players[q as usize].board[0].health = 2000;
    // a new Knight in hand, to see a made-later Knight read the count
    end_all_turns(&mut g);
    assert!(g.players[0].knight_deaths >= 1);
    let deaths = g.players[0].knight_deaths as i32;
    let hand: Vec<Minion> = g.players[0].hand.iter().copied().collect();
    assert_eq!(stats(&hand[0]), (4 + 4 * deaths, 2 + 2 * deaths));
    assert_eq!(stats(&hand[1]), (8 + 8 * deaths, 4 + 4 * deaths), "golden reads +8/+4");
    let c = id(&g.data.cards, "Eternal Knight");
    let fresh = g.players[0].make_card(&g.data.cards, c, false, 0);
    assert_eq!(stats(&fresh), (4 + 4 * deaths, 2 + 2 * deaths));
    g.check_invariants().unwrap();
}

#[test]
fn a_knight_destroyed_in_the_shop_makes_the_others_grow() {
    let mut g = game(231);
    put_on_board(&mut g, 0, "Dead Bellringer", false);
    put_on_board(&mut g, 0, "Eternal Knight", false);
    put_in_hand(&mut g, 0, "Eternal Knight", false);
    set_gold(&mut g, 0, 5);
    act(&mut g, 0, Action::Activate(0));
    // The Knight beside the Bellringer is the only other Undead: it is picked
    // for you.
    assert!(g.players[0].pending_target.is_none());
    assert_eq!(g.players[0].knight_deaths, 1);
    // the Knight in hand: 4/2 + 4/2. (Two on the board and one in hand would
    // be a triple, so this is the survivor and one copy.)
    assert_eq!(stats(&g.players[0].hand[0]), (8, 4));
    g.check_invariants().unwrap();
}

#[test]
fn eternal_summoner_summons_a_knight_at_the_current_size() {
    let db = db();
    let counters = Counters { knight_deaths: 2, ..Counters::default() };
    let mut summoner = with_health(taunt(mk(&db, "Eternal Summoner", false)), 1);
    summoner.keywords.clear(Keywords::REBORN);
    let (_, log) = run(&db, &[summoner], &[killer(&db), killer(&db)], [counters, Counters::default()], 1, 1);
    let knight = summons(&log, 0).into_iter().find(|(c, _)| *c == id(&db, "Eternal Knight")).unwrap().1;
    assert_eq!(stats(&knight), (4 + 8, 2 + 4));
    // golden: a Golden Knight, +8/+4 per death
    let mut gold = with_health(taunt(mk(&db, "Eternal Summoner", true)), 1);
    gold.keywords.clear(Keywords::REBORN);
    let (_, log) = run(&db, &[gold], &[killer(&db), killer(&db)], [counters, Counters::default()], 1, 1);
    let knight = summons(&log, 0).into_iter().find(|(c, _)| *c == id(&db, "Eternal Knight")).unwrap().1;
    assert!(knight.golden);
    assert_eq!(stats(&knight), (8 + 16, 4 + 8));
}

#[test]
fn eternal_tycoons_knight_attacks_right_after_the_attack_that_caused_the_death() {
    let db = db();
    let counters = Counters { knight_deaths: 100, ..Counters::default() };
    let mut tycoon = mk(&db, "Eternal Tycoon", false);
    tycoon.attack = 0;
    tycoon.health = 5000;
    let mut board = vec![tycoon];
    for _ in 0..5 {
        board.push(fodder(&db));
    }
    let (_, log) = run(&db, &board, &[killer(&db), wall(&db)], [counters, Counters::default()], 8, 1);
    let knight = id(&db, "Eternal Knight");
    let knight_attacks: Vec<usize> = log
        .0
        .iter()
        .enumerate()
        .filter_map(|(i, e)| match e {
            Ev::Attack { side: 0, card, .. } if *card == knight => Some(i),
            _ => None,
        })
        .collect();
    assert!(knight_attacks.len() >= 2, "the immediate attack and then its normal turn");
    // the two attacks come one right after the other: no enemy attack
    // between the immediate one and the Knight's normal turn
    let between = &log.0[knight_attacks[0] + 1..knight_attacks[1]];
    assert!(
        !between.iter().any(|e| matches!(e, Ev::Attack { side: 1, .. })),
        "the Knight swung out of turn"
    );
    // and it came right after the enemy's fifth attack
    let before = &log.0[..knight_attacks[0]];
    let enemy_attacks = before.iter().filter(|e| matches!(e, Ev::Attack { side: 1, .. })).count();
    assert_eq!(enemy_attacks, 5);
}

// ================= Prosthetic Hand =================

#[test]
fn prosthetic_hand_magnetizes_to_an_undead_as_well_as_a_mech() {
    let mut g = game(240);
    put_on_board(&mut g, 0, "Risen Rider", false);
    put_in_hand(&mut g, 0, "Prosthetic Hand", false);
    let hand = db().get(id(&g.data.cards, "Prosthetic Hand")).tribes;
    assert!(hand.has(Tribe::Mech) && hand.has(Tribe::Undead));
    play(&mut g, 0, 0, 0);
    assert_eq!(g.players[0].board.len(), 1, "merged into the Rider");
    assert_eq!(stats(&g.players[0].board[0]), (2 + 3, 1 + 1));
    assert!(g.players[0].board[0].keywords.has(Keywords::REBORN));
    g.check_invariants().unwrap();

    // A Mech-only Magnetic can't merge into an Undead, and neither can the
    // Hand into something that is neither (with a full board it can't be
    // played at all).
    let mut g = game(241);
    for _ in 0..6 {
        put_on_board(&mut g, 0, "Wolf Pup", false);
    }
    put_on_board(&mut g, 0, "Risen Rider", false);
    put_in_hand(&mut g, 0, "Lullabot", false);
    put_in_hand(&mut g, 0, "Prosthetic Hand", false);
    assert!(!legal(&mut g, 0, Action::Play { hand: 0, pos: 6 }), "Lullabot onto an Undead");
    assert!(legal(&mut g, 0, Action::Play { hand: 1, pos: 6 }), "Hand onto an Undead");
    assert!(!legal(&mut g, 0, Action::Play { hand: 1, pos: 0 }), "Hand onto a Beast");
}

// ================= The tribe in the lobby =================

#[test]
fn undead_are_a_tribe_of_the_pool_and_all_types_include_them() {
    let db = db();
    let undead: Vec<&str> = (0..db.pool_cards)
        .filter(|&c| db.cards[c].tribes.has(Tribe::Undead))
        .map(|c| db.cards[c].name.as_str())
        .collect();
    // the 21 Undead, Prosthetic Hand, the three all-type minions, and the
    // five Dark Paradox versions (all-type too)
    assert_eq!(undead.len(), 30, "{undead:?}");
    assert!(!undead.contains(&"Rot Hide Gnoll"));
    assert!(db.by_name("Rot Hide Gnoll").is_none());
    assert!(db.by_name("Skeleton").is_some());
    assert!(db.by_name("Helping Hand").is_some());
    assert!(db.by_name("Tomb Turning").is_some());
    assert!(db.by_name("Butchering").is_some());
    assert!(bgcore::Tribes::ALL.has(Tribe::Undead));
    // Prosthetic Hand keeps both tribes.
    let hand = db.get(db.by_name("Prosthetic Hand").unwrap());
    assert!(hand.tribes.has(Tribe::Mech) && hand.tribes.has(Tribe::Undead));
}

#[test]
fn shop_destroys_keep_the_pool_exact() {
    // Bellringer destroys Undead with pool copies, golden ones included.
    let mut g = game(250);
    put_on_board(&mut g, 0, "Dead Bellringer", false);
    put_on_board(&mut g, 0, "Cadaver Caretaker", true);
    put_on_board(&mut g, 0, "Mummifier", false);
    set_gold(&mut g, 0, 5);
    act(&mut g, 0, Action::Activate(0));
    act(&mut g, 0, Action::Target(1));
    g.check_invariants().unwrap();
    end_all_turns(&mut g);
    g.check_invariants().unwrap();
}

#[test]
fn random_games_with_an_undead_tavern_keep_every_invariant() {
    // A sandbox rigged to Undead and neutrals, played at random: Reborn,
    // shop destroys, the bonus and the counters all run, and the pool stays
    // exact after every action.
    let data = Arc::new(Data::bundled());
    let mut played_undead = 0;
    for seed in 0..40u64 {
        let mut g = Game::new_sandbox(data.clone(), seed, Some(bgcore::Tribes::of(Tribe::Undead)), 3);
        let mut rng = Rng::new(seed + 1000);
        let mut steps = 0;
        while !g.over && steps < 3000 {
            let a = bgcore::bots::random_action(&g, &mut rng);
            g.step(a).unwrap();
            g.check_invariants().unwrap();
            steps += 1;
        }
        played_undead += g.players[0]
            .board
            .iter()
            .filter(|m| g.data.cards.get(m.card).tribes.has(Tribe::Undead))
            .count();
    }
    assert!(played_undead > 0);
}

#[test]
fn balinda_does_not_cast_butchering_twice() {
    // Like Channel the Devourer: the second cast does nothing, and it is not
    // a second spell cast either.
    let mut g = game(260);
    put_on_board(&mut g, 0, "Balinda Stonehearth", false);
    put_on_board(&mut g, 0, "Harmless Bonehead", false);
    cast(&mut g, "Butchering");
    act(&mut g, 0, Action::Target(1));
    assert_eq!(g.players[0].undead_bonus, (8, 0));
    assert_eq!(board_names(&g, 0), ["Balinda Stonehearth", "Skeleton", "Skeleton"]);
    assert_eq!(stats(&g.players[0].board[1]), (9, 1));
    g.check_invariants().unwrap();

    // The target has Reborn: its copy is not destroyed again, and there is
    // no second +8.
    let mut g = game(261);
    put_on_board(&mut g, 0, "Balinda Stonehearth", false);
    put_on_board(&mut g, 0, "Risen Rider", false);
    cast(&mut g, "Butchering");
    act(&mut g, 0, Action::Target(1));
    assert_eq!(g.players[0].undead_bonus, (8, 0));
    assert_eq!(board_names(&g, 0), ["Balinda Stonehearth", "Risen Rider"]);
    assert_eq!(stats(&g.players[0].board[1]), (2 + 8, 1));
    g.check_invariants().unwrap();
}

#[test]
fn forsaken_weaver_reacts_to_spells_cast_in_a_fight_and_the_bonus_is_kept() {
    let db = db();
    // Runic Arcanist casts Shiny Ring twice at the start of combat.
    let arcanist = mk(&db, "Runic Arcanist", false);
    let weaver = mk(&db, "Forsaken Weaver", false);
    let (out, _) = run(&db, &[arcanist, weaver], &[wall(&db)], [Counters::default(); 2], 1, 1);
    assert_eq!(out.sides[0].undead_gain, (6, 0));
    let weaver = mk(&db, "Forsaken Weaver", true);
    let arcanist = mk(&db, "Runic Arcanist", false);
    let (out, _) = run(&db, &[arcanist, weaver], &[wall(&db)], [Counters::default(); 2], 1, 1);
    assert_eq!(out.sides[0].undead_gain, (12, 0));

    // After a real fight it is part of the player's bonus, on the board and
    // in the hand.
    let mut g = game(262);
    let Some(bgcore::Opponent::Player(q)) = g.players[0].opponent else { panic!() };
    put_on_board(&mut g, 0, "Runic Arcanist", false);
    put_on_board(&mut g, 0, "Forsaken Weaver", false);
    put_in_hand(&mut g, 0, "Mummifier", false);
    put_on_board(&mut g, q as usize, "Cage Gnawer", false);
    g.players[q as usize].board[0].attack = 1;
    g.players[q as usize].board[0].health = 2000;
    end_all_turns(&mut g);
    assert_eq!(g.players[0].undead_bonus, (6, 0));
    assert_eq!(stats(&g.players[0].hand[0]), (5 + 6, 2));
    let weaver = g.players[0].board.iter().find(|m| m.card == id(&g.data.cards, "Forsaken Weaver")).unwrap();
    assert_eq!(weaver.attack, 3 + 6);
    g.check_invariants().unwrap();
}

#[test]
fn a_second_empty_cast_is_not_a_spell_cast() {
    // Butchering and Channel the Devourer with Balinda: only one cast counts,
    // so Forsaken Weaver reacts once.
    let mut g = game(270);
    put_on_board(&mut g, 0, "Forsaken Weaver", false);
    put_on_board(&mut g, 0, "Balinda Stonehearth", false);
    put_on_board(&mut g, 0, "Harmless Bonehead", false);
    cast(&mut g, "Butchering");
    act(&mut g, 0, Action::Target(2));
    assert_eq!(g.players[0].undead_bonus, (8 + 3, 0));

    let mut g = game(271);
    put_on_board(&mut g, 0, "Forsaken Weaver", false);
    put_on_board(&mut g, 0, "Balinda Stonehearth", false);
    put_on_board(&mut g, 0, "Wolf Pup", false);
    cast(&mut g, "Channel the Devourer");
    act(&mut g, 0, Action::Target(2));
    assert_eq!(g.players[0].undead_bonus, (3, 0));
    g.check_invariants().unwrap();
}

#[test]
fn three_skeletons_a_shop_destroy_summons_triple_into_the_hand() {
    // docs/rules/design.md "Triples": three Skeletons a Cadaver Caretaker
    // leaves make a Golden Skeleton in the hand, and the shop phase does it at
    // the end of the action (log 2026_10_30_19_45_15-g2 turn 5, Disguised
    // Graverobber on a Cadaver Caretaker). The Graverobber's copy of the
    // Caretaker is a plain card and joins the hand first, and the Golden comes
    // after it.
    let mut g = game(272);
    put_on_board(&mut g, 0, "Cadaver Caretaker", false);
    put_in_hand(&mut g, 0, "Disguised Graverobber", false);
    play(&mut g, 0, 0, 1);
    // The Caretaker is the only friendly Undead, so the target is picked for
    // you.
    assert!(g.players[0].pending_target.is_none());
    assert_eq!(board_names(&g, 0), ["Disguised Graverobber"], "the Skeletons tripled away");
    let hand: Vec<(String, bool, (i32, i32))> = g.players[0]
        .hand
        .iter()
        .map(|m| (g.data.cards.get(m.card).name.clone(), m.golden, stats(m)))
        .collect();
    assert_eq!(
        hand,
        [
            ("Cadaver Caretaker".to_string(), false, (3, 3)),
            ("Skeleton".to_string(), true, (2, 2)),
        ]
    );
    g.check_invariants().unwrap();
}
