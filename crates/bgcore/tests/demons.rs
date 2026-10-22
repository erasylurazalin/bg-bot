//! The Demon minions and spells of the 5-tribe lobby, with the real cards
//! from cards.toml: consuming Tavern minions, Demon Fodder, hero Health as a
//! resource, the damage and spell counters, and the fight-side Hellcaller.

use std::sync::Arc;

use bgcore::fight::{Counters, fight_with};
use bgcore::{Action, CardDb, Data, Game, Keywords, Minion, Rng, Tribe};

fn db() -> CardDb {
    Data::bundled().cards
}

fn mk(db: &CardDb, name: &str, golden: bool) -> Minion {
    db.make(db.by_name(name).expect(name), golden)
}

/// A minion with `attack` that takes a long time to kill.
fn fighter(db: &CardDb, attack: i32) -> Minion {
    let mut w = mk(db, "Microbot", false);
    w.attack = attack;
    w.health = 2000;
    w
}

fn game(seed: u64) -> Game {
    Game::new(Arc::new(Data::bundled()), seed)
}

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
        if !g.data.cards.get(m.card).token {
            let slot = g.data.cards.get(m.card).pool_slot as usize;
            g.pool[slot] += m.pool_copies;
        }
    }
    for n in names {
        let c = id(g, n);
        let slot = g.data.cards.get(c).pool_slot as usize;
        g.pool[slot] -= 1;
        let m = g.players[p].make_card(&g.data.cards, c, false, 1);
        g.players[p].shop.push(m);
    }
}

/// Empty a player's shop, returning its pool copies.
fn clear_shop(g: &mut Game, p: usize) {
    set_shop(g, p, &[]);
}

/// Put a generated card (a token) in the Tavern; it takes no pool copy.
fn put_token_in_shop(g: &mut Game, p: usize, name: &str, golden: bool) {
    let c = id(g, name);
    let m = g.data.cards.make(c, golden);
    g.players[p].shop.push(m);
}

fn fodder_count(g: &Game, p: usize) -> usize {
    g.players[p]
        .shop
        .iter()
        .filter(|m| g.data.cards.get(m.card).is_fodder)
        .count()
}

fn stats(m: &Minion) -> (i32, i32) {
    (m.attack, m.health)
}

/// Every other player gets one huge minion, so player 0 loses the fight.
fn gang_up_on_player_0(g: &mut Game) {
    let microbot = id(g, "Microbot");
    for q in 1..8 {
        let mut m = g.data.cards.make(microbot, false);
        m.attack = 50;
        m.health = 50;
        g.players[q].board.push(m);
    }
}

fn hand_has(g: &Game, name: &str) -> bool {
    g.players[0]
        .hand
        .iter()
        .any(|m| g.data.cards.get(m.card).name == name)
}

// ---- Demon Fodder ----

#[test]
fn a_fodder_feeds_a_demon_and_its_slot_is_refilled() {
    let mut g = game(1);
    clear_shop(&mut g, 0);
    put_token_in_shop(&mut g, 0, "Demon Fodder", false);
    g.give(0, "Wrath Weaver").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    // Wrath Weaver's own "after you play a Demon" (+2/+2) then the Fodder's
    // +2/+2. The Fodder is gone and its slot drew a real card.
    assert_eq!(stats(&g.players[0].board[0]), (1 + 2 + 2, 3 + 2 + 2));
    assert_eq!(g.players[0].shop.len(), 1);
    assert!(!g.data.cards.get(g.players[0].shop[0].card).is_fodder);
    g.check_invariants().unwrap();
}

#[test]
fn a_fodder_stays_buyable_with_no_demon_on_board() {
    let mut g = game(2);
    clear_shop(&mut g, 0);
    put_token_in_shop(&mut g, 0, "Demon Fodder", false);
    g.players[0].gold = 3;
    assert!(is_legal(&mut g, 0, Action::Buy(0)));
    act(&mut g, 0, Action::Buy(0));
    assert!(g.players[0].shop.is_empty());
    assert_eq!(g.players[0].hand.len(), 1);
    let m = g.players[0].hand[0];
    assert!(g.data.cards.get(m.card).is_fodder);
    assert_eq!(stats(&m), (2, 2));
    // it never had a pool copy to return
    g.check_invariants().unwrap();
}

#[test]
fn a_fodder_from_a_refresh_stays_until_a_demon_turns_up() {
    let mut g = game(3);
    clear_shop(&mut g, 0);
    g.players[0].gold = 10;
    g.players[0].fodder_refreshes.clear();
    g.players[0].fodder_refreshes.push(1);
    act(&mut g, 0, Action::Roll);
    assert_eq!(fodder_count(&g, 0), 1, "no Demon on board, so it waits");
    assert!(g.players[0].fodder_refreshes.is_empty());
    // playing any Demon makes it feed at once
    g.give(0, "Devout Hellcaller").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    assert_eq!(fodder_count(&g, 0), 0);
    assert_eq!(stats(&g.players[0].board[0]), (4 + 2, 4 + 2));
    g.check_invariants().unwrap();
}

// ---- Refreshes getting Fodders ----

#[test]
fn laboratory_assistant_puts_a_fodder_in_each_of_the_next_three_refreshes() {
    let mut g = game(4);
    g.players[0].gold = 10;
    g.give(0, "Laboratory Assistant").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    assert_eq!(g.players[0].fodder_refreshes.as_slice(), [1, 1, 1]);
    // no Demon once it is sold, so Fodders stay in the Tavern
    act(&mut g, 0, Action::Sell(0));
    for want in [1, 1, 1, 0] {
        act(&mut g, 0, Action::Roll);
        assert_eq!(fodder_count(&g, 0), want);
    }
    g.check_invariants().unwrap();

    // a golden one adds two Fodders to each of the three
    let mut g = game(5);
    g.players[0].gold = 10;
    g.give(0, "Laboratory Assistant").unwrap();
    g.players[0].hand[0].golden = true;
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    assert_eq!(g.players[0].fodder_refreshes.as_slice(), [2, 2, 2]);
    // playing a golden minion also opens its own Discover: take it, then sell
    act(&mut g, 0, Action::Choose(0));
    act(&mut g, 0, Action::Sell(0));
    act(&mut g, 0, Action::Roll);
    assert_eq!(fodder_count(&g, 0), 2);
    g.check_invariants().unwrap();
}

#[test]
fn twisted_wrathguard_adds_to_the_very_next_refresh() {
    let mut g = game(6);
    g.players[0].gold = 10;
    put_on_board(&mut g, 0, "Twisted Wrathguard");
    put_on_board(&mut g, 0, "Buzzing Vermin");
    g.players[0].fodder_refreshes.clear();
    g.players[0].fodder_refreshes.extend([1, 1, 1]);
    act(&mut g, 0, Action::Sell(1));
    // the next Refresh (the first entry), not the end of the queue
    assert_eq!(g.players[0].fodder_refreshes.as_slice(), [2, 1, 1]);
    // the two Fodders feed the Wrathguard as soon as they appear
    act(&mut g, 0, Action::Roll);
    assert_eq!(g.players[0].fodder_refreshes.as_slice(), [1, 1]);
    assert_eq!(stats(&g.players[0].board[0]), (8 + 4, 8 + 4));
    g.check_invariants().unwrap();
}

// ---- Hero health ----

#[test]
fn malchezaars_health_refreshes_cannot_kill() {
    let mut g = game(7);
    g.players[0].gold = 0;
    g.players[0].health = 2;
    put_on_board(&mut g, 0, "Malchezaar, Prince of Dance");
    act(&mut g, 0, Action::Roll);
    assert_eq!(g.players[0].health, 1);
    // the second one would take the hero to 0
    assert!(!is_legal(&mut g, 0, Action::Roll));

    // with a rewind on the board it is payable at 1 Health, and the rewinder
    // still gains its +2 Health
    let mut g = game(8);
    g.players[0].gold = 0;
    g.players[0].health = 1;
    put_on_board(&mut g, 0, "Malchezaar, Prince of Dance");
    put_on_board(&mut g, 0, "Soul Rewinder");
    let before = g.players[0].board[1].health;
    assert!(is_legal(&mut g, 0, Action::Roll));
    act(&mut g, 0, Action::Roll);
    assert_eq!(g.players[0].health, 1);
    assert_eq!(g.players[0].board[1].health, before + 2);
    g.check_invariants().unwrap();
}

#[test]
fn a_later_rewind_gives_nothing_back_but_still_buffs_the_tavern() {
    let mut g = game(32);
    g.players[0].gold = 0;
    g.players[0].health = 1;
    put_on_board(&mut g, 0, "Malchezaar, Prince of Dance");
    put_on_board(&mut g, 0, "Soul Rewinder");
    put_on_board(&mut g, 0, "Ashen Corruptor");
    let soul = g.players[0].board[1].health;
    act(&mut g, 0, Action::Roll);
    // Soul Rewinder (left) gives the Health back; Ashen Corruptor has nothing
    // left to rewind but still gives the Tavern +2/+2 this turn
    assert_eq!(g.players[0].health, 1);
    assert_eq!(g.players[0].board[1].health, soul + 2);
    for m in &g.players[0].shop {
        let c = g.data.cards.get(m.card);
        assert_eq!((m.attack, m.health), (c.attack + 2, c.health + 2), "{}", c.name);
    }
    g.check_invariants().unwrap();
}

#[test]
fn wrath_weaver_damage_can_kill_the_hero() {
    let mut g = game(9);
    g.players[0].health = 1;
    put_on_board(&mut g, 0, "Wrath Weaver");
    g.give(0, "Devout Hellcaller").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 1 });
    assert_eq!(g.players[0].health, 0);
    assert_eq!(g.players[0].last_damage, 1);
    // the hero finishes the turn; the knockout comes after the fights
    assert!(g.players[0].alive);
    next_turn(&mut g);
    assert!(!g.players[0].alive);
    g.check_invariants().unwrap();
}

#[test]
fn tichondrius_and_eredar_react_to_health_paid_for_a_refresh() {
    let mut g = game(10);
    g.players[0].gold = 0;
    g.players[0].health = 30;
    put_on_board(&mut g, 0, "Malchezaar, Prince of Dance");
    put_on_board(&mut g, 0, "Tichondrius");
    put_on_board(&mut g, 0, "Eredar Escapist");
    // one damage short of the Corrupted Cupcakes
    g.players[0].board[2].improvements = 3;
    let before = stats(&g.players[0].board[1]);
    act(&mut g, 0, Action::Roll);
    assert_eq!(g.players[0].health, 29);
    // Tichondrius gives every Demon +4/+4, the Escapist included
    assert_eq!(stats(&g.players[0].board[1]), (before.0 + 4, before.1 + 4));
    assert_eq!(stats(&g.players[0].board[2]), (6 + 4, 8 + 4));
    assert!(hand_has(&g, "Corrupted Cupcakes"));
    assert_eq!(
        g.players[0].board[2].events_left(&g.data.cards),
        Some(4),
        "the counter starts over"
    );
    g.check_invariants().unwrap();
}

// ---- Sacrificial Wrathguard ----

#[test]
fn sacrificial_wrathguard_buffs_the_tavern_for_the_game() {
    let db = db();
    // two Activates: the Deathrattle gives 2/2 + 2 * 1/1
    let mut w = mk(&db, "Sacrificial Wrathguard", false);
    w.improvements = 2;
    let wall = fighter(&db, 50);
    let out = fight_with(
        &db,
        &[w],
        &[wall],
        [Counters::default(); 2],
        20,
        &mut Rng::new(1),
        &mut (),
    );
    assert_eq!(out.sides[0].tavern_buff_gain, (4, 4));

    // a golden one doubles the whole Deathrattle
    let mut w = mk(&db, "Sacrificial Wrathguard", true);
    w.improvements = 2;
    let out = fight_with(
        &db,
        &[w],
        &[fighter(&db, 50)],
        [Counters::default(); 2],
        20,
        &mut Rng::new(1),
        &mut (),
    );
    assert_eq!(out.sides[0].tavern_buff_gain, (8, 8));

    // in a whole game the Activate raises the number and the fight applies
    // the Tavern buff to the next Refresh
    let mut g = game(11);
    g.players[0].gold = 5;
    put_on_board(&mut g, 0, "Sacrificial Wrathguard");
    act(&mut g, 0, Action::Activate(0));
    act(&mut g, 0, Action::Activate(0));
    assert_eq!(g.players[0].board[0].improvements, 2);
    gang_up_on_player_0(&mut g);
    next_turn(&mut g);
    let buff = g
        .players[0]
        .tavern_buffs
        .iter()
        .find(|b| !b.this_turn && b.tribe.is_none())
        .copied()
        .expect("the Deathrattle's Tavern buff is kept");
    assert_eq!((buff.attack, buff.health), (4, 4));
    for m in &g.players[0].shop {
        let c = g.data.cards.get(m.card);
        assert_eq!((m.attack, m.health), (c.attack + 4, c.health + 4));
    }
    g.check_invariants().unwrap();
}

// ---- Demons consuming ----

#[test]
fn mind_muck_has_a_demon_consume_the_minion_you_choose() {
    let mut g = game(12);
    put_on_board(&mut g, 0, "Devout Hellcaller");
    set_shop(&mut g, 0, &["Buzzing Vermin", "Cord Puller"]);
    g.give(0, "Mind Muck").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 1 });
    // only the Hellcaller can be chosen (the source is not)
    assert!(g.players[0].pending_shop_target.is_some());
    act(&mut g, 0, Action::TargetShop(1));
    // Cord Puller is a 1/1
    assert_eq!(stats(&g.players[0].board[0]), (5, 5));
    assert_eq!(g.players[0].shop.len(), 1);
    g.check_invariants().unwrap();

    // a golden Mind Muck takes double its stats
    let mut g = game(13);
    put_on_board(&mut g, 0, "Devout Hellcaller");
    set_shop(&mut g, 0, &["Cord Puller"]);
    g.give(0, "Mind Muck").unwrap();
    g.players[0].hand[0].golden = true;
    act(&mut g, 0, Action::Play { hand: 0, pos: 1 });
    // with one Tavern card there is nothing to ask
    assert!(g.players[0].pending_shop_target.is_none());
    assert_eq!(stats(&g.players[0].board[0]), (6, 6));
    g.check_invariants().unwrap();
}

#[test]
fn insatiable_ur_zul_consumes_when_it_is_played_and_returns_the_copy() {
    let mut g = game(14);
    let slot = g.data.cards.get(id(&g, "Buzzing Vermin")).pool_slot as usize;
    set_shop(&mut g, 0, &["Buzzing Vermin"]);
    let before = g.pool[slot];
    g.give(0, "Insatiable Ur'zul").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    // 4/6 plus the 1/1 it ate
    assert_eq!(stats(&g.players[0].board[0]), (5, 7));
    assert!(g.players[0].shop.is_empty());
    assert_eq!(g.pool[slot], before + 1, "the consumed copy is back");
    g.check_invariants().unwrap();
}

#[test]
fn soulkeeping_jailer_has_every_demon_consume_one() {
    let mut g = game(15);
    g.players[0].gold = 2;
    put_on_board(&mut g, 0, "Soulkeeping Jailer");
    put_on_board(&mut g, 0, "Devout Hellcaller");
    let slot = g.data.cards.get(id(&g, "Buzzing Vermin")).pool_slot as usize;
    set_shop(&mut g, 0, &["Buzzing Vermin", "Buzzing Vermin"]);
    let before = g.pool[slot];
    act(&mut g, 0, Action::Activate(0));
    assert_eq!(stats(&g.players[0].board[0]), (3 + 1, 5 + 1));
    assert_eq!(stats(&g.players[0].board[1]), (4 + 1, 4 + 1));
    assert!(g.players[0].shop.is_empty());
    assert_eq!(g.pool[slot], before + 2);
    g.check_invariants().unwrap();

    // golden Jailer: each Demon takes double stats
    let mut g = game(16);
    g.players[0].gold = 2;
    put_on_board(&mut g, 0, "Soulkeeping Jailer");
    g.players[0].board[0].golden = true;
    let before = stats(&g.players[0].board[0]);
    set_shop(&mut g, 0, &["Buzzing Vermin"]);
    act(&mut g, 0, Action::Activate(0));
    assert_eq!(
        stats(&g.players[0].board[0]),
        (before.0 + 2, before.1 + 2)
    );
    g.check_invariants().unwrap();
}

#[test]
fn flaming_enforcer_eats_the_highest_health_minion_at_end_of_turn() {
    let mut g = game(17);
    put_on_board(&mut g, 0, "Flaming Enforcer");
    set_shop(&mut g, 0, &["Buzzing Vermin", "Lullabot"]);
    g.players[0].shop[1].health = 8;
    act(&mut g, 0, Action::EndTurn);
    // Lullabot is 2/8, Buzzing Vermin 1/1
    assert_eq!(stats(&g.players[0].board[0]), (4 + 2, 5 + 8));
    assert_eq!(g.players[0].shop.len(), 1);
    g.check_invariants().unwrap();
}

// ---- Tavern buffs made by Demons ----

#[test]
fn deft_deserter_gives_the_tavern_stats_and_one_shared_keyword() {
    let mask = Keywords::TAUNT | Keywords::DIVINE_SHIELD | Keywords::WINDFURY;
    for seed in 0..8 {
        let mut g = game(seed);
        g.players[0].gold = 1;
        put_on_board(&mut g, 0, "Deft Deserter");
        set_shop(&mut g, 0, &["Flittering Bat", "Bubble Gunner", "Lullabot"]);
        let before: Vec<_> = g.players[0].shop.iter().map(|m| (m.attack, m.health)).collect();
        act(&mut g, 0, Action::Activate(0));
        let mut chosen = 0u8;
        for (m, base) in g.players[0].shop.iter().zip(before) {
            assert_eq!((m.attack, m.health), (base.0 + 8, base.1 + 8));
            let k = m.keywords.0 & mask;
            assert_eq!(k.count_ones(), 1, "seed {seed}: exactly one new keyword");
            if chosen == 0 {
                chosen = k;
            }
            assert_eq!(k, chosen, "seed {seed}: the same keyword everywhere");
        }
        g.check_invariants().unwrap();
    }
}

#[test]
fn devilish_distractor_buffs_the_tavern_this_game() {
    let mut g = game(18);
    put_on_board(&mut g, 0, "Devilish Distractor");
    set_shop(&mut g, 0, &["Buzzing Vermin"]);
    g.give(0, "Fortify").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    act(&mut g, 0, Action::Target(0));
    // Fortify's own +0/+3, then the Distractor's +1/+2 for the game
    assert_eq!(stats(&g.players[0].board[0]), (4, 7 + 3));
    assert_eq!(stats(&g.players[0].shop[0]), (1 + 1, 1 + 2));
    // later arrivals get it too
    act(&mut g, 0, Action::Roll);
    for m in &g.players[0].shop {
        let c = g.data.cards.get(m.card);
        assert_eq!((m.attack, m.health), (c.attack + 1, c.health + 2), "{}", c.name);
    }
    g.check_invariants().unwrap();
}

// ---- Counters ----

#[test]
fn ominous_seer_discounts_the_next_tavern_spell() {
    let mut g = game(19);
    g.players[0].tier = 3;
    g.give(0, "Ominous Seer").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    g.players[0].shop_spell = Some(id(&g, "Shiny Ring"));
    assert_eq!(g.players[0].spell_cost(&g.data.cards), Some(1));
    g.players[0].gold = 3;
    act(&mut g, 0, Action::BuySpell);
    assert_eq!(g.players[0].gold, 2);
    assert_eq!(g.players[0].spell_discount, 0, "the discount is used up");

    // golden: (2) less
    let mut g = game(20);
    g.players[0].tier = 3;
    g.give(0, "Ominous Seer").unwrap();
    g.players[0].hand[0].golden = true;
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    g.players[0].shop_spell = Some(id(&g, "Shiny Ring"));
    assert_eq!(g.players[0].spell_cost(&g.data.cards), Some(0));
}

#[test]
fn felboar_consumes_after_three_spells_and_starts_over() {
    let mut g = game(21);
    put_on_board(&mut g, 0, "Felboar");
    set_shop(&mut g, 0, &["Buzzing Vermin"]);
    assert_eq!(g.players[0].board[0].events_left(&g.data.cards), Some(3));
    for _ in 0..2 {
        g.give(0, "Tavern Coin").unwrap();
        act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    }
    assert_eq!(stats(&g.players[0].board[0]), (2, 6));
    assert_eq!(g.players[0].board[0].events_left(&g.data.cards), Some(1));
    g.give(0, "Tavern Coin").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    assert_eq!(stats(&g.players[0].board[0]), (3, 7));
    assert!(g.players[0].shop.is_empty());
    assert_eq!(g.players[0].board[0].events_left(&g.data.cards), Some(3));
    g.check_invariants().unwrap();
}

#[test]
fn trapper_clapper_queues_fodders_after_a_fight() {
    let db = db();
    let out = fight_with(
        &db,
        &[mk(&db, "Trapped Clapper", false)],
        &[fighter(&db, 50)],
        [Counters::default(); 2],
        20,
        &mut Rng::new(1),
        &mut (),
    );
    assert_eq!(out.sides[0].n_fodder_refreshes, 3);
    assert_eq!(out.sides[0].fodder_refreshes[..3], [1, 1, 1]);

    let out = fight_with(
        &db,
        &[mk(&db, "Trapped Clapper", true)],
        &[fighter(&db, 50)],
        [Counters::default(); 2],
        20,
        &mut Rng::new(1),
        &mut (),
    );
    assert_eq!(out.sides[0].fodder_refreshes[..3], [2, 2, 2]);

    // in a game the queued Fodders arrive with the next Refresh, and the
    // Clapper (still the only Demon) eats the first one
    let mut g = game(22);
    put_on_board(&mut g, 0, "Trapped Clapper");
    gang_up_on_player_0(&mut g);
    next_turn(&mut g);
    assert_eq!(g.players[0].board[0].attack, 2 + 2);
    assert_eq!(g.players[0].board[0].health, 2 + 2);
    assert_eq!(g.players[0].fodder_refreshes.as_slice(), [1, 1]);
    g.check_invariants().unwrap();
}

#[test]
fn imposing_percussionist_damages_the_hero_for_the_demons_tier() {
    let mut g = game(23);
    g.players[0].tier = 3;
    g.players[0].health = 30;
    g.give(0, "Imposing Percussionist").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    assert_eq!(g.players[0].discover.len(), 3);
    assert!(g.players[0].discover.iter().all(|&c| {
        let card = g.data.cards.get(c);
        card.tribes.has(Tribe::Demon) && card.tier <= 3
    }));
    let tier = g.data.cards.get(g.players[0].discover[0]).tier as i32;
    act(&mut g, 0, Action::Choose(0));
    assert_eq!(g.players[0].health, 30 - tier);
    assert_eq!(g.players[0].last_damage, tier);
    g.check_invariants().unwrap();

    // golden: two Discovers, damage for both
    let mut g = game(24);
    g.players[0].tier = 3;
    g.players[0].health = 30;
    g.give(0, "Imposing Percussionist").unwrap();
    g.players[0].hand[0].golden = true;
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    // the second Demon Discover waits, after the one a golden play opens
    assert_eq!(g.players[0].discover_queue.len(), 2);
    assert!(g.players[0]
        .discover
        .iter()
        .all(|&c| g.data.cards.get(c).tribes.has(Tribe::Demon)));
    let t1 = g.data.cards.get(g.players[0].discover[0]).tier as i32;
    act(&mut g, 0, Action::Choose(0));
    assert_eq!(g.players[0].discover.len(), 3, "the second Discover follows");
    assert!(g.players[0]
        .discover
        .iter()
        .all(|&c| g.data.cards.get(c).tribes.has(Tribe::Demon)));
    let t2 = g.data.cards.get(g.players[0].discover[0]).tier as i32;
    act(&mut g, 0, Action::Choose(0));
    assert_eq!(g.players[0].health, 30 - t1 - t2);
    g.check_invariants().unwrap();
}

#[test]
fn eredar_escapist_is_damage_counted_and_carries_over() {
    let mut g = game(25);
    put_on_board(&mut g, 0, "Eredar Escapist");
    g.players[0].board[0].improvements = 3;
    g.give(0, "Wrath Weaver").unwrap();
    g.players[0].health = 30;
    act(&mut g, 0, Action::Play { hand: 0, pos: 1 });
    // the demon play is 1 damage: 3 + 1 = 4, so the Cupcakes arrive
    assert!(hand_has(&g, "Corrupted Cupcakes"));
    assert_eq!(g.players[0].board[0].events_left(&g.data.cards), Some(4));
    g.players[0].board[0].improvements = 2;
    // the counter lives on the minion, so the end of the turn leaves it be
    act(&mut g, 0, Action::EndTurn);
    assert_eq!(g.players[0].board[0].improvements, 2);
    g.check_invariants().unwrap();
}

// ---- Demon spells ----

#[test]
fn imp_lusionist_grants_methodical_madness() {
    let db = db();
    let out = fight_with(
        &db,
        &[mk(&db, "Imp-lusionist", false)],
        &[fighter(&db, 50)],
        [Counters::default(); 2],
        20,
        &mut Rng::new(1),
        &mut (),
    );
    let gained = out.sides[0].cards_gained();
    assert_eq!(gained.len(), 1);
    assert_eq!(db.get(gained[0]).name, "Methodical Madness");
    // golden gets two
    let out = fight_with(
        &db,
        &[mk(&db, "Imp-lusionist", true)],
        &[fighter(&db, 50)],
        [Counters::default(); 2],
        20,
        &mut Rng::new(1),
        &mut (),
    );
    assert_eq!(out.sides[0].cards_gained().len(), 2);

    // the spell consumes two Tavern minions, stats and Bonus Keywords
    let mut g = game(26);
    put_on_board(&mut g, 0, "Devout Hellcaller");
    set_shop(&mut g, 0, &["Buzzing Vermin", "Cord Puller"]);
    g.give(0, "Methodical Madness").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    act(&mut g, 0, Action::Target(0));
    // both 1/1s, Taunt and Divine Shield
    assert_eq!(stats(&g.players[0].board[0]), (6, 6));
    assert!(g.players[0].board[0].keywords.has(Keywords::TAUNT));
    assert!(g.players[0].board[0].keywords.has(Keywords::DIVINE_SHIELD));
    assert!(g.players[0].shop.is_empty());
    g.check_invariants().unwrap();
}

#[test]
fn a_taught_methodical_madness_with_no_demon_fizzles() {
    // Magicfin Mycologist teaches the bought spell to a token; casting it
    // with no friendly Demon must not leave a choice that can never be made.
    let mut g = game(28);
    g.players[0].tier = 4;
    g.players[0].gold = 10;
    put_on_board(&mut g, 0, "Magicfin Mycologist");
    g.players[0].shop_spell = Some(id(&g, "Methodical Madness"));
    act(&mut g, 0, Action::BuySpell);
    let token = g.players[0]
        .hand
        .iter()
        .position(|m| g.data.cards.get(m.card).taught_token)
        .unwrap();
    act(&mut g, 0, Action::Play { hand: token as u8, pos: 1 });
    assert!(g.players[0].pending_target.is_none());
    act(&mut g, 0, Action::EndTurn);
    g.check_invariants().unwrap();
}

#[test]
fn corrupted_cupcakes_consumes_three_for_stats_only() {
    let mut g = game(27);
    put_on_board(&mut g, 0, "Devout Hellcaller");
    set_shop(&mut g, 0, &["Buzzing Vermin", "Cord Puller", "Lullabot"]);
    g.give(0, "Corrupted Cupcakes").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    act(&mut g, 0, Action::Target(0));
    // 1/1 + 1/1 + 2/2
    assert_eq!(stats(&g.players[0].board[0]), (4 + 4, 4 + 4));
    assert!(!g.players[0].board[0].keywords.has(Keywords::TAUNT));
    assert!(g.players[0].shop.is_empty());
    g.check_invariants().unwrap();
}

// ---- Devout Hellcaller in combat ----

#[test]
fn devout_hellcaller_grows_when_another_demon_deals_damage() {
    let db = db();
    let wall = fighter(&db, 0);
    // Wrath Weaver attacks first and deals 1: the Hellcaller gains +2/+2
    let a = [mk(&db, "Wrath Weaver", false), mk(&db, "Devout Hellcaller", false)];
    let out = fight_with(
        &db,
        &a,
        &[wall],
        [Counters::default(); 2],
        1,
        &mut Rng::new(1),
        &mut (),
    );
    assert_eq!(out.sides[0].permanent_stats[1], (2, 2));
    // the golden Hellcaller gains +4/+4
    let a = [mk(&db, "Wrath Weaver", false), mk(&db, "Devout Hellcaller", true)];
    let out = fight_with(
        &db,
        &a,
        &[wall],
        [Counters::default(); 2],
        1,
        &mut Rng::new(1),
        &mut (),
    );
    assert_eq!(out.sides[0].permanent_stats[1], (4, 4));
    // its own damage is not "another friendly Demon"
    let a = [mk(&db, "Devout Hellcaller", false), mk(&db, "Wrath Weaver", false)];
    let out = fight_with(
        &db,
        &a,
        &[wall],
        [Counters::default(); 2],
        1,
        &mut Rng::new(1),
        &mut (),
    );
    assert!(out.sides[0].permanent_stats.iter().all(|&s| s == (0, 0)));
}

// ---- The run 3 fix ----

#[test]
fn random_murlocs_and_discovers_stay_within_the_players_tier() {
    // Primalfin Lookout at tier 1: only the two tier 1 Murlocs exist
    let mut g = game(28);
    g.players[0].tier = 1;
    put_on_board(&mut g, 0, "Tad");
    g.give(0, "Primalfin Lookout").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 1 });
    assert_eq!(g.players[0].discover.len(), 2);
    assert!(g.players[0].discover.iter().all(|&c| g.data.cards.get(c).tier == 1));
    act(&mut g, 0, Action::Choose(0));

    // Tad: a random Murloc on sell, never above tier 2
    for seed in 0..20 {
        let mut g = game(seed);
        g.players[0].tier = 2;
        put_on_board(&mut g, 0, "Tad");
        act(&mut g, 0, Action::Sell(0));
        let card = g.data.cards.get(g.players[0].hand[0].card);
        assert!(card.tribes.has(Tribe::Murloc));
        assert!(card.tier <= 2, "seed {seed}: {} is tier {}", card.name, card.tier);
    }

    // Cloning Conch: a random Murloc and its copy, up to tier 2
    for seed in 30..50 {
        let mut g = game(seed);
        g.players[0].tier = 2;
        g.give(0, "Cloning Conch").unwrap();
        act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
        assert_eq!(g.players[0].hand[0].card, g.players[0].hand[1].card);
        let card = g.data.cards.get(g.players[0].hand[0].card);
        assert!(card.tribes.has(Tribe::Murloc));
        assert!(card.tier <= 2, "seed {seed}: {} is tier {}", card.name, card.tier);
    }
}
