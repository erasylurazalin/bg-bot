//! The Murloc minions of the 5-tribe lobby, with the real cards from
//! cards.toml: the hand taking part in combat (Expert Aviator, Diremuck
//! Forager, Flighty Scout, Costume Enthusiast, Choral Mrrrglr, Very Hungry
//! Winterfinner), and the rest of the tribe's mechanics.

use std::sync::Arc;

use bgcore::fight::{Watch, fight_with, fight_with_hands};
use bgcore::{Tribe, Action, CardDb, Counters, Data, Game, Keywords, Minion, Opponent, Rng};

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

/// A minion with `attack` that takes a long time to kill.
fn fighter(db: &CardDb, attack: i32) -> Minion {
    let mut w = mk(db, "Microbot", false);
    w.attack = attack;
    w.health = 2000;
    w
}

#[derive(Default)]
struct Log {
    attacks: Vec<(String, i32, i32, u8)>,
    summons: Vec<(String, i32, i32)>,
    db: Option<CardDb>,
}

impl Watch for Log {
    fn attack(&mut self, _side: usize, a: &Minion, _d: &Minion) {
        let db = self.db.as_ref().unwrap();
        self.attacks.push((db.get(a.card).name.clone(), a.attack, a.health, a.keywords.0));
    }
    fn summon(&mut self, _side: usize, _pos: usize, m: &Minion) {
        let db = self.db.as_ref().unwrap();
        self.summons.push((db.get(m.card).name.clone(), m.attack, m.health));
    }
}

fn run(
    db: &CardDb,
    a: &[Minion],
    b: &[Minion],
    hands: [&[Minion]; 2],
    max_attacks: u32,
    seed: u64,
) -> (bgcore::Outcome, Log) {
    let mut log = Log { db: Some(db.clone()), ..Default::default() };
    let out = fight_with_hands(
        db,
        a,
        b,
        hands,
        [Counters::default(); 2],
        max_attacks,
        &mut Rng::new(seed),
        &mut log,
    );
    (out, log)
}

// ---- The hand in combat ----

#[test]
fn two_expert_aviators_take_the_two_highest_attack_murlocs_from_hand() {
    let db = db();
    let hand = vec![mk(&db, "Hackerfin", false), mk(&db, "Bream Counter", false)];
    let before = hand.clone();
    let a = [mk(&db, "Expert Aviator", false), mk(&db, "Expert Aviator", false)];
    // The Aviator's summoned Murloc takes the next attack on that side
    // (Hearthstone_2026_11_13_14_50_52 lines 491120 and 491284), so the
    // second Aviator swings after it, not on the next turn of the fight.
    let (out, log) = run(&db, &a, &[wall(&db)], [&hand, &[]], 6, 1);
    // Bream Counter has 6 Attack, Hackerfin 5.
    assert_eq!(
        log.summons.iter().map(|s| s.0.as_str()).collect::<Vec<_>>(),
        ["Bream Counter", "Hackerfin"]
    );
    // The summoned copies have their card stats.
    assert_eq!(log.summons[0].1, 6);
    assert_eq!(log.summons[1].1, 5);
    // The real hand is untouched: nothing was removed, buffed or kept.
    assert_eq!(hand, before);
    assert!(out.sides[0].hand_stats.iter().all(|&s| s == (0, 0)));
}

#[test]
fn diremuck_forager_summons_from_hand_only_with_space() {
    let db = db();
    let hand = [mk(&db, "Bream Counter", false)];
    // With space, it summons the highest-Attack Murloc.
    let a = [mk(&db, "Diremuck Forager", false)];
    let (_, log) = run(&db, &a, &[wall(&db)], [&hand, &[]], 0, 1);
    assert_eq!(log.summons.iter().map(|s| s.0.as_str()).collect::<Vec<_>>(), ["Bream Counter"]);
    assert_eq!((log.summons[0].1, log.summons[0].2), (6, 6));

    // With a full board there is no space, so nothing comes out.
    let mut a = vec![mk(&db, "Diremuck Forager", false)];
    for _ in 0..6 {
        a.push(mk(&db, "Microbot", false));
    }
    let (_, log) = run(&db, &a, &[wall(&db)], [&hand, &[]], 0, 1);
    assert!(log.summons.is_empty());
}

#[test]
fn choral_mrrrglr_reads_the_hand_as_it_stands_at_its_turn() {
    let db = db();
    let hand = [mk(&db, "Bream Counter", false)];
    // Diremuck (left of Choral) takes the only hand Murloc out first, so
    // Choral gains nothing. Diremuck's Attack is set to 0 so Choral attacks
    // first and the log shows its stats.
    let mut diremuck = mk(&db, "Diremuck Forager", false);
    diremuck.attack = 0;
    let mut choral = mk(&db, "Choral Mrrrglr", false);
    choral.keywords.set(Keywords::TAUNT);
    let a = [diremuck, choral];
    let (_, log) = run(&db, &a, &[wall(&db)], [&hand, &[]], 1, 1);
    assert_eq!(log.summons[0].0, "Bream Counter");
    assert_eq!((log.attacks[0].1, log.attacks[0].2), (6, 6), "the hand was empty at Choral's turn");

    // Choral on the left counts the Murloc before Diremuck takes it.
    let mut choral = mk(&db, "Choral Mrrrglr", false);
    choral.keywords.set(Keywords::TAUNT);
    let mut diremuck = mk(&db, "Diremuck Forager", false);
    diremuck.attack = 0;
    let a = [choral, diremuck];
    let (_, log) = run(&db, &a, &[wall(&db)], [&hand, &[]], 1, 1);
    assert_eq!((log.attacks[0].1, log.attacks[0].2), (12, 12));
    assert_eq!(log.summons[0].0, "Bream Counter");
}

#[test]
fn costume_enthusiast_gains_the_attack_of_the_highest_attack_hand_minion() {
    let db = db();
    let hand = [mk(&db, "Tad", false), mk(&db, "Hackerfin", false)];
    let a = [mk(&db, "Costume Enthusiast", false)];
    let (_, log) = run(&db, &a, &[wall(&db)], [&hand, &[]], 1, 1);
    // 4 Attack plus Hackerfin's 5, Health unchanged.
    assert_eq!((log.attacks[0].1, log.attacks[0].2), (9, 5));
    // Golden gains double: 8 + 5 * 2.
    let a = [mk(&db, "Costume Enthusiast", true)];
    let (_, log) = run(&db, &a, &[wall(&db)], [&hand, &[]], 1, 1);
    assert_eq!((log.attacks[0].1, log.attacks[0].2), (18, 10));
}

#[test]
fn flighty_scout_in_hand_summons_a_copy_and_stays_in_hand() {
    let db = db();
    let hand = [mk(&db, "Flighty Scout", false)];
    let (out, log) = run(&db, &[], &[wall(&db)], [&hand, &[]], 0, 1);
    assert_eq!(log.summons.iter().map(|s| s.0.as_str()).collect::<Vec<_>>(), ["Flighty Scout"]);
    assert_eq!((log.summons[0].1, log.summons[0].2), (3, 3));
    assert_eq!(out.sides[0].hand_stats[0], (0, 0));

    // A golden hand card summons a golden copy with double the stats it has
    // now, the hand buffs it carries included: the Golden 6/6 plus +2/+4
    // doubles to 16/20, not the 8/10 it holds.
    let mut scout = mk(&db, "Flighty Scout", true);
    scout.attack += 2;
    scout.health += 4;
    let hand = [scout];
    let (_, log) = run(&db, &[], &[wall(&db)], [&hand, &[]], 0, 1);
    assert_eq!((log.summons[0].1, log.summons[0].2), (16, 20));
}

#[test]
fn very_hungry_winterfinner_buffs_a_hand_minion_permanently() {
    let db = db();
    let hand = [mk(&db, "Tad", false)];
    // The enemy hits the Winterfinner until it dies: each hit gives the hand
    // minion +2/+1.
    let a = [mk(&db, "Very Hungry Winterfinner", false)];
    let (out, _) = run(&db, &a, &[fighter(&db, 1)], [&hand, &[]], 6, 1);
    let (da, dh) = out.sides[0].hand_stats[0];
    assert!(da >= 2 && dh >= 1, "{da}/{dh}");
    assert_eq!(da, dh * 2);

    // The game puts the gains on the real hand after the fight.
    let mut g = game(7);
    put_on_board(&mut g, 0, "Very Hungry Winterfinner");
    g.give(0, "Tad").unwrap();
    let q = opponent(&g, 0);
    let mut attacker = mk(&g.data.cards, "Microbot", false);
    attacker.attack = 1;
    attacker.health = 500;
    g.players[q].board.push(attacker);
    while g.turn == 1 {
        g.step(Action::EndTurn).unwrap();
    }
    assert!(g.players[0].hand[0].attack > 2, "the hand buff should be permanent");
    assert_eq!(g.players[0].hand[0].attack - 2, (g.players[0].hand[0].health - 2) * 2);
    g.check_invariants().unwrap();
}

#[test]
fn winterfinner_buffs_the_hand_even_after_an_aviator_took_the_top_murloc() {
    let db = db();
    // The Aviator takes Bream Counter out for the combat; it stays on its
    // hand slot with no gains. Tad stays in hand and takes Winterfinner's
    // buffs, which are permanent.
    let hand = [mk(&db, "Bream Counter", false), mk(&db, "Tad", false)];
    let a = [
        mk(&db, "Expert Aviator", false),
        mk(&db, "Very Hungry Winterfinner", false),
    ];
    let (out, log) = run(&db, &a, &[fighter(&db, 1)], [&hand, &[]], 40, 1);
    assert_eq!(log.summons[0].0, "Bream Counter");
    assert_eq!(out.sides[0].hand_stats[0], (0, 0), "the summoned copy keeps nothing");
    assert!(out.sides[0].hand_stats[1].0 >= 2);
}

// ---- Bile Spitter ----

#[test]
fn bile_spitter_gives_another_murloc_venomous() {
    let db = db();
    let mut spitter = mk(&db, "Bile Spitter", false);
    spitter.health = 100;
    // Venomous kills a wall in one hit, so the enemy has two shielded walls:
    // the first Bile Spitter swing pops a shield and leaves Tad a turn.
    let shielded = |db: &CardDb| {
        let mut w = mk(db, "Annoy-o-Module", false);
        w.attack = 0;
        w.health = 100;
        w
    };
    let a = [spitter, mk(&db, "Tad", false)];
    let (_, log) = run(&db, &a, &[shielded(&db), shielded(&db)], [&[], &[]], 6, 1);
    let tad = log
        .attacks
        .iter()
        .find(|t| t.0 == "Tad")
        .unwrap_or_else(|| panic!("Tad attacks: {:?}", log.attacks));
    assert!(tad.3 & Keywords::VENOMOUS != 0, "Tad should be Venomous when it attacks");
}

// ---- The shop side ----

#[test]
fn kelp_keeper_triggers_a_battlecry_that_kalecgos_sees() {
    let mut g = game(11);
    put_on_board(&mut g, 0, "Kelp Keeper");
    put_on_board(&mut g, 0, "Kalecgos, Arcane Aspect");
    put_on_board(&mut g, 0, "Hackerfin");
    act(&mut g, 0, Action::Activate(0));
    // Hackerfin's Battlecry gives the other minions +3/+2, then Kalecgos
    // answers the Battlecry with +2/+2 to every Dragon (itself).
    assert_eq!(stats(&g.players[0].board[1]), (4 + 3 + 2, 12 + 2 + 2));
    // Hackerfin's "other minions" leaves Hackerfin itself alone.
    assert_eq!(stats(&g.players[0].board[2]), (5, 3));
    g.check_invariants().unwrap();
}

#[test]
fn golden_kelp_keeper_triggers_the_battlecry_twice_from_one_use() {
    let mut g = game(61);
    put_on_board(&mut g, 0, "Kelp Keeper");
    g.players[0].board[0].golden = true;
    put_on_board(&mut g, 0, "Hackerfin");
    let before = stats(&g.players[0].board[0]);
    act(&mut g, 0, Action::Activate(0));
    // Hackerfin's +3/+2 to the other minions, twice
    let after = stats(&g.players[0].board[0]);
    assert_eq!((after.0 - before.0, after.1 - before.1), (6, 4));
    // that was the one use for the turn
    assert!(!g.is_legal(Action::Activate(0)));
    g.check_invariants().unwrap();
}

#[test]
fn young_murk_eye_triggers_the_adjacent_battlecries_at_end_of_turn() {
    let mut g = game(12);
    put_on_board(&mut g, 0, "Hackerfin");
    put_on_board(&mut g, 0, "Young Murk-Eye");
    put_on_board(&mut g, 0, "Kalecgos, Arcane Aspect");
    act(&mut g, 0, Action::EndTurn);
    // Hackerfin's Battlecry fires, then Kalecgos's after-Battlecry buff.
    assert_eq!(stats(&g.players[0].board[1]), (8 + 3, 5 + 2));
    assert_eq!(stats(&g.players[0].board[2]), (4 + 3 + 2, 12 + 2 + 2));
    // Hackerfin itself is not buffed by its own Battlecry.
    assert_eq!(stats(&g.players[0].board[0]), (5, 3));
}

#[test]
fn magicfin_mycologist_teaches_a_targeted_spell_once_per_turn() {
    let mut g = game(13);
    put_on_board(&mut g, 0, "Magicfin Mycologist");
    let fortify = g.data.cards.by_name("Fortify").unwrap();
    // First buy: one taught 1/1 token.
    g.players[0].shop_spell = Some(fortify);
    act(&mut g, 0, Action::BuySpell);
    assert_eq!(g.players[0].hand.len(), 2);
    let token = g.players[0]
        .hand
        .iter()
        .find(|m| g.data.cards.get(m.card).taught_token)
        .copied()
        .expect("a taught token");
    assert_eq!(token.taught_spell, Some(fortify));
    assert_eq!((token.attack, token.health), (1, 1));
    // Second buy this turn teaches nothing more.
    g.players[0].shop_spell = Some(fortify);
    act(&mut g, 0, Action::BuySpell);
    let taught = g.players[0]
        .hand
        .iter()
        .filter(|m| g.data.cards.get(m.card).taught_token)
        .count();
    assert_eq!(taught, 1);
    g.check_invariants().unwrap();

    // Playing the token casts Fortify, with its own target choice.
    let h = g.players[0].hand.iter().position(|m| m.taught_spell.is_some()).unwrap();
    act(&mut g, 0, Action::Play { hand: h as u8, pos: 1 });
    act(&mut g, 0, Action::Target(0));
    assert_eq!(stats(&g.players[0].board[0]), (4, 8 + 3));
    assert!(g.players[0].board[0].keywords.has(Keywords::TAUNT));
    g.check_invariants().unwrap();
}

/// A Magicfin token in hand taught Tricky Trousers, ready to be played.
fn taught_token_in_hand(g: &mut Game) -> u8 {
    g.give(0, "Magicfin Apprentice").expect("a token in hand");
    let tricky = g.data.cards.by_name("Tricky Trousers").expect("Tricky Trousers");
    let h = g.players[0].hand.len() - 1;
    g.players[0].hand[h].taught_spell = Some(tricky);
    h as u8
}

#[test]
fn brann_casts_a_taught_targeting_spell_twice_on_the_one_target() {
    // docs/rules/design.md "Set versus add": a repeated Battlecry or spell
    // that picks a target uses the same target every time (Era 2026-11-04,
    // log Hearthstone_2026_10_28_02_28_32-g1 turn 10 step 2).
    let mut g = game(15);
    put_on_board(&mut g, 0, "Brann Bronzebeard");
    put_on_board(&mut g, 0, "Lullabot");
    let h = taught_token_in_hand(&mut g);
    act(&mut g, 0, Action::Play { hand: h, pos: 2 });
    // The spell asks for its target once, for both casts.
    assert_eq!(g.players[0].pending_target.map(|pt| pt.runs), Some(2));
    act(&mut g, 0, Action::Target(1));
    // Tricky Trousers gives +1/+2, twice, and its Taunt comes and goes.
    assert_eq!(stats(&g.players[0].board[1]), (2 + 2, 2 + 4));
    assert!(!g.players[0].board[1].keywords.has(Keywords::TAUNT));
    assert_eq!(stats(&g.players[0].board[0]), (2, 4), "the other minion is untouched");
    g.check_invariants().unwrap();
}

#[test]
fn a_golden_setter_casts_a_taught_targeting_spell_three_times_on_one_target() {
    // A golden Brann sets the count at three ("Set versus add"): three casts,
    // all on the one minion picked.
    let mut g = game(16);
    put_on_board(&mut g, 0, "Brann Bronzebeard");
    let brann = g.data.cards.by_name("Brann Bronzebeard").expect("Brann");
    let slot = g.data.cards.get(brann).pool_slot as usize;
    g.pool[slot] -= 2;
    g.players[0].board[0].golden = true;
    g.players[0].board[0].pool_copies = 3;
    put_on_board(&mut g, 0, "Lullabot");
    let h = taught_token_in_hand(&mut g);
    act(&mut g, 0, Action::Play { hand: h, pos: 2 });
    assert_eq!(g.players[0].pending_target.map(|pt| pt.runs), Some(3));
    act(&mut g, 0, Action::Target(1));
    assert_eq!(stats(&g.players[0].board[1]), (2 + 3, 2 + 6));
    assert!(
        g.players[0].board[1].keywords.has(Keywords::TAUNT),
        "the third cast sets Taunt again"
    );
    assert_eq!(stats(&g.players[0].board[0]), (2, 4), "the other minion is untouched");
    g.check_invariants().unwrap();
}

#[test]
fn a_mycologist_triple_teaches_with_fresh_uses() {
    // the copy used up its one teaching this turn; the golden from the
    // triple comes with its own two
    let mut g = game(14);
    put_on_board(&mut g, 0, "Magicfin Mycologist");
    let fortify = g.data.cards.by_name("Fortify").unwrap();
    g.players[0].gold = 20;
    let taught = |g: &Game| {
        g.players[0].hand.iter().filter(|m| g.data.cards.get(m.card).taught_token).count()
    };
    g.players[0].shop_spell = Some(fortify);
    act(&mut g, 0, Action::BuySpell);
    g.players[0].shop_spell = Some(fortify);
    act(&mut g, 0, Action::BuySpell);
    assert_eq!(taught(&g), 1, "the copy's single use is spent");
    // buy two more copies: the second one triples
    let myco = g.data.cards.by_name("Magicfin Mycologist").unwrap();
    for _ in 0..2 {
        let slot = g.data.cards.get(myco).pool_slot as usize;
        g.pool[slot] -= 1;
        let m = g.players[0].make_card(&g.data.cards, myco, false, 1);
        let at = g.players[0].shop.len();
        g.players[0].shop.push(m);
        act(&mut g, 0, Action::Buy(at as u8));
    }
    let gold = g.players[0].hand.iter().position(|m| m.golden).unwrap();
    act(&mut g, 0, Action::Play { hand: gold as u8, pos: 0 });
    for _ in 0..2 {
        g.players[0].shop_spell = Some(fortify);
        act(&mut g, 0, Action::BuySpell);
    }
    assert_eq!(taught(&g), 3, "the golden teaches twice more");
    g.players[0].shop_spell = Some(fortify);
    act(&mut g, 0, Action::BuySpell);
    assert_eq!(taught(&g), 3, "and then it is done for this turn");
    g.check_invariants().unwrap();
}

#[test]
fn bream_counter_grows_in_hand_after_a_murloc_is_played() {
    let mut g = game(14);
    g.give(0, "Bream Counter").unwrap();
    g.give(0, "Tad").unwrap();
    act(&mut g, 0, Action::Play { hand: 1, pos: 0 });
    assert_eq!(stats(&g.players[0].hand[0]), (12, 12));
    // A non-Murloc play leaves it alone.
    g.give(0, "Annoy-o-Module").unwrap();
    let h = g.players[0].hand.len() as u8 - 1;
    act(&mut g, 0, Action::Play { hand: h, pos: 1 });
    assert_eq!(stats(&g.players[0].hand[0]), (12, 12));
}

#[test]
fn hackerfin_and_seafood_stew_count_different_bonus_keywords() {
    // Annoy-o-Module has Taunt and Divine Shield: two Bonus Keywords.
    let mut g = game(15);
    put_on_board(&mut g, 0, "Annoy-o-Module");
    g.give(0, "Hackerfin").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 1 });
    // +3/+2 plus +3/+2 for each of the two keywords.
    assert_eq!(stats(&g.players[0].board[0]), (2 + 9, 4 + 6));

    // Golden: the whole Battlecry doubles, so +6/+4 plus +6/+4 per keyword.
    let mut g = game(150);
    put_on_board(&mut g, 0, "Annoy-o-Module");
    let c = g.data.cards.by_name("Hackerfin").unwrap();
    let slot = g.data.cards.get(c).pool_slot as usize;
    g.pool[slot] -= 3;
    let hf = g.players[0].make_card(&g.data.cards, c, true, 3);
    g.players[0].hand.push(hf);
    act(&mut g, 0, Action::Play { hand: 0, pos: 1 });
    assert_eq!(stats(&g.players[0].board[0]), (2 + 18, 4 + 12));

    let mut g = game(16);
    put_on_board(&mut g, 0, "Annoy-o-Module");
    g.give(0, "Seafood Stew").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    act(&mut g, 0, Action::Target(0));
    // Give a minion +1/+1, repeated for each different Bonus Keyword: three
    // times in all.
    assert_eq!(stats(&g.players[0].board[0]), (5, 7));
}

#[test]
fn hackerfin_counts_venomous_but_not_poisonous() {
    // Venomous is a Bonus Keyword and Poisonous is not (docs/rules/design.md
    // "Keywords"), so only the Venomous one pays Hackerfin's "+3/+2 for each
    // different Bonus Keyword in your warband" (plus its own +3/+2).
    let mut g = game(15);
    put_on_board(&mut g, 0, "Deadly Spore"); // a 1/1 Venomous
    g.give(0, "Hackerfin").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 1 });
    assert_eq!(stats(&g.players[0].board[0]), (1 + 3 + 3, 1 + 2 + 2));

    // The same 1/1 with Poisonous instead: no Bonus Keyword.
    let mut g = game(15);
    put_on_board(&mut g, 0, "Deadly Spore");
    let m = &mut g.players[0].board[0];
    m.keywords.clear(Keywords::VENOMOUS);
    m.keywords.set(Keywords::POISONOUS);
    g.give(0, "Hackerfin").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 1 });
    assert_eq!(stats(&g.players[0].board[0]), (1 + 3, 1 + 2));

    // Wearing both still counts once, for the Venomous.
    let mut g = game(15);
    put_on_board(&mut g, 0, "Deadly Spore");
    g.players[0].board[0].keywords.set(Keywords::POISONOUS);
    g.give(0, "Hackerfin").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 1 });
    assert_eq!(stats(&g.players[0].board[0]), (1 + 3 + 3, 1 + 2 + 2));
}

#[test]
fn tad_shoalfin_gormling_and_the_rest() {
    // Tad: a random Murloc on sell, never above the owner's tier.
    let mut g = game(17);
    put_on_board(&mut g, 0, "Tad");
    g.players[0].tier = 2;
    act(&mut g, 0, Action::Sell(0));
    assert_eq!(g.players[0].hand.len(), 1);
    assert!(g.data.cards.get(g.players[0].hand[0].card).tribes.has(bgcore::Tribe::Murloc));
    for seed in 0..20 {
        let mut g = game(seed);
        put_on_board(&mut g, 0, "Tad");
        g.players[0].tier = 2;
        act(&mut g, 0, Action::Sell(0));
        let c = g.data.cards.get(g.players[0].hand[0].card);
        assert!(c.tier <= 2, "{} is tier {}", c.name, c.tier);
    }

    // Shoalfin Mystic: the Tavern spell bonus goes up this game.
    let mut g = game(18);
    put_on_board(&mut g, 0, "Shoalfin Mystic");
    act(&mut g, 0, Action::Sell(0));
    assert_eq!(g.players[0].spell_bonus, (1, 1));

    // Gormling Gourmet: a Seafood Stew from the Battlecry and the Deathrattle.
    let mut g = game(19);
    g.give(0, "Gormling Gourmet").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    assert_eq!(g.players[0].hand.len(), 1);
    assert_eq!(name(&g, &g.players[0].hand[0]), "Seafood Stew");
    let db = db();
    let out = fight_with(
        &db,
        &[mk(&db, "Gormling Gourmet", false)],
        &[fighter(&db, 20)],
        [Counters::default(); 2],
        500,
        &mut Rng::new(1),
        &mut (),
    );
    assert_eq!(out.sides[0].cards_gained().len(), 1);
    assert_eq!(db.get(out.sides[0].cards_gained()[0]).name, "Seafood Stew");

    // Bubble Gunner: a random Bonus Keyword.
    let mut g = game(20);
    g.give(0, "Bubble Gunner").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    let mask = g.data.rules.bonus_mask;
    assert!(g.players[0].board[0].keywords.0 & mask != 0);
}

#[test]
fn twilight_shamanic_and_sewer_escapee_buff_the_hand_and_the_board() {
    // Twilight Tidehunter: a spell on it gives the left-most hand minion +9/+9.
    let mut g = game(21);
    put_on_board(&mut g, 0, "Twilight Tidehunter");
    g.give(0, "Tad").unwrap();
    g.give(0, "Fortify").unwrap();
    act(&mut g, 0, Action::Play { hand: 1, pos: 0 });
    act(&mut g, 0, Action::Target(0));
    assert_eq!(stats(&g.players[0].hand[0]), (11, 11));

    // Shamanic Tidecaller: a spell on a Murloc buffs the tribe on board and
    // in hand. Fortify's own +0/+3 lands on the target too.
    let mut g = game(22);
    put_on_board(&mut g, 0, "Shamanic Tidecaller");
    put_on_board(&mut g, 0, "Tad");
    g.give(0, "Bubble Gunner").unwrap();
    g.give(0, "Fortify").unwrap();
    act(&mut g, 0, Action::Play { hand: 1, pos: 0 });
    act(&mut g, 0, Action::Target(1));
    assert_eq!(stats(&g.players[0].board[0]), (9, 13));
    assert_eq!(stats(&g.players[0].board[1]), (5, 9));
    assert_eq!(stats(&g.players[0].hand[0]), (5, 7));

    // Sewer Escapee: Activate gives another Murloc +7/+7 and a keyword.
    let mut g = game(23);
    put_on_board(&mut g, 0, "Sewer Escapee");
    put_on_board(&mut g, 0, "Tad");
    act(&mut g, 0, Action::Activate(0));
    // The Tavern's Murlocs are offered too; pick the board's Tad (with none
    // there the pick is the only answer and no prompt opens).
    if g.players[0].pending_target.is_some() {
        act(&mut g, 0, Action::Target(1));
    }
    let mask = g.data.rules.bonus_mask;
    assert_eq!(stats(&g.players[0].board[1]), (9, 9));
    assert!(g.players[0].board[1].keywords.0 & mask != 0);
}

#[test]
fn primalfin_lookout_discovers_a_murloc_cloning_conch_gives_two() {
    // With another Murloc on the board, Primalfin Lookout discovers one, and
    // only up to the owner's tier (tier 1 has just two Murlocs, so tier 3 is
    // needed for three options).
    let mut g = game(24);
    put_on_board(&mut g, 0, "Tad");
    g.players[0].tier = 3;
    g.give(0, "Primalfin Lookout").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 1 });
    assert_eq!(g.players[0].discover.len(), 3);
    assert!(g.players[0].discover.iter().all(|&c| {
        let card = g.data.cards.get(c);
        card.tribes.has(bgcore::Tribe::Murloc) && card.tier <= 3
    }));
    act(&mut g, 0, Action::Choose(0));

    // With no other Murloc, nothing.
    let mut g = game(25);
    g.give(0, "Primalfin Lookout").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    assert!(g.players[0].discover.is_empty());

    // Cloning Conch: a random Murloc and a copy of it, never above the tier.
    for seed in 26..46 {
        let mut g = game(seed);
        g.players[0].tier = 2;
        g.give(0, "Cloning Conch").unwrap();
        act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
        assert_eq!(g.players[0].hand.len(), 2);
        assert_eq!(g.players[0].hand[0].card, g.players[0].hand[1].card);
        let card = g.data.cards.get(g.players[0].hand[0].card);
        assert!(card.tribes.has(bgcore::Tribe::Murloc));
        assert!(card.tier <= 2, "{} is tier {}", card.name, card.tier);
    }
}

// ---- The shop harness ----

fn lobby(ts: &[Tribe]) -> bgcore::Tribes {
    ts.iter().copied().fold(bgcore::Tribes::NONE, |m, t| m.with(t))
}

fn game(seed: u64) -> Game {
    Game::new_with_lobby(Arc::new(Data::bundled()), seed, lobby(&[Tribe::Beast, Tribe::Mech, Tribe::Dragon, Tribe::Murloc, Tribe::Demon]))
}

fn opponent(g: &Game, p: usize) -> usize {
    match g.players[p].opponent {
        Some(Opponent::Player(q)) => q as usize,
        _ => panic!("player {p} has no opponent"),
    }
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

fn stats(m: &Minion) -> (i32, i32) {
    (m.attack, m.health)
}

// ---- Murcules (back in the pool with patch 37.0.2) ----

#[test]
fn murcules_is_a_tier_two_murloc_in_the_pool_with_plus_three() {
    let db = db();
    let c = id_of(&db, "Murcules");
    let card = db.get(c);
    assert!((c as usize) < db.pool_cards, "a pool minion, not a token");
    assert_eq!(card.tier, 2);
    assert!(card.tribes.has(Tribe::Murloc));
    assert!(!card.out_of_pool);
    assert!(card.lobby_ok(bgcore::Tribes::ALL));
    assert_eq!((card.attack, card.health), (5, 3));
    let golden = db.make(c, true);
    assert_eq!((golden.attack, golden.health), (10, 6), "the golden doubles");
    assert_eq!(
        card.on_kill,
        Some(bgcore::cards::KillReward::BuffHand { attack: 3, health: 3 })
    );
}

fn id_of(db: &CardDb, name: &str) -> bgcore::CardId {
    db.by_name(name).unwrap_or_else(|| panic!("no card {name}"))
}

#[test]
fn murcules_gives_a_minion_in_hand_plus_three_after_a_kill() {
    let db = db();
    for golden in [false, true] {
        // Hackerfin (5/3) in the hand; Murcules kills a 1/1 Sewer Rat.
        let hand = [mk(&db, "Hackerfin", false)];
        let murcules = mk(&db, "Murcules", golden);
        let enemy = [mk(&db, "Sewer Rat", false)];
        let (out, _) = run(&db, &[murcules], &enemy, [&hand, &[]], 3, 3);
        let want = if golden { (6, 6) } else { (3, 3) };
        assert_eq!(
            out.sides[0].hand_stats[0], want,
            "the hand card took the kill's +3/+3 (golden +6/+6): {golden}"
        );
    }
    // No kill, no buff: a 0-Attack wall survives.
    let hand = [mk(&db, "Hackerfin", false)];
    let (out, _) = run(&db, &[mk(&db, "Murcules", false)], &[wall(&db)], [&hand, &[]], 3, 3);
    assert_eq!(out.sides[0].hand_stats[0], (0, 0), "nothing died");
}

/// The hand only goes into a fight when a card on the board needs it
/// (`Card::needs_hand_in_fight`): Murcules's reward is one of those cards, so
/// a real combat pays the hand out. The same path serves Tide Oracle Morgl.
#[test]
fn murcules_reaches_the_hand_in_a_whole_game() {
    let mut g = game(70);
    for p in 0..8 {
        g.players[p].board.clear();
    }
    put_on_board(&mut g, 0, "Murcules");
    // A minion in hand to take the buff; its pool copy goes with it.
    let hand_card = id_of(&g.data.cards, "Hackerfin");
    let slot = g.data.cards.get(hand_card).pool_slot as usize;
    g.pool[slot] -= 1;
    let m = g.players[0].make_card(&g.data.cards, hand_card, false, 1);
    g.players[0].hand.push(m);
    let q = opponent(&g, 0);
    // A 1/1 for Murcules to kill, and nothing else on the other side.
    g.players[q].board.push(g.players[q].make_card(&g.data.cards, hand_card, false, 0));
    g.players[q].board[0].attack = 0;
    g.players[q].board[0].health = 1;
    let t = g.turn;
    while g.turn == t && !g.over {
        g.step(Action::EndTurn).unwrap();
    }
    let base = g.data.cards.get(hand_card);
    let got = stats(&g.players[0].hand[0]);
    assert_eq!(
        got,
        (base.attack + 3, base.health + 3),
        "the kill's +3/+3 reached the hand"
    );
    g.check_invariants().unwrap();
}
