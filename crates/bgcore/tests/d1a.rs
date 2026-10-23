//! Tests for the phase D1a systems, with the test-only cards in
//! `data/d1a-cards.toml`: five tribes and multi-tribe cards, Activate, Bonus
//! Keywords and Stealth, consuming Tavern minions, Tavern buffs, event
//! counters, and hero health as a resource.

use std::sync::Arc;

use bgcore::bots::random_action;
use bgcore::fight::{Watch, fight_watched};
use bgcore::{
    ACTION_KINDS, Action, CardDb, Data, Game, Keywords, Minion, N_ACTIONS, Rng, Tribe, Tribes,
};

fn data() -> Data {
    Data::from_toml(include_str!("data/d1a-cards.toml"), include_str!("data/v01-rules.toml"))
        .unwrap()
}

fn game(seed: u64) -> Game {
    Game::new(Arc::new(data()), seed)
}

/// Let the other players toggle freeze until it's `p`'s action.
fn turn_of(g: &mut Game, p: usize) {
    while g.current_player() != Some(p) {
        g.step(Action::Freeze).unwrap();
    }
}

fn act(g: &mut Game, p: usize, a: Action) {
    turn_of(g, p);
    g.step(a).unwrap_or_else(|_| panic!("{a:?} illegal for player {p}"));
}

fn is_legal(g: &mut Game, p: usize, a: Action) -> bool {
    turn_of(g, p);
    g.is_legal(a)
}

/// Everyone ends their turn until the next turn starts.
fn next_turn(g: &mut Game) {
    let t = g.turn;
    while g.turn == t && !g.over {
        g.step(Action::EndTurn).unwrap();
    }
}

fn id(g: &Game, name: &str) -> bgcore::CardId {
    g.data.cards.by_name(name).unwrap_or_else(|| panic!("no card {name}"))
}

/// Put a card from the pool straight onto a board.
fn put_on_board(g: &mut Game, p: usize, name: &str) {
    let c = id(g, name);
    let slot = g.data.cards.get(c).pool_slot as usize;
    g.pool[slot] -= 1;
    let m = g.players[p].make_card(&g.data.cards, c, false, 1);
    g.players[p].board.push(m);
}

/// Replace a player's shop with the named cards, through the pool.
fn set_shop(g: &mut Game, p: usize, names: &[&str]) {
    let old: Vec<Minion> = g.players[p].shop.drain(..).collect();
    for m in old {
        let slot = g.data.cards.get(m.card).pool_slot as usize;
        g.pool[slot] += m.pool_copies;
    }
    for n in names {
        let c = id(g, n);
        let slot = g.data.cards.get(c).pool_slot as usize;
        g.pool[slot] -= 1;
        let m = g.players[p].make_card(&g.data.cards, c, false, 1);
        g.players[p].shop.push(m);
    }
}

/// Give a card and play it at the end of the board.
fn play(g: &mut Game, p: usize, name: &str) {
    let m = g.give(p, name).unwrap();
    let h = g.players[p].hand.len() as u8 - 1;
    let _ = m;
    let pos = g.players[p].board.len() as u8;
    act(g, p, Action::Play { hand: h, pos });
}

fn stats(g: &Game, p: usize, slot: usize) -> (i32, i32) {
    let m = &g.players[p].board[slot];
    (m.attack, m.health)
}

fn base(g: &Game, name: &str) -> (i32, i32) {
    let c = g.data.cards.get(id(g, name));
    (c.attack, c.health)
}

fn legal(g: &Game) -> Vec<Action> {
    let mut mask = [false; N_ACTIONS];
    g.legal_mask(&mut mask);
    (0..N_ACTIONS).filter(|&i| mask[i]).map(|i| Action::from_index(i).unwrap()).collect()
}

// ---- tribes ----

#[test]
fn tribe_sets_hold_one_two_or_all() {
    let db = data().cards;
    let t = |n: &str| db.get(db.by_name(n).unwrap()).tribes;
    assert!(t("Test Beast").has(Tribe::Beast) && !t("Test Beast").has(Tribe::Dragon));
    assert!(t("Test Neutral").is_neutral());
    let dd = t("Test Dragon Demon");
    assert!(dd.has(Tribe::Dragon) && dd.has(Tribe::Demon) && !dd.has(Tribe::Murloc));
    assert!(!dd.is_neutral());
    for tribe in Tribe::ALL {
        assert!(t("Test Chameleon").has(tribe));
    }
    assert!(!t("Test Chameleon").is_neutral());
    assert!(dd.overlaps(Tribes::of(Tribe::Demon)) && !dd.overlaps(Tribes::of(Tribe::Mech)));
}

#[test]
fn a_card_needs_exactly_one_of_tribe_and_tribes() {
    let both = "[[minion]]\nname = \"A\"\ntier = 1\ntribe = \"beast\"\ntribes = [\"mech\"]\nstats = [1, 1]\n";
    assert!(CardDb::from_toml(both).unwrap_err().contains("both tribe and tribes"));
    let none = "[[minion]]\nname = \"A\"\ntier = 1\nstats = [1, 1]\n";
    assert!(CardDb::from_toml(none).unwrap_err().contains("needs tribe"));
    let bad = "[[minion]]\nname = \"A\"\ntier = 1\ntribes = [\"beast\", \"quilboar\"]\nstats = [1, 1]\n";
    assert!(CardDb::from_toml(bad).unwrap_err().contains("unknown tribe"));
}

#[test]
fn tribe_filters_count_multi_tribe_and_all_minions() {
    // Dragon Buffer reaches every Dragon, the Dragon Demon and the Chameleon.
    let mut g = game(1);
    for n in ["Test Dragon", "Test Dragon Demon", "Test Chameleon", "Test Beast", "Test Demon"] {
        put_on_board(&mut g, 0, n);
    }
    play(&mut g, 0, "Test Dragon Buffer");
    act(&mut g, 0, Action::Sell(5));
    let got: Vec<_> = (0..5).map(|s| stats(&g, 0, s)).collect();
    assert_eq!(got, [(3, 3), (4, 4), (4, 4), (2, 2), (2, 2)]);

    // Demon Buffer: the Demon, the Dragon Demon and the Chameleon.
    play(&mut g, 0, "Test Demon Buffer");
    act(&mut g, 0, Action::Sell(5));
    let got: Vec<_> = (0..5).map(|s| stats(&g, 0, s)).collect();
    assert_eq!(got, [(3, 3), (5, 5), (5, 5), (2, 2), (3, 3)]);

    // Beast Buffer: the Beast and the Chameleon only.
    play(&mut g, 0, "Test Beast Buffer");
    let got: Vec<_> = (0..5).map(|s| stats(&g, 0, s)).collect();
    assert_eq!(got, [(3, 3), (5, 5), (6, 6), (3, 3), (3, 3)]);
    g.check_invariants().unwrap();
}

#[test]
fn board_tribe_counts_use_each_tribe_of_a_minion() {
    let mut g = game(2);
    for n in ["Test Beast", "Test Dragon", "Test Dragon Demon", "Test Chameleon", "Test Neutral"] {
        put_on_board(&mut g, 0, n);
    }
    // Beast, Mech, neutral, Dragon, Murloc, Demon
    assert_eq!(g.players[0].board_tribes(&g.data.cards), [2, 1, 1, 3, 1, 2]);
}

#[test]
fn an_all_minion_is_a_mech_for_magnetic() {
    let mut g = game(3);
    put_on_board(&mut g, 0, "Test Chameleon");
    g.give(0, "Test Magnet").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    assert_eq!(g.players[0].board.len(), 1);
    assert_eq!(g.players[0].board[0].magnetizations, 1);
    assert_eq!(stats(&g, 0, 0), (4, 4));
}

#[test]
fn a_same_tribe_spell_reaches_multi_tribe_minions() {
    let mut g = game(4);
    for n in ["Test Dragon", "Test Demon", "Test Chameleon", "Test Dragon Demon", "Test Neutral"] {
        put_on_board(&mut g, 0, n);
    }
    g.give(0, "Test Tribe Buff").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    act(&mut g, 0, Action::Target(0)); // the Dragon
    let got: Vec<_> = (0..5).map(|s| stats(&g, 0, s)).collect();
    // Dragon, Chameleon and Dragon Demon share a tribe with it, the Demon
    // and the neutral don't.
    assert_eq!(got, [(3, 3), (2, 2), (4, 4), (4, 4), (2, 2)]);
}

// ---- Activate ----

#[test]
fn activate_has_its_own_actions() {
    let (_, n) = ACTION_KINDS.iter().find(|(name, _)| *name == "activate").unwrap();
    assert_eq!(*n, bgcore::BOARD);
    for s in 0..bgcore::BOARD as u8 {
        let a = Action::Activate(s);
        assert_eq!(Action::from_index(a.index()), Some(a));
    }
}

#[test]
fn activate_is_legal_with_the_ability_and_the_gold() {
    let mut g = game(5);
    put_on_board(&mut g, 0, "Test Neutral");
    put_on_board(&mut g, 0, "Test Activator"); // costs 2
    turn_of(&mut g, 0);
    g.players[0].gold = 1;
    assert!(!g.is_legal(Action::Activate(1)), "not enough gold");
    g.players[0].gold = 2;
    assert!(g.is_legal(Action::Activate(1)));
    assert!(!g.is_legal(Action::Activate(0)), "no ability");
    assert!(!g.is_legal(Action::Activate(2)), "no minion");
    assert!(legal(&g).contains(&Action::Activate(1)));
    assert!(!legal(&g).contains(&Action::Activate(0)));
}

#[test]
fn activate_pays_its_cost_and_runs_its_effect() {
    let mut g = game(6);
    put_on_board(&mut g, 0, "Test Neutral");
    put_on_board(&mut g, 0, "Test Beast");
    put_on_board(&mut g, 0, "Test Activator");
    turn_of(&mut g, 0);
    g.players[0].gold = 5;
    act(&mut g, 0, Action::Activate(2));
    assert_eq!(g.players[0].gold, 3);
    // two targets, so it waits for a pick, and the source can't be it
    assert!(g.players[0].pending_target.is_some());
    assert!(!is_legal(&mut g, 0, Action::Target(2)));
    assert!(!is_legal(&mut g, 0, Action::EndTurn));
    act(&mut g, 0, Action::Target(1));
    assert_eq!(stats(&g, 0, 1), (5, 5));
    assert_eq!(stats(&g, 0, 0), (2, 2));
    // once per turn: a second use is illegal even with the gold
    assert!(!is_legal(&mut g, 0, Action::Activate(2)));
    assert!(!legal(&g).contains(&Action::Activate(2)));
    g.check_invariants().unwrap();
}

#[test]
fn activate_is_legal_again_next_turn_and_a_moved_minion_stays_used() {
    let mut g = game(60);
    put_on_board(&mut g, 0, "Test Neutral");
    put_on_board(&mut g, 0, "Test Activator");
    turn_of(&mut g, 0);
    g.players[0].gold = 9;
    act(&mut g, 0, Action::Activate(1));
    assert!(!is_legal(&mut g, 0, Action::Activate(1)));
    // moving it doesn't reset it
    act(&mut g, 0, Action::Move { from: 1, to: 0 });
    assert!(!is_legal(&mut g, 0, Action::Activate(0)));
    next_turn(&mut g);
    turn_of(&mut g, 0);
    g.players[0].gold = 9;
    assert!(g.is_legal(Action::Activate(0)));
    assert!(legal(&g).contains(&Action::Activate(0)));
    // a copy bought later is a new minion and starts unused
    act(&mut g, 0, Action::Activate(0));
    act(&mut g, 0, Action::Sell(0));
    put_on_board(&mut g, 0, "Test Activator");
    let last = g.players[0].board.len() as u8 - 1;
    assert!(is_legal(&mut g, 0, Action::Activate(last)));
}

#[test]
fn activate_with_one_target_picks_it_and_with_none_still_costs() {
    let mut g = game(7);
    put_on_board(&mut g, 0, "Test Neutral");
    put_on_board(&mut g, 0, "Test Activator");
    turn_of(&mut g, 0);
    g.players[0].gold = 4;
    act(&mut g, 0, Action::Activate(1));
    assert!(g.players[0].pending_target.is_none());
    assert_eq!(stats(&g, 0, 0), (5, 5));
    act(&mut g, 0, Action::Sell(0));
    // the Activator is spent, so a fresh copy takes its place
    act(&mut g, 0, Action::Sell(0));
    put_on_board(&mut g, 0, "Test Activator");
    g.players[0].gold = 4;
    act(&mut g, 0, Action::Activate(0));
    assert_eq!(g.players[0].gold, 2);
    assert_eq!(stats(&g, 0, 0), (2, 2));
    g.check_invariants().unwrap();
}

#[test]
fn activate_is_not_legal_while_a_choice_is_waiting() {
    let mut g = game(8);
    put_on_board(&mut g, 0, "Test Neutral");
    put_on_board(&mut g, 0, "Test Beast");
    put_on_board(&mut g, 0, "Test Activator");
    turn_of(&mut g, 0);
    g.players[0].gold = 9;
    act(&mut g, 0, Action::Activate(2));
    assert!(!is_legal(&mut g, 0, Action::Activate(2)));
    assert!(legal(&g).iter().all(|a| matches!(a, Action::Target(_))));
}

#[test]
fn a_golden_activate_doubles_its_effect() {
    let mut g = game(9);
    put_on_board(&mut g, 0, "Test Neutral");
    put_on_board(&mut g, 0, "Test Activator");
    g.players[0].board[1].golden = true;
    turn_of(&mut g, 0);
    g.players[0].gold = 2;
    act(&mut g, 0, Action::Activate(1));
    assert_eq!(stats(&g, 0, 0), (8, 8));
}

// ---- Bonus Keywords and Stealth ----

const BONUS: [u8; 6] = [
    Keywords::TAUNT,
    Keywords::DIVINE_SHIELD,
    Keywords::WINDFURY,
    Keywords::VENOMOUS,
    Keywords::REBORN,
    Keywords::STEALTH,
];

#[test]
fn the_bonus_keyword_set_comes_from_rules() {
    let d = data();
    assert_eq!(d.rules.bonus_mask, BONUS.iter().fold(0, |a, b| a | b));
    assert_eq!(d.rules.bonus_mask & Keywords::MAGNETIC, 0);
    let bad = include_str!("data/v01-rules.toml").replace("\"stealth\"]", "\"flying\"]");
    assert!(bgcore::Rules::from_toml(&bad).unwrap_err().contains("flying"));
}

#[test]
fn a_random_bonus_keyword_is_never_one_it_has() {
    for seed in 0..40 {
        let mut g = game(seed);
        put_on_board(&mut g, 0, "Test Keyword Activator");
        turn_of(&mut g, 0);
        let mut have = 0u32;
        for _ in 0..6 {
            // once per turn
            next_turn(&mut g);
            turn_of(&mut g, 0);
            act(&mut g, 0, Action::Activate(0));
            let now = (g.players[0].board[0].keywords.0 & g.data.rules.bonus_mask).count_ones();
            assert_eq!(now, have + 1, "seed {seed}: it must gain a new keyword every time");
            have = now;
        }
        // it has them all, so nothing more happens
        next_turn(&mut g);
        let before = g.players[0].board[0].keywords;
        act(&mut g, 0, Action::Activate(0));
        assert_eq!(g.players[0].board[0].keywords, before);
        assert!(!before.has(Keywords::MAGNETIC));
    }
}

#[test]
fn a_random_bonus_keyword_by_battlecry_and_by_chosen_buff() {
    let mut g = game(10);
    play(&mut g, 0, "Test Keyword Giver");
    assert_eq!((g.players[0].board[0].keywords.0 & g.data.rules.bonus_mask).count_ones(), 1);

    let mut g = game(11);
    put_on_board(&mut g, 0, "Test Beast");
    put_on_board(&mut g, 0, "Test Mech");
    g.give(0, "Test Keyword Gifter").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 2 });
    act(&mut g, 0, Action::Target(1));
    assert_eq!(stats(&g, 0, 1), (3, 3));
    assert_eq!((g.players[0].board[1].keywords.0 & g.data.rules.bonus_mask).count_ones(), 1);
    assert_eq!(g.players[0].board[0].keywords.0, 0);
}

#[test]
fn different_bonus_keywords_in_the_warband_are_counted_once_each() {
    let mut g = game(12);
    for n in ["Test Beast", "Test Mech", "Test Dragon", "Test Murloc"] {
        put_on_board(&mut g, 0, n);
    }
    // Taunt twice, Divine Shield and Magnetic (not a Bonus Keyword): 2 different.
    g.players[0].board[0].keywords.set(Keywords::TAUNT);
    g.players[0].board[1].keywords.set(Keywords::TAUNT);
    g.players[0].board[1].keywords.set(Keywords::MAGNETIC);
    g.players[0].board[2].keywords.set(Keywords::DIVINE_SHIELD);
    assert_eq!(g.players[0].distinct_bonus_keywords(g.data.rules.bonus_mask), 2);
    // Scaler: +1/+1 and +1/+2 per different keyword, on the other minions.
    play(&mut g, 0, "Test Keyword Scaler");
    assert_eq!(stats(&g, 0, 3), (2 + 3, 2 + 5));
    assert_eq!(stats(&g, 0, 4), base(&g, "Test Keyword Scaler"));
    // The Scaler's own keywords change nothing, and with none it is +1/+1.
    let mut g = game(13);
    put_on_board(&mut g, 0, "Test Beast");
    play(&mut g, 0, "Test Keyword Scaler");
    assert_eq!(stats(&g, 0, 0), (3, 3));
}

#[derive(Default)]
struct Log {
    /// (attacker card, attacker had Stealth, defender card, defender had Stealth)
    attacks: Vec<(bgcore::CardId, bool, bgcore::CardId, bool)>,
}

impl Watch for Log {
    fn attack(&mut self, _side: usize, a: &Minion, d: &Minion) {
        self.attacks.push((
            a.card,
            a.keywords.has(Keywords::STEALTH),
            d.card,
            d.keywords.has(Keywords::STEALTH),
        ));
    }
}

fn board(db: &CardDb, names: &[&str]) -> Vec<Minion> {
    names.iter().map(|n| db.make(db.by_name(n).unwrap(), false)).collect()
}

fn watched(a: &[Minion], b: &[Minion], seed: u64) -> (bgcore::Outcome, Log) {
    let db = data().cards;
    let mut log = Log::default();
    let out = fight_watched(&db, a, b, 500, &mut Rng::new(seed), &mut log);
    (out, log)
}

#[test]
fn stealth_cannot_be_targeted_until_it_attacks() {
    let db = data().cards;
    let stealthy = db.by_name("Test Stealthy").unwrap();
    let plain = db.by_name("Test Neutral").unwrap();
    // Side 0 has more minions so it attacks first and can't pick the Stealth one.
    let a = board(&db, &["Test Beast", "Test Beast", "Test Beast"]);
    let b = board(&db, &["Test Stealthy", "Test Neutral"]);
    let mut stealthy_attacked = 0;
    for seed in 0..300 {
        let (_, log) = watched(&a, &b, seed);
        let mut revealed = false;
        for &(att, att_stealth, def, def_stealth) in &log.attacks {
            if att == stealthy {
                // attacking ends Stealth, so the attacker no longer has it
                assert!(!att_stealth);
                revealed = true;
                stealthy_attacked += 1;
            }
            if def == stealthy {
                assert!(revealed, "seed {seed}: hit a hidden minion");
                assert!(!def_stealth);
            }
        }
        // the first attack is by side 0 and has to hit the plain minion
        assert_eq!(log.attacks[0].2, plain);
    }
    assert!(stealthy_attacked > 0);
}

#[test]
fn a_minion_that_attacked_can_be_hit_afterwards() {
    let db = data().cards;
    let stealthy = db.by_name("Test Stealthy").unwrap();
    let a = board(&db, &["Test Beast", "Test Beast", "Test Beast"]);
    let mut b = board(&db, &["Test Stealthy", "Test Neutral"]);
    b[1].health = 500;
    let mut hit_after = false;
    for seed in 0..200 {
        let (_, log) = watched(&a, &b, seed);
        let first = log.attacks.iter().position(|x| x.0 == stealthy);
        if let Some(i) = first {
            hit_after |= log.attacks[i..].iter().any(|x| x.2 == stealthy);
        }
    }
    assert!(hit_after);
}

#[test]
fn a_side_facing_only_stealth_loses_its_turn() {
    // Stealth is never ignored: whoever would attack first, the side facing
    // only Stealth minions passes, so the Stealth side always attacks first.
    let db = data().cards;
    let stealthy = db.by_name("Test Stealthy").unwrap();
    let a = board(&db, &["Test Beast"]);
    let b = board(&db, &["Test Stealthy"]);
    for seed in 0..50 {
        let (_, log) = watched(&a, &b, seed);
        assert_eq!(log.attacks[0].0, stealthy);
    }
}

#[test]
fn two_boards_of_only_stealth_tie_without_attacking() {
    let db = data().cards;
    let a = board(&db, &["Test Stealthy"]);
    let b = board(&db, &["Test Stealthy"]);
    for seed in 0..20 {
        let (out, log) = watched(&a, &b, seed);
        assert!(log.attacks.is_empty());
        assert_eq!((out.winner, out.attacks), (None, 0));
    }
}

#[test]
fn stealth_beats_taunt() {
    let db = data().cards;
    let mut b = board(&db, &["Test Stealthy", "Test Neutral"]);
    b[0].keywords.set(Keywords::TAUNT);
    let a = board(&db, &["Test Beast", "Test Beast", "Test Beast"]);
    for seed in 0..100 {
        let (_, log) = watched(&a, &b, seed);
        assert_eq!(log.attacks[0].2, db.by_name("Test Neutral").unwrap());
    }
}

// ---- consuming Tavern minions ----

#[test]
fn consuming_a_random_tavern_minion_gains_its_stats_and_returns_the_copy() {
    for seed in 0..10 {
        let mut g = game(seed);
        set_shop(&mut g, 0, &["Test Beast", "Test Mech", "Test Dragon"]);
        g.players[0].shop[0].attack = 5;
        g.players[0].shop[1].health = 7;
        let before: Vec<bgcore::CardId> = g.players[0].shop.iter().map(|m| m.card).collect();
        let pool_before = g.pool.clone();
        play(&mut g, 0, "Test Eater Random");
        assert_eq!(g.players[0].shop.len(), 2);
        let (a, h) = stats(&g, 0, 0);
        let gone = before.iter().find(|c| !g.players[0].shop.iter().any(|m| m.card == **c)).unwrap();
        let slot = g.data.cards.get(*gone).pool_slot as usize;
        // the pool only grew by the consumed copy (other players' Taverns also draw, and
        // the Eater itself was played from the hand)
        assert_eq!(g.pool[slot], pool_before[slot] + 1, "the copy is back in the pool");
        let (ga, gh) = (a - 1, h - 1);
        let eaten = g.data.cards.get(*gone);
        let want = [(eaten.attack, eaten.health), (5, 2), (2, 7)];
        assert!(want.contains(&(ga, gh)), "{ga}/{gh}");
        g.check_invariants().unwrap();
    }
}

#[test]
fn consuming_the_highest_health_takes_the_leftmost_of_equals_and_its_bonus_keywords() {
    let mut g = game(20);
    set_shop(&mut g, 0, &["Test Beast", "Test Mech", "Test Dragon"]);
    g.players[0].shop[1].health = 6;
    g.players[0].shop[2].health = 6;
    g.players[0].shop[1].attack = 4;
    g.players[0].shop[1].keywords.set(Keywords::TAUNT);
    g.players[0].shop[1].keywords.set(Keywords::MAGNETIC);
    play(&mut g, 0, "Test Eater Highest");
    assert_eq!(stats(&g, 0, 0), (1 + 4, 1 + 6));
    // Taunt is a Bonus Keyword and comes along, Magnetic is not
    assert!(g.players[0].board[0].keywords.has(Keywords::TAUNT));
    assert!(!g.players[0].board[0].keywords.has(Keywords::MAGNETIC));
    let names: Vec<_> = g.players[0].shop.iter().map(|m| m.card).collect();
    assert_eq!(names, [id(&g, "Test Beast"), id(&g, "Test Dragon")]);
    g.check_invariants().unwrap();
}

#[test]
fn consuming_without_keywords_gains_only_stats() {
    let mut g = game(21);
    set_shop(&mut g, 0, &["Test Beast", "Test Mech"]);
    g.players[0].shop[0].keywords.set(Keywords::TAUNT);
    g.players[0].shop[1].keywords.set(Keywords::TAUNT);
    play(&mut g, 0, "Test Eater Random");
    assert_eq!(g.players[0].board[0].keywords.0, 0);
}

#[test]
fn a_chosen_consume_waits_for_a_tavern_pick() {
    let mut g = game(22);
    set_shop(&mut g, 0, &["Test Beast", "Test Mech", "Test Dragon"]);
    g.players[0].shop[1].attack = 6;
    g.players[0].shop[1].keywords.set(Keywords::WINDFURY);
    g.players[0].shop[1].keywords.set(Keywords::MAGNETIC);
    let slot = g.data.cards.get(id(&g, "Test Mech")).pool_slot as usize;
    let pool_before = g.pool[slot];
    play(&mut g, 0, "Test Eater Chosen");
    assert!(g.players[0].pending_shop_target.is_some());
    turn_of(&mut g, 0);
    let l = legal(&g);
    assert_eq!(l, [Action::TargetShop(0), Action::TargetShop(1), Action::TargetShop(2)]);
    assert!(!g.is_legal(Action::EndTurn));
    g.check_invariants().unwrap();
    act(&mut g, 0, Action::TargetShop(1));
    assert!(g.players[0].pending_shop_target.is_none());
    assert_eq!(stats(&g, 0, 0), (1 + 6, 1 + 2));
    assert!(g.players[0].board[0].keywords.has(Keywords::WINDFURY));
    assert!(!g.players[0].board[0].keywords.has(Keywords::MAGNETIC));
    assert_eq!(g.players[0].shop.len(), 2);
    assert_eq!(g.pool[slot], pool_before + 1);
    g.check_invariants().unwrap();
}

#[test]
fn a_chosen_consume_with_one_choice_takes_it_and_with_none_does_nothing() {
    let mut g = game(23);
    set_shop(&mut g, 0, &["Test Beast"]);
    play(&mut g, 0, "Test Eater Chosen");
    assert!(g.players[0].pending_shop_target.is_none());
    assert!(g.players[0].shop.is_empty());
    assert_eq!(stats(&g, 0, 0), (3, 3));
    play(&mut g, 0, "Test Eater Chosen");
    assert!(g.players[0].pending_shop_target.is_none());
    assert_eq!(stats(&g, 0, 1), (1, 1));
    g.check_invariants().unwrap();
}

#[test]
fn consuming_twice_picks_twice() {
    let mut g = game(24);
    g.players[0].tier = 3;
    set_shop(&mut g, 0, &["Test Beast", "Test Mech", "Test Dragon", "Test Murloc"]);
    play(&mut g, 0, "Test Eater Greedy");
    act(&mut g, 0, Action::TargetShop(0));
    assert!(g.players[0].pending_shop_target.is_some());
    assert!(matches!(
        g.players[0].pending_shop_target.unwrap().kind,
        bgcore::game::ShopTargetKind::Consume { remaining: 1, .. }
    ));
    act(&mut g, 0, Action::TargetShop(1));
    assert!(g.players[0].pending_shop_target.is_none());
    assert_eq!(stats(&g, 0, 0), (1 + 2 + 2, 1 + 2 + 2));
    assert_eq!(g.players[0].shop.len(), 2);
    g.check_invariants().unwrap();
}

#[test]
fn a_waiting_tavern_pick_takes_the_first_when_the_turn_runs_out() {
    let mut g = game(25);
    set_shop(&mut g, 0, &["Test Beast", "Test Mech"]);
    g.players[0].shop[0].attack = 9;
    g.give(0, "Test Eater Chosen").unwrap();
    turn_of(&mut g, 0);
    g.players[0].actions = g.data.rules.max_actions_per_turn - 1;
    g.step(Action::Play { hand: 0, pos: 0 }).unwrap();
    assert!(g.players[0].done);
    assert!(g.players[0].pending_shop_target.is_none());
    assert_eq!(stats(&g, 0, 0), (1 + 9, 1 + 2));
    g.check_invariants().unwrap();
}

#[test]
fn a_chosen_friendly_minion_does_the_consuming() {
    let mut g = game(26);
    put_on_board(&mut g, 0, "Test Demon");
    put_on_board(&mut g, 0, "Test Beast");
    put_on_board(&mut g, 0, "Test Chameleon");
    set_shop(&mut g, 0, &["Test Mech"]);
    g.players[0].shop[0].attack = 5;
    g.give(0, "Test Demon Feeder").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 3 });
    // the Demon and the Chameleon are Demons, the Beast and the Feeder aren't
    turn_of(&mut g, 0);
    assert_eq!(legal(&g), [Action::Target(0), Action::Target(2)]);
    act(&mut g, 0, Action::Target(2));
    assert_eq!(stats(&g, 0, 2), (3 + 5, 3 + 2));
    assert_eq!(stats(&g, 0, 0), (2, 2));
    assert!(g.players[0].shop.is_empty());
    g.check_invariants().unwrap();
}

#[test]
fn an_end_of_turn_consume_takes_the_highest_health() {
    let mut g = game(27);
    put_on_board(&mut g, 0, "Test Feaster");
    set_shop(&mut g, 0, &["Test Beast", "Test Mech"]);
    g.players[0].shop[1].health = 8;
    act(&mut g, 0, Action::EndTurn);
    assert_eq!(stats(&g, 0, 0), (1 + 2, 1 + 8));
    assert_eq!(g.players[0].shop.len(), 1);
    g.check_invariants().unwrap();
}

// ---- Tavern buffs ----

fn shop_is_buffed(g: &Game, p: usize, d: (i32, i32)) {
    assert!(!g.players[p].shop.is_empty());
    for m in &g.players[p].shop {
        let c = g.data.cards.get(m.card);
        assert_eq!((m.attack, m.health), (c.attack + d.0, c.health + d.1), "{}", c.name);
    }
}

#[test]
fn a_this_game_tavern_buff_reaches_minions_rolled_later() {
    let mut g = game(30);
    play(&mut g, 0, "Test Tavern Buffer");
    shop_is_buffed(&g, 0, (1, 1));
    g.players[0].gold = 10;
    for _ in 0..3 {
        act(&mut g, 0, Action::Roll);
        shop_is_buffed(&g, 0, (1, 1));
    }
    // and the next turn's Tavern, and a second buff stacks
    next_turn(&mut g);
    shop_is_buffed(&g, 0, (1, 1));
    play(&mut g, 0, "Test Tavern Buffer");
    shop_is_buffed(&g, 0, (2, 2));
    assert_eq!(g.players[0].tavern_buffs.len(), 1);
    g.check_invariants().unwrap();
}

#[test]
fn a_bought_minion_keeps_its_tavern_buff() {
    let mut g = game(31);
    play(&mut g, 0, "Test Tavern Buffer");
    let c = g.players[0].shop[0].card;
    act(&mut g, 0, Action::Buy(0));
    let m = g.players[0].hand[0];
    let (ba, bh) = (g.data.cards.get(c).attack, g.data.cards.get(c).health);
    assert_eq!((m.attack, m.health), (ba + 1, bh + 1));
    // it stays after the turn ends
    act(&mut g, 0, Action::EndTurn);
    let m = g.players[0].hand[0];
    assert_eq!((m.attack, m.health), (ba + 1, bh + 1));
}

#[test]
fn a_this_turn_tavern_buff_ends_with_the_turn_but_bought_minions_keep_it() {
    let mut g = game(32);
    g.players[0].gold = 10;
    play(&mut g, 0, "Test Turn Buffer");
    shop_is_buffed(&g, 0, (2, 2));
    act(&mut g, 0, Action::Roll);
    shop_is_buffed(&g, 0, (2, 2));
    let c = g.players[0].shop[0].card;
    act(&mut g, 0, Action::Buy(0));
    act(&mut g, 0, Action::Freeze);
    act(&mut g, 0, Action::EndTurn);
    // the minions left in the Tavern are plain again, the bought one is not
    shop_is_buffed(&g, 0, (0, 0));
    let b = g.data.cards.get(c);
    let m = g.players[0].hand[0];
    assert_eq!((m.attack, m.health), (b.attack + 2, b.health + 2));
    assert!(g.players[0].tavern_buffs.is_empty());
    // next turn's frozen Tavern is still plain, and new ones too
    next_turn(&mut g);
    shop_is_buffed(&g, 0, (0, 0));
    g.check_invariants().unwrap();
}

#[test]
fn this_turn_and_this_game_buffs_stack_and_only_the_turn_one_ends() {
    let mut g = game(33);
    play(&mut g, 0, "Test Tavern Buffer");
    play(&mut g, 0, "Test Turn Buffer");
    shop_is_buffed(&g, 0, (3, 3));
    act(&mut g, 0, Action::EndTurn);
    shop_is_buffed(&g, 0, (1, 1));
    assert_eq!(g.players[0].tavern_buffs.len(), 1);
}

#[test]
fn a_tribe_tavern_buff_only_reaches_that_tribe() {
    let mut g = game(34);
    g.players[0].tier = 3;
    set_shop(&mut g, 0, &["Test Dragon", "Test Beast", "Test Chameleon"]);
    play(&mut g, 0, "Test Dragon Tavern Buffer");
    let s: Vec<_> = g.players[0].shop.iter().map(|m| (m.attack, m.health)).collect();
    assert_eq!(s, [(3, 3), (2, 2), (4, 4)]);
    // later arrivals: a Dragon and the Chameleon get it, a Beast and a Demon don't
    let pl = &g.players[0];
    for (name, d) in [("Test Dragon", 1), ("Test Beast", 0), ("Test Chameleon", 1), ("Test Demon", 0)]
    {
        let mut m = g.data.cards.make(id(&g, name), false);
        let base = (m.attack, m.health);
        pl.apply_tavern_buffs(&g.data.cards, &mut m);
        assert_eq!((m.attack, m.health), (base.0 + d, base.1 + d), "{name}");
    }
}

// ---- event counters ----

fn left(g: &Game, slot: usize) -> u8 {
    g.players[0].board[slot].events_left(&g.data.cards).unwrap()
}

#[test]
fn a_spell_counter_fires_on_the_third_spell_and_starts_over() {
    let mut g = game(40);
    put_on_board(&mut g, 0, "Test Spell Counter");
    assert_eq!(left(&g, 0), 3);
    for (cast, want_stats, want_left) in
        [(1, (1, 1), 2), (2, (1, 1), 1), (3, (2, 2), 3), (4, (2, 2), 2), (5, (2, 2), 1), (6, (3, 3), 3)]
    {
        g.give(0, "Test Coin").unwrap();
        act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
        assert_eq!((stats(&g, 0, 0), left(&g, 0)), (want_stats, want_left), "after {cast}");
    }
}

#[test]
fn a_gold_counter_counts_gold_and_keeps_the_remainder() {
    let mut g = game(41);
    put_on_board(&mut g, 0, "Test Gold Counter");
    turn_of(&mut g, 0);
    g.players[0].gold = 10;
    for _ in 0..3 {
        act(&mut g, 0, Action::Roll);
    }
    assert_eq!((stats(&g, 0, 0), left(&g, 0)), ((1, 1), 1));
    act(&mut g, 0, Action::Roll);
    assert_eq!((stats(&g, 0, 0), left(&g, 0)), ((2, 2), 4));
    // a free Refresh spends no gold; a buy spends 3: 3 + 3 = 6 is one trigger and 2 over
    g.players[0].free_rolls = 1;
    act(&mut g, 0, Action::Roll);
    assert_eq!(left(&g, 0), 4);
    act(&mut g, 0, Action::Buy(0));
    act(&mut g, 0, Action::Buy(0));
    assert_eq!((stats(&g, 0, 0), left(&g, 0)), ((3, 3), 2));
    assert_eq!(g.players[0].gold, 10 - 4 - 6);
    g.check_invariants().unwrap();
}

#[test]
fn a_damage_counter_counts_damage_to_your_hero() {
    let mut g = game(42);
    put_on_board(&mut g, 0, "Test Damage Counter");
    play(&mut g, 0, "Test Pain");
    // 5 damage: one trigger of 4 and 1 over
    assert_eq!((stats(&g, 0, 0), left(&g, 0)), ((2, 2), 3));
    assert_eq!(g.players[0].health, 25);
    play(&mut g, 0, "Test Pain");
    assert_eq!((stats(&g, 0, 0), left(&g, 0)), ((3, 3), 2));
}

#[test]
fn a_sell_counter_counts_minions_sold() {
    let mut g = game(43);
    put_on_board(&mut g, 0, "Test Sell Counter");
    put_on_board(&mut g, 0, "Test Neutral");
    put_on_board(&mut g, 0, "Test Neutral");
    act(&mut g, 0, Action::Sell(1));
    assert_eq!((stats(&g, 0, 0), left(&g, 0)), ((1, 1), 1));
    act(&mut g, 0, Action::Sell(1));
    assert_eq!((stats(&g, 0, 0), left(&g, 0)), ((2, 2), 2));
    g.check_invariants().unwrap();
}

#[test]
fn a_buy_counter_counts_minions_and_spells_bought() {
    let mut g = game(44);
    put_on_board(&mut g, 0, "Test Buy Counter");
    set_shop(&mut g, 0, &["Test Beast", "Test Mech"]);
    g.players[0].shop_spell = Some(id(&g, "Test Coin"));
    turn_of(&mut g, 0);
    g.players[0].gold = 10;
    act(&mut g, 0, Action::Buy(0));
    act(&mut g, 0, Action::BuySpell);
    assert_eq!((stats(&g, 0, 0), left(&g, 0)), ((1, 1), 1));
    act(&mut g, 0, Action::Buy(0));
    assert_eq!((stats(&g, 0, 0), left(&g, 0)), ((2, 2), 3));
}

#[test]
fn another_players_events_do_not_count() {
    let mut g = game(45);
    put_on_board(&mut g, 0, "Test Sell Counter");
    put_on_board(&mut g, 1, "Test Neutral");
    put_on_board(&mut g, 1, "Test Neutral");
    act(&mut g, 1, Action::Sell(0));
    act(&mut g, 1, Action::Sell(0));
    assert_eq!((stats(&g, 0, 0), left(&g, 0)), ((1, 1), 2));
}

#[test]
fn auto_reveille_is_an_event_counter_now() {
    let g = Game::new(Arc::new(Data::bundled()), 1);
    let c = g.data.cards.get(g.data.cards.by_name("Auto Reveille").unwrap());
    let t = c.on_event.unwrap();
    assert_eq!((t.kind, t.every), (bgcore::cards::EventKind::CardsBought, 3));
    let m = g.data.cards.make(g.data.cards.by_name("Auto Reveille").unwrap(), false);
    assert_eq!(m.events_left(&g.data.cards), Some(3));
}

#[test]
fn an_event_trigger_cannot_ask_for_a_choice_and_needs_a_count() {
    let card = |ev: &str| {
        format!(
            "[[minion]]\nname = \"A\"\ntier = 1\ntribe = \"beast\"\nstats = [1, 1]\non_event = {ev}\n"
        )
    };
    let chosen = card("{ event = \"spells_cast\", every = 2, effect = { buff = [1, 1], target = \"chosen\" } }");
    assert!(CardDb::from_toml(&chosen).unwrap_err().contains("choice"));
    let zero = card("{ event = \"spells_cast\", every = 0, effect = { random_keyword = true } }");
    assert!(CardDb::from_toml(&zero).unwrap_err().contains("at least 1"));
    let bad = card("{ event = \"lunch\", every = 1, effect = { random_keyword = true } }");
    assert!(CardDb::from_toml(&bad).unwrap_err().contains("unknown event"));
}

// ---- hero health ----

#[test]
fn damage_from_an_effect_can_kill() {
    let mut g = game(50);
    play(&mut g, 0, "Test Pain");
    assert_eq!(g.players[0].health, 25);
    put_on_board(&mut g, 0, "Test Damage Counter");
    g.players[0].health = 3;
    play(&mut g, 0, "Test Pain");
    // nothing stops it: the hero goes below 0 and dies on the spot, in the
    // middle of its turn, taking the worst place still open
    assert_eq!(g.players[0].health, -2);
    assert_eq!(g.players[0].last_damage, 5);
    assert!(!g.players[0].alive);
    assert_eq!(g.players[0].placement, 8);
    assert_ne!(g.current_player(), Some(0));
    g.check_invariants().unwrap();
}

#[test]
fn rewind_undoes_the_damage_just_taken() {
    let mut g = game(51);
    put_on_board(&mut g, 0, "Test Rewinder");
    play(&mut g, 0, "Test Pain");
    assert_eq!(g.players[0].health, 30);
    assert_eq!(stats(&g, 0, 0), (4, 4));
    play(&mut g, 0, "Test Pain");
    assert_eq!(g.players[0].health, 30);
    assert_eq!(stats(&g, 0, 0), (4, 6));
}

#[test]
fn after_damage_triggers_fire_on_self_damage() {
    let mut g = game(52);
    put_on_board(&mut g, 0, "Test Punisher");
    play(&mut g, 0, "Test Pain");
    assert_eq!(g.players[0].health, 25);
    assert_eq!(stats(&g, 0, 0), (3, 3));
}

/// Every other player gets one huge minion, so player 0 loses the fight.
fn gang_up_on_player_0(g: &mut Game) {
    let neutral = id(g, "Test Neutral");
    for q in 1..8 {
        let mut m = g.data.cards.make(neutral, false);
        m.attack = 50;
        m.health = 50;
        g.players[q].board.push(m);
    }
}

#[test]
fn losing_a_fight_triggers_no_after_damage_minion() {
    let mut g = game(53);
    put_on_board(&mut g, 0, "Test Punisher");
    let before = stats(&g, 0, 0);
    gang_up_on_player_0(&mut g);
    next_turn(&mut g);
    // the opponent's tier 1 plus its one tier 1 minion: taken normally
    assert_eq!(g.players[0].health, 28);
    assert_eq!(stats(&g, 0, 0), before, "the Punisher didn't react");
    assert_eq!(g.players[0].last_damage, 0);
}

#[test]
fn combat_damage_is_not_rewound_and_triggers_nothing() {
    let mut g = game(54);
    put_on_board(&mut g, 0, "Test Rewinder");
    let before = stats(&g, 0, 0);
    gang_up_on_player_0(&mut g);
    next_turn(&mut g);
    assert_eq!(g.players[0].health, 28);
    assert_eq!(stats(&g, 0, 0), before, "no +2 Health from the Rewinder");

    // a lethal loss knocks the hero out even with a rewind on the board
    let mut g = game(55);
    put_on_board(&mut g, 0, "Test Rewinder");
    g.players[0].health = 1;
    gang_up_on_player_0(&mut g);
    next_turn(&mut g);
    assert!(!g.players[0].alive);
}

#[test]
fn combat_damage_ignores_the_floor() {
    let mut g = game(56);
    g.players[0].health = 1;
    gang_up_on_player_0(&mut g);
    next_turn(&mut g);
    assert!(!g.players[0].alive);
}

#[test]
fn refreshes_can_cost_health_a_limited_number_of_times_a_turn() {
    let mut g = game(57);
    put_on_board(&mut g, 0, "Test Refresher");
    turn_of(&mut g, 0);
    g.players[0].gold = 0;
    assert_eq!(g.players[0].health_refreshes_left(&g.data.cards), 2);
    act(&mut g, 0, Action::Roll);
    act(&mut g, 0, Action::Roll);
    assert_eq!((g.players[0].health, g.players[0].gold), (28, 0));
    assert!(!is_legal(&mut g, 0, Action::Roll));
    // with gold the next one is paid in gold
    g.players[0].gold = 1;
    act(&mut g, 0, Action::Roll);
    assert_eq!((g.players[0].health, g.players[0].gold), (28, 0));
    // the uses come back next turn
    next_turn(&mut g);
    assert_eq!(g.players[0].health_refreshes_left(&g.data.cards), 2);
    g.check_invariants().unwrap();
}

#[test]
fn a_health_refresh_cannot_kill_without_a_rewind() {
    let mut g = game(58);
    put_on_board(&mut g, 0, "Test Refresher");
    turn_of(&mut g, 0);
    g.players[0].gold = 0;
    g.players[0].health = 2;
    act(&mut g, 0, Action::Roll);
    assert_eq!(g.players[0].health, 1);
    // one more would take it to 0
    assert!(!is_legal(&mut g, 0, Action::Roll));
}

#[test]
fn a_rewind_lets_a_health_refresh_be_paid_at_1_health() {
    let mut g = game(61);
    put_on_board(&mut g, 0, "Test Refresher");
    put_on_board(&mut g, 0, "Test Rewinder");
    turn_of(&mut g, 0);
    g.players[0].gold = 0;
    g.players[0].health = 1;
    assert!(is_legal(&mut g, 0, Action::Roll));
    let rewinder = g.players[0].board[1];
    act(&mut g, 0, Action::Roll);
    // paid and given straight back, and the after-damage buff still fired
    assert_eq!(g.players[0].health, 1);
    assert_eq!(g.players[0].board[1].health, rewinder.health + 2);
    g.check_invariants().unwrap();
}

#[test]
fn a_golden_refresher_has_twice_the_uses() {
    let mut g = game(59);
    put_on_board(&mut g, 0, "Test Refresher");
    g.players[0].board[0].golden = true;
    assert_eq!(g.players[0].health_refreshes_left(&g.data.cards), 4);
}

#[test]
fn health_paid_for_a_refresh_is_damage_and_can_be_rewound() {
    let mut g = game(60);
    put_on_board(&mut g, 0, "Test Refresher");
    put_on_board(&mut g, 0, "Test Damage Counter");
    put_on_board(&mut g, 0, "Test Rewinder");
    turn_of(&mut g, 0);
    g.players[0].gold = 0;
    act(&mut g, 0, Action::Roll);
    // paid and given back, and it still counted as 1 damage
    assert_eq!(g.players[0].health, 30);
    assert_eq!(left(&g, 1), 3);
    assert_eq!(stats(&g, 0, 2), (4, 4));
}

// ---- the whole game ----

#[test]
fn random_games_with_every_new_system_keep_the_invariants() {
    let d = Arc::new(data());
    for seed in 0..30 {
        let mut g = Game::new(d.clone(), seed);
        let mut rng = Rng::new(seed ^ 0xD1A);
        let mut steps = 0;
        while !g.over && steps < 20_000 {
            let a = random_action(&g, &mut rng);
            g.step(a).unwrap();
            g.check_invariants().unwrap_or_else(|e| panic!("seed {seed}: {e}"));
            steps += 1;
        }
        assert!(g.over, "seed {seed} did not finish");
    }
}
