//! Fixes from docs/notes/card-audit.md, with the real cards from cards.toml:
//! Gunpowder Courier at 5 Gold, the generated-card tiers and the Satellite,
//! "after you play" never counting the played card, Disguised Graverobber,
//! and the four tier 7 spells (which no draw reaches).

use std::sync::Arc;

use bgcore::fight::{Watch, fight_with};
use bgcore::{Tribe, Action, CardDb, CardId, Counters, Data, Game, HAND, Keywords, Minion, Rng,};

fn db() -> CardDb {
    Data::bundled().cards
}

fn lobby(ts: &[Tribe]) -> bgcore::Tribes {
    ts.iter().copied().fold(bgcore::Tribes::NONE, |m, t| m.with(t))
}

fn game(seed: u64) -> Game {
    Game::new_with_lobby(Arc::new(Data::bundled()), seed, lobby(&[Tribe::Beast, Tribe::Demon, Tribe::Elemental, Tribe::Mech, Tribe::Pirate, Tribe::Undead]))
}

fn id(db: &CardDb, name: &str) -> CardId {
    db.by_name(name).expect(name)
}

fn stats(m: &Minion) -> (i32, i32) {
    (m.attack, m.health)
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
        if !g.data.cards.get(m.card).token {
            let slot = g.data.cards.get(m.card).pool_slot as usize;
            g.pool[slot] += m.pool_copies;
        }
    }
    for n in names {
        let c = id(&g.data.cards, n);
        let slot = g.data.cards.get(c).pool_slot as usize;
        g.pool[slot] -= 1;
        let m = g.players[p].make_card(&g.data.cards, c, false, 1);
        g.players[p].shop.push(m);
    }
}

fn play(g: &mut Game, p: usize, hand: u8, pos: u8) {
    act(g, p, Action::Play { hand, pos });
}

fn names(g: &Game, cards: &[Minion]) -> Vec<String> {
    cards.iter().map(|m| g.data.cards.get(m.card).name.clone()).collect()
}

// ================= Gunpowder Courier =================

#[test]
fn gunpowder_courier_fires_every_five_gold() {
    let db = db();
    assert_eq!(db.get(id(&db, "Gunpowder Courier")).on_event.unwrap().every, 5);
    let mut g = game(1);
    put_on_board(&mut g, 0, "Gunpowder Courier", false);
    g.players[0].gold = 10;
    for _ in 0..4 {
        act(&mut g, 0, Action::Roll);
    }
    assert_eq!(stats(&g.players[0].board[0]), (2, 5));
    act(&mut g, 0, Action::Roll);
    assert_eq!(stats(&g.players[0].board[0]), (2 + 3, 5 + 1));
}

// ================= generated cards =================

#[test]
fn conflagration_is_tier_4_and_lockbox_is_tier_0_and_neither_is_ever_drawn() {
    let db = db();
    assert_eq!(db.get(id(&db, "Conflagration")).tier, 4);
    assert_eq!(db.get(id(&db, "Lockbox")).tier, 0);
    let mut rng = Rng::new(7);
    for max_tier in 0..=7u8 {
        for _ in 0..400 {
            for c in [
                db.random_spell(max_tier, bgcore::Tribes::ALL, &mut rng),
                db.random_spell_up_to(max_tier, &[], bgcore::Tribes::ALL, &mut rng),
                db.random_tier_spell(max_tier, &[], bgcore::Tribes::ALL, &mut rng),
            ]
            .into_iter()
            .flatten()
            {
                let card = db.get(c);
                assert!(!card.token, "{} is generated only", card.name);
                assert!((1..=6).contains(&card.tier), "{} has tier {}", card.name, card.tier);
            }
        }
    }
    // A random spell by cost, of any tier, still stops at tier 6.
    for cost in 0..=8u8 {
        for _ in 0..400 {
            if let Some(c) = db.random_spell_of_cost(cost, bgcore::Tribes::ALL, &mut rng) {
                assert!(db.get(c).tier <= 6, "{} is tier 7", db.get(c).name);
            }
        }
    }
}

#[test]
fn the_satellite_token_is_4_4_and_attachers_pass_their_own_stats() {
    let db = db();
    let sat = db.make(id(&db, "Satellite"), false);
    assert_eq!(stats(&sat), (4, 4));
    // Spark Snapper attaches a 2/3 that grows with its improvements.
    let mut g = game(2);
    put_on_board(&mut g, 0, "Spark Snapper", false);
    put_in_hand(&mut g, 0, "Cord Puller", false);
    play(&mut g, 0, 0, 1);
    assert_eq!(stats(&g.players[0].board[1]), (1 + 2, 1 + 3));
    put_in_hand(&mut g, 0, "Cord Puller", false);
    play(&mut g, 0, 0, 2);
    // second play: Snapper improved once, so (1 + 1) stacks of 2/3
    assert_eq!(stats(&g.players[0].board[2]), (1 + 4, 1 + 6));
    g.check_invariants().unwrap();
}

// ================= "After you play an X" =================

#[test]
fn wrath_weaver_ignores_its_own_play_and_reacts_to_the_next_demon() {
    let mut g = game(3);
    put_in_hand(&mut g, 0, "Wrath Weaver", false);
    g.players[0].health = 30;
    let before = stats(&g.data.cards.make(id(&g.data.cards, "Wrath Weaver"), false));
    play(&mut g, 0, 0, 0);
    assert_eq!(g.players[0].health, 30, "no damage for its own play");
    assert_eq!(stats(&g.players[0].board[0]), before);
    put_in_hand(&mut g, 0, "Flaming Enforcer", false);
    play(&mut g, 0, 0, 1);
    assert_eq!(g.players[0].health, 29);
    assert_eq!(stats(&g.players[0].board[0]), (before.0 + 2, before.1 + 2));
    g.check_invariants().unwrap();
}

#[test]
fn insatiable_urzul_ignores_its_own_play_and_reacts_to_the_next_demon() {
    let mut g = game(4);
    set_shop(&mut g, 0, &["Buzzing Vermin", "Wolf Pup"]);
    put_in_hand(&mut g, 0, "Insatiable Ur'zul", false);
    play(&mut g, 0, 0, 0);
    assert_eq!(g.players[0].shop.len(), 2, "it ate nothing for its own play");
    let base = stats(&g.players[0].board[0]);
    put_in_hand(&mut g, 0, "Flaming Enforcer", false);
    play(&mut g, 0, 0, 1);
    assert_eq!(g.players[0].shop.len(), 1, "it ate one for the next Demon");
    let (a, h) = stats(&g.players[0].board[0]);
    assert!(a > base.0 && h > base.1);
    g.check_invariants().unwrap();
}

#[test]
fn ultraviolet_ascendant_ignores_its_own_play() {
    let mut g = game(5);
    put_in_hand(&mut g, 0, "Ultraviolet Ascendant", false);
    play(&mut g, 0, 0, 0);
    assert_eq!(g.players[0].board[0].improvements, 0);
    put_in_hand(&mut g, 0, "Flaming Enforcer", false);
    play(&mut g, 0, 0, 1);
    assert_eq!(g.players[0].board[0].improvements, 1);
}

#[test]
fn no_card_reacts_to_its_own_play_in_the_data() {
    // Every after_play_tribe card is covered above; this guards the list.
    let db = db();
    let mut found: Vec<String> = Vec::new();
    for c in 0..db.cards.len() {
        if db.cards[c].after_play_tribe.is_some() {
            found.push(db.cards[c].name.clone());
        }
    }
    found.sort();
    assert_eq!(found, ["Insatiable Ur'zul", "Ultraviolet Ascendant", "Wrath Weaver"]);
}

// ================= Disguised Graverobber =================

#[test]
fn graverobber_is_a_tier_3_neutral_4_4_in_the_pool() {
    let db = db();
    let c = id(&db, "Disguised Graverobber");
    let card = db.get(c);
    assert_eq!(card.tier, 3);
    assert!(card.tribes.is_neutral());
    assert_eq!(stats(&db.make(c, false)), (4, 4));
    assert_eq!(stats(&db.make(c, true)), (8, 8));
    assert_eq!(card.pool_slot as usize, c as usize, "an ordinary pool card");
    assert!((c as usize) < db.pool_cards);
}

#[test]
fn graverobber_destroys_a_chosen_undead_fully_and_gets_a_plain_copy() {
    let mut g = game(6);
    // A golden, buffed Bonehead: the copy is the plain card.
    put_on_board(&mut g, 0, "Harmless Bonehead", true);
    g.players[0].board[0].attack += 9;
    put_on_board(&mut g, 0, "Wolf Pup", false);
    let slot = g.data.cards.get(id(&g.data.cards, "Harmless Bonehead")).pool_slot as usize;
    let pool_before = g.pool[slot];
    put_in_hand(&mut g, 0, "Disguised Graverobber", false);
    play(&mut g, 0, 0, 2);
    // Only one Undead, so no choice is asked. Its Deathrattle ran: a golden
    // Bonehead still summons a plain 1/1 Skeleton per copy, two each, so four
    // of them. Three of those make a Golden Skeleton in the hand at once
    // (docs/rules/design.md "Triples"), which leaves one on the board.
    assert_eq!(
        names(&g, &g.players[0].board),
        ["Skeleton", "Wolf Pup", "Disguised Graverobber"].map(String::from)
    );
    assert_eq!(g.players[0].hand.len(), 2);
    // The copy the Battlecry gives joins the hand first, and the game's own
    // triple check runs as the action ends, so the Golden Skeleton it makes
    // comes after it (log 2026_10_30_19_45_15-g2 turn 5 step 4).
    let copy = g.players[0].hand[0];
    assert_eq!(g.data.cards.get(copy.card).name, "Harmless Bonehead");
    assert!(!copy.golden);
    assert_eq!(stats(&copy), (1, 1));
    let golden = g.players[0].hand[1];
    assert_eq!(g.data.cards.get(golden.card).name, "Skeleton");
    assert!(golden.golden);
    assert_eq!(stats(&golden), (2, 2));
    assert_eq!(copy.pool_copies, 0, "generated: no pool copy");
    // The destroyed golden's three copies went back, the copy took none.
    assert_eq!(g.pool[slot], pool_before + 3);
    g.check_invariants().unwrap();
}

#[test]
fn graverobber_with_two_undead_waits_for_the_choice_and_only_takes_undead() {
    let mut g = game(7);
    put_on_board(&mut g, 0, "Harmless Bonehead", false);
    put_on_board(&mut g, 0, "Wolf Pup", false);
    put_on_board(&mut g, 0, "Risen Rider", false);
    put_in_hand(&mut g, 0, "Disguised Graverobber", false);
    play(&mut g, 0, 0, 3);
    assert!(g.players[0].pending_target.is_some());
    turn_of(&mut g, 0);
    assert!(!g.is_legal(Action::Target(1)), "a Beast is not a valid target");
    assert!(!g.is_legal(Action::Target(3)), "nor is the Graverobber itself");
    act(&mut g, 0, Action::Target(2));
    // The Rider fully died and came back through Reborn with 1 Health.
    let n = names(&g, &g.players[0].board);
    assert_eq!(
        n,
        ["Harmless Bonehead", "Wolf Pup", "Risen Rider", "Disguised Graverobber"].map(String::from)
    );
    assert_eq!(g.players[0].board[2].health, 1);
    assert_eq!(g.data.cards.get(g.players[0].hand[0].card).name, "Risen Rider");
    // The copy is the plain card, Reborn keyword included.
    assert!(g.players[0].hand[0].keywords.has(Keywords::REBORN));
    g.check_invariants().unwrap();
}

#[test]
fn graverobber_with_no_undead_does_nothing() {
    let mut g = game(8);
    put_on_board(&mut g, 0, "Wolf Pup", false);
    put_in_hand(&mut g, 0, "Disguised Graverobber", false);
    play(&mut g, 0, 0, 1);
    assert!(g.players[0].pending_target.is_none());
    assert!(g.players[0].hand.is_empty());
    assert_eq!(g.players[0].board.len(), 2);
}

#[test]
fn the_graverobber_destroy_is_a_real_shop_death() {
    // Plaguerunner's shop Deathrattle gives the Undead +4 Attack, and the
    // copy in hand carries that player bonus.
    let mut g = game(9);
    put_on_board(&mut g, 0, "Plaguerunner", false);
    put_in_hand(&mut g, 0, "Disguised Graverobber", false);
    play(&mut g, 0, 0, 1);
    assert_eq!(g.players[0].undead_bonus, (4, 0));
    assert_eq!(stats(&g.players[0].hand[0]), (4 + 4, 2));
    g.check_invariants().unwrap();

    // Eternal Knight: its death is counted.
    let mut g = game(10);
    put_on_board(&mut g, 0, "Eternal Knight", false);
    put_in_hand(&mut g, 0, "Disguised Graverobber", false);
    play(&mut g, 0, 0, 1);
    assert_eq!(g.players[0].knight_deaths, 1);
    g.check_invariants().unwrap();

    // Reborn: a Risen Rider comes back as a 1 Health Rider in the same slot.
    let mut g = game(11);
    put_on_board(&mut g, 0, "Risen Rider", false);
    put_in_hand(&mut g, 0, "Disguised Graverobber", false);
    play(&mut g, 0, 0, 1);
    assert_eq!(g.players[0].board.len(), 2);
    assert_eq!(g.data.cards.get(g.players[0].board[0].card).name, "Risen Rider");
    assert_eq!(g.players[0].board[0].health, 1);
    assert!(!g.players[0].board[0].keywords.has(Keywords::REBORN));
    // and the plain copy in hand still has Reborn
    assert!(g.players[0].hand[0].keywords.has(Keywords::REBORN));
    g.check_invariants().unwrap();
}

#[test]
fn golden_graverobber_gets_two_copies_and_a_full_hand_loses_the_rest() {
    let mut g = game(12);
    put_on_board(&mut g, 0, "Harmless Bonehead", false);
    put_in_hand(&mut g, 0, "Disguised Graverobber", true);
    play(&mut g, 0, 0, 1);
    let riders = g.players[0]
        .hand
        .iter()
        .filter(|m| g.data.cards.get(m.card).name == "Harmless Bonehead")
        .count();
    assert_eq!(riders, 2);
    g.check_invariants().unwrap();

    // One free slot after the play: the Triple Reward comes before anything
    // the Battlecry adds to the hand, so it takes the slot and both copies
    // are lost.
    let mut g = game(13);
    put_on_board(&mut g, 0, "Harmless Bonehead", false);
    put_in_hand(&mut g, 0, "Disguised Graverobber", true);
    while g.players[0].hand.len() < HAND {
        g.give(0, "Fortify").unwrap();
    }
    play(&mut g, 0, 0, 1);
    assert_eq!(g.players[0].hand.len(), HAND - 1 + 1);
    let riders = g.players[0]
        .hand
        .iter()
        .filter(|m| g.data.cards.get(m.card).name == "Harmless Bonehead")
        .count();
    assert_eq!(riders, 0);
    let rewards = g.players[0]
        .hand
        .iter()
        .filter(|m| g.data.cards.get(m.card).name == "Triple Reward")
        .count();
    assert_eq!(rewards, 1);
    g.check_invariants().unwrap();
}

// ================= Tier 7 spells =================

#[test]
fn the_tier_7_spells_exist_with_their_real_tier_and_cost() {
    let db = db();
    for (name, cost) in [
        ("Sacred Gift", 4),
        ("Sharing is Caring", 2),
        ("Hallowed Ritual", 5),
        ("Menagerie Tableware", 4),
    ] {
        let c = db.get(id(&db, name));
        assert!(c.is_spell());
        assert_eq!((c.tier, c.cost), (7, cost), "{name}");
    }
}

#[test]
fn a_whole_game_of_tavern_draws_never_offers_a_tier_7_spell() {
    // Every refresh, Discover and random spell stops at tier 6: roll a lot
    // at the top tier and look at every spell the Tavern shows.
    let mut g = game(14);
    g.players[0].tier = 6;
    g.players[0].gold = 255;
    for _ in 0..200 {
        turn_of(&mut g, 0);
        if !g.is_legal(Action::Roll) {
            break;
        }
        act(&mut g, 0, Action::Roll);
        let shop = &g.players[0].shop;
        for m in shop.iter() {
            assert!(g.data.cards.get(m.card).tier <= 6);
        }
        if let Some(s) = g.players[0].shop_spell {
            assert!(g.data.cards.get(s).tier <= 6);
        }
    }
}

#[test]
fn sacred_gift_gives_divine_shield_and_no_stats_even_with_spell_bonus() {
    let mut g = game(15);
    put_on_board(&mut g, 0, "Wolf Pup", false);
    g.players[0].spell_bonus = (1, 1);
    let before = stats(&g.players[0].board[0]);
    assert!(!g.players[0].board[0].keywords.has(Keywords::DIVINE_SHIELD));
    g.give(0, "Sacred Gift").unwrap();
    play(&mut g, 0, 0, 0);
    act(&mut g, 0, Action::Target(0));
    assert!(g.players[0].board[0].keywords.has(Keywords::DIVINE_SHIELD));
    assert_eq!(stats(&g.players[0].board[0]), before);
    g.check_invariants().unwrap();
}

#[test]
fn hallowed_ritual_discovers_a_tier_7_minion() {
    let mut g = game(16);
    g.give(0, "Hallowed Ritual").unwrap();
    play(&mut g, 0, 0, 0);
    let options = g.players[0].discover.clone();
    assert_eq!(options.len(), 3);
    for c in &options {
        assert_eq!(g.data.cards.get(*c).tier, 7);
    }
    act(&mut g, 0, Action::Choose(1));
    assert_eq!(g.players[0].hand.len(), 1);
    assert_eq!(g.players[0].hand[0].card, options[1]);
    g.check_invariants().unwrap();
}

#[test]
fn menagerie_tableware_repeats_for_each_different_friendly_type() {
    let mut g = game(17);
    put_on_board(&mut g, 0, "Wolf Pup", false); // Beast
    put_on_board(&mut g, 0, "Cord Puller", false); // Mech
    put_on_board(&mut g, 0, "Deadly Spore", false); // neutral, no type
    put_on_board(&mut g, 0, "Cadaver Caretaker", false); // Undead
    let before: Vec<_> = g.players[0].board.iter().map(stats).collect();
    g.give(0, "Menagerie Tableware").unwrap();
    play(&mut g, 0, 0, 0);
    // once, then once more for each of three different types
    for (m, b) in g.players[0].board.iter().zip(before) {
        assert_eq!(stats(m), (b.0 + 12, b.1 + 12));
    }
    g.check_invariants().unwrap();

    // With only neutrals it is just the one +3/+3.
    let mut g = game(18);
    put_on_board(&mut g, 0, "Deadly Spore", false);
    g.give(0, "Menagerie Tableware").unwrap();
    play(&mut g, 0, 0, 0);
    assert_eq!(stats(&g.players[0].board[0]), (1 + 3, 1 + 3));
}

#[derive(Default)]
struct Attacks(Vec<(usize, Minion)>);

impl Watch for Attacks {
    fn attack(&mut self, side: usize, attacker: &Minion, _defender: &Minion) {
        self.0.push((side, *attacker));
    }
}

fn blank(db: &CardDb, attack: i32, health: i32) -> Minion {
    let mut m = db.make(id(db, "Microbot"), false);
    m.attack = attack;
    m.health = health;
    m
}

#[test]
fn sharing_is_caring_gives_the_left_most_the_nearest_enemys_stats_at_start_of_combat() {
    let db = db();
    let a = [blank(&db, 2, 50), blank(&db, 1, 1)];
    let b = [blank(&db, 7, 40), blank(&db, 100, 1)];
    let one = Counters { sharing_is_caring: 1, ..Default::default() };
    let mut w = Attacks::default();
    fight_with(&db, &a, &b, [one, Counters::default()], 6, &mut Rng::new(1), &mut w);
    let first = w.0.iter().find(|(s, _)| *s == 0).unwrap().1;
    assert_eq!(stats(&first), (2 + 7, 50 + 40));
    // The enemy is untouched.
    let theirs = w.0.iter().find(|(s, _)| *s == 1).map(|x| x.1);
    if let Some(t) = theirs {
        assert_eq!(stats(&t), (7, 40));
    }
    // Two casts gain it twice.
    let two = Counters { sharing_is_caring: 2, ..Default::default() };
    let mut w = Attacks::default();
    fight_with(&db, &a, &b, [two, Counters::default()], 6, &mut Rng::new(1), &mut w);
    let first = w.0.iter().find(|(s, _)| *s == 0).unwrap().1;
    assert_eq!(stats(&first), (2 + 14, 50 + 80));
}

#[test]
fn sharing_is_caring_lasts_for_one_fight() {
    let mut g = game(19);
    put_on_board(&mut g, 0, "Wolf Pup", false);
    g.give(0, "Sharing is Caring").unwrap();
    play(&mut g, 0, 0, 0);
    assert_eq!(g.players[0].sharing_is_caring, 1);
    let t = g.turn;
    while g.turn == t {
        if !answer(&mut g) {
            g.step(Action::EndTurn).unwrap();
        }
    }
    assert_eq!(g.players[0].sharing_is_caring, 0);
    g.check_invariants().unwrap();
}
