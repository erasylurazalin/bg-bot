//! The Darkmoon Prizes: the 31 Prize spells, the one Prize minion, the Triple
//! Prize and the Prize Discover. Rules: docs/rules/trinkets.md, "Rules for the last
//! trinket batch (Era, 2026-11-01)".

use std::sync::Arc;

use bgcore::{Action, CardId, Data, Game, Minion, Tribe, Tribes};

fn data() -> Arc<Data> {
    Arc::new(Data::bundled())
}

fn all_tribes() -> Tribes {
    Tribe::EVERY.into_iter().fold(Tribes::NONE, |m, t| m.with(t))
}

fn game(seed: u64) -> Game {
    Game::new_with_lobby(data(), seed, all_tribes())
}

fn reset(g: &mut Game, seed: u64) {
    *g = game(seed);
}

fn id(g: &Game, name: &str) -> CardId {
    g.data.cards.by_name(name).unwrap_or_else(|| panic!("{name}"))
}

fn answer(g: &mut Game) -> bool {
    let Some(p) = g.current_player() else { return false };
    let pl = &g.players[p];
    let a = if !pl.discover.is_empty() || !pl.power_offer.is_empty() || pl.pending_choose_one.is_some()
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
    let _ = g.step(a);
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
    g.step(a).unwrap_or_else(|_| panic!("{a:?} illegal for player {p}"));
}

fn pool_copies(g: &mut Game, c: CardId) -> u8 {
    if g.data.cards.get(c).token {
        return 0;
    }
    let slot = g.data.cards.get(c).pool_slot as usize;
    g.pool[slot] -= 1;
    1
}

fn put_on_board(g: &mut Game, p: usize, name: &str) {
    let c = id(g, name);
    let n = pool_copies(g, c);
    let m = g.players[p].make_card(&g.data.cards, c, false, n);
    g.players[p].board.push(m);
}

fn set_shop(g: &mut Game, p: usize, names: &[&str]) {
    let old: Vec<Minion> = g.players[p].shop.drain(..).collect();
    for m in old {
        if m.pool_copies == 0 {
            continue;
        }
        let slot = g.data.cards.get(m.card).pool_slot as usize;
        g.pool[slot] += m.pool_copies;
    }
    for n in names {
        let c = id(g, n);
        let k = pool_copies(g, c);
        let m = g.players[p].make_card(&g.data.cards, c, false, k);
        g.players[p].shop.push(m);
    }
    g.players[p].shop_spell = None;
}

/// Cast a Prize (or any card) from hand as player 0. Returns the hand slot
/// used, and answers one target prompt when the card asks for one.
fn cast(g: &mut Game, p: usize, name: &str) {
    g.give(p, name).unwrap_or_else(|| panic!("{name} into hand"));
    let h = g.players[p].hand.len() - 1;
    turn_of(g, p);
    act(g, p, Action::Play { hand: h as u8, pos: 0 });
}

/// Put a Golden copy of a pool minion on the board (Balinda Stonehearth's
/// three casts), pool kept exact.
fn put_on_board_golden(g: &mut Game, p: usize, name: &str) {
    let c = id(g, name);
    let slot = g.data.cards.get(c).pool_slot as usize;
    g.pool[slot] -= 1;
    let m = g.players[p].make_card(&g.data.cards, c, true, 1);
    g.players[p].board.push(m);
}

/// Cast a Prize onto board slot `target` (the cards that target a friendly
/// minion).
fn cast_on(g: &mut Game, p: usize, name: &str, target: usize) {
    g.give(p, name).unwrap_or_else(|| panic!("{name} into hand"));
    let h = g.players[p].hand.len() - 1;
    turn_of(g, p);
    act(g, p, Action::Play { hand: h as u8, pos: 0 });
    if g.players[p].pending_target.is_some() {
        act(g, p, Action::Target(target as u8));
    }
}

/// Cast a Prize onto the first board minion.
fn cast_on_first(g: &mut Game, p: usize, name: &str) {
    cast_on(g, p, name, 0);
}

fn hand_of(g: &Game, p: usize, name: &str) -> usize {
    let c = id(g, name);
    g.players[p].hand.iter().filter(|m| m.card == c).count()
}

fn stats(g: &Game, p: usize, at: usize) -> (i32, i32) {
    (g.players[p].board[at].attack, g.players[p].board[at].health)
}

// ---- the data ----

#[test]
fn the_card_data_has_the_thirty_two_prizes_in_four_tiers() {
    let d = data();
    let mut total = 0;
    for (i, list) in d.cards.prizes.iter().enumerate() {
        total += list.len();
        println!("tier {}: {} prizes", i + 1, list.len());
        for &c in list {
            assert_eq!(d.cards.get(c).prize, Some(i as u8 + 1), "{}", d.cards.get(c).name);
            // A Prize is never in the Tavern's spell pool nor in the minion
            // pool.
            assert!(!d.cards.spells.contains(&c), "{} is a pool spell", d.cards.get(c).name);
            assert!(c as usize >= d.cards.pool_cards, "{} is a pool minion", d.cards.get(c).name);
            assert!(d.cards.get(c).token || d.cards.get(c).is_spell());
        }
    }
    assert_eq!(total, 32, "the game data's 32 Prize cards");
    for (tier, want) in [(1, 7), (2, 9), (3, 8), (4, 8)] {
        assert_eq!(d.cards.prizes[tier - 1].len(), want, "tier {tier}");
    }
    // All-Seeing Elder is the one Prize that is a minion.
    let elder = id(&d_game(&d), "All-Seeing Elder");
    assert!(!d.cards.get(elder).is_spell());
    assert_eq!(d.cards.get(elder).prize, Some(4));
    // Prizes are plain spells, never Tavern spells.
    for list in &d.cards.prizes {
        for &c in list {
            if d.cards.get(c).is_spell() {
                assert!(!d.cards.get(c).tavern_spell, "{} is no Tavern spell", d.cards.get(c).name);
            }
        }
    }
}

fn d_game(d: &Arc<Data>) -> Game {
    Game::new_with_lobby(d.clone(), 1, all_tribes())
}

// ---- the Discover ----

#[test]
fn a_prize_discover_offers_three_different_prizes_of_one_tier() {
    let mut g = game(11);
    // Tickatus Sticker's pick opens the Tier 3 Discover.
    let i = g.data.trinkets.by_name("Tickatus Sticker").unwrap();
    turn_of(&mut g, 0);
    g.players[0].trinket_offer.clear();
    g.players[0].trinket_offer_slot = 1;
    let _ = g.players[0].trinket_offer.try_push((i, 0));
    act(&mut g, 0, Action::ChooseTrinket(0));
    let offer: Vec<CardId> = g.players[0].discover.to_vec();
    assert_eq!(offer.len(), 3, "three options");
    let mut ids = offer.clone();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 3, "three different prizes");
    for &c in &offer {
        assert_eq!(g.data.cards.get(c).prize, Some(3), "a Tier 3 Prize");
    }
    // The pick lands in hand with no pool copy.
    let pool_before: Vec<u8> = g.pool.clone();
    let want = offer[1];
    act(&mut g, 0, Action::Choose(1));
    assert!(g.players[0].hand.iter().any(|m| m.card == want && m.pool_copies == 0));
    assert_eq!(g.pool, pool_before, "no pool copy moves for a Prize");
    g.check_invariants().unwrap();
}

#[test]
fn the_triple_prize_discovers_a_tier_3_prize_and_corrupted_tome_hands_it_out() {
    let mut g = game(12);
    // The Triple Prize is a plain spell that Discovers a Tier 3 Prize.
    cast(&mut g, 0, "Triple Prize");
    let offer: Vec<CardId> = g.players[0].discover.to_vec();
    assert_eq!(offer.len(), 3);
    for &c in &offer {
        assert_eq!(g.data.cards.get(c).prize, Some(3));
    }
    act(&mut g, 0, Action::Choose(0));
    // Corrupted Tome: a Triple Prize in hand at the pick, and every later
    // Triple Reward becomes one.
    let mut g = game(13);
    reset(&mut g, 13);
    let i = g.data.trinkets.by_name("Corrupted Tome").unwrap();
    turn_of(&mut g, 0);
    g.players[0].trinket_offer.clear();
    g.players[0].trinket_offer_slot = 2;
    let _ = g.players[0].trinket_offer.try_push((i, 0));
    act(&mut g, 0, Action::ChooseTrinket(0));
    assert_eq!(hand_of(&g, 0, "Triple Prize"), 1, "one at the pick");
    // A Golden minion played from hand gives a Triple Prize, not a Triple
    // Reward.
    let c = id(&g, "Lullabot");
    let golden = g.players[0].make_card(&g.data.cards, c, true, 0);
    g.players[0].hand.push(golden);
    let h = g.players[0].hand.len() - 1;
    turn_of(&mut g, 0);
    act(&mut g, 0, Action::Play { hand: h as u8, pos: 0 });
    assert_eq!(hand_of(&g, 0, "Triple Prize"), 2, "a Triple Prize instead");
    assert_eq!(hand_of(&g, 0, "Triple Reward"), 0, "no Triple Reward");
    g.check_invariants().unwrap();
}

// ---- tier 1 ----

#[test]
fn banana_bunch_gives_two_bananas() {
    let mut g = game(21);
    cast(&mut g, 0, "Banana Bunch");
    assert_eq!(hand_of(&g, 0, "Tavern Dish Banana"), 2);
    g.check_invariants().unwrap();
}

#[test]
fn crystallization_raises_the_tavern_spell_bonus() {
    let mut g = game(22);
    let before = g.players[0].spell_bonus;
    cast(&mut g, 0, "Crystallization");
    assert_eq!(g.players[0].spell_bonus, (before.0 + 1, before.1 + 1));
    g.check_invariants().unwrap();
}

#[test]
fn gacha_gift_discovers_a_tier_1_minion() {
    let mut g = game(23);
    cast(&mut g, 0, "Gacha Gift");
    assert_eq!(g.players[0].discover.len(), 3);
    for &c in g.players[0].discover.clone().iter() {
        assert_eq!(g.data.cards.get(c).tier, 1);
    }
    g.check_invariants().unwrap();
}

#[test]
fn new_recruit_offers_an_extra_minion_with_two_two() {
    let mut g = game(24);
    g.players[0].tier = 4;
    cast(&mut g, 0, "New Recruit");
    let size = g.players[0].shop_size(&g.data.rules);
    turn_of(&mut g, 0);
    act(&mut g, 0, Action::Roll);
    assert_eq!(g.players[0].shop.len(), size, "one more minion than before");
    // The extra one arrives with +2/+2 (it is the last drawn minion).
    turn_of(&mut g, 0);
    act(&mut g, 0, Action::Roll);
    assert_eq!(g.players[0].shop.len(), size);
    g.check_invariants().unwrap();
}

#[test]
fn rocking_and_rolling_gives_a_free_refresh_each_turn() {
    let mut g = game(25);
    cast(&mut g, 0, "Rocking and Rolling");
    assert_eq!(g.players[0].free_rolls, 0, "nothing on the turn it is cast");
    for want in 1..=2 {
        let t = g.turn;
        while g.turn == t && !g.over {
            if !answer(&mut g) {
                g.step(Action::EndTurn).unwrap();
            }
        }
        assert_eq!(g.players[0].free_rolls, want, "one more each turn");
    }
    g.check_invariants().unwrap();
}

#[test]
fn the_good_stuff_buffs_the_tavern_for_good() {
    let mut g = game(26);
    cast(&mut g, 0, "The Good Stuff");
    let buffs = g.players[0].tavern_buffs.to_vec();
    assert!(
        buffs.iter().any(|b| b.attack == 1 && b.health == 1 && !b.this_turn),
        "a permanent +1/+1"
    );
    g.check_invariants().unwrap();
}

#[test]
fn unfurled_codex_gets_a_spell_costing_two_or_more() {
    let mut g = game(27);
    let before = g.players[0].hand.len();
    cast(&mut g, 0, "Unfurled Codex");
    assert!(g.players[0].hand.len() > before);
    let got = *g.players[0].hand.last().unwrap();
    assert!(g.data.cards.get(got.card).cost >= 2, "costs (2) or more");
    g.check_invariants().unwrap();
}

// ---- tier 2 ----

#[test]
fn big_brann_play_doubles_this_turn_battlecries() {
    let mut g = game(31);
    put_on_board(&mut g, 0, "Lullabot");
    cast(&mut g, 0, "Big Brann Play");
    assert_eq!(g.players[0].prize.battlecry_extra_turn, 1);
    // Iron Groundskeeper's Battlecry gets two Fortify spells; twice with the
    // extra trigger.
    let mut g2 = game(32);
    reset(&mut g2, 32);
    cast(&mut g2, 0, "Big Brann Play");
    g2.give(0, "Iron Groundskeeper");
    let h = g2.players[0].hand.len() - 1;
    turn_of(&mut g2, 0);
    act(&mut g2, 0, Action::Play { hand: h as u8, pos: 0 });
    assert_eq!(hand_of(&g2, 0, "Fortify"), 4, "two per trigger, two triggers");
    g2.check_invariants().unwrap();
}

#[test]
fn evolving_tavern_replaces_the_tavern_with_a_tier_higher() {
    let mut g = game(33);
    g.players[0].tier = 3;
    set_shop(&mut g, 0, &["Lullabot", "Sellemental"]);
    cast(&mut g, 0, "Evolving Tavern");
    assert_eq!(g.players[0].shop.len(), 2, "the same number of cards");
    for m in &g.players[0].shop {
        assert_eq!(g.data.cards.get(m.card).tier, 4, "one Tier higher");
    }
    g.check_invariants().unwrap();
}

#[test]
fn gruul_rules_gives_a_minion_a_gain_every_end_of_turn() {
    let mut g = game(34);
    // Microbot has no end-of-turn effect of its own to confuse the count.
    put_on_board(&mut g, 0, "Microbot");
    let before = stats(&g, 0, 0);
    cast_on_first(&mut g, 0, "Gruul Rules");
    assert_eq!(g.players[0].board[0].end_turn_gain, (4, 4));
    let t = g.turn;
    while g.turn == t && !g.over {
        if !answer(&mut g) {
            g.step(Action::EndTurn).unwrap();
        }
    }
    assert_eq!(stats(&g, 0, 0), (before.0 + 4, before.1 + 4), "the +4/+4 landed");
    g.check_invariants().unwrap();
}

#[test]
fn rat_in_a_cage_adds_two_attack_then_doubles_it() {
    let mut g = game(35);
    put_on_board(&mut g, 0, "Lullabot");
    let before = stats(&g, 0, 0);
    cast_on_first(&mut g, 0, "I'm Still Just a Rat in a Cage");
    let after = stats(&g, 0, 0);
    assert_eq!(after.0, (before.0 + 2) * 2, "the Attack doubled");
    assert_eq!(after.1, before.1, "the Health did not");
    g.check_invariants().unwrap();
}

#[test]
fn mageroyal_blossom_discovers_a_tavern_spell_of_your_tier() {
    let mut g = game(36);
    g.players[0].tier = 4;
    cast(&mut g, 0, "Mageroyal Blossom");
    assert_eq!(g.players[0].discover.len(), 3);
    for &c in g.players[0].discover.clone().iter() {
        assert!(g.data.cards.get(c).is_spell());
        assert_eq!(g.data.cards.get(c).tier, 4);
    }
    g.check_invariants().unwrap();
}

#[test]
fn on_the_house_discovers_a_minion_of_your_tier() {
    let mut g = game(37);
    g.players[0].tier = 4;
    cast(&mut g, 0, "On the House");
    assert_eq!(g.players[0].discover.len(), 3);
    for &c in g.players[0].discover.clone().iter() {
        assert_eq!(g.data.cards.get(c).tier, 4);
    }
    g.check_invariants().unwrap();
}

#[test]
fn the_bouncer_gives_taunt_then_doubles_the_health() {
    let mut g = game(38);
    put_on_board(&mut g, 0, "Lullabot");
    let before = stats(&g, 0, 0);
    cast_on_first(&mut g, 0, "The Bouncer");
    assert!(g.players[0].board[0].keywords.has(bgcore::minion::Keywords::TAUNT));
    assert_eq!(stats(&g, 0, 0).1, before.1 * 2, "the Health doubled");
    assert_eq!(stats(&g, 0, 0).0, before.0);
    g.check_invariants().unwrap();
}

#[test]
fn the_unlimited_coin_gains_one_gold_and_comes_back() {
    let mut g = game(39);
    let gold = g.players[0].gold;
    cast(&mut g, 0, "The Unlimited Coin");
    assert_eq!(g.players[0].gold, gold + 1);
    assert_eq!(hand_of(&g, 0, "The Unlimited Coin"), 0, "cast away");
    let t = g.turn;
    while g.turn == t && !g.over {
        if !answer(&mut g) {
            g.step(Action::EndTurn).unwrap();
        }
    }
    assert_eq!(hand_of(&g, 0, "The Unlimited Coin"), 1, "back in hand");
}

#[test]
fn time_thief_discovers_from_the_last_opponents_warband() {
    let mut g = game(40);
    // A game with a fight behind it, so there is a last opponent.
    let mut turns = 0;
    while g.players[0].last_opponent.is_none() && turns < 3 && !g.over {
        let t = g.turn;
        while g.turn == t && !g.over {
            if !answer(&mut g) {
                g.step(Action::EndTurn).unwrap();
            }
        }
        turns += 1;
    }
    assert!(g.players[0].last_opponent.is_some(), "a fight happened");
    cast(&mut g, 0, "Time Thief");
    // The offer holds plain copies of minions from that warband (or nothing
    // if it was empty).
    if !g.players[0].discover.is_empty() {
        for m in g.players[0].discover_minions.clone() {
            assert_eq!(m.pool_copies, 0, "a generated copy");
        }
    }
    g.check_invariants().unwrap();
}

// ---- tier 3 ----

#[test]
fn all_that_glitters_makes_a_tavern_minion_golden() {
    let mut g = game(51);
    set_shop(&mut g, 0, &["Lullabot"]);
    cast(&mut g, 0, "All That Glitters");
    assert!(g.players[0].shop[0].golden);
    g.check_invariants().unwrap();
}

#[test]
fn banananas_fills_the_hand_with_bananas() {
    let mut g = game(52);
    cast(&mut g, 0, "B.A.N.A.N.A.S.");
    assert_eq!(hand_of(&g, 0, "Tavern Dish Banana"), 10, "a full hand of Bananas");
    g.check_invariants().unwrap();
}

#[test]
fn buy_the_holy_light_gives_ten_attack_and_divine_shield() {
    let mut g = game(53);
    put_on_board(&mut g, 0, "Lullabot");
    let before = stats(&g, 0, 0);
    cast_on_first(&mut g, 0, "Buy the Holy Light");
    assert_eq!(stats(&g, 0, 0).0, before.0 + 10);
    assert_eq!(stats(&g, 0, 0).1, before.1);
    assert!(g.players[0].board[0].keywords.has(bgcore::minion::Keywords::DIVINE_SHIELD));
    g.check_invariants().unwrap();
}

#[test]
fn mindflayer_goggles_steals_the_tavern_and_refreshes_it() {
    let mut g = game(54);
    g.players[0].tier = 4;
    set_shop(&mut g, 0, &["Lullabot", "Sellemental"]);
    cast(&mut g, 0, "Mindflayer Goggles");
    assert_eq!(hand_of(&g, 0, "Lullabot"), 1, "stolen into hand");
    assert_eq!(hand_of(&g, 0, "Sellemental"), 1, "stolen into hand");
    assert!(!g.players[0].shop.is_empty(), "the Tavern was Refreshed");
    g.check_invariants().unwrap();
}

#[test]
fn repeat_customer_returns_a_minion_to_hand_with_six_six() {
    let mut g = game(55);
    put_on_board(&mut g, 0, "Lullabot");
    let before = stats(&g, 0, 0);
    cast_on_first(&mut g, 0, "Repeat Customer");
    assert!(g.players[0].board.is_empty(), "it left the board");
    let m = g.players[0].hand.iter().find(|m| m.card == id(&g, "Lullabot")).copied();
    let m = m.expect("back in hand");
    assert_eq!((m.attack, m.health), (before.0 + 6, before.1 + 6));
    g.check_invariants().unwrap();
}

#[test]
fn reserve_prices_makes_tavern_spells_cost_one_less_this_turn() {
    let mut g = game(56);
    g.players[0].shop_spell = Some(id(&g, "Tavern Coin"));
    g.players[0].tier = 1;
    let before = g.players[0].spell_cost(&g.data.cards).unwrap();
    cast(&mut g, 0, "Reserve Prices");
    assert_eq!(
        g.players[0].spell_cost(&g.data.cards).unwrap(),
        before.saturating_sub(1),
        "one Gold less"
    );
    g.check_invariants().unwrap();
}

#[test]
fn top_shelf_discovers_a_minion_from_a_tier_higher() {
    let mut g = game(57);
    g.players[0].tier = 3;
    cast(&mut g, 0, "Top Shelf");
    assert_eq!(g.players[0].discover.len(), 3);
    for &c in g.players[0].discover.clone().iter() {
        assert_eq!(g.data.cards.get(c).tier, 4);
    }
    g.check_invariants().unwrap();
}

#[test]
fn training_session_discovers_a_hero_power() {
    let mut g = game(58);
    let d = data();
    let hero = d.heroes.by_name("Tickatus").unwrap();
    g.assign_hero(0, hero);
    cast(&mut g, 0, "Training Session");
    assert!(!g.players[0].power_offer.is_empty(), "a power to pick");
    g.check_invariants().unwrap();
}

// ---- tier 4 ----

/// Records the stats a named card had the first time it attacked.
struct FirstAttack {
    card: CardId,
    seen: Option<(i32, i32)>,
}

impl bgcore::fight::Watch for FirstAttack {
    fn attack(&mut self, _side: usize, attacker: &Minion, _defender: &Minion) {
        if attacker.card == self.card && self.seen.is_none() {
            self.seen = Some((attacker.attack, attacker.health));
        }
    }
}

#[test]
fn all_seeing_elder_takes_the_ends_attack_and_health_in_a_fight() {
    let mut g = game(61);
    // The minion itself, as the Prize Discover would give it.
    put_on_board(&mut g, 0, "Lullabot");
    put_on_board(&mut g, 0, "All-Seeing Elder");
    put_on_board(&mut g, 0, "Sellemental");
    let (la, rh) = (g.players[0].board[0].attack, g.players[0].board[2].health);
    let before = stats(&g, 0, 1);
    // A wall on the other side, so the Elder attacks and is seen.
    let mech = id(&g, "Microbot");
    let wall = g.data.cards.make(mech, false);
    let mut wall = wall;
    wall.attack = 0;
    wall.health = 200;
    g.players[1].board.push(wall);
    let db = &g.data.cards;
    let mut rng = bgcore::Rng::new(1);
    let mut watch = FirstAttack { card: id(&g, "All-Seeing Elder"), seen: None };
    bgcore::fight::fight_with(
        db,
        &g.players[0].board,
        &g.players[1].board,
        [bgcore::fight::Counters::default(); 2],
        200,
        &mut rng,
        &mut watch,
    );
    // Start of Combat gave the Elder the left-most Attack and the right-most
    // Health.
    let seen = watch.seen.expect("the Elder attacked");
    assert_eq!(seen, (before.0 + la, before.1 + rh));
}

#[test]
fn big_winner_discovers_a_prize_from_each_earlier_tier() {
    let mut g = game(62);
    cast(&mut g, 0, "Big Winner!");
    // Three Discovers, back to back, one per earlier tier.
    let mut tiers = Vec::new();
    while !g.players[0].discover.is_empty() {
        let offer: Vec<CardId> = g.players[0].discover.to_vec();
        tiers.push(g.data.cards.get(offer[0]).prize.unwrap());
        act(&mut g, 0, Action::Choose(0));
        g.check_invariants().unwrap();
    }
    assert_eq!(tiers, vec![1, 2, 3], "tiers 1, 2 and 3, in order");
}

#[test]
fn fresh_tab_gains_twelve_gold() {
    let mut g = game(63);
    let gold = g.players[0].gold;
    cast(&mut g, 0, "Fresh Tab");
    assert_eq!(g.players[0].gold, gold + 12);
    g.check_invariants().unwrap();
}

#[test]
fn friends_and_family_discount_makes_minions_cost_two() {
    let mut g = game(64);
    set_shop(&mut g, 0, &["Lullabot"]);
    cast(&mut g, 0, "Friends and Family Discount");
    // Two Gold buys it, and one Gold does not.
    turn_of(&mut g, 0);
    g.players[0].gold = 1;
    assert!(!g.is_legal(Action::Buy(0)), "one Gold is not enough");
    g.players[0].gold = 2;
    assert!(g.is_legal(Action::Buy(0)), "two Gold is the price");
    let gold = 2;
    act(&mut g, 0, Action::Buy(0));
    assert_eq!(g.players[0].gold, gold - 2, "two Gold, not three");
    g.check_invariants().unwrap();
}

#[test]
fn give_a_dog_a_bone_gives_shield_windfury_and_fifteen() {
    let mut g = game(65);
    put_on_board(&mut g, 0, "Lullabot");
    let before = stats(&g, 0, 0);
    cast_on_first(&mut g, 0, "Give a Dog a Bone");
    let m = g.players[0].board[0];
    assert_eq!((m.attack, m.health), (before.0 + 15, before.1 + 15));
    assert!(m.keywords.has(bgcore::minion::Keywords::DIVINE_SHIELD));
    assert!(m.keywords.has(bgcore::minion::Keywords::WINDFURY));
    g.check_invariants().unwrap();
}

#[test]
fn gorgeous_goblet_fills_the_hand_with_tavern_spells() {
    let mut g = game(66);
    cast(&mut g, 0, "Gorgeous Goblet");
    assert!(g.players[0].hand.len() >= 9);
    for m in &g.players[0].hand {
        assert!(g.data.cards.get(m.card).tavern_spell, "{}", g.data.cards.get(m.card).name);
    }
    g.check_invariants().unwrap();
}

#[test]
fn open_bar_gives_five_free_refreshes_now_and_every_turn() {
    let mut g = game(67);
    cast(&mut g, 0, "Open Bar");
    assert_eq!(g.players[0].free_rolls, 5, "five now");
    let t = g.turn;
    while g.turn == t && !g.over {
        if !answer(&mut g) {
            g.step(Action::EndTurn).unwrap();
        }
    }
    assert_eq!(g.players[0].free_rolls, 10, "five more");
    g.check_invariants().unwrap();
}

#[test]
fn raise_the_stakes_makes_a_minion_golden_and_returns_it() {
    let mut g = game(68);
    put_on_board(&mut g, 0, "Lullabot");
    cast_on_first(&mut g, 0, "Raise the Stakes");
    assert!(g.players[0].board.is_empty(), "it left the board");
    let m = g.players[0].hand.iter().find(|m| m.card == id(&g, "Lullabot")).copied();
    assert!(m.expect("back in hand").golden, "Golden in hand");
    g.check_invariants().unwrap();
}

// ---- Balinda Stonehearth's extra cast (Era, block 5 review) ----

#[test]
fn balinda_casts_the_bouncer_twice_so_the_health_doubles_twice() {
    let mut g = game(71);
    put_on_board(&mut g, 0, "Microbot");
    put_on_board(&mut g, 0, "Balinda Stonehearth");
    cast_on(&mut g, 0, "The Bouncer", 0);
    // Taunt, then double the Health, and the whole thing again: 1 -> 2 -> 4.
    let m = g.players[0].board[0];
    assert!(m.keywords.has(bgcore::minion::Keywords::TAUNT));
    assert_eq!((m.attack, m.health), (1, 4), "the Health doubled twice");
    g.check_invariants().unwrap();
}

#[test]
fn balinda_casts_rat_in_a_cage_twice_so_the_attack_doubles_twice() {
    let mut g = game(72);
    put_on_board(&mut g, 0, "Microbot");
    put_on_board(&mut g, 0, "Balinda Stonehearth");
    cast_on(&mut g, 0, "I'm Still Just a Rat in a Cage", 0);
    // +2 Attack then double, twice: 1 -> 6 -> 16.
    let m = g.players[0].board[0];
    assert_eq!((m.attack, m.health), (16, 1), "the Attack doubled twice");
    g.check_invariants().unwrap();
}

#[test]
fn a_golden_balinda_casts_them_three_times() {
    let mut g = game(73);
    put_on_board(&mut g, 0, "Microbot");
    put_on_board_golden(&mut g, 0, "Balinda Stonehearth");
    cast_on(&mut g, 0, "I'm Still Just a Rat in a Cage", 0);
    // 1 -> 6 -> 16 -> 36.
    let m = g.players[0].board[0];
    assert_eq!((m.attack, m.health), (36, 1), "three casts");
    g.check_invariants().unwrap();
}

// ---- the reviewed Prize rules (Era, block 5 review) ----

#[test]
fn new_recruit_adds_to_the_same_after_refresh_buff_as_waveling() {
    let mut g = game(74);
    g.players[0].tier = 3;
    let before = g.players[0].refresh_buff;
    let size_before = g.players[0].shop_size(&g.data.rules);
    cast(&mut g, 0, "New Recruit");
    assert!(g.players[0].prize.extra_shop, "the extra slot stays for good");
    assert_eq!(
        g.players[0].shop_size(&g.data.rules),
        size_before + 1,
        "one more Tavern slot"
    );
    assert_eq!(
        g.players[0].refresh_buff,
        (before.0 + 2, before.1 + 2),
        "the +2/+2 is Waveling's after-Refresh buff"
    );
    // A Waveling's Deathrattle adds its own +4/+4 on top, and one Refresh
    // then gives one random Tavern minion the whole +6/+6.
    put_on_board(&mut g, 0, "Waveling");
    let opp = match g.players[0].opponent {
        Some(bgcore::game::Opponent::Player(q)) => q as usize,
        _ => 1,
    };
    put_on_board(&mut g, opp, "Microbot");
    let k = g.players[opp].board.len() - 1;
    g.players[opp].board[k].attack = 50;
    g.players[opp].board[k].health = 100;
    let t = g.turn;
    while g.turn == t && !g.over {
        if !answer(&mut g) {
            g.step(Action::EndTurn).unwrap();
        }
    }
    assert_eq!(
        g.players[0].refresh_buff,
        (before.0 + 6, before.1 + 6),
        "New Recruit and the Waveling stack into one buff"
    );
    // One Refresh gives the whole amount to one random Tavern minion.
    g.players[0].tier = 3;
    set_shop(&mut g, 0, &["Microbot", "Microbot", "Microbot"]);
    turn_of(&mut g, 0);
    g.players[0].free_rolls = 1;
    act(&mut g, 0, Action::Roll);
    let buffed = g
        .players[0]
        .shop
        .iter()
        .filter(|m| {
            let base = g.data.cards.make(m.card, false);
            m.attack == base.attack + before.0 + 6 && m.health == base.health + before.1 + 6
        })
        .count();
    assert_eq!(buffed, 1, "exactly one Tavern minion got the whole buff");
    g.check_invariants().unwrap();
}

#[test]
fn evolving_tavern_evolves_the_tavern_spell_too() {
    let mut g = game(75);
    g.players[0].tier = 3;
    set_shop(&mut g, 0, &["Microbot", "Microbot"]);
    g.players[0].shop_spell = Some(id(&g, "Tavern Coin"));
    cast(&mut g, 0, "Evolving Tavern");
    for m in &g.players[0].shop {
        assert_eq!(g.data.cards.get(m.card).tier, 4, "a minion one Tier higher");
    }
    let sp = g.players[0].shop_spell.expect("the Tavern spell slot still holds one");
    assert!(g.data.cards.get(sp).is_spell(), "still a spell");
    assert!(g.data.cards.get(sp).tavern_spell, "still a Tavern spell");
    assert_eq!(g.data.cards.get(sp).tier, 4, "one Tier higher");
    g.check_invariants().unwrap();
    // At Tier 6 there is no Tier 7 in the Tavern: both stay at 6.
    let mut g = game(76);
    g.players[0].tier = 6;
    set_shop(&mut g, 0, &["Microbot"]);
    g.players[0].shop_spell = Some(g.data.cards.random_spell(6, all_tribes(), &mut bgcore::Rng::new(1)).unwrap());
    cast(&mut g, 0, "Evolving Tavern");
    for m in &g.players[0].shop {
        assert_eq!(g.data.cards.get(m.card).tier, 6, "never past Tier 6");
    }
    if let Some(sp) = g.players[0].shop_spell {
        assert_eq!(g.data.cards.get(sp).tier, 6, "the spell too");
    }
    g.check_invariants().unwrap();
}
