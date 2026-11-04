//! Hero batch 2 (docs/briefs/heroes-batch2.md): the 22 stats, target and
//! Tavern powers, one test each, plus the Spellcraft target fix the brief
//! asks for. Era's rules for their non-obvious parts are in docs/rules/heroes.md
//! "Hero batch rules (Era, 2026-11-01)".

use std::sync::Arc;

use bgcore::{Action, CardId, Data, Game, Minion, Opponent, Tribe, Tribes};

fn data() -> Arc<Data> {
    Arc::new(Data::bundled())
}

fn lobby(ts: &[Tribe]) -> Tribes {
    ts.iter().copied().fold(Tribes::NONE, |m, t| m.with(t))
}

/// A no-pick game with every tribe in the lobby, player 0 forced to `hero`.
/// Every other player has no hero, so only player 0's power runs.
fn hero_game(seed: u64, hero: &str) -> Game {
    let d = data();
    let mut g = Game::new_with_lobby(d.clone(), seed, lobby(&Tribe::EVERY));
    let h = d.heroes.by_name(hero).expect(hero);
    g.assign_hero(0, h);
    g
}

fn id(g: &Game, name: &str) -> CardId {
    g.data.cards.by_name(name).expect(name)
}

fn power(g: &Game, p: usize) -> &bgcore::HeroPower {
    g.data.heroes.power(g.players[p].power.expect("no power").of)
}

/// Answer whatever the current player has open (a Discover, a power offer, a
/// target, a discard, a trinket offer), so a helper that only wants the next
/// turn never stalls on a prompt. False when nothing is open.
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

fn choose(g: &mut Game, p: usize, k: u8) {
    turn_of(g, p);
    g.step(Action::Choose(k)).unwrap_or_else(|_| panic!("Choose({k}) illegal for player {p}"));
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

/// Take a card into the player's hand, taking a pool copy when the card has
/// one (a spell or a token has none), pool kept exact.
fn put_in_hand(g: &mut Game, p: usize, name: &str) -> Minion {
    let c = id(g, name);
    let card = g.data.cards.get(c);
    let copies = u8::from(!card.is_spell() && !card.token);
    if copies > 0 {
        g.pool[card.pool_slot as usize] -= 1;
    }
    let m = g.players[p].make_card(&g.data.cards, c, false, copies);
    g.players[p].hand.push(m);
    m
}

fn legal_for_0(g: &mut Game, a: Action) -> bool {
    turn_of(g, 0);
    g.is_legal(a)
}

/// Give the power its uses back, so a test can press it more than once in a
/// turn without walking whole rounds.
fn again(g: &mut Game, p: usize) {
    if let Some(st) = g.players[p].power.as_mut() {
        st.uses_left = g.data.heroes.power(st.of).uses_per_turn;
    }
}

/// Press the power and answer a target it opened, if any (with one legal
/// target the press applies it by itself).
fn press(g: &mut Game, p: usize) {
    act(g, p, Action::HeroPower);
    while g.players[p].pending_target.is_some() {
        turn_of(g, p);
        if !answer(g) {
            break;
        }
    }
}

fn put_on_board(g: &mut Game, p: usize, name: &str) {
    let m = put_in_hand(g, p, name);
    g.players[p].hand.pop();
    g.players[p].board.push(m);
}

/// Every board minion off, pool copies back (a plain `clear()` would break
/// the exact pool accounting the tests check).
fn clear_board(g: &mut Game, p: usize) {
    for m in g.players[p].board.drain(..) {
        let c = g.data.cards.get(m.card);
        if m.pool_copies > 0 && !c.token && !c.is_spell() {
            g.pool[c.pool_slot as usize] += m.pool_copies;
        }
    }
}

/// Set the player's shop to exactly these minions, pool kept exact.
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
    g.players[p].shop_spell = None;
}

/// Leave only players 0 and 1 alive and paired with each other, so the next
/// fight is theirs whatever the round's pairing would have been.
fn duel(g: &mut Game) {
    for p in 2..8 {
        g.players[p].alive = false;
        g.players[p].done = true;
        g.players[p].placement = 3 + (p as u8 - 2);
    }
    g.players[1].health = 30;
    g.players[1].armor = 0;
    g.players[0].opponent = Some(Opponent::Player(1));
    g.players[1].opponent = Some(Opponent::Player(0));
}

fn tier(g: &Game, name: &str) -> u8 {
    g.data.cards.get(id(g, name)).tier
}

// ---- the Tavern and the board ----

#[test]
fn galakronds_greed_replaces_a_tavern_minion_with_a_discover_one_tier_higher() {
    let mut g = hero_game(1, "Galakrond");
    assert_eq!(power(&g, 0).cost, 1);
    set_shop(&mut g, 0, &["Buzzing Vermin", "Southsea Busker"]);
    g.players[0].gold = 5;
    act(&mut g, 0, Action::HeroPower);
    act(&mut g, 0, Action::TargetShop(0));
    assert_eq!(g.players[0].discover.len(), 3, "a Discover opened");
    for &c in &g.players[0].discover {
        assert_eq!(g.data.cards.get(c).tier, 2, "exactly one Tier higher");
    }
    let picked = g.players[0].discover[0];
    choose(&mut g, 0, 0);
    assert_eq!(g.players[0].shop.len(), 2, "the Discover took the old slot");
    assert_eq!(g.players[0].shop[0].card, picked);
    assert_eq!(g.players[0].shop[0].pool_copies, 1, "a pool copy, as drawn");
    g.check_invariants().unwrap();
    // Nothing above Tier 6: the press is still one the game allows and it
    // does nothing, having spent the Gold (Hearthstone_2026_11_01_04_27_54-g2
    // turn 8: a Tavern of Tier 6 minions and a press at 1 Gold).
    set_shop(&mut g, 0, &["Crimson Vindicator"]);
    g.players[0].tier = 6;
    g.players[0].gold = 5;
    g.players[0].pending_target = None;
    again(&mut g, 0);
    assert!(legal_for_0(&mut g, Action::HeroPower), "the press is allowed");
    act(&mut g, 0, Action::HeroPower);
    assert!(g.players[0].pending_target.is_none(), "no minion has a Tier above it");
    assert!(g.players[0].discover.is_empty());
    assert_eq!(g.players[0].gold, 4, "the press is spent");
    g.check_invariants().unwrap();
}

#[test]
fn galakronds_greed_pick_lands_with_the_tavens_standing_buffs() {
    let mut g = hero_game(19, "Galakrond");
    set_shop(&mut g, 0, &["Buzzing Vermin", "Southsea Busker"]);
    g.players[0].gold = 5;
    // A standing "this game" Tavern buff, as a Tavern spell gives it.
    let b = bgcore::game::TavernBuff {
        tribe: None,
        attack: 40,
        health: 40,
        this_turn: false,
        typed_only: false,
    };
    g.players[0].add_tavern_buff(&g.data.cards, b);
    act(&mut g, 0, Action::HeroPower);
    act(&mut g, 0, Action::TargetShop(0));
    let picked = g.players[0].discover[0];
    let base = g.data.cards.make(picked, false);
    choose(&mut g, 0, 0);
    let m = g.players[0].shop[0];
    assert_eq!(m.card, picked, "the pick took the old slot");
    assert_eq!(
        (m.attack, m.health),
        (base.attack + 40, base.health + 40),
        "the pick sits in the Tavern, so the standing buffs land on it"
    );
    g.check_invariants().unwrap();
}

#[test]
fn reborn_rites_gives_reborn_until_the_next_turn() {
    let mut g = hero_game(2, "The Lich King");
    assert_eq!(power(&g, 0).cost, 0);
    put_on_board(&mut g, 0, "Buzzing Vermin");
    g.players[0].board[0].health = 40;
    press(&mut g, 0);
    assert!(g.players[0].board[0].keywords.has(bgcore::Keywords::REBORN));
    // The Tavern's minions are minions too.
    set_shop(&mut g, 0, &["Buzzing Vermin"]);
    again(&mut g, 0);
    act(&mut g, 0, Action::HeroPower);
    act(&mut g, 0, Action::TargetShop(0));
    assert!(g.players[0].shop[0].keywords.has(bgcore::Keywords::REBORN));
    // It lasts until the next turn: the minion keeps it through the fight.
    duel(&mut g);
    end_all_turns(&mut g);
    let m = g.players[0].board[0];
    assert!(!m.keywords.has(bgcore::Keywords::REBORN), "Reborn is off next turn");
    g.check_invariants().unwrap();
}

#[test]
fn stay_frosty_makes_minions_cost_two_and_shrinks_and_freezes_the_tavern() {
    let mut g = hero_game(3, "Sindragosa");
    assert!(power(&g, 0).passive);
    duel(&mut g);
    let base = g.data.rules.shop_size(g.players[0].tier);
    assert_eq!(g.players[0].shop_size(&g.data.rules), base - 1, "one fewer minion");
    g.players[0].gold = 5;
    act(&mut g, 0, Action::Roll);
    assert_eq!(g.players[0].shop.len(), base - 1, "one fewer minion");
    // Minions cost 2; a Refresh keeps its own price.
    set_shop(&mut g, 0, &["Buzzing Vermin"]);
    g.players[0].gold = 3;
    // Buying it at 2 Gold works and leaves 1.
    act(&mut g, 0, Action::Buy(0));
    assert_eq!(g.players[0].gold, 1, "a minion costs 2");
    assert_eq!(g.refresh_cost(0), g.data.rules.roll_cost);
    act(&mut g, 0, Action::Roll);
    assert!(!g.players[0].frozen, "a Refresh thaws it");
    act(&mut g, 0, Action::EndTurn);
    assert!(g.players[0].frozen, "it Freezes at the end of the turn");
    g.check_invariants().unwrap();
}

#[test]
fn temporal_tavern_refreshes_with_two_minions_from_a_higher_tier() {
    let mut g = hero_game(4, "Infinite Toki");
    assert_eq!(power(&g, 0).cost, 1);
    duel(&mut g);
    let base = g.data.rules.shop_size(g.players[0].tier);
    g.players[0].gold = 5;
    press(&mut g, 0);
    assert_eq!(g.players[0].shop.len(), base, "normal slots, not extra ones");
    let high = g.players[0]
        .shop
        .iter()
        .filter(|m| g.data.cards.get(m.card).tier > g.players[0].tier)
        .count();
    assert_eq!(high, 2, "two of them are from a Tier above");
    for m in &g.players[0].shop {
        assert!(g.data.cards.get(m.card).tier <= bgcore::MAX_TIER);
    }
    g.check_invariants().unwrap();
}

#[test]
fn saturday_cthuns_repeats_one_more_time_each_turn() {
    let mut g = hero_game(5, "C'Thun");
    assert_eq!(power(&g, 0).cost, 1);
    put_on_board(&mut g, 0, "Buzzing Vermin");
    g.players[0].board[0].attack = 30;
    g.players[0].board[0].health = 60;
    duel(&mut g);
    // Turn 1: one repeat.
    g.players[0].gold = 5;
    press(&mut g, 0);
    let (a0, h0) = (g.players[0].board[0].attack, g.players[0].board[0].health);
    act(&mut g, 0, Action::EndTurn);
    let m = g.players[0].board[0];
    assert_eq!((m.attack - a0, m.health - h0), (1, 1), "one repeat on turn 1");
    // Turn 2: two repeats.
    turn_of(&mut g, 0);
    g.players[0].gold = 5;
    let (a1, h1) = (g.players[0].board[0].attack, g.players[0].board[0].health);
    press(&mut g, 0);
    act(&mut g, 0, Action::EndTurn);
    let m = g.players[0].board[0];
    assert_eq!((m.attack - a1, m.health - h1), (2, 2), "two repeats on turn 2");
    g.check_invariants().unwrap();
}

#[test]
fn hat_trick_puts_a_hat_on_every_minion_played_and_moves_it_on_a_sale() {
    let mut g = hero_game(6, "Dancin' Deryl");
    assert!(power(&g, 0).passive);
    put_in_hand(&mut g, 0, "Buzzing Vermin");
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    let m = g.players[0].board[0];
    assert_eq!(m.hats, 1, "the played minion wears a hat");
    assert_eq!((m.attack, m.health), (2, 2), "1/1 plus the hat");
    // A second minion, then the first is sold: its hat moves over.
    put_in_hand(&mut g, 0, "Buzzing Vermin");
    act(&mut g, 0, Action::Play { hand: 0, pos: 1 });
    let m2 = g.players[0].board[1];
    assert_eq!(m2.hats, 1);
    let before = (g.players[0].board[1].attack, g.players[0].board[1].health);
    let gold = g.players[0].gold;
    act(&mut g, 0, Action::Sell(0));
    assert_eq!(g.players[0].gold, gold + 1);
    let m2 = g.players[0].board[0];
    assert_eq!(m2.hats, 2, "two hats now");
    assert_eq!((m2.attack, m2.health), (before.0 + 1, before.1 + 1));
    // Hats stack and survive a triple: three copies, two of them hatted.
    clear_board(&mut g, 0);
    let mut a = put_in_hand(&mut g, 0, "Buzzing Vermin");
    a.hats = 1;
    a.attack += 1;
    let mut b = put_in_hand(&mut g, 0, "Buzzing Vermin");
    b.hats = 1;
    b.attack += 1;
    g.players[0].hand[0] = a;
    g.players[0].hand[1] = b;
    put_in_hand(&mut g, 0, "Buzzing Vermin");
    set_shop(&mut g, 0, &["Buzzing Vermin"]);
    g.players[0].gold = 5;
    act(&mut g, 0, Action::Buy(0));
    let golden = g.players[0].hand.iter().find(|m| m.golden).expect("a golden");
    assert_eq!(golden.hats, 2, "the golden keeps both hats");
    g.check_invariants().unwrap();
}

#[test]
fn bobs_burgles_refreshes_with_plain_copies_of_the_last_warband() {
    let mut g = hero_game(7, "Tess Greymane");
    assert_eq!(power(&g, 0).cost, 1);
    // The warband last fought: a buffed minion, as seen.
    let c = id(&g, "Buzzing Vermin");
    let mut m = g.players[0].make_card(&g.data.cards, c, false, 0);
    m.attack += 5;
    g.players[0].seen[1].board.clear();
    g.players[0].seen[1].board.push(m);
    g.players[0].last_opponent = Some(Opponent::Player(1));
    g.players[0].gold = 5;
    press(&mut g, 0);
    assert!(!g.players[0].shop.is_empty());
    for s in &g.players[0].shop {
        assert_eq!(s.card, c, "the same minion");
        assert_eq!((s.attack, s.health), (m.attack - 5, m.health), "a plain copy");
        assert_eq!(s.pool_copies, 0, "made fresh, no pool copy");
    }
    g.check_invariants().unwrap();
}

#[test]
fn bobs_burgles_holds_the_whole_warband_and_no_spell_slot_when_it_is_seven() {
    // Seven copies fill the Tavern's seven cards, so the spell slot has no
    // room (2026_11_02_00_10_22-g7 turn 8 step 40 and turn 12 step 4: a
    // warband of seven, seven Tavern minions and no spell, at Tier 4 and at
    // Tier 5). A smaller warband sits next to the spell.
    let mut g = hero_game(11, "Tess Greymane");
    let c = id(&g, "Buzzing Vermin");
    g.players[0].seen[1].board.clear();
    for _ in 0..bgcore::BOARD {
        g.players[0].seen[1].board.push(g.players[0].make_card(&g.data.cards, c, false, 0));
    }
    g.players[0].last_opponent = Some(Opponent::Player(1));
    g.players[0].tier = 4;
    g.players[0].gold = 5;
    press(&mut g, 0);
    assert_eq!(g.players[0].shop.len(), 7, "one copy per warband minion");
    assert!(g.players[0].shop.iter().all(|m| m.card == c));
    assert!(g.players[0].shop_spell.is_none(), "no room left for the spell");
    g.check_invariants().unwrap();

    // Three copies at Tier 4: three cards and the spell slot.
    let mut g = hero_game(12, "Tess Greymane");
    g.players[0].seen[1].board.clear();
    for _ in 0..3 {
        g.players[0].seen[1].board.push(g.players[0].make_card(&g.data.cards, c, false, 0));
    }
    g.players[0].last_opponent = Some(Opponent::Player(1));
    g.players[0].tier = 4;
    g.players[0].gold = 5;
    press(&mut g, 0);
    assert_eq!(g.players[0].shop.len(), 3, "one copy each, no padding");
    assert!(g.players[0].shop_spell.is_some(), "the spell slot is dealt as usual");
    g.check_invariants().unwrap();
}

#[test]
fn dream_portal_offers_one_more_minion_and_it_is_a_dragon() {
    let mut g = hero_game(8, "Ysera");
    assert!(power(&g, 0).passive);
    assert!(
        g.data.heroes.get(g.players[0].hero.unwrap()).tribes.has(Tribe::Dragon),
        "Ysera is offered only with a Dragon in the lobby"
    );
    let base = g.data.rules.shop_size(g.players[0].tier);
    duel(&mut g);
    g.players[0].gold = 5;
    act(&mut g, 0, Action::Roll);
    assert_eq!(g.players[0].shop.len(), base + 1, "one more minion");
    assert!(
        g.players[0]
            .shop
            .iter()
            .any(|m| g.data.cards.tribes_of(m).has(Tribe::Dragon)),
        "the extra minion is a Dragon"
    );
    g.check_invariants().unwrap();
}

#[test]
fn arcane_alteration_replaces_a_card_with_one_of_the_same_tier() {
    let mut g = hero_game(9, "Malygos");
    assert_eq!(power(&g, 0).cost, 0);
    assert_eq!(power(&g, 0).uses_per_turn, 2);
    set_shop(&mut g, 0, &["Buzzing Vermin", "Southsea Busker"]);
    g.players[0].shop[0].attack += 3;
    g.players[0].shop[0].health += 3;
    act(&mut g, 0, Action::HeroPower);
    act(&mut g, 0, Action::TargetShop(0));
    assert_eq!(g.players[0].shop.len(), 2);
    let m = g.players[0].shop[0];
    assert_eq!(tier(&g, "Buzzing Vermin"), g.data.cards.get(m.card).tier, "the same Tier");
    assert!(m.attack >= 4, "the old card's buffs carry: {} {}", m.attack, m.health);
    assert_eq!(m.pool_copies, 1, "the new card is a pool copy");
    g.check_invariants().unwrap();
    // Twice a turn.
    act(&mut g, 0, Action::HeroPower);
    act(&mut g, 0, Action::TargetShop(0));
    assert!(!legal_for_0(&mut g, Action::HeroPower), "only twice a turn");
}

#[test]
fn imprison_locks_a_tavern_card_in_hand_for_two_turns() {
    let mut g = hero_game(10, "Maiev Shadowsong");
    assert_eq!(power(&g, 0).cost, 1);
    set_shop(&mut g, 0, &["Buzzing Vermin"]);
    g.players[0].gold = 5;
    press(&mut g, 0);
    assert_eq!(g.players[0].shop.len(), 0, "the card left the Tavern");
    assert_eq!(g.players[0].hand.len(), 1);
    let h = g.players[0].hand[0];
    assert_eq!(h.locked_turns, 2);
    assert_eq!(h.pool_copies, 1, "it left the pool with the card");
    assert!(!legal_for_0(&mut g, Action::Play { hand: 0, pos: 0 }), "locked");
    duel(&mut g);
    g.players[0].health = 100;
    end_all_turns(&mut g);
    turn_of(&mut g, 0);
    assert_eq!(g.players[0].hand[0].locked_turns, 1, "one turn left");
    assert!(!legal_for_0(&mut g, Action::Play { hand: 0, pos: 0 }));
    end_all_turns(&mut g);
    turn_of(&mut g, 0);
    assert_eq!(g.players[0].hand[0].locked_turns, 0, "unlocked");
    assert!(legal_for_0(&mut g, Action::Play { hand: 0, pos: 0 }), "playable now");
    g.check_invariants().unwrap();
}

#[test]
fn tavern_lighting_improves_the_spell_bonus_every_three_turns() {
    let mut g = hero_game(11, "Rakanishu");
    assert!(power(&g, 0).passive);
    duel(&mut g);
    assert_eq!(g.players[0].spell_bonus, (1, 1), "an extra +1/+1 from the start");
    while g.turn < 3 {
        end_all_turns(&mut g);
    }
    turn_of(&mut g, 0);
    assert_eq!(g.players[0].spell_bonus, (2, 2), "one more at the start of turn 3");
    g.check_invariants().unwrap();
}

#[test]
fn come_one_come_all_takes_three_darkmoon_tickets_for_a_discover() {
    let mut g = hero_game(12, "Silas Darkmoon");
    assert!(power(&g, 0).passive);
    // Tickets land on Tavern cards; a certain chance here, so the test is
    // about the count and not the roll.
    g.players[0].ticket_chance = 100;
    set_shop(&mut g, 0, &["Buzzing Vermin", "Buzzing Vermin", "Buzzing Vermin"]);
    g.players[0].shop_spell = Some(id(&g, "Tavern Coin"));
    g.players[0].shop_spell_ticket = 0;
    g.players[0].tier = 2;
    g.players[0].gold = 20;
    act(&mut g, 0, Action::Roll);
    set_shop(&mut g, 0, &["Buzzing Vermin", "Buzzing Vermin", "Buzzing Vermin"]);
    g.players[0].shop_spell = Some(id(&g, "Tavern Coin"));
    g.players[0].shop_spell_ticket = 0;
    act(&mut g, 0, Action::Roll);
    // The Refresh marks the new cards.
    assert!(g.players[0].shop.iter().all(|m| m.tickets == 1));
    act(&mut g, 0, Action::Buy(0));
    assert_eq!(g.players[0].tickets, 1);
    act(&mut g, 0, Action::Buy(0));
    assert_eq!(g.players[0].tickets, 2);
    act(&mut g, 0, Action::Buy(0));
    assert_eq!(g.players[0].tickets, 0, "the third starts over");
    let reward = g
        .players[0]
        .hand
        .iter()
        .find(|m| Some(m.card) == g.data.cards.triple_reward)
        .expect("a reward spell");
    assert_eq!(reward.improvements as u8, g.players[0].tier, "a minion of your own Tier");
    g.check_invariants().unwrap();
}

#[test]
fn see_the_light_steals_a_tavern_minion_as_a_two_two() {
    let mut g = hero_game(13, "Xyrella");
    assert_eq!(power(&g, 0).cost, 2);
    set_shop(&mut g, 0, &["Buzzing Vermin"]);
    g.players[0].shop[0].attack = 9;
    g.players[0].gold = 5;
    press(&mut g, 0);
    assert_eq!(g.players[0].shop.len(), 0);
    let h = g.players[0].hand[0];
    assert_eq!((h.attack, h.health), (2, 2), "stats set to 2/2");
    assert_eq!(h.pool_copies, 1, "as a steal: the pool copy comes along");
    g.check_invariants().unwrap();
}

#[test]
fn spirit_swap_takes_a_tavern_minion_for_either_pick() {
    // The log's own case (15_25_26-g1 turn 1): a board minion, then a
    // Tavern minion. docs/rules/heroes.md: any minion, the Tavern's included.
    let mut g = hero_game(18, "Vol'jin");
    put_on_board(&mut g, 0, "Buzzing Vermin");
    put_on_board(&mut g, 0, "Southsea Busker");
    set_shop(&mut g, 0, &["Buzzing Vermin", "Southsea Busker"]);
    g.players[0].board[0].attack = 3;
    g.players[0].board[1].attack = 5;
    g.players[0].shop[0].attack = 9;
    g.players[0].shop[1].attack = 7;
    act(&mut g, 0, Action::HeroPower);
    act(&mut g, 0, Action::Target(0));
    act(&mut g, 0, Action::TargetShop(0));
    assert_eq!(g.players[0].board[0].attack, 9, "the Tavern minion's Attack");
    assert_eq!(g.players[0].shop[0].attack, 3, "and it took the board one's");
    // The Tavern first, the board second: the same swap, the other way round.
    again(&mut g, 0);
    act(&mut g, 0, Action::HeroPower);
    act(&mut g, 0, Action::TargetShop(1));
    act(&mut g, 0, Action::Target(1));
    assert_eq!(g.players[0].shop[1].attack, 5, "the board minion's Attack");
    assert_eq!(g.players[0].board[1].attack, 7, "and the Tavern card's");
    g.check_invariants().unwrap();
}

#[test]
fn spirit_swap_swaps_two_attacks_until_the_next_turn() {
    let mut g = hero_game(14, "Vol'jin");
    assert_eq!(power(&g, 0).cost, 0);
    put_on_board(&mut g, 0, "Buzzing Vermin");
    put_on_board(&mut g, 0, "Buzzing Vermin");
    g.players[0].board[0].attack = 1;
    g.players[0].board[1].attack = 7;
    act(&mut g, 0, Action::HeroPower);
    act(&mut g, 0, Action::Target(0));
    act(&mut g, 0, Action::Target(1));
    assert_eq!(g.players[0].board[0].attack, 7);
    assert_eq!(g.players[0].board[1].attack, 1);
    // A buff after the swap stays after the swap comes off.
    g.players[0].board[0].attack += 3;
    duel(&mut g);
    g.players[0].board[0].health = 60;
    g.players[0].board[1].health = 60;
    end_all_turns(&mut g);
    assert_eq!(g.players[0].board[0].attack, 4, "the swap's +6 goes, the +3 stays");
    assert_eq!(g.players[0].board[1].attack, 7, "its own Attack again");
    g.check_invariants().unwrap();
}

#[test]
fn devour_sells_a_minion_and_spits_its_stats_onto_another() {
    let mut g = hero_game(15, "Mutanus the Devourer");
    assert_eq!(power(&g, 0).cost, 0);
    put_on_board(&mut g, 0, "Buzzing Vermin");
    put_on_board(&mut g, 0, "Buzzing Vermin");
    put_on_board(&mut g, 0, "Buzzing Vermin");
    g.players[0].board[0].attack = 4;
    g.players[0].board[0].health = 6;
    let (a1, h1) = (g.players[0].board[1].attack, g.players[0].board[1].health);
    let gold = g.players[0].gold;
    act(&mut g, 0, Action::HeroPower);
    act(&mut g, 0, Action::Target(0));
    assert_eq!(g.players[0].gold, gold + 1, "sold: the Gold comes in");
    assert_eq!(g.players[0].board.len(), 2, "the sold one is gone");
    act(&mut g, 0, Action::Target(0));
    let m = g.players[0].board[0];
    assert_eq!((m.attack, m.health), (a1 + 4, h1 + 6), "its stats, buffs included");
    g.check_invariants().unwrap();
}

#[test]
fn i_spy_discovers_a_plain_copy_from_the_next_opponents_warband() {
    let mut g = hero_game(16, "Scabbs Cutterbutter");
    assert_eq!(power(&g, 0).cost, 2);
    assert!(!legal_for_0(&mut g, Action::HeroPower), "an empty warband can't be spied on");
    let c = id(&g, "Buzzing Vermin");
    let mut m = g.players[0].make_card(&g.data.cards, c, false, 0);
    m.attack += 4;
    g.players[0].next_board.clear();
    g.players[0].next_board.push(m);
    g.players[0].gold = 5;
    press(&mut g, 0);
    assert_eq!(g.players[0].discover.len(), 1, "one minion to offer");
    choose(&mut g, 0, 0);
    let h = g.players[0].hand[0];
    assert_eq!(h.card, c);
    assert_eq!(h.attack, 1, "a plain copy, its buffs gone");
    assert_eq!(h.pool_copies, 0, "made fresh");
    g.check_invariants().unwrap();
}

#[test]
fn twice_as_nice_copies_the_highest_tier_minion_and_freezes_both() {
    let mut g = hero_game(17, "Varden Dawngrasp");
    assert!(power(&g, 0).passive);
    duel(&mut g);
    g.players[0].tier = 2;
    set_shop(&mut g, 0, &["Buzzing Vermin", "Southsea Busker", "Sellemental"]);
    g.players[0].gold = 5;
    act(&mut g, 0, Action::Roll);
    let shop = &g.players[0].shop;
    let base = g.data.rules.shop_size(2);
    assert_eq!(shop.len(), base + 1, "one card outside the normal slots");
    let copy = *shop.last().unwrap();
    let same: Vec<usize> = shop
        .iter()
        .enumerate()
        .filter(|(_, m)| m.card == copy.card)
        .map(|(i, _)| i)
        .collect();
    assert_eq!(same.len(), 2, "the highest-Tier minion, and its copy");
    assert!(shop[same[0]].frozen && copy.frozen, "both frozen");
    let top = g.data.cards.get(copy.card).tier;
    for m in shop.iter() {
        assert!(g.data.cards.get(m.card).tier <= top, "it is the highest Tier one");
    }
    g.check_invariants().unwrap();
}

#[test]
fn enhancification_gives_out_random_bonus_keywords_after_a_refresh() {
    let mut g = hero_game(18, "Enhance-o Mechano");
    assert!(power(&g, 0).passive);
    duel(&mut g);
    g.players[0].tier = 3;
    let mask = g.data.rules.bonus_mask;
    // Each Refresh gives two random Bonus Keywords to random Tavern minions,
    // both possibly on the same one.
    for _ in 0..3 {
        g.players[0].gold = 5;
        act(&mut g, 0, Action::Roll);
        let extra: u32 = g.players[0]
            .shop
            .iter()
            .map(|m| {
                let printed = g.data.cards.get(m.card).keywords.0 & mask;
                (m.keywords.0 & mask & !printed).count_ones()
            })
            .sum();
        assert_eq!(extra, 2, "two Bonus Keywords a Refresh");
    }
    g.check_invariants().unwrap();
}

#[test]
fn rune_of_damnation_destroys_an_undead_for_a_random_one() {
    let mut g = hero_game(19, "The Jailer");
    assert_eq!(power(&g, 0).cost, 1);
    put_on_board(&mut g, 0, "Harmless Bonehead");
    // "Unlocks at Tier 2": levelling for real sets the lock.
    g.players[0].gold = 20;
    act(&mut g, 0, Action::Level);
    assert_eq!(g.players[0].tier, 2);
    g.players[0].gold = 5;
    let hand = g.players[0].hand.len();
    press(&mut g, 0);
    assert!(
        !g.players[0].board.iter().any(|m| m.card == id(&g, "Harmless Bonehead")),
        "it died for real"
    );
    let skeleton = id(&g, "Skeleton");
    assert_eq!(
        g.players[0].board.iter().filter(|m| m.card == skeleton).count(),
        2,
        "its Deathrattle fired"
    );
    assert_eq!(g.players[0].hand.len(), hand + 1, "a random Undead arrived");
    let got = g.players[0].hand[hand];
    assert!(g.data.cards.tribes_of(&got).has(Tribe::Undead));
    g.check_invariants().unwrap();
}

#[test]
fn major_hymn_gives_attack_and_minor_hymn_health_the_next_turn() {
    let mut g = hero_game(20, "Inge, the Iron Hymn");
    assert_eq!(power(&g, 0).name, "Major Hymn");
    assert_eq!(power(&g, 0).uses_per_turn, 2);
    assert_eq!(g.players[0].tier, 1);
    put_on_board(&mut g, 0, "Buzzing Vermin");
    let (a, h) = (g.players[0].board[0].attack, g.players[0].board[0].health);
    press(&mut g, 0);
    let m = g.players[0].board[0];
    assert_eq!((m.attack, m.health), (a + 1, h), "Attack equal to the Tier");
    again(&mut g, 0);
    press(&mut g, 0);
    let m = g.players[0].board[0];
    assert_eq!(m.attack, a + 2, "twice a turn");
    duel(&mut g);
    g.players[0].board[0].health = 60;
    end_all_turns(&mut g);
    assert_eq!(power(&g, 0).name, "Minor Hymn", "it swaps every turn");
    let (a, h) = (g.players[0].board[0].attack, g.players[0].board[0].health);
    press(&mut g, 0);
    let m = g.players[0].board[0];
    assert_eq!((m.attack, m.health), (a, h + 1), "Health equal to the Tier");
    g.check_invariants().unwrap();
}

/// Master Nguyen borrows a hero power for the turn, so a borrowed Major Hymn
/// does not turn into Minor Hymn at the end of it: the game ends the turn on
/// the power it borrowed and hands out a new one at the start of the next
/// (Hearthstone_2026_11_02_00_10_22-g6 turn 6 ends on Major Hymn).
#[test]
fn a_borrowed_hymn_does_not_turn_over_at_the_end_of_the_turn() {
    let mut g = hero_game(200, "Master Nguyen");
    turn_of(&mut g, 0);
    let data = g.data.clone();
    // The power the turn borrowed is Major Hymn: the offer Master Nguyen
    // opens at the start of the turn is already answered.
    g.players[0].power_offer.clear();
    let hymn = data.heroes.power_by_card_id("BG26_HERO_102p").expect("Major Hymn");
    g.players[0].hold_power(&data, hymn, 0, 6);
    g.players[0].storm_power[0] = true;
    g.shop_end_of_turn(0);
    assert_eq!(power(&g, 0).name, "Major Hymn", "still the borrowed power");
    g.check_invariants().unwrap();
}

#[test]
fn cloning_gallery_summons_an_exact_copy_once_a_game() {
    let mut g = hero_game(21, "Zerek, Master Cloner");
    assert_eq!(power(&g, 0).uses_per_game, 1);
    put_on_board(&mut g, 0, "Buzzing Vermin");
    g.players[0].board[0].attack = 6;
    g.players[0].board[0].health = 8;
    g.players[0].board[0].golden = true;
    press(&mut g, 0);
    assert_eq!(g.players[0].board.len(), 2, "summoned beside it");
    let copy = g.players[0].board[1];
    assert_eq!((copy.attack, copy.health), (6, 8), "an exact copy");
    assert!(copy.golden);
    assert_eq!(copy.pool_copies, 0, "no pool copy taken");
    assert!(!legal_for_0(&mut g, Action::HeroPower), "once a game");
    g.check_invariants().unwrap();
}

#[test]
fn double_time_makes_two_copies_golden_and_pays_a_coin_instead_of_a_reward() {
    let mut g = hero_game(22, "Mister Clocksworth");
    assert!(power(&g, 0).passive);
    put_in_hand(&mut g, 0, "Buzzing Vermin");
    g.players[0].hand[0].health = 9;
    set_shop(&mut g, 0, &["Buzzing Vermin"]);
    g.players[0].gold = 5;
    act(&mut g, 0, Action::Buy(0));
    // Two copies are enough: the pair turned Golden, keeping both copies'
    // buffs (1/1 printed doubled, plus the 8 the first one had gained).
    let golden = g.players[0].hand.iter().find(|m| m.golden).copied().expect("Golden from two copies");
    assert_eq!((golden.attack, golden.health), (2, 10), "both copies' buffs");
    assert_eq!(g.players[0].hand.len(), 1, "the two copies became one");
    g.check_invariants().unwrap();
    // Playing it gives a Tavern Coin, not a Triple Reward.
    let hand = g.players[0].hand.len();
    let played = g.players[0].hand.iter().position(|m| m.golden).unwrap();
    act(&mut g, 0, Action::Play { hand: played as u8, pos: 0 });
    let coin = id(&g, "Tavern Coin");
    assert!(
        g.players[0].hand.iter().any(|m| m.card == coin),
        "a Tavern Coin instead of a Triple Reward"
    );
    assert!(
        !g.players[0]
            .hand
            .iter()
            .any(|m| Some(m.card) == g.data.cards.triple_reward),
        "no Triple Reward"
    );
    assert!(g.players[0].hand.len() >= hand, "the golden left the hand");
    g.check_invariants().unwrap();
}

#[test]
fn double_time_pairs_an_amalgamation_minion_with_a_surprise() {
    // docs/rules/heroes.md "Double Time": with an Elemental of Surprise, one
    // copy of an Elemental plus one Surprise is a pair, so a minion the
    // Amalgamation Dark Gift made every type does it too (Era, 2026-11-03).
    let mut g = hero_game(23, "Mister Clocksworth");
    put_on_board(&mut g, 0, "Titus Rivendare");
    let host = g.players[0].board.len() - 1;
    assert!(g.give_gift(0, host, "Amalgamation", g.turn.max(1)));
    set_shop(&mut g, 0, &["Elemental of Surprise"]);
    g.players[0].gold = 5;
    act(&mut g, 0, Action::Buy(0));
    assert_eq!(g.players[0].hand.len(), 1, "the pair and the Surprise are one card");
    let golden = g.players[0].hand[0];
    assert!(golden.golden, "one Titus and one Surprise are a pair");
    assert_eq!(g.data.cards.get(golden.card).name, "Titus Rivendare");
    assert!(
        golden.keywords.has(bgcore::Keywords::DIVINE_SHIELD),
        "the Surprise's shield carries"
    );
    assert!(g.players[0].board.is_empty(), "the board copy went into it");
    // Playing it pays the Tavern Coin, as Double Time does for every pair.
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    let coin = id(&g, "Tavern Coin");
    assert!(
        g.players[0].hand.iter().any(|m| m.card == coin),
        "a Tavern Coin instead of a Triple Reward"
    );
    g.check_invariants().unwrap();
    // The same pair with no gift is no pair: a plain Titus is no Elemental.
    let mut g = hero_game(24, "Mister Clocksworth");
    put_on_board(&mut g, 0, "Titus Rivendare");
    set_shop(&mut g, 0, &["Elemental of Surprise"]);
    g.players[0].gold = 5;
    act(&mut g, 0, Action::Buy(0));
    assert!(
        g.players[0].hand.iter().chain(&g.players[0].board).all(|m| !m.golden),
        "a plain Titus and a Surprise are no pair"
    );
    g.check_invariants().unwrap();
}

// ---- the Spellcraft target fix (the brief's last note) ----

#[test]
fn a_spellcraft_spell_with_no_legal_target_cannot_be_played() {
    let mut g = hero_game(24, "George the Fallen");
    for (spell, tribe) in [
        ("Vibrant Bubble", "Bubble Gunner"),
        ("Ophidian Staff", "Buzzing Vermin"),
        ("Jailer Sticker", "Harmless Bonehead"),
    ] {
        g.players[0].hand.clear();
        g.players[0].board.clear();
        set_shop(&mut g, 0, &[]);
        put_in_hand(&mut g, 0, spell);
        assert!(
            !legal_for_0(&mut g, Action::Play { hand: 0, pos: 0 }),
            "{spell} with nothing to land on"
        );
        // A minion of the right tribe on the board makes it legal.
        put_on_board(&mut g, 0, tribe);
        assert!(
            legal_for_0(&mut g, Action::Play { hand: 0, pos: 0 }),
            "{spell} with a {tribe}"
        );
        // So does one in the Tavern, for a spell that may be cast there (the
        // Jailer Sticker destroys a minion of yours, so the Tavern is no
        // target for it).
        if spell != "Jailer Sticker" {
            clear_board(&mut g, 0);
            set_shop(&mut g, 0, &[tribe]);
            assert!(
                legal_for_0(&mut g, Action::Play { hand: 0, pos: 0 }),
                "{spell} with a {tribe} in the Tavern"
            );
        }
    }
    g.check_invariants().unwrap();
}

#[test]
fn imprison_and_arcane_alteration_reach_the_spell_slot() {
    // Imprison takes the spell slot too (it is a Tavern card like any other).
    let mut g = hero_game(25, "Maiev Shadowsong");
    set_shop(&mut g, 0, &[]);
    g.players[0].shop_spell = Some(id(&g, "Tavern Coin"));
    g.players[0].gold = 5;
    press(&mut g, 0);
    assert_eq!(g.players[0].shop_spell, None, "the spell left the slot");
    assert_eq!(g.players[0].hand.len(), 1);
    assert_eq!(g.players[0].hand[0].locked_turns, 2);
    g.check_invariants().unwrap();
    // Arcane Alteration replaces it with a random Tavern spell of its Tier.
    let mut g = hero_game(26, "Malygos");
    set_shop(&mut g, 0, &[]);
    let coin = id(&g, "Tavern Coin");
    g.players[0].shop_spell = Some(coin);
    // The spell slot is the only legal target, so the press applies at once.
    press(&mut g, 0);
    let now = g.players[0].shop_spell.expect("a spell still sits there");
    assert!(g.data.cards.get(now).is_spell());
    assert_eq!(g.data.cards.get(now).tier, g.data.cards.get(coin).tier, "the same Tier");
    g.check_invariants().unwrap();
}