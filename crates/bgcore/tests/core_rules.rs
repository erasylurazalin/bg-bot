//! The generic rules, on the real card data (`Data::bundled`): the shop
//! economy and its limits, pairing and knockouts, replays, the fight basics,
//! Activate, Bonus Keywords, Stealth and the parser's refusals. The tribe
//! files cover the cards; this file covers what every card shares.

use std::sync::Arc;

use bgcore::fight::{Watch, fight_watched};
use bgcore::{
    ACTION_KINDS, Action, CardDb, Data, Game, Keywords, Minion, N_ACTIONS, Opponent, Rng, Tribe,
    Tribes,
};

fn db() -> CardDb {
    Data::bundled().cards
}

fn lobby(ts: &[Tribe]) -> bgcore::Tribes {
    ts.iter().copied().fold(bgcore::Tribes::NONE, |m, t| m.with(t))
}

fn game(seed: u64) -> Game {
    Game::new_with_lobby(Arc::new(Data::bundled()), seed, lobby(&[Tribe::Beast, Tribe::Mech, Tribe::Dragon, Tribe::Murloc, Tribe::Demon, Tribe::Undead]))
}

/// Answer whatever the current player has open (a trinket offer, or a
/// Discover a trinket opened at the start of the turn). False when nothing is.
fn answer(g: &mut Game) -> bool {
    let Some(p) = g.current_player() else { return false };
    let pl = &g.players[p];
    // The same priority `is_legal` gives: another choice first, then the
    // trinket offer.
    let a = if !pl.discover.is_empty()
        || !pl.power_offer.is_empty()
        || pl.pending_choose_one.is_some()
    {
        Action::Choose(0)
    } else if pl.pending_discard.is_some() {
        Action::Discard(0)
    } else if pl.pending_shop_target.is_some() {
        Action::TargetShop(0)
    } else if pl.pending_target.is_some() {
        Action::Target(0)
    } else if !pl.trinket_offer.is_empty() {
        Action::ChooseTrinket(0)
    } else {
        return false;
    };
    g.step(a).unwrap_or_else(|_| panic!("could not answer {a:?}"));
    true
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
    // A trinket offer opens at the start of turn 6 and blocks the shopping
    // actions until it is answered: take the first option and come back.
    while !g.players[p].trinket_offer.is_empty() {
        g.step(Action::ChooseTrinket(0)).unwrap();
        turn_of(g, p);
    }
    g.step(a).unwrap_or_else(|_| panic!("{a:?} illegal for player {p}"));
}

/// Everyone ends their turn until the next turn starts.
fn next_turn(g: &mut Game) {
    let t = g.turn;
    while g.turn == t && !g.over {
        if !answer(g) {
            g.step(Action::EndTurn).unwrap();
        }
    }
}

/// Put a card from the pool straight onto a board; golden takes 3 copies.
fn put_on_board(g: &mut Game, p: usize, name: &str, golden: bool) {
    let c = g.data.cards.by_name(name).unwrap();
    let slot = g.data.cards.get(c).pool_slot as usize;
    let copies = if golden { 3 } else { 1 };
    g.pool[slot] -= copies;
    let m = g.players[p].make_card(&g.data.cards, c, golden, copies);
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
        let c = g.data.cards.by_name(n).unwrap();
        let slot = g.data.cards.get(c).pool_slot as usize;
        g.pool[slot] -= 1;
        let m = g.players[p].make_card(&g.data.cards, c, false, 1);
        g.players[p].shop.push(m);
    }
}

/// Whether `a` is legal for `p` right now: first lets the other players
/// freeze until it is `p`'s action again.
fn legal_for(g: &mut Game, p: usize, a: Action) -> bool {
    turn_of(g, p);
    g.is_legal(a)
}

fn legal(g: &Game) -> Vec<Action> {
    let mut mask = [false; N_ACTIONS];
    g.legal_mask(&mut mask);
    (0..N_ACTIONS).filter(|&i| mask[i]).map(|i| Action::from_index(i).unwrap()).collect()
}

// ---- the shop economy ----

#[test]
fn gold_goes_up_by_one_to_ten() {
    let mut g = game(1);
    let mut golds = vec![];
    for _ in 0..10 {
        golds.push(g.players[0].gold);
        next_turn(&mut g);
    }
    assert_eq!(golds, [3, 4, 5, 6, 7, 8, 9, 10, 10, 10]);
}

#[test]
fn the_tier_five_to_six_upgrade_costs_eleven() {
    // Every level-up into tier 5 in Era's logs (44 of 44 when this was
    // written) shows the next button at 11, not the 10 rules.toml had.
    assert_eq!(Data::bundled().rules.upgrade_cost(5), 11);
    let mut g = game(7);
    g.players[0].tier = 4;
    g.players[0].upgrade_cost = 11;
    g.players[0].gold = 20;
    act(&mut g, 0, Action::Level);
    assert_eq!((g.players[0].tier, g.players[0].upgrade_cost), (5, 11));
}

#[test]
fn once_this_reaches_attack_applies_in_the_tavern_and_in_hand() {
    // Scarlet Survivor: "Once this reaches 6 Attack, gain Divine Shield". The
    // text has no zone: a Tavern minion and a hand card gain it, not only a
    // board minion (the log: a Tavern Scarlet Survivor buffed by Suspicious
    // Prisonguard's +3/+3 gains the shield).
    let mut g = game(11);
    set_shop(&mut g, 0, &["Scarlet Survivor"]); // 3/3
    let b = bgcore::game::TavernBuff {
        tribe: None,
        attack: 3,
        health: 0,
        this_turn: false,
        typed_only: false,
        buy_keep: None,
    };
    g.players[0].add_tavern_buff(&g.data.cards, b);
    assert_eq!(g.players[0].shop[0].attack, 6);
    act(&mut g, 0, Action::Freeze); // any step runs the checks
    assert!(g.players[0].shop[0].keywords.has(Keywords::DIVINE_SHIELD), "in the Tavern");

    // In hand: buffed to 6 Attack there.
    g.give(0, "Scarlet Survivor").unwrap();
    let h = g.players[0].hand.len() - 1;
    assert!(!g.players[0].hand[h].keywords.has(Keywords::DIVINE_SHIELD));
    g.players[0].hand[h].attack = 6;
    act(&mut g, 0, Action::Freeze);
    assert!(g.players[0].hand[h].keywords.has(Keywords::DIVINE_SHIELD), "in hand");
    g.check_invariants().unwrap();
}

#[test]
fn upgrade_cost_drops_each_turn_and_resets_on_upgrade() {
    let mut g = game(2);
    assert_eq!(g.players[0].upgrade_cost, 5);
    next_turn(&mut g);
    assert_eq!(g.players[0].upgrade_cost, 4);
    act(&mut g, 0, Action::Level);
    assert_eq!((g.players[0].tier, g.players[0].gold, g.players[0].upgrade_cost), (2, 0, 7));
    next_turn(&mut g);
    assert_eq!(g.players[0].upgrade_cost, 6);
    for _ in 0..10 {
        next_turn(&mut g);
    }
    assert_eq!(g.players[0].upgrade_cost, 0);
}

#[test]
fn shop_size_and_tiers_follow_the_tavern_tier() {
    let mut g = game(3);
    assert_eq!(g.players[0].shop.len(), 3);
    assert!(g.players[0].shop.iter().all(|m| g.data.cards.get(m.card).tier == 1));
    next_turn(&mut g);
    act(&mut g, 0, Action::Level);
    next_turn(&mut g);
    assert_eq!(g.players[0].shop.len(), 4);
    assert!(g.players[0].shop.iter().all(|m| g.data.cards.get(m.card).tier <= 2));
}

#[test]
fn buy_sell_roll_cost_and_pay() {
    let mut g = game(4);
    act(&mut g, 0, Action::Buy(0));
    assert_eq!((g.players[0].gold, g.players[0].hand.len(), g.players[0].shop.len()), (0, 1, 2));
    assert!(!g.is_legal(Action::Roll) || g.current_player() != Some(0));
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    act(&mut g, 0, Action::Sell(0));
    assert_eq!((g.players[0].gold, g.players[0].board.len()), (1, 0));
    act(&mut g, 0, Action::Roll);
    assert_eq!((g.players[0].gold, g.players[0].shop.len()), (0, 3));
}

#[test]
fn hand_holds_ten() {
    let mut g = game(5);
    for _ in 0..10 {
        g.give(0, "Buzzing Vermin").unwrap();
    }
    g.players[0].gold = 10;
    turn_of(&mut g, 0);
    assert!(!g.is_legal(Action::Buy(0)));
    assert!(!legal(&g).iter().any(|a| matches!(a, Action::Buy(_))));
}

#[test]
fn board_holds_seven() {
    let mut g = game(6);
    for _ in 0..7 {
        put_on_board(&mut g, 0, "Buzzing Vermin", false);
    }
    g.give(0, "Cord Puller").unwrap();
    turn_of(&mut g, 0);
    assert!(!legal(&g).iter().any(|a| matches!(a, Action::Play { .. })));
}

#[test]
fn selling_a_golden_returns_three_copies() {
    let mut g = game(9);
    let vermin = g.data.cards.by_name("Buzzing Vermin").unwrap() as usize;
    put_on_board(&mut g, 0, "Buzzing Vermin", true);
    let before = g.pool[vermin];
    act(&mut g, 0, Action::Sell(0));
    assert_eq!(g.pool[vermin], before + 3);
}

#[test]
fn frozen_shop_is_kept_and_topped_up() {
    let mut g = game(15);
    let kept: Vec<_> = g.players[0].shop[1..].to_vec();
    act(&mut g, 0, Action::Buy(0));
    act(&mut g, 0, Action::Freeze);
    next_turn(&mut g);
    let p = &g.players[0];
    assert!(!p.frozen);
    assert_eq!(p.shop.len(), 3);
    assert_eq!(p.shop[..2], kept[..]);
}

#[test]
fn rolling_clears_freeze() {
    let mut g = game(16);
    act(&mut g, 0, Action::Freeze);
    act(&mut g, 0, Action::Roll);
    assert!(!g.players[0].frozen);
}

#[test]
fn turn_ends_after_the_action_cap() {
    let mut g = game(17);
    let cap = g.data.rules.max_actions_per_turn;
    for _ in 0..cap {
        act(&mut g, 0, Action::Freeze);
    }
    assert!(g.players[0].done);
    assert!(g.current_player() != Some(0));
}

#[test]
fn illegal_actions_are_refused() {
    let mut g = game(21);
    assert!(g.step(Action::Sell(0)).is_err());
    assert!(g.step(Action::Target(0)).is_err());
    assert!(g.step(Action::Level).is_err()); // 3 gold, costs 5
}

// ---- pairing, fight damage, endings ----

#[test]
fn losing_a_fight_costs_tier_plus_survivors_capped() {
    let mut g = game(18);
    let Some(Opponent::Player(q)) = g.players[0].opponent else { panic!() };
    put_on_board(&mut g, 0, "Sewer Lord", false);
    next_turn(&mut g);
    // tier 1 + Sewer Lord's tier 5 = 6, capped at 5 on turn 1
    assert_eq!(g.players[q as usize].health, 25);
    assert_eq!(g.players[0].health, 30);
    // what each saw of the other
    assert_eq!(g.players[q as usize].seen[0].board.len(), 1);
    assert_eq!(g.players[q as usize].seen[0].turn, 1);
}

#[test]
fn ties_deal_no_damage() {
    let mut g = game(19);
    next_turn(&mut g);
    assert!(g.players.iter().all(|p| p.health == 30));
}

#[test]
fn pairing_avoids_the_last_opponent() {
    let mut g = game(20);
    let mut repeats = 0;
    for _ in 0..30 {
        let last: Vec<_> = g.players.iter().map(|p| p.last_opponent).collect();
        for (p, l) in g.players.iter().zip(&last) {
            if l.is_some() && p.opponent == *l {
                repeats += 1;
            }
        }
        next_turn(&mut g);
    }
    assert_eq!(repeats, 0);
}

fn random_game(seed: u64, record: Option<&mut Vec<Action>>) -> Game {
    let mut g = game(seed);
    let mut rng = Rng::new(seed ^ 0xabcdef);
    let mut rec = record;
    while !g.over {
        let alive = g.alive();
        let ghosts = g.players.iter().filter(|p| p.alive && matches!(p.opponent, Some(Opponent::Ghost(_)))).count();
        assert_eq!(ghosts, alive % 2, "turn {}", g.turn);
        let a = bgcore::bots::random_action(&g, &mut rng);
        if let Some(r) = rec.as_deref_mut() {
            r.push(a);
        }
        g.step(a).unwrap();
        g.check_invariants().unwrap();
    }
    g
}

#[test]
fn random_games_finish_with_a_full_ranking() {
    for seed in 0..30 {
        let g = random_game(seed, None);
        let mut p = g.placements();
        p.sort();
        assert_eq!(p, [1, 2, 3, 4, 5, 6, 7, 8]);
        assert!(g.turn <= g.data.rules.max_turns);
    }
}

#[test]
fn seed_and_actions_replay_a_game_exactly() {
    let mut actions = vec![];
    let a = random_game(42, Some(&mut actions));
    let mut b = game(42);
    for &x in &actions {
        b.step(x).unwrap();
    }
    assert!(b.over);
    assert_eq!(a.players, b.players);
    assert_eq!(a.pool, b.pool);
    assert_eq!(a.rng, b.rng);
}

// ---- fight basics ----

fn board(db: &CardDb, names: &[&str]) -> Vec<Minion> {
    names.iter().map(|n| db.make(db.by_name(n).expect(n), false)).collect()
}

#[derive(Default)]
struct Log {
    attacks: Vec<(usize, String, String)>,
    /// The attacker's keywords at each attack (Venomous is spent by the hit
    /// that kills, so this shows it going).
    attacker_keywords: Vec<Keywords>,
    deaths: Vec<(usize, String)>,
    summons: Vec<(usize, usize, Minion)>,
    db: Option<CardDb>,
}

impl Watch for Log {
    fn attack(&mut self, side: usize, a: &Minion, d: &Minion) {
        let db = self.db.as_ref().unwrap();
        self.attacker_keywords.push(a.keywords);
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
    let a = board(&db, &["Buzzing Vermin"]);
    let b = board(&db, &["Buzzing Vermin", "Buzzing Vermin"]);
    for seed in 0..50 {
        let (_, log) = watched(&a, &b, seed);
        assert_eq!(log.attacks[0].0, 1);
    }
}

#[test]
fn equal_boards_flip_a_coin_for_first_attack() {
    let db = db();
    let a = board(&db, &["Buzzing Vermin"]);
    let firsts: usize = (0..400).map(|s| watched(&a, &a, s).1.attacks[0].0).sum();
    assert!((150..250).contains(&firsts), "{firsts} of 400");
}

#[test]
fn attackers_go_left_to_right_and_wrap() {
    let db = db();
    let a = board(&db, &["Buzzing Vermin", "Amber Guardian", "Flittering Bat"]);
    let mut b = board(&db, &["Sewer Lord"]);
    b[0].health = 1000;
    b[0].attack = 0;
    let (_, log) = watched(&a, &b, 1);
    let order: Vec<&str> = log.attacks.iter().filter(|x| x.0 == 0).map(|x| x.1.as_str()).take(6).collect();
    // Flittering Bat's Foraging Bat joins the line on its right and attacks
    // before the wrap: Hearthstone_2026_11_13_14_50_52 lines 491120 and
    // 491284. The old order kept the token back until the next round.
    assert_eq!(order, ["Buzzing Vermin", "Amber Guardian", "Flittering Bat", "Foraging Bat", "Buzzing Vermin", "Amber Guardian"]);
}

#[test]
fn taunt_must_be_attacked_first() {
    let db = db();
    let a = board(&db, &["Flittering Bat", "Flittering Bat", "Flittering Bat"]);
    let mut b = board(&db, &["Flittering Bat", "Amber Guardian", "Flittering Bat"]);
    b[1].health = 100;
    for seed in 0..100 {
        let (_, log) = watched(&a, &b, seed);
        for (side, _, def) in &log.attacks {
            if *side == 0 {
                assert_eq!(def, "Amber Guardian");
            }
        }
    }
}

#[test]
fn divine_shield_blocks_one_hit() {
    // 1/1 into 1/1: without the shield whoever hits first wins the trade and
    // the Beetle that replaces the Vermin ends it: side 1 always wins. With
    // the shield the first hit is absorbed and the fight grinds into a tie.
    let db = db();
    let a = board(&db, &["Cord Puller"]);
    let b = board(&db, &["Buzzing Vermin"]);
    for seed in 0..50 {
        let (out, _) = watched(&a, &b, seed);
        assert_eq!(out.winner, None, "seed {seed}");
    }
    let mut plain = a.clone();
    plain[0].keywords = Keywords::default();
    for seed in 0..50 {
        let (out, _) = watched(&plain, &b, seed);
        assert_eq!(out.winner, Some(1), "seed {seed}");
    }
}

#[test]
fn zero_damage_does_not_pop_divine_shield() {
    let db = db();
    let mut a = board(&db, &["Cord Puller"]);
    let mut b = board(&db, &["Flittering Bat"]);
    b[0].attack = 0;
    b[0].health = 100;
    a[0].attack = 1;
    let (out, log) = watched(&a, &b, 3);
    // exactly 100 hits to kill the Bat, and nothing dies on the Puller's
    // side: the Bat's 0-damage hits never popped the shield
    assert_eq!(out.attacks, 100);
    assert_eq!(out.winner, Some(0));
    assert!(log.deaths.iter().all(|d| d.0 == 1));
}

#[test]
fn venomous_kills_anything_it_damages() {
    let db = db();
    let spore = board(&db, &["Deadly Spore"]);
    let mut big = board(&db, &["Amber Guardian"]);
    big[0].attack = 0;
    big[0].health = 1000;
    big[0].keywords.clear(Keywords::TAUNT);
    for seed in 0..50 {
        let (out, _) = watched(&spore, &big, seed);
        assert_eq!(out.winner, Some(0), "seed {seed}");
    }
    // the control: without Venomous the 1 Attack never gets through before
    // the attack cap
    let mut plain = spore.clone();
    plain[0].keywords = Keywords::default();
    for seed in 0..50 {
        let (out, _) = watched(&plain, &big, seed);
        assert_eq!(out.winner, None, "seed {seed}");
    }
}

/// 0-Attack walls, so the other side does all the attacking and nothing kills
/// the attacker. Their 1000 Health outlasts the whole fight (500 attacks), so
/// only a keyword can kill one.
fn walls(db: &CardDb, n: usize) -> Vec<Minion> {
    // Water Droplet is a plain token: no effect, no keyword, no Deathrattle.
    let mut out = board(db, &vec!["Water Droplet"; n]);
    for w in out.iter_mut() {
        w.attack = 0;
        w.health = 1000;
        w.keywords.clear(Keywords::TAUNT);
    }
    out
}

#[test]
fn poisonous_kills_every_time_it_damages() {
    // Tide Oracle Morgl is Poisonous: it kills the minion it damages and is
    // never used up, so it kills a second one too (docs/rules/design.md
    // "Keywords"). Venomous would be spent by the first kill.
    let db = db();
    let morgl = board(&db, &["Tide Oracle Morgl"]);
    assert!(morgl[0].keywords.has(Keywords::POISONOUS));
    assert!(!morgl[0].keywords.has(Keywords::VENOMOUS));
    let two = walls(&db, 2);
    for seed in 0..20 {
        let (out, log) = watched(&morgl, &two, seed);
        assert_eq!(out.winner, Some(0), "seed {seed}");
        assert_eq!(log.deaths.iter().filter(|d| d.0 == 1).count(), 2, "both walls died, seed {seed}");
        assert_eq!(log.attacker_keywords.len(), 2, "one swing each, seed {seed}");
        assert!(
            log.attacker_keywords[1].has(Keywords::POISONOUS),
            "still Poisonous after the first kill, seed {seed}"
        );
    }
    // Divine Shield takes the hit and leaves the Poisonous unused, like the
    // Venomous: the shield pops, the next swing kills.
    let mut shielded = walls(&db, 1);
    shielded[0].keywords.set(Keywords::DIVINE_SHIELD);
    for seed in 0..20 {
        let (out, log) = watched(&morgl, &shielded, seed);
        assert_eq!(out.winner, Some(0), "seed {seed}");
        assert_eq!(log.deaths.iter().filter(|d| d.0 == 1).count(), 1, "seed {seed}");
        assert!(
            log.attacker_keywords.iter().all(|k| k.has(Keywords::POISONOUS)),
            "the shield's hit did not spend it, seed {seed}"
        );
    }
}

#[test]
fn a_minion_with_venomous_and_poisonous_keeps_its_venomous() {
    // A minion can wear both (Era, 2026-11-04). The Poisonous is permanent, so
    // an assumed rule has it do the killing and leave the Venomous unused:
    // nothing in the docs or the logs says which of the two is spent.
    let db = db();
    let mut both = board(&db, &["Deadly Spore"]);
    both[0].keywords.set(Keywords::POISONOUS);
    let two = walls(&db, 2);
    for seed in 0..20 {
        let (out, log) = watched(&both, &two, seed);
        assert_eq!(out.winner, Some(0), "seed {seed}");
        assert_eq!(log.deaths.iter().filter(|d| d.0 == 1).count(), 2, "seed {seed}");
        let second = log.attacker_keywords[1];
        assert!(second.has(Keywords::POISONOUS), "seed {seed}");
        assert!(second.has(Keywords::VENOMOUS), "the Venomous was not needed, seed {seed}");
    }
    // The control: Venomous alone is spent by the first kill, so the second
    // swing is a plain 1 damage and the wall lives.
    let venomous = board(&db, &["Deadly Spore"]);
    for seed in 0..20 {
        let (_, log) = watched(&venomous, &two, seed);
        assert_eq!(log.deaths.len(), 1, "one kill only, seed {seed}");
        assert!(
            !log.attacker_keywords[1].has(Keywords::VENOMOUS),
            "Venomous is gone after it kills, seed {seed}"
        );
    }
}

#[test]
fn divine_shield_stops_venomous_and_the_shielded_minion_wins() {
    let db = db();
    let spore = board(&db, &["Deadly Spore"]);
    let a = board(&db, &["Cord Puller"]);
    for seed in 0..50 {
        let (out, _) = watched(&spore, &a, seed);
        assert_eq!(out.winner, Some(1), "seed {seed}");
        assert_eq!(out.survivor_tiers, 1, "seed {seed}");
    }
}

#[test]
fn windfury_attacks_twice_in_a_row() {
    let db = db();
    let a = board(&db, &["Crackling Cyclone"]);
    let mut b = board(&db, &["Sewer Lord"]);
    b[0].attack = 0;
    b[0].health = 100;
    let (_, log) = watched(&a, &b, 0);
    let sides: Vec<usize> = log.attacks.iter().map(|x| x.0).take(4).collect();
    assert_eq!(sides, [0, 0, 0, 0]); // the wall can't attack, the Cyclone swings twice per turn
    let mut a = board(&db, &["Crackling Cyclone", "Buzzing Vermin"]);
    a[0].health = 100;
    let mut b = board(&db, &["Amber Guardian", "Amber Guardian"]);
    b[0].health = 100;
    b[1].health = 100;
    let (_, log) = watched(&a, &b, 0);
    let first_a = log.attacks.iter().position(|x| x.0 == 0).unwrap();
    assert_eq!(log.attacks[first_a].1, "Crackling Cyclone");
    assert_eq!(log.attacks[first_a + 1].1, "Crackling Cyclone");
    assert_eq!(log.attacks[first_a + 2].0, 1);
}

#[test]
fn neither_side_able_to_attack_is_a_tie() {
    let db = db();
    let mut a = board(&db, &["Buzzing Vermin"]);
    let mut b = board(&db, &["Buzzing Vermin"]);
    a[0].attack = 0;
    b[0].attack = 0;
    for seed in 0..50 {
        let (out, _) = watched(&a, &b, seed);
        assert_eq!((out.winner, out.attacks), (None, 0));
    }
}

#[test]
fn a_side_that_cant_attack_is_skipped() {
    let db = db();
    let mut a = board(&db, &["Flittering Bat", "Flittering Bat"]);
    a[0].attack = 0;
    a[1].attack = 0;
    let b = board(&db, &["Sewer Lord"]);
    for seed in 0..50 {
        let (out, _) = watched(&a, &b, seed);
        assert_eq!(out.winner, Some(1));
    }
}

#[test]
fn attack_cap_ends_in_a_tie() {
    let db = db();
    let mut a = board(&db, &["Buzzing Vermin"]);
    a[0].health = 1_000_000;
    let b = a.clone();
    for seed in 0..50 {
        let (out, _) = watched(&a, &b, seed);
        assert_eq!((out.winner, out.attacks), (None, 500));
    }
}

#[test]
fn empty_boards() {
    let db = db();
    let a = board(&db, &["Buzzing Vermin"]);
    for seed in 0..50 {
        let (out, _) = watched(&[], &[], seed);
        assert_eq!(out.winner, None);
        let (out, _) = watched(&a, &[], seed);
        assert_eq!((out.winner, out.survivor_tiers), (Some(0), 1));
    }
}

#[test]
fn same_seed_same_fight() {
    let db = db();
    let a = board(&db, &["Sewer Lord", "Deadly Spore", "Crackling Cyclone", "Bronze Warden"]);
    let b = board(&db, &["Cord Puller", "Amber Guardian", "Heroic Underdog", "Flittering Bat", "Buzzing Vermin"]);
    for seed in 0..100 {
        let x = bgcore::fight_with(&db, &a, &b, [Default::default(), Default::default()], 500, &mut Rng::new(seed), &mut ());
        let y = bgcore::fight_with(&db, &a, &b, [Default::default(), Default::default()], 500, &mut Rng::new(seed), &mut ());
        assert_eq!(x, y);
    }
}

#[test]
fn survivor_tiers_count_tokens_as_their_tier() {
    let db = db();
    // Cadaver Caretaker (3/3) into an even trade with Amber Guardian (3/3):
    // the Caretaker either survives on its own or dies into three 1/1
    // Skeletons that finish the Guardian. Either way side 0 wins, and a win
    // carried by tier-1 tokens never counts more than 3.
    let mut b = board(&db, &["Amber Guardian"]);
    b[0].attack = 3;
    b[0].health = 3;
    b[0].keywords.clear(Keywords::TAUNT);
    let a = board(&db, &["Cadaver Caretaker"]);
    for seed in 0..100 {
        let (out, log) = watched(&a, &b, seed);
        assert_eq!(out.winner, Some(0), "seed {seed}");
        if log.summons.is_empty() {
            assert_eq!(out.survivor_tiers, 3, "seed {seed}");
        } else {
            assert_eq!(log.summons.len(), 3, "seed {seed}");
            assert!((1..=3).contains(&out.survivor_tiers), "seed {seed}");
        }
    }
}

// ---- a pending pick must be answered ----

#[test]
fn a_pending_pick_must_be_answered_first() {
    let mut g = game(10);
    put_on_board(&mut g, 0, "Buzzing Vermin", false);
    put_on_board(&mut g, 0, "Cord Puller", false);
    set_shop(&mut g, 0, &[]);
    g.give(0, "Fortify").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    assert!(g.players[0].pending_target.is_some());
    turn_of(&mut g, 0);
    // while the pick waits, only answering it is legal
    let l = legal(&g);
    assert_eq!(l, [Action::Target(0), Action::Target(1)]);
    act(&mut g, 0, Action::Target(1));
    assert!(g.players[0].pending_target.is_none());
    let b = &g.players[0].board;
    assert_eq!((b[1].attack, b[1].health), (1, 4));
    assert!(b[1].keywords.has(Keywords::TAUNT));
    assert_eq!((b[0].attack, b[0].health), (1, 1));
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
    put_on_board(&mut g, 0, "Buzzing Vermin", false);
    put_on_board(&mut g, 0, "Suspicious Prisonguard", false); // costs 1
    turn_of(&mut g, 0);
    g.players[0].gold = 0;
    assert!(!g.is_legal(Action::Activate(1)), "not enough gold");
    g.players[0].gold = 1;
    assert!(g.is_legal(Action::Activate(1)));
    assert!(!g.is_legal(Action::Activate(0)), "no ability");
    assert!(!g.is_legal(Action::Activate(2)), "no minion");
    assert!(legal(&g).contains(&Action::Activate(1)));
    assert!(!legal(&g).contains(&Action::Activate(0)));
}

#[test]
fn activate_pays_its_cost_runs_once_per_turn_and_waits_for_its_pick() {
    let mut g = game(6);
    put_on_board(&mut g, 0, "Buzzing Vermin", false);
    put_on_board(&mut g, 0, "Amber Guardian", false);
    put_on_board(&mut g, 0, "Suspicious Prisonguard", false);
    set_shop(&mut g, 0, &[]);
    turn_of(&mut g, 0);
    g.players[0].gold = 5;
    act(&mut g, 0, Action::Activate(2));
    assert_eq!(g.players[0].gold, 4);
    // two targets, so it waits for a pick, and the source can't be it
    assert!(g.players[0].pending_target.is_some());
    assert!(!legal_for(&mut g, 0, Action::Target(2)));
    assert!(!legal_for(&mut g, 0, Action::EndTurn));
    act(&mut g, 0, Action::Target(1));
    assert_eq!((g.players[0].board[1].attack, g.players[0].board[1].health), (6, 5));
    assert_eq!((g.players[0].board[0].attack, g.players[0].board[0].health), (1, 1));
    // once per turn: a second use is illegal even with the gold
    assert!(!legal_for(&mut g, 0, Action::Activate(2)));
    assert!(!legal(&g).contains(&Action::Activate(2)));
    g.check_invariants().unwrap();
}

#[test]
fn activate_is_legal_again_next_turn_and_a_moved_minion_stays_used() {
    let mut g = game(60);
    put_on_board(&mut g, 0, "Buzzing Vermin", false);
    put_on_board(&mut g, 0, "Suspicious Prisonguard", false);
    turn_of(&mut g, 0);
    g.players[0].gold = 9;
    set_shop(&mut g, 0, &["Cord Puller"]);
    act(&mut g, 0, Action::Activate(1));
    act(&mut g, 0, Action::Target(0));
    assert!(!legal_for(&mut g, 0, Action::Activate(1)));
    // moving it doesn't reset it
    act(&mut g, 0, Action::Move { from: 1, to: 0 });
    assert!(!legal_for(&mut g, 0, Action::Activate(0)));
    next_turn(&mut g);
    turn_of(&mut g, 0);
    g.players[0].gold = 9;
    assert!(g.is_legal(Action::Activate(0)));
    assert!(legal(&g).contains(&Action::Activate(0)));
    // a copy bought later is a new minion and starts unused
    act(&mut g, 0, Action::Activate(0));
    act(&mut g, 0, Action::Target(1));
    act(&mut g, 0, Action::Sell(0));
    put_on_board(&mut g, 0, "Suspicious Prisonguard", false);
    let last = g.players[0].board.len() as u8 - 1;
    assert!(legal_for(&mut g, 0, Action::Activate(last)));
    g.check_invariants().unwrap();
}

#[test]
fn activate_without_a_target_is_refused() {
    let mut g = game(7);
    put_on_board(&mut g, 0, "Suspicious Prisonguard", false);
    turn_of(&mut g, 0);
    g.players[0].gold = 4;
    set_shop(&mut g, 0, &[]);
    // the only friendly minion is the source, so there is nothing to pick
    assert!(!g.is_legal(Action::Activate(0)));
    assert!(!legal(&g).iter().any(|a| matches!(a, Action::Activate(_))));
}

#[test]
fn a_golden_activate_doubles_its_effect() {
    let mut g = game(9);
    put_on_board(&mut g, 0, "Buzzing Vermin", false);
    put_on_board(&mut g, 0, "Suspicious Prisonguard", false);
    g.players[0].board[1].golden = true;
    turn_of(&mut g, 0);
    g.players[0].gold = 2;
    set_shop(&mut g, 0, &[]);
    act(&mut g, 0, Action::Activate(1));
    // the Vermin is the only legal target and gets the doubled buff
    assert!(g.players[0].pending_target.is_none());
    assert_eq!((g.players[0].board[0].attack, g.players[0].board[0].health), (1 + 6, 1 + 6));
    g.check_invariants().unwrap();
}

// ---- Bonus Keywords ----

#[test]
fn the_bonus_keyword_set_comes_from_the_rules() {
    let d = Data::bundled();
    let want = Keywords::TAUNT
        | Keywords::DIVINE_SHIELD
        | Keywords::WINDFURY
        | Keywords::VENOMOUS
        | Keywords::REBORN
        | Keywords::STEALTH;
    assert_eq!(d.rules.bonus_mask, want);
    assert_eq!(d.rules.bonus_mask & Keywords::MAGNETIC, 0);
    // an unknown Bonus Keyword is refused
    let bad = include_str!("../../../rules.toml").replace("\"stealth\"]", "\"flying\"]");
    assert!(bgcore::Rules::from_toml(&bad).unwrap_err().contains("flying"));
}

#[test]
fn a_random_bonus_keyword_is_never_one_it_has() {
    // Sewer Escapee's Activate gives another Murloc a random Bonus Keyword.
    // One a turn: the Tad gains a new one every time until it has all six,
    // and then nothing more happens.
    for seed in 0..4 {
        let mut g = game(seed);
        put_on_board(&mut g, 0, "Sewer Escapee", false);
        put_on_board(&mut g, 0, "Tad", false);
        let mut have = 0u32;
        for _ in 0..6 {
            next_turn(&mut g);
            turn_of(&mut g, 0);
            g.players[0].gold = 9;
            act(&mut g, 0, Action::Activate(0));
            // "another Murloc" is a Murloc in the Tavern too (docs/rules/design.md
            // "Targets in the shop"): pick the board's Tad when the pick is
            // asked (with no Murloc in the Tavern it is the only answer).
            if g.players[0].pending_target.is_some() {
                act(&mut g, 0, Action::Target(1));
            }
            let now = (g.players[0].board[1].keywords.0 & g.data.rules.bonus_mask).count_ones();
            assert_eq!(now, have + 1, "seed {seed}: it must gain a new keyword every time");
            have = now;
        }
        // it has them all, so nothing more happens
        next_turn(&mut g);
        turn_of(&mut g, 0);
        g.players[0].gold = 9;
        act(&mut g, 0, Action::Activate(0));
        let before = g.players[0].board[1].keywords;
        assert_eq!((before.0 & g.data.rules.bonus_mask).count_ones(), 6);
        assert!(!before.has(Keywords::MAGNETIC));
        g.check_invariants().unwrap();
    }
}

// ---- event counters ----

#[test]
fn a_buy_counter_counts_spells_bought_and_not_other_players_buys() {
    let mut g = game(44);
    put_on_board(&mut g, 0, "Auto Reveille", false); // every 3 cards bought
    set_shop(&mut g, 0, &["Buzzing Vermin", "Buzzing Vermin"]);
    g.players[0].shop_spell = Some(g.data.cards.by_name("Tavern Coin").unwrap());
    turn_of(&mut g, 0);
    g.players[0].gold = 10;
    // another player buying does not count
    g.players[1].gold = 20;
    act(&mut g, 1, Action::Buy(0));
    act(&mut g, 1, Action::Buy(0));
    act(&mut g, 1, Action::Buy(0));
    assert_eq!(g.players[0].board[0].magnetizations, 0);
    act(&mut g, 0, Action::Buy(0));
    act(&mut g, 0, Action::Buy(0));
    assert_eq!(g.players[0].board[0].magnetizations, 0, "two cards are not three");
    // a bought Tavern spell counts as a bought card
    act(&mut g, 0, Action::BuySpell);
    assert_eq!(g.players[0].board[0].magnetizations, 1, "the third buy triggers it");
    g.check_invariants().unwrap();
}

// ---- the parser ----

#[test]
fn a_card_needs_exactly_one_of_tribe_and_tribes() {
    let both = "[[minion]]\nname = \"A\"\ntier = 1\ntribe = \"beast\"\ntribes = [\"mech\"]\nstats = [1, 1]\n";
    assert!(CardDb::from_toml(both).unwrap_err().contains("both tribe and tribes"));
    let none = "[[minion]]\nname = \"A\"\ntier = 1\nstats = [1, 1]\n";
    assert!(CardDb::from_toml(none).unwrap_err().contains("needs tribe"));
    let bad = "[[minion]]\nname = \"A\"\ntier = 1\ntribes = [\"beast\", \"naga\"]\nstats = [1, 1]\n";
    assert!(CardDb::from_toml(bad).unwrap_err().contains("unknown tribe"));
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

// ---- tribes ----

#[test]
fn tribe_sets_hold_one_two_or_all() {
    let db = db();
    let t = |n: &str| db.get(db.by_name(n).unwrap()).tribes;
    assert!(t("Buzzing Vermin").has(Tribe::Beast) && !t("Buzzing Vermin").has(Tribe::Dragon));
    assert!(t("Heroic Underdog").is_neutral());
    let felboar = t("Felboar");
    assert!(felboar.has(Tribe::Demon) && felboar.has(Tribe::Quilboar) && !felboar.has(Tribe::Murloc));
    assert!(!felboar.is_neutral());
    assert!(felboar.overlaps(Tribes::of(Tribe::Demon)) && !felboar.overlaps(Tribes::of(Tribe::Mech)));
    for tribe in Tribe::EVERY {
        assert!(t("Gatekeeper Amalgam").has(tribe));
    }
    assert!(!t("Gatekeeper Amalgam").is_neutral());
}

#[test]
fn board_tribes_count_each_tribe_of_a_minion() {
    let mut g = game(2);
    for n in ["Buzzing Vermin", "Cord Puller", "Heroic Underdog", "Bronze Warden", "Felboar"] {
        put_on_board(&mut g, 0, n, false);
    }
    put_on_board(&mut g, 0, "Prosthetic Hand", false);
    // The ten tribes in Tribe::EVERY order, then neutral: the Hand is Mech
    // and Undead, the Felboar Demon and Quilboar, the Underdog neutral.
    assert_eq!(g.players[0].board_tribes(&g.data.cards), [1, 2, 1, 0, 1, 0, 0, 1, 1, 0, 1]);
}

// ---- Stealth ----

#[derive(Default)]
struct AttackLog {
    /// (attacker card, attacker had Stealth, defender card, defender had Stealth)
    attacks: Vec<(bgcore::CardId, bool, bgcore::CardId, bool)>,
}

impl Watch for AttackLog {
    fn attack(&mut self, _side: usize, a: &Minion, d: &Minion) {
        self.attacks.push((
            a.card,
            a.keywords.has(Keywords::STEALTH),
            d.card,
            d.keywords.has(Keywords::STEALTH),
        ));
    }
}

fn stealth_fight(a: &[Minion], b: &[Minion], seed: u64) -> (bgcore::Outcome, AttackLog) {
    let db = db();
    let mut log = AttackLog::default();
    let out = fight_watched(&db, a, b, 500, &mut Rng::new(seed), &mut log);
    (out, log)
}

#[test]
fn stealth_cannot_be_targeted_until_it_attacks() {
    let db = db();
    let underdog = db.by_name("Heroic Underdog").unwrap();
    let plain = db.by_name("Flittering Bat").unwrap();
    // Side 0 has more minions so it attacks first and can't pick the hidden one.
    let a = board(&db, &["Flittering Bat", "Flittering Bat", "Flittering Bat"]);
    let b = board(&db, &["Heroic Underdog", "Flittering Bat"]);
    let mut underdog_attacked = 0;
    for seed in 0..300 {
        let (_, log) = stealth_fight(&a, &b, seed);
        let mut revealed = false;
        for &(att, att_stealth, def, def_stealth) in &log.attacks {
            if att == underdog {
                // attacking ends Stealth, so the attacker no longer has it
                assert!(!att_stealth);
                revealed = true;
                underdog_attacked += 1;
            }
            if def == underdog {
                assert!(revealed, "seed {seed}: hit a hidden minion");
                assert!(!def_stealth);
            }
        }
        // the first attack is by side 0 and has to hit the plain minion
        assert_eq!(log.attacks[0].2, plain);
    }
    assert!(underdog_attacked > 0);
}

/// The side of the first attack in each fight.
#[derive(Default)]
struct FirstSide(Option<usize>);

impl Watch for FirstSide {
    fn attack(&mut self, side: usize, _a: &Minion, _d: &Minion) {
        self.0.get_or_insert(side);
    }
}

/// Who attacks first is counted after Start of Combat (Power.log
/// 2026_11_11_02_43_24, game 1, turn 9: a 6 minion side summoned a Beetle at
/// Start of Combat, met 7 with 7 and won the coin flip). Here Stitched
/// Salvager destroys the minion to its left, so side 0 goes from 3 to 2 and
/// side 1, with 3, always attacks first.
#[test]
fn the_first_attacker_is_counted_after_start_of_combat() {
    let db = db();
    let a = board(&db, &["Flittering Bat", "Stitched Salvager", "Flittering Bat"]);
    let b = board(&db, &["Flittering Bat", "Flittering Bat", "Flittering Bat"]);
    for seed in 0..100 {
        let mut first = FirstSide::default();
        fight_watched(&db, &a, &b, 500, &mut Rng::new(seed), &mut first);
        assert_eq!(first.0, Some(1), "seed {seed}");
    }
}

#[test]
fn a_minion_that_attacked_can_be_hit_afterwards() {
    let db = db();
    let underdog = db.by_name("Heroic Underdog").unwrap();
    let a = board(&db, &["Flittering Bat", "Flittering Bat", "Flittering Bat"]);
    let mut b = board(&db, &["Heroic Underdog", "Flittering Bat"]);
    b[1].health = 500;
    let mut hit_after = false;
    for seed in 0..200 {
        let (_, log) = stealth_fight(&a, &b, seed);
        let first = log.attacks.iter().position(|x| x.0 == underdog);
        if let Some(i) = first {
            hit_after |= log.attacks[i..].iter().any(|x| x.2 == underdog);
        }
    }
    assert!(hit_after);
}

#[test]
fn a_side_facing_only_stealth_loses_its_turn() {
    // Stealth is never ignored: whoever would attack first, the side facing
    // only Stealth minions passes, so the Stealth side always attacks first.
    let db = db();
    let a = board(&db, &["Flittering Bat"]);
    let b = board(&db, &["Heroic Underdog"]);
    for seed in 0..50 {
        let (_, log) = stealth_fight(&a, &b, seed);
        assert_eq!(log.attacks[0].0, db.by_name("Heroic Underdog").unwrap());
    }
}

#[test]
fn two_boards_of_only_stealth_tie_without_attacking() {
    let db = db();
    let a = board(&db, &["Heroic Underdog"]);
    let b = board(&db, &["Heroic Underdog"]);
    for seed in 0..20 {
        let (out, log) = stealth_fight(&a, &b, seed);
        assert!(log.attacks.is_empty());
        assert_eq!((out.winner, out.attacks), (None, 0));
    }
}

#[test]
fn stealth_beats_taunt() {
    let db = db();
    let mut b = board(&db, &["Heroic Underdog", "Flittering Bat"]);
    b[0].keywords.set(Keywords::TAUNT);
    let a = board(&db, &["Flittering Bat", "Flittering Bat", "Flittering Bat"]);
    for seed in 0..100 {
        let (_, log) = stealth_fight(&a, &b, seed);
        assert_eq!(log.attacks[0].2, db.by_name("Flittering Bat").unwrap());
    }
}

/// "A friendly minion of each type" shares its buffs so that as many minions
/// as possible get one (Era 2026-11-03, docs/rules/design.md "Types"): a
/// Demon/Dragon and a Dragon both get a buff on every seed, the dual one as
/// the Demon. Two Demons share one buff at random.
#[test]
fn each_type_buffs_go_to_as_many_minions_as_possible() {
    use bgcore::cards::pick_one_per_type;
    use bgcore::{Rng, Tribe, Tribes};
    let dual = Tribes::of(Tribe::Demon).with(Tribe::Dragon);
    let dragon = Tribes::of(Tribe::Dragon);
    let demon = Tribes::of(Tribe::Demon);
    for seed in 0..500 {
        let mut rng = Rng::new(seed);
        let mut got = pick_one_per_type(&[dual, dragon], &mut rng).to_vec();
        got.sort();
        assert_eq!(got, vec![0, 1], "seed {seed}");
        // An Amalgam always gets one, and so does everyone else here.
        let mut got = pick_one_per_type(
            &[dragon, demon.with(Tribe::Quilboar), Tribes::of(Tribe::Quilboar),
              demon.with(Tribe::Elemental), Tribes::ALL, Tribes::NONE],
            &mut rng,
        )
        .to_vec();
        got.sort();
        assert_eq!(got, vec![0, 1, 2, 3, 4], "seed {seed}");
    }
    let mut first = [0u32; 2];
    for seed in 0..400 {
        let got = pick_one_per_type(&[demon, demon], &mut Rng::new(seed));
        assert_eq!(got.len(), 1);
        first[got[0]] += 1;
    }
    assert!(first[0] > 100 && first[1] > 100, "two Demons share at random: {first:?}");
}
