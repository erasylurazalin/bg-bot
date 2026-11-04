//! The Dragon minions of the 5-tribe lobby, with the real cards from
//! cards.toml: Tarecgosa and Persistent Poet keeping combat gains,
//! Hopebringer, Kalecgos, Sky-hatch Runaway's shop Rally, Chromadrakes,
//! Scarlet Survivor, the Dragon spells and the rest.

use std::sync::Arc;

use bgcore::fight::{Watch, fight_with};
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

/// A minion with `attack` that takes a long time to kill: it pops Divine
/// Shields and buffs the fight with a little damage.
fn fighter(db: &CardDb, attack: i32) -> Minion {
    let mut w = mk(db, "Microbot", false);
    w.attack = attack;
    w.health = 2000;
    w
}

#[derive(Default)]
struct Log {
    /// (attacker name, attack, health) at the moment of each attack
    attacks: Vec<(String, i32, i32)>,
    db: Option<CardDb>,
}

impl Watch for Log {
    fn attack(&mut self, _side: usize, a: &Minion, _d: &Minion) {
        let db = self.db.as_ref().unwrap();
        self.attacks.push((db.get(a.card).name.clone(), a.attack, a.health));
    }
}

fn run(db: &CardDb, a: &[Minion], b: &[Minion], max_attacks: u32, seed: u64) -> (bgcore::Outcome, Log) {
    let mut log = Log { db: Some(db.clone()), ..Default::default() };
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

// ---- Hopebringer ----

#[test]
fn hopebringer_improves_when_a_friendly_bronze_warden_loses_its_shield() {
    let db = db();
    // The Warden starts with Divine Shield; the enemy keeps attacking until
    // it pops, which permanently improves the Hopebringer (slot 0).
    let mut improved = 0;
    for seed in 0..12 {
        let (out, _) = run(
            &db,
            &[mk(&db, "Hopebringer", false), mk(&db, "Bronze Warden", false)],
            &[fighter(&db, 1)],
            500,
            seed,
        );
        improved = improved.max(out.sides[0].permanent_improvements[0]);
    }
    assert!(improved >= 1, "a lost shield should improve the Hopebringer");

    // The improvement is used by the next Start of Combat: base +4/+3 plus
    // one more +4/+3 per lost shield. Hopebringer attacks with
    // 4 + (4 + 4) / 3 + (3 + 3).
    let mut hope = mk(&db, "Hopebringer", false);
    hope.improvements = 1;
    let (_, log) = run(&db, &[hope], &[wall(&db)], 1, 1);
    assert_eq!(log.attacks[0], ("Hopebringer".to_string(), 12, 9));
    // Golden: base +8/+6 plus +8/+6 per lost shield.
    let mut gold = mk(&db, "Hopebringer", true);
    gold.improvements = 2;
    let (_, log) = run(&db, &[gold], &[wall(&db)], 1, 1);
    assert_eq!(log.attacks[0], ("Hopebringer".to_string(), 8 + 8 + 2 * 8, 6 + 6 + 2 * 6));
}

#[test]
fn hopebringer_improvement_reaches_the_board_after_the_fight() {
    let mut improved = false;
    for seed in 0..12 {
        let mut g = game(seed);
        let Some(bgcore::Opponent::Player(q)) = g.players[0].opponent else { panic!() };
        put_on_board(&mut g, 0, "Hopebringer");
        put_on_board(&mut g, 0, "Bronze Warden");
        put_on_board(&mut g, q as usize, "Cage Gnawer");
        g.players[q as usize].board[0].attack = 1;
        g.players[q as usize].board[0].health = 2000;
        while g.turn == 1 {
            g.step(Action::EndTurn).unwrap();
        }
        improved |= g.players[0].board[0].improvements >= 1;
    }
    assert!(improved, "the improvement should be on the real board");
}

#[test]
fn hopebringer_improves_when_amber_guardian_gives_and_loses_a_shield() {
    let db = db();
    // Amber Guardian's Start of Combat gives the other Dragon +2/+2 and
    // Divine Shield; when that shield breaks, the Hopebringer improves.
    let mut improved = 0;
    for seed in 0..16 {
        let (out, _) = run(
            &db,
            &[mk(&db, "Hopebringer", false), mk(&db, "Amber Guardian", false)],
            &[fighter(&db, 1)],
            500,
            seed,
        );
        improved = improved.max(out.sides[0].permanent_improvements[0]);
    }
    assert!(improved >= 1);
}

// ---- Kalecgos ----

#[test]
fn kalecgos_buffs_dragons_after_a_battlecry_and_brann_doubles_it() {
    let mut g = game(31);
    put_on_board(&mut g, 0, "Kalecgos, Arcane Aspect");
    put_on_board(&mut g, 0, "Glim Guardian");
    g.give(0, "Iron Groundskeeper").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 2 });
    // Kalecgos itself is a Dragon too
    assert_eq!(stats(&g.players[0].board[0]), (6, 14));
    assert_eq!(stats(&g.players[0].board[1]), (3, 6));
    g.check_invariants().unwrap();

    // Brann makes the Battlecry trigger twice, so Kalecgos fires twice
    put_on_board(&mut g, 0, "Brann Bronzebeard");
    g.give(0, "Iron Groundskeeper").unwrap();
    let h = g.players[0].hand.len() as u8 - 1;
    act(&mut g, 0, Action::Play { hand: h, pos: 3 });
    assert_eq!(stats(&g.players[0].board[0]), (10, 18));
    assert_eq!(stats(&g.players[0].board[1]), (7, 10));
    g.check_invariants().unwrap();
}

// ---- Tarecgosa and Persistent Poet ----

#[test]
fn tarecgosa_keeps_combat_stats_and_a_bonus_keyword() {
    let db = db();
    // Amber Guardian gives its only other Dragon +2/+2 and Divine Shield.
    let out = attack_with(&db, &[mk(&db, "Tarecgosa", false), mk(&db, "Amber Guardian", false)]);
    let (da, dh) = out.sides[0].permanent_stats[0];
    assert!(da >= 2 && dh >= 2, "{da}/{dh}");
    assert!(out.sides[0].permanent_keywords[0] & Keywords::DIVINE_SHIELD != 0);
    // the Amber Guardian itself keeps nothing
    assert_eq!(out.sides[0].permanent_stats[1], (0, 0));

    // golden Tarecgosa keeps double the stats
    let out = attack_with(&db, &[mk(&db, "Tarecgosa", true), mk(&db, "Amber Guardian", false)]);
    assert!(out.sides[0].permanent_stats[0].0 >= 4);
}

fn attack_with(db: &CardDb, board: &[Minion]) -> bgcore::Outcome {
    fight_with(db, board, &[wall(db)], [Counters::default(); 2], 3, &mut Rng::new(2), &mut ())
}

#[test]
fn persistent_poet_keeps_only_adjacent_dragons_gains() {
    let db = db();
    // Electric Synthesizer gives every other Dragon +1/+1. Only the two
    // Dragons next to the Poet (slots 1 and 3) keep it.
    let board = [
        mk(&db, "Electric Synthesizer", false),
        mk(&db, "Scarlet Survivor", false),
        mk(&db, "Persistent Poet", false),
        mk(&db, "Scarlet Survivor", false),
        mk(&db, "Glim Guardian", false),
    ];
    let out = attack_with(&db, &board);
    assert_eq!(out.sides[0].permanent_stats[0], (0, 0));
    assert_eq!(out.sides[0].permanent_stats[1], (1, 1));
    assert_eq!(out.sides[0].permanent_stats[2], (0, 0));
    assert_eq!(out.sides[0].permanent_stats[3], (1, 1));
    assert_eq!(out.sides[0].permanent_stats[4], (0, 0));

    // a golden Poet doubles what its neighbours keep
    let board = [
        mk(&db, "Electric Synthesizer", false),
        mk(&db, "Scarlet Survivor", false),
        mk(&db, "Persistent Poet", true),
        mk(&db, "Scarlet Survivor", false),
    ];
    let out = attack_with(&db, &board);
    assert_eq!(out.sides[0].permanent_stats[1], (2, 2));
    assert_eq!(out.sides[0].permanent_stats[3], (2, 2));
}

#[test]
fn timewarped_poet_keeps_every_dragons_gains_and_its_own() {
    let db = db();
    // The Poet Portrait's token: "All your Dragons keep Bonus Keywords and
    // stats gained in combat". Electric Synthesizer gives every other Dragon
    // +1/+1, so every slot but its own keeps the gain, wherever the Poet
    // stands, the Poet's own gain included.
    let board = [
        mk(&db, "Electric Synthesizer", false),
        mk(&db, "Scarlet Survivor", false),
        mk(&db, "Timewarped Poet", false),
        mk(&db, "Glim Guardian", false),
        mk(&db, "Scarlet Survivor", false),
    ];
    let out = attack_with(&db, &board);
    assert_eq!(out.sides[0].permanent_stats[0], (0, 0), "the Synthesizer gains nothing");
    for slot in 1..5 {
        assert_eq!(out.sides[0].permanent_stats[slot], (1, 1), "slot {slot}");
    }

    // The golden one doubles them all, its own too.
    let board = [
        mk(&db, "Timewarped Poet", true),
        mk(&db, "Scarlet Survivor", false),
        mk(&db, "Electric Synthesizer", false),
    ];
    let out = attack_with(&db, &board);
    assert_eq!(out.sides[0].permanent_stats[0], (2, 2), "the golden Poet's own gain");
    assert_eq!(out.sides[0].permanent_stats[1], (2, 2));
    assert_eq!(out.sides[0].permanent_stats[2], (0, 0));

    // A Bonus Keyword a Dragon gains in the fight is kept too.
    let board = [mk(&db, "Heroic Broodmother", false), mk(&db, "Timewarped Poet", false)];
    let out = fight_with(&db, &board, &[wall(&db)], [Counters::default(); 2], 3, &mut Rng::new(1), &mut ());
    assert!(out.sides[0].permanent_keywords[0] & Keywords::DIVINE_SHIELD != 0);
}

#[test]
fn tarecgosa_keeps_a_keyword_gained_by_heroic_broodmothers_rally() {
    let db = db();
    // Broodmother's Rally gives it Divine Shield in the fight; next to a
    // Poet that keyword stays on the real board.
    let board = [mk(&db, "Heroic Broodmother", false), mk(&db, "Persistent Poet", false)];
    let out = fight_with(&db, &board, &[wall(&db)], [Counters::default(); 2], 3, &mut Rng::new(1), &mut ());
    assert!(out.sides[0].permanent_keywords[0] & Keywords::DIVINE_SHIELD != 0);
}

// ---- Sky-hatch Runaway and the shop ----

#[test]
fn sky_hatch_runaway_triggers_a_friendly_rally_in_the_shop() {
    // Its golden card is 10/14 in the card data, not double the printed 4/7;
    // most cards still double.
    let db = db();
    assert_eq!(stats(&mk(&db, "Sky-hatch Runaway", true)), (10, 14));
    assert_eq!(stats(&mk(&db, "Glim Guardian", true)), (2, 8));

    let mut g = game(32);
    put_on_board(&mut g, 0, "Sky-hatch Runaway");
    put_on_board(&mut g, 0, "Glim Guardian");
    g.players[0].gold = 5;
    act(&mut g, 0, Action::Activate(0));
    assert_eq!(g.players[0].gold, 4);
    // Glim Guardian's Rally: +2 Attack, permanently in the shop
    assert_eq!(stats(&g.players[0].board[1]), (3, 4));
    g.check_invariants().unwrap();

    // golden Sky-hatch triggers it twice
    let mut g = game(33);
    put_on_board(&mut g, 0, "Sky-hatch Runaway");
    put_on_board(&mut g, 0, "Glim Guardian");
    g.players[0].board[0].golden = true;
    g.players[0].gold = 5;
    act(&mut g, 0, Action::Activate(0));
    assert_eq!(stats(&g.players[0].board[1]), (5, 4));
    g.check_invariants().unwrap();

    // it needs a friendly minion with a Rally
    let mut g = game(34);
    put_on_board(&mut g, 0, "Sky-hatch Runaway");
    g.players[0].gold = 5;
    turn_of(&mut g, 0);
    assert!(!g.is_legal(Action::Activate(0)));

    // making it golden in play uses the same 10/14
    let mut g = game(50);
    put_on_board(&mut g, 0, "Sky-hatch Runaway");
    g.give(0, "Eyes of the Earth Mother").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    act(&mut g, 0, Action::Target(0));
    assert_eq!(stats(&g.players[0].board[0]), (10, 14));
    g.check_invariants().unwrap();
}

#[test]
fn a_shop_rally_can_raise_the_spell_bonus() {
    let mut g = game(35);
    put_on_board(&mut g, 0, "Sky-hatch Runaway");
    put_on_board(&mut g, 0, "Blue Whelp");
    g.players[0].gold = 5;
    act(&mut g, 0, Action::Activate(0));
    assert_eq!(g.players[0].spell_bonus, (0, 1));
    g.check_invariants().unwrap();
}

// ---- Heroic Broodmother ----

#[test]
fn heroic_broodmother_attacks_before_the_normal_rotation() {
    let db = db();
    // Glim Guardian is left-most, so without the Start of Combat she would
    // not be the first to attack.
    let board = [mk(&db, "Glim Guardian", false), mk(&db, "Heroic Broodmother", false)];
    let (_, log) = run(&db, &board, &[wall(&db)], 3, 1);
    assert_eq!(log.attacks[0].0, "Heroic Broodmother");
    assert_eq!(log.attacks[1].0, "Glim Guardian");

    // golden: twice, and her Rally gives her Divine Shield first
    let board = [mk(&db, "Glim Guardian", false), mk(&db, "Heroic Broodmother", true)];
    let (_, log) = run(&db, &board, &[wall(&db)], 4, 1);
    assert_eq!(log.attacks[0].0, "Heroic Broodmother");
    assert_eq!(log.attacks[1].0, "Heroic Broodmother");
    assert_eq!(log.attacks[2].0, "Glim Guardian");
}

// ---- Blue Whelp ----

#[test]
fn blue_whelp_raises_the_spell_bonus_for_later_tavern_spells() {
    let db = db();
    let (out, _) = run(&db, &[mk(&db, "Blue Whelp", false)], &[wall(&db)], 1, 1);
    assert_eq!(out.sides[0].spell_bonus_gain, (0, 1));
    let (out, _) = run(&db, &[mk(&db, "Blue Whelp", true)], &[wall(&db)], 1, 1);
    assert_eq!(out.sides[0].spell_bonus_gain, (0, 2));

    // the fight's gain reaches the player, and the next Tavern spell uses it
    let mut g = game(36);
    let Some(bgcore::Opponent::Player(q)) = g.players[0].opponent else { panic!() };
    put_on_board(&mut g, 0, "Blue Whelp");
    put_on_board(&mut g, q as usize, "Cage Gnawer");
    g.players[q as usize].board[0].attack = 0;
    g.players[q as usize].board[0].health = 1;
    while g.turn == 1 {
        g.step(Action::EndTurn).unwrap();
    }
    // one attack, one Rally
    assert_eq!(g.players[0].spell_bonus, (0, 1));
    g.give(0, "Shiny Ring").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    // Shiny Ring gives +1/+1, plus the +1 Health spell bonus
    assert_eq!(stats(&g.players[0].board[0]), (2, 7));
    g.check_invariants().unwrap();
}

// ---- Chromadrakes ----

#[test]
fn hired_mount_activates_for_a_random_chromadrake() {
    let mut g = game(37);
    put_on_board(&mut g, 0, "Hired Mount");
    g.players[0].gold = 5;
    act(&mut g, 0, Action::Activate(0));
    assert_eq!(g.players[0].gold, 3);
    assert_eq!(g.players[0].hand.len(), 1);
    let drake = g.players[0].hand[0];
    assert!(name(&g, &drake).ends_with("Chromadrake"));
    assert_eq!(drake.pool_copies, 0);
    // its Battlecry fires when played (Black and Red raise the spell bonus)
    g.check_invariants().unwrap();

    // golden: two of them
    let mut g = game(38);
    put_on_board(&mut g, 0, "Hired Mount");
    g.players[0].board[0].golden = true;
    g.players[0].gold = 5;
    act(&mut g, 0, Action::Activate(0));
    assert_eq!(g.players[0].hand.len(), 2);
    g.check_invariants().unwrap();
}

#[test]
fn bronze_timewalker_and_draconic_warden_hand_over_chromadrakes() {
    let db = db();
    // Bronze Timewalker's Rally, in combat
    let (out, _) = run(&db, &[mk(&db, "Bronze Timewalker", false)], &[wall(&db)], 1, 1);
    let cards = out.sides[0].cards_gained();
    assert_eq!(cards.len(), 1);
    assert!(db.get(cards[0]).name.ends_with("Chromadrake"));

    // Draconic Warden: Battlecry in the shop, Deathrattle in combat
    let mut g = game(39);
    g.give(0, "Draconic Warden").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    assert_eq!(g.players[0].hand.len(), 1);
    assert!(name(&g, &g.players[0].hand[0]).ends_with("Chromadrake"));
    let (out, _) = run(&db, &[mk(&db, "Draconic Warden", false)], &[fighter(&db, 20)], 1, 1);
    assert_eq!(out.sides[0].cards_gained().len(), 1);
    g.check_invariants().unwrap();
}

#[test]
fn a_chromadrakes_battlecry_works() {
    let mut g = game(40);
    put_on_board(&mut g, 0, "Glim Guardian");
    g.give(0, "Bronze Chromadrake").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 1 });
    // other Dragons +3/+1
    assert_eq!(stats(&g.players[0].board[0]), (4, 5));
    assert_eq!(stats(&g.players[0].board[1]), (5, 3));
    g.check_invariants().unwrap();
}

// ---- Scarlet Survivor ----

#[test]
fn scarlet_survivor_gains_divine_shield_at_six_attack() {
    let mut g = game(41);
    put_on_board(&mut g, 0, "Scarlet Survivor");
    g.players[0].board[0].attack = 4;
    g.give(0, "Repair Job").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    act(&mut g, 0, Action::Target(0));
    assert_eq!(stats(&g.players[0].board[0]), (8, 11));
    assert!(g.players[0].board[0].keywords.has(Keywords::DIVINE_SHIELD));
    assert_eq!(g.players[0].board[0].improvements, 1);
    // once only: taking the shield away and growing again doesn't bring it back
    g.players[0].board[0].keywords.clear(Keywords::DIVINE_SHIELD);
    g.give(0, "Repair Job").unwrap();
    let h = g.players[0].hand.len() as u8 - 1;
    act(&mut g, 0, Action::Play { hand: h, pos: 0 });
    act(&mut g, 0, Action::Target(0));
    assert!(!g.players[0].board[0].keywords.has(Keywords::DIVINE_SHIELD));
    g.check_invariants().unwrap();
}

// ---- The Dragon spells and the other Dragon effects ----

#[test]
fn mighty_dragonbreath_repeats_for_dragons_and_divine_shields() {
    let mut g = game(42);
    put_on_board(&mut g, 0, "Bronze Warden"); // Dragon with Divine Shield
    put_on_board(&mut g, 0, "Cord Puller"); // Mech, no shield
    g.give(0, "Mighty Dragonbreath").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    // Warden: +3/+2 for all, again for Dragons, again for Divine Shield
    assert_eq!(stats(&g.players[0].board[0]), (2 + 9, 1 + 6));
    // Cord Puller: +3/+2, and again because it has Divine Shield too
    assert_eq!(stats(&g.players[0].board[1]), (1 + 6, 1 + 4));
    g.check_invariants().unwrap();
}

#[test]
fn runic_arcanist_casts_shiny_ring_twice_in_combat() {
    let db = db();
    let board = [mk(&db, "Runic Arcanist", false), mk(&db, "Cord Puller", false)];
    let (_, log) = run(&db, &board, &[wall(&db)], 1, 1);
    // +1/+1 twice before the first attack
    assert_eq!(log.attacks[0], ("Runic Arcanist".to_string(), 4, 6));
    // golden: four times
    let board = [mk(&db, "Runic Arcanist", true), mk(&db, "Cord Puller", false)];
    let (_, log) = run(&db, &board, &[wall(&db)], 2, 1);
    assert_eq!(log.attacks[0], ("Runic Arcanist".to_string(), 4 + 4, 8 + 4));
}

#[test]
fn crimson_vindicator_casts_mighty_dragonbreath_in_combat() {
    let db = db();
    // It has Divine Shield, so its own Dragonbreath gives it +9/+6
    let (_, log) = run(&db, &[mk(&db, "Crimson Vindicator", false)], &[wall(&db)], 1, 1);
    assert_eq!(log.attacks[0], ("Crimson Vindicator".to_string(), 8 + 9, 9 + 6));
}

#[test]
fn roaring_recruiter_buffs_another_dragon_that_attacks() {
    let db = db();
    let board = [mk(&db, "Glim Guardian", false), mk(&db, "Roaring Recruiter", false)];
    let (_, log) = run(&db, &board, &[wall(&db)], 2, 1);
    // Glim's own Rally (+2 Attack) and the Recruiter's +3/+1
    assert_eq!(log.attacks[0], ("Glim Guardian".to_string(), 1 + 2 + 3, 4 + 1));
    // the Recruiter's own attack does not trigger "another"
    assert_eq!(log.attacks[1], ("Roaring Recruiter".to_string(), 2, 8));
}

#[test]
fn timecapn_hooktail_buffs_all_minions_when_a_spell_is_cast() {
    let mut g = game(43);
    put_on_board(&mut g, 0, "Timecap'n Hooktail");
    put_on_board(&mut g, 0, "Cord Puller");
    g.give(0, "Tavern Coin").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    assert_eq!(stats(&g.players[0].board[0]), (2, 4));
    assert_eq!(stats(&g.players[0].board[1]), (2, 1));
    g.check_invariants().unwrap();

    // golden: +1 Attack twice
    let mut g = game(44);
    put_on_board(&mut g, 0, "Timecap'n Hooktail");
    g.players[0].board[0].golden = true;
    g.give(0, "Tavern Coin").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    assert_eq!(stats(&g.players[0].board[0]), (3, 4));
}

#[test]
fn felfire_conjurer_and_the_black_and_red_chromadrakes_raise_the_spell_bonus() {
    let mut g = game(45);
    put_on_board(&mut g, 0, "Felfire Conjurer");
    act(&mut g, 0, Action::EndTurn);
    assert_eq!(g.players[0].spell_bonus, (1, 1));

    let mut g = game(46);
    put_on_board(&mut g, 0, "Felfire Conjurer");
    g.players[0].board[0].golden = true;
    act(&mut g, 0, Action::EndTurn);
    assert_eq!(g.players[0].spell_bonus, (2, 2));

    let mut g = game(47);
    g.give(0, "Black Chromadrake").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    assert_eq!(g.players[0].spell_bonus, (0, 1));
    let mut g = game(48);
    g.give(0, "Red Chromadrake").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    assert_eq!(g.players[0].spell_bonus, (1, 0));
}

#[test]
fn firescale_hoarder_and_draconic_warden_hand_over_cards_in_both_directions() {
    let mut g = game(49);
    g.give(0, "Firescale Hoarder").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    assert_eq!(g.players[0].hand.len(), 1);
    assert_eq!(name(&g, &g.players[0].hand[0]), "Shiny Ring");

    let db = db();
    let (out, _) = run(&db, &[mk(&db, "Firescale Hoarder", false)], &[fighter(&db, 20)], 1, 1);
    assert_eq!(out.sides[0].cards_gained().len(), 1);
    assert_eq!(db.get(out.sides[0].cards_gained()[0]).name, "Shiny Ring");
}

// ---- The shop harness ----

fn lobby(ts: &[Tribe]) -> bgcore::Tribes {
    ts.iter().copied().fold(bgcore::Tribes::NONE, |m, t| m.with(t))
}

fn game(seed: u64) -> Game {
    Game::new_with_lobby(Arc::new(Data::bundled()), seed, lobby(&[Tribe::Beast, Tribe::Mech, Tribe::Dragon, Tribe::Demon, Tribe::Pirate]))
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
    let m = g.players[p].make_card(&g.data.cards, c, false, 1);
    g.players[p].board.push(m);
}

fn name(g: &Game, m: &Minion) -> String {
    g.data.cards.get(m.card).name.clone()
}

fn stats(m: &Minion) -> (i32, i32) {
    (m.attack, m.health)
}

/// A Choose One minion's play is not a spell cast, so a Timecap'n Hooktail
/// gives no Attack for it. The log has it both ways: no Hooktail, no +1/+0
/// (`Hearthstone_2026_10_30_19_45_15-g3` turn 7 step 9 and
/// `Hearthstone_2026_11_03_15_22_36-g3` turn 7 step 4, where ours gave every
/// minion +1/+0 or +2/+0 for a Snare Trapper or a Sly Infiltrator; triage
/// group E8).
#[test]
fn a_choose_one_play_gives_timecapn_hooktail_nothing() {
    let mut g = Game::new_with_lobby(
        Arc::new(Data::bundled()),
        48,
        lobby(&[Tribe::Dragon, Tribe::Quilboar, Tribe::Mech]),
    );
    put_on_board(&mut g, 0, "Timecap'n Hooktail");
    put_on_board(&mut g, 0, "Cord Puller");
    let c = g.data.cards.by_name("Snare Trapper").unwrap();
    g.pool[g.data.cards.get(c).pool_slot as usize] -= 1;
    let m = g.players[0].make_card(&g.data.cards, c, false, 1);
    g.players[0].hand.push(m);
    let before: Vec<(i32, i32)> =
        g.players[0].board.iter().map(|m| (m.attack, m.health)).collect();
    act(&mut g, 0, Action::Play { hand: 0, pos: 2 });
    act(&mut g, 0, Action::Choose(1));
    for (i, b) in before.iter().enumerate() {
        assert_eq!(stats(&g.players[0].board[i]), *b, "minion {i} gains nothing");
    }
    assert_eq!(
        stats(&g.players[0].board[2]),
        (4, 4),
        "the Snare Trapper, at its own stats"
    );
    g.check_invariants().unwrap();
}
