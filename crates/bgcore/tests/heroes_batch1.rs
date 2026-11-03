//! Hero batch 1 (docs/briefs/heroes-batch1.md): the 28 gold and price,
//! counter, Discover and generate powers, one test each. Era's rules for
//! their non-obvious parts are in docs/rules/heroes.md "Hero batch rules
//! (Era, 2026-11-01)".

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

/// Player `p` answers the choice open for them with option `k`. Players shop
/// at the same time and the cursor moves on after every action, so the turn
/// has to be walked back to `p` first.
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

/// Whether player 0 may take `a` right now (the checks are asked of the
/// player whose turn it is, so walk to player 0 first).
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

// ---- gold and prices ----

#[test]
fn sharpen_blades_grows_a_step_for_every_four_cards_bought() {
    let mut g = hero_game(1, "Edwin VanCleef");
    assert_eq!(power(&g, 0).cost, 1);
    put_on_board(&mut g, 0, "Buzzing Vermin");
    g.players[0].gold = 10;
    // Two cards bought: still +2/+2.
    set_shop(&mut g, 0, &["Flittering Bat", "Southsea Busker"]);
    act(&mut g, 0, Action::Buy(0));
    act(&mut g, 0, Action::Buy(0));
    assert_eq!(g.players[0].counts.cards_bought, 2);
    press(&mut g, 0);
    let bat = g.players[0].board[0];
    assert_eq!((bat.attack, bat.health), (1 + 2, 1 + 2), "a Beast 1/1 plus +2/+2");
    // Four cards bought: one more of each.
    g.players[0].counts.cards_bought = 4;
    again(&mut g, 0);
    press(&mut g, 0);
    let bat = g.players[0].board[0];
    assert_eq!((bat.attack, bat.health), (3 + 3, 3 + 3), "+3/+3 at 4 cards");
    // Eight cards: +4/+4.
    g.players[0].counts.cards_bought = 8;
    again(&mut g, 0);
    press(&mut g, 0);
    let bat = g.players[0].board[0];
    assert_eq!((bat.attack, bat.health), (6 + 4, 6 + 4), "+4/+4 at 8 cards");
}

#[test]
fn tinker_discovers_a_magnetic_mech_up_to_the_tier_and_waits_for_tier_four() {
    let mut g = hero_game(2, "Millificent Manastorm");
    g.players[0].gold = 10;
    act(&mut g, 0, Action::Freeze);
    assert!(!legal_for_0(&mut g, Action::HeroPower), "locked below Tier 4");
    g.players[0].tier = 4;
    g.players[0].power.as_mut().unwrap().unlocked = true;
    act(&mut g, 0, Action::HeroPower);
    let picks: Vec<CardId> = g.players[0].discover.iter().copied().collect();
    assert!(!picks.is_empty());
    for c in picks {
        let card = g.data.cards.get(c);
        assert!(card.keywords.has(bgcore::Keywords::MAGNETIC), "{}", card.name);
        assert!(card.tribes.has(Tribe::Mech), "{}", card.name);
        assert!(card.tier <= 4);
    }
    assert_eq!(g.players[0].gold, 9, "the press costs 1");
}

#[test]
fn pirate_parrrrty_gives_a_pirate_and_a_discount_for_each_pirate_bought() {
    let mut g = hero_game(3, "Patches the Pirate");
    g.players[0].gold = 20;
    assert_eq!(g.power_cost(0, false), 3);
    act(&mut g, 0, Action::HeroPower);
    assert!(g.players[0].hand.iter().any(|m| {
        g.data.cards.tribes_of(m).has(Tribe::Pirate)
    }), "a Pirate arrived");
    // Every Pirate bought takes 1 off the next press, and they stack.
    set_shop(&mut g, 0, &["Southsea Busker", "Southsea Busker"]);
    act(&mut g, 0, Action::Buy(0));
    assert_eq!(g.power_cost(0, false), 2);
    set_shop(&mut g, 0, &["Southsea Busker"]);
    act(&mut g, 0, Action::Buy(0));
    assert_eq!(g.power_cost(0, false), 1);
    // A Beast bought takes nothing off.
    set_shop(&mut g, 0, &["Flittering Bat"]);
    act(&mut g, 0, Action::Buy(0));
    assert_eq!(g.power_cost(0, false), 1);
    // The press uses the discount up.
    let gold = g.players[0].gold;
    again(&mut g, 0);
    act(&mut g, 0, Action::HeroPower);
    assert_eq!(gold - g.players[0].gold, 1);
    assert_eq!(g.power_cost(0, false), 3);
}

#[test]
fn lead_explorer_discovers_its_own_tier_and_costs_more_every_press() {
    let mut g = hero_game(4, "Elise Starseeker");
    g.players[0].gold = 20;
    assert_eq!(g.power_cost(0, false), 1);
    act(&mut g, 0, Action::HeroPower);
    for &c in g.players[0].discover.iter() {
        assert_eq!(g.data.cards.get(c).tier, g.players[0].tier);
    }
    choose(&mut g, 0, 0);
    assert_eq!(g.power_cost(0, false), 2);
    again(&mut g, 0);
    act(&mut g, 0, Action::HeroPower);
    choose(&mut g, 0, 0);
    assert_eq!(g.power_cost(0, false), 3);
    // The growth lasts the game: next turn it is still 3.
    end_all_turns(&mut g);
    turn_of(&mut g, 0);
    assert_eq!(g.power_cost(0, false), 3);
}

#[test]
fn manastorm_sets_the_minion_and_refresh_prices_and_one_more_to_upgrade() {
    let mut g = hero_game(5, "Millhouse Manastorm");
    let base_upgrade = g.players[0].upgrade_cost;
    assert_eq!(g.upgrade_price(0), base_upgrade + 1);
    assert_eq!(g.refresh_cost(0), 2);
    g.players[0].gold = 10;
    set_shop(&mut g, 0, &["Flittering Bat"]);
    act(&mut g, 0, Action::Buy(0));
    assert_eq!(g.players[0].gold, 8, "a minion costs 2");
    act(&mut g, 0, Action::Roll);
    assert_eq!(g.players[0].gold, 6, "a Refresh costs 2");
    // A Tavern spell keeps its own cost.
    let coin = id(&g, "Tavern Coin");
    g.players[0].shop_spell = Some(coin);
    let gold = g.players[0].gold;
    act(&mut g, 0, Action::BuySpell);
    assert_eq!(gold - g.players[0].gold, g.data.cards.get(coin).cost);
    // Upgrading costs one more than the Tier's own price.
    g.players[0].gold = 20;
    act(&mut g, 0, Action::Level);
    assert_eq!(g.players[0].gold, 20 - base_upgrade - 1);
    assert_eq!(g.players[0].upgrade_cost, g.data.rules.upgrade_cost(2));
}

#[test]
fn piggy_bank_grows_one_gold_a_turn_and_pays_once_a_game() {
    let mut g = hero_game(6, "Skycap'n Kragg");
    g.players[0].gold = 0;
    assert!(legal_for_0(&mut g, Action::HeroPower), "a free button");
    act(&mut g, 0, Action::HeroPower);
    assert_eq!(g.players[0].gold, 2, "2 Gold on turn 1");
    assert!(!legal_for_0(&mut g, Action::HeroPower), "once a game");
    // Waiting is worth 1 Gold a turn: on turn 3 it pays 4.
    end_all_turns(&mut g);
    turn_of(&mut g, 0);
    end_all_turns(&mut g);
    turn_of(&mut g, 0);
    assert_eq!(g.turn, 3);
    let mut g2 = hero_game(6, "Skycap'n Kragg");
    g2.players[0].gold = 0;
    end_all_turns(&mut g2);
    turn_of(&mut g2, 0);
    end_all_turns(&mut g2);
    turn_of(&mut g2, 0);
    assert_eq!(g2.turn, 3);
    g2.players[0].gold = 0;
    act(&mut g2, 0, Action::HeroPower);
    assert_eq!(g2.players[0].gold, 4, "2 Gold plus one for each turn waited");
}

#[test]
fn the_perfect_crime_steals_the_whole_tavern_and_gets_cheaper_each_turn() {
    let mut g = hero_game(7, "Heistbaron Togwaggle");
    set_shop(&mut g, 0, &["Flittering Bat", "Buzzing Vermin", "Cord Puller"]);
    let spell = id(&g, "Tavern Coin");
    g.players[0].shop_spell = Some(spell);
    g.players[0].gold = 20;
    assert_eq!(g.power_cost(0, false), 11);
    act(&mut g, 0, Action::HeroPower);
    assert!(g.players[0].shop.is_empty(), "the Tavern is empty");
    assert!(g.players[0].shop_spell.is_none(), "the spell slot too");
    assert_eq!(g.players[0].hand.len(), 4, "three minions and the spell");
    // Each turn the next press costs 1 less, down to 0.
    end_all_turns(&mut g);
    turn_of(&mut g, 0);
    assert_eq!(g.power_cost(0, false), 10);
    for _ in 0..12 {
        end_all_turns(&mut g);
        turn_of(&mut g, 0);
        if g.turn >= g.data.rules.max_turns {
            break;
        }
    }
    assert_eq!(g.power_cost(0, false), 0, "never below 0");
    // The steal leaves the pool short of nothing: a stolen card keeps its
    // copy, so selling it gives one back.
    g.check_invariants().unwrap();
}

#[test]
fn reliquary_research_makes_every_third_tavern_spell_free() {
    let mut g = hero_game(8, "Tae'thelan Bloodwatcher");
    g.players[0].gold = 20;
    let coin = id(&g, "Tavern Coin");
    for k in 1..=3 {
        g.players[0].shop_spell = Some(coin);
        let gold = g.players[0].gold;
        act(&mut g, 0, Action::BuySpell);
        let paid = gold - g.players[0].gold;
        if k == 3 {
            assert_eq!(paid, 0, "the third Tavern spell is free");
        } else {
            assert_eq!(paid, g.data.cards.get(coin).cost, "spell {k} pays");
        }
    }
    assert_eq!(g.players[0].counts.tavern_spells_bought, 3);
    // The count starts over: the next two pay, the sixth is free.
    for k in 4..=6 {
        g.players[0].shop_spell = Some(coin);
        let gold = g.players[0].gold;
        act(&mut g, 0, Action::BuySpell);
        let paid = gold - g.players[0].gold;
        assert_eq!(paid == 0, k == 6, "spell {k}");
    }
}

#[test]
fn the_galaxys_lens_copies_the_last_tavern_spell_and_gets_cheaper() {
    let mut g = hero_game(9, "Farseer Nobundo");
    g.players[0].gold = 20;
    assert!(!legal_for_0(&mut g, Action::HeroPower), "no Tavern spell cast yet");
    // Cast a Tavern spell from hand: Fortify.
    put_in_hand(&mut g, 0, "Fortify");
    put_on_board(&mut g, 0, "Buzzing Vermin");
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    act(&mut g, 0, Action::Target(0));
    let hand = g.players[0].hand.len();
    assert_eq!(g.power_cost(0, false), 3);
    press(&mut g, 0);
    assert_eq!(g.players[0].hand.len(), hand + 1, "a copy of the spell");
    assert_eq!(g.players[0].hand.last().unwrap().card, id(&g, "Fortify"));
    // The price falls by 1 at the start of each turn, down to 0. It is a
    // start-of-turn change, so a Drakkari Enchanter doesn't repeat it (Era,
    // 2026-11-03).
    put_on_board(&mut g, 0, "Drakkari Enchanter");
    end_all_turns(&mut g);
    turn_of(&mut g, 0);
    assert_eq!(g.power_cost(0, false), 2, "one less, Drakkari or not");
    end_all_turns(&mut g);
    turn_of(&mut g, 0);
    assert_eq!(g.power_cost(0, false), 1);
    // A press puts it back to the full 3, and it starts falling again.
    g.players[0].gold = 20;
    press(&mut g, 0);
    assert_eq!(g.power_cost(0, false), 3, "a press resets the price");
    end_all_turns(&mut g);
    turn_of(&mut g, 0);
    assert_eq!(g.power_cost(0, false), 2);
}

// ---- counters ----

#[test]
fn battle_brand_hands_out_one_brann_for_four_battlecry_minions() {
    let mut g = hero_game(10, "Dinotamer Brann");
    g.players[0].gold = 40;
    let brann = id(&g, "Brann Bronzebeard");
    for _ in 0..3 {
        set_shop(&mut g, 0, &["Southsea Busker"]);
        act(&mut g, 0, Action::Buy(0));
    }
    assert!(!g.players[0].hand.iter().any(|m| m.card == brann));
    set_shop(&mut g, 0, &["Southsea Busker"]);
    act(&mut g, 0, Action::Buy(0));
    assert!(g.players[0].hand.iter().any(|m| m.card == brann), "Brann arrived");
    assert_eq!(g.players[0].counts.battlecry_bought, 4);
    // Once a game: four more Battlecry minions give nothing.
    let before = g.players[0].hand.iter().filter(|m| m.card == brann).count();
    g.players[0].gold = 40;
    for _ in 0..4 {
        set_shop(&mut g, 0, &["Southsea Busker"]);
        act(&mut g, 0, Action::Buy(0));
    }
    let after = g.players[0].hand.iter().filter(|m| m.card == brann).count();
    assert_eq!(before, after, "once per game");
}

#[test]
fn gone_fishing_gives_a_murloc_after_every_five_sales() {
    let mut g = hero_game(11, "Fungalmancer Flurgl");
    for _ in 0..5 {
        put_on_board(&mut g, 0, "Buzzing Vermin");
    }
    let hand = g.players[0].hand.len();
    for _ in 0..4 {
        act(&mut g, 0, Action::Sell(0));
    }
    assert_eq!(g.players[0].counts.minions_sold, 4);
    assert_eq!(g.players[0].hand.len(), hand, "four sales give nothing");
    act(&mut g, 0, Action::Sell(0));
    assert_eq!(g.players[0].hand.len(), hand + 1, "the fifth gives a Murloc");
    let got = g.players[0].hand.last().unwrap();
    assert!(g.data.cards.tribes_of(got).has(Tribe::Murloc));
}

#[test]
fn verdant_spheres_gives_a_tavern_coin_for_every_three_minions_bought() {
    let mut g = hero_game(12, "Kael'thas Sunstrider");
    g.players[0].gold = 40;
    let coin = id(&g, "Tavern Coin");
    for _ in 0..2 {
        set_shop(&mut g, 0, &["Flittering Bat"]);
        act(&mut g, 0, Action::Buy(0));
    }
    assert!(!g.players[0].hand.iter().any(|m| m.card == coin));
    set_shop(&mut g, 0, &["Flittering Bat"]);
    act(&mut g, 0, Action::Buy(0));
    assert!(g.players[0].hand.iter().any(|m| m.card == coin), "a Tavern Coin");
    assert_eq!(g.players[0].counts.minions_bought, 3);
    // A Tavern spell bought is not a minion: it does not count.
    g.players[0].shop_spell = Some(coin);
    act(&mut g, 0, Action::BuySpell);
    assert_eq!(g.players[0].counts.minions_bought, 3);
    assert_eq!(g.players[0].counts.cards_bought, 4);
}

#[test]
fn natural_balance_gives_a_triple_reward_for_twenty_tiers_bought() {
    let mut g = hero_game(13, "Guff Runetotem");
    g.players[0].gold = 40;
    // Nineteen Tiers in the count: a Tier 1 buy is the twentieth.
    g.players[0].power.as_mut().unwrap().progress = 19;
    let reward = id(&g, "Triple Reward");
    set_shop(&mut g, 0, &["Flittering Bat"]);
    act(&mut g, 0, Action::Buy(0));
    let got: Vec<_> = g.players[0].hand.iter().filter(|m| m.card == reward).collect();
    assert_eq!(got.len(), 1, "a Triple Reward");
    // Its Tier is the player's plus one.
    assert_eq!(got[0].improvements as u8 - 1, g.players[0].tier);
    assert_eq!(g.players[0].counts.tiers_bought, 1);
    assert_eq!(g.players[0].power.unwrap().progress, 0, "the count starts over");
}

#[test]
fn for_the_horde_buffs_the_tavern_one_more_step_for_every_three_minions() {
    let mut g = hero_game(14, "Overlord Saurfang");
    assert!(
        g.players[0].tavern_buffs.iter().any(|b| (b.attack, b.health) == (1, 1) && !b.this_turn),
        "the Tavern starts at +1/+1"
    );
    g.players[0].gold = 40;
    for _ in 0..2 {
        set_shop(&mut g, 0, &["Flittering Bat"]);
        act(&mut g, 0, Action::Buy(0));
    }
    let buff = |g: &Game| {
        g.players[0]
            .tavern_buffs
            .iter()
            .filter(|b| !b.this_turn)
            .fold((0, 0), |a, b| (a.0 + b.attack, a.1 + b.health))
    };
    assert_eq!(buff(&g), (1, 1), "two minions bought");
    set_shop(&mut g, 0, &["Flittering Bat"]);
    act(&mut g, 0, Action::Buy(0));
    assert_eq!(buff(&g), (2, 2), "the third minion improves it");
    // A Tavern spell does not count as a minion: two more minions leave it
    // at +2/+2, and the next three improve it again.
    g.players[0].shop_spell = Some(id(&g, "Tavern Coin"));
    act(&mut g, 0, Action::BuySpell);
    for _ in 0..2 {
        set_shop(&mut g, 0, &["Flittering Bat"]);
        act(&mut g, 0, Action::Buy(0));
    }
    assert_eq!(buff(&g), (2, 2), "five minions and a spell");
    set_shop(&mut g, 0, &["Flittering Bat"]);
    act(&mut g, 0, Action::Buy(0));
    assert_eq!(buff(&g), (3, 3), "it repeats");
}

#[test]
fn glaive_ricochet_copies_one_of_three_minions_once_a_turn() {
    let mut g = hero_game(15, "Kurtrus Ashfallen");
    g.players[0].gold = 40;
    let hand = g.players[0].hand.len();
    set_shop(&mut g, 0, &["Flittering Bat", "Buzzing Vermin", "Cord Puller"]);
    act(&mut g, 0, Action::Buy(0));
    act(&mut g, 0, Action::Buy(0));
    assert_eq!(g.players[0].hand.len(), hand + 2, "no copy after two");
    act(&mut g, 0, Action::Buy(0));
    assert_eq!(g.players[0].hand.len(), hand + 4, "the third buy copies one");
    // The copy is a plain one of the three, so a fourth hand card matches.
    let bought: Vec<CardId> = g.players[0].hand[hand..hand + 3].iter().map(|m| m.card).collect();
    let copy = g.players[0].hand[hand + 3];
    assert!(bought.contains(&copy.card));
    // Once a turn: three more buys give nothing.
    g.players[0].gold = 40;
    for name in ["Buzzing Vermin", "Cord Puller", "Lullabot"] {
        set_shop(&mut g, 0, &[name]);
        act(&mut g, 0, Action::Buy(0));
    }
    assert_eq!(g.players[0].hand.len(), hand + 7, "three buys, no second copy");
}

#[test]
fn avalanche_takes_three_off_the_upgrade_for_every_three_elementals() {
    let mut g = hero_game(16, "Chenvaala");
    let base = g.players[0].upgrade_cost;
    for _ in 0..2 {
        put_in_hand(&mut g, 0, "Crackling Cyclone");
    }
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    act(&mut g, 0, Action::Play { hand: 0, pos: 1 });
    assert_eq!(g.players[0].upgrade_cost, base, "two Elementals");
    put_in_hand(&mut g, 0, "Crackling Cyclone");
    act(&mut g, 0, Action::Play { hand: 0, pos: 2 });
    assert_eq!(g.players[0].upgrade_cost, base - 3, "3 off the upgrade");
    // Another three Elementals stack, and the price never goes below 0.
    for _ in 0..3 {
        put_in_hand(&mut g, 0, "Crackling Cyclone");
        let at = (g.players[0].hand.len() - 1) as u8;
        let pos = g.players[0].board.len() as u8;
        act(&mut g, 0, Action::Play { hand: at, pos });
    }
    assert_eq!(g.players[0].upgrade_cost, 0);
}

#[test]
fn heroic_inspiration_gives_a_triple_reward_every_fifteen_attacks() {
    let mut g = hero_game(17, "Loh, the Living Legend");
    duel(&mut g);
    put_on_board(&mut g, 0, "Buzzing Vermin");
    put_on_board(&mut g, 1, "Flittering Bat");
    // Fourteen attacks in: the next friendly attack is the fifteenth.
    g.players[0].power.as_mut().unwrap().progress = 14;
    end_all_turns(&mut g);
    let reward = id(&g, "Triple Reward");
    assert!(
        g.players[0].hand.iter().any(|m| m.card == reward) || !g.players[0].alive,
        "15 attacks give a Triple Reward: {} attacks, progress {}",
        g.players[0].counts.attacks,
        g.players[0].power.map_or(0, |st| st.progress)
    );
    assert!(g.players[0].counts.attacks >= 1, "attacks {}", g.players[0].counts.attacks);
}

// ---- Discover and generate ----

#[test]
fn buried_treasure_digs_three_times_then_gives_a_golden_minion() {
    let mut g = hero_game(18, "Captain Eudora");
    g.players[0].gold = 40;
    let hand = g.players[0].hand.len();
    for _ in 0..3 {
        again(&mut g, 0);
        act(&mut g, 0, Action::HeroPower);
    }
    assert_eq!(g.players[0].hand.len(), hand, "three digs give nothing");
    again(&mut g, 0);
    act(&mut g, 0, Action::HeroPower);
    assert_eq!(g.players[0].hand.len(), hand + 1, "the fourth digs up a minion");
    let dug = *g.players[0].hand.last().unwrap();
    assert!(dug.golden);
    assert!(g.data.cards.get(dug.card).tier <= g.players[0].tier);
    assert_eq!(g.players[0].power.unwrap().progress, 0, "the digging starts over");
}

#[test]
fn trash_for_treasure_removes_a_minion_and_discovers_one_tier_lower() {
    let mut g = hero_game(19, "Captain Hooktusk");
    // A Tier 3 minion: the Discover is Tier 2.
    let tier3 = "Accord-o-Tron";
    assert_eq!(g.data.cards.get(id(&g, tier3)).tier, 3);
    put_on_board(&mut g, 0, tier3);
    let slot = g.data.cards.get(id(&g, tier3)).pool_slot as usize;
    let copies = g.pool[slot];
    let hand = g.players[0].hand.len();
    press(&mut g, 0);
    assert!(g.players[0].board.is_empty(), "the minion is gone");
    assert_eq!(g.pool[slot], copies + 1, "its pool copy came back");
    assert_eq!(g.players[0].hand.len(), hand, "removing is not a sale");
    assert!(!g.players[0].discover.is_empty(), "a Discover opened");
    for &c in g.players[0].discover.iter() {
        assert_eq!(g.data.cards.get(c).tier, 2, "one Tier below the removed 3");
    }
    // On a Tier 1 minion it Discovers another Tier 1.
    choose(&mut g, 0, 0);
    clear_board(&mut g, 0);
    put_on_board(&mut g, 0, "Buzzing Vermin");
    again(&mut g, 0);
    press(&mut g, 0);
    for &c in g.players[0].discover.iter() {
        assert_eq!(g.data.cards.get(c).tier, 1);
    }
}

#[test]
fn swap_lock_and_shop_it_trades_a_minion_with_the_tavern() {
    let mut g = hero_game(20, "Jandice Barov");
    put_on_board(&mut g, 0, "Buzzing Vermin");
    g.players[0].board[0].attack += 5;
    g.players[0].board[0].health += 5;
    set_shop(&mut g, 0, &["Cord Puller"]);
    let mine = g.players[0].board[0];
    let theirs = g.players[0].shop[0];
    press(&mut g, 0);
    assert_eq!(g.players[0].board[0].card, theirs.card, "the Tavern minion took its place");
    assert_eq!(g.players[0].shop.len(), 1);
    assert_eq!(g.players[0].shop[0].card, mine.card, "my minion is in the Tavern");
    assert_eq!(g.players[0].shop[0].attack, mine.attack, "buffs kept");
    assert_eq!(g.players[0].shop[0].health, mine.health);
    g.check_invariants().unwrap();
}

#[test]
fn three_wishes_finds_the_third_copy_three_times_a_game() {
    let mut g = hero_game(21, "Zephrys, the Great");
    g.players[0].gold = 30;
    assert!(!legal_for_0(&mut g, Action::HeroPower), "no pair to finish");
    put_in_hand(&mut g, 0, "Flittering Bat");
    put_in_hand(&mut g, 0, "Flittering Bat");
    assert!(legal_for_0(&mut g, Action::HeroPower));
    act(&mut g, 0, Action::HeroPower);
    // The third copy makes the triple: one Golden Flittering Bat in hand.
    let golden: Vec<Minion> = g
        .players[0]
        .hand
        .iter()
        .filter(|m| m.card == id(&g, "Flittering Bat") && m.golden)
        .copied()
        .collect();
    assert_eq!(golden.len(), 1, "the pair became golden");
    assert_eq!(g.players[0].power.unwrap().used_game, 1);
    // Three wishes a game: the fourth press is illegal.
    for k in 2..=3 {
        put_in_hand(&mut g, 0, "Cord Puller");
        put_in_hand(&mut g, 0, "Cord Puller");
        g.players[0].gold = 30;
        again(&mut g, 0);
        act(&mut g, 0, Action::HeroPower);
        assert_eq!(g.players[0].power.unwrap().used_game, k);
    }
    put_in_hand(&mut g, 0, "Cord Puller");
    put_in_hand(&mut g, 0, "Cord Puller");
    g.players[0].gold = 30;
    again(&mut g, 0);
    assert!(!legal_for_0(&mut g, Action::HeroPower), "three wishes only");
}

#[test]
fn stir_the_pot_discovers_one_minion_per_thrown_type() {
    let mut g = hero_game(22, "Cookie the Cook");
    // A neutral minion has no type, so it cannot be thrown: with one on the
    // board and nothing else the press is not even legal.
    put_on_board(&mut g, 0, "Suspicious Prisonguard");
    set_shop(&mut g, 0, &[]);
    g.players[0].gold = 20;
    assert!(!legal_for_0(&mut g, Action::HeroPower), "a neutral minion cannot be thrown");
    // With two typed minions beside it the press is legal and the pick stays
    // open, and the neutral one is no target.
    put_on_board(&mut g, 0, "Wrath Weaver");
    put_on_board(&mut g, 0, "Buzzing Vermin");
    assert!(legal_for_0(&mut g, Action::HeroPower));
    act(&mut g, 0, Action::HeroPower);
    let neutral = g.players[0]
        .board
        .iter()
        .position(|m| g.data.cards.tribes_of(m).is_neutral())
        .expect("the neutral minion is on the board");
    assert!(
        !legal_for_0(&mut g, Action::Target(neutral as u8)),
        "the neutral minion is no target"
    );
    assert!(legal_for_0(&mut g, Action::Target(1)), "the Demon is");
    act(&mut g, 0, Action::Target(1));
    assert_eq!(g.players[0].pot.len(), 1, "the Demon's type went in");
    // The pot starts over for the three throws: a Demon, a Beast and a Mech.
    clear_board(&mut g, 0);
    g.players[0].pot.clear();
    g.players[0].power.as_mut().unwrap().progress = 0;
    set_shop(&mut g, 0, &["Wrath Weaver", "Flittering Bat", "Cord Puller"]);
    for k in 0..3 {
        again(&mut g, 0);
        press(&mut g, 0);
        // The third throw empties the pot into the Discover.
        let want = if k < 2 { k + 1 } else { 0 };
        assert_eq!(g.players[0].pot.len(), want, "the pot collects the types");
        assert_eq!(g.players[0].shop.len(), 2 - k, "the thrown minion is gone");
    }
    let picks: Vec<CardId> = g.players[0].discover.iter().copied().collect();
    assert_eq!(picks.len(), 3, "one option per thrown type");
    // A Demon, a Beast and a Mech were thrown, so there is one of each.
    for (k, c) in picks.iter().enumerate() {
        let t = g.data.cards.get(*c).tribes;
        let want = [Tribe::Demon, Tribe::Beast, Tribe::Mech][k];
        assert!(t.has(want), "option {k} of the {}", g.data.cards.get(*c).name);
    }
    g.check_invariants().unwrap();
}

#[test]
fn reclaimed_souls_discovers_from_the_dead_of_the_last_fight() {
    let mut g = hero_game(23, "Sylvanas Windrunner");
    duel(&mut g);
    g.players[0].tier = 4;
    g.players[0].power.as_mut().unwrap().unlocked = true;
    put_on_board(&mut g, 0, "Buzzing Vermin");
    put_on_board(&mut g, 1, "Buzzing Vermin");
    g.players[1].board[0].attack = 1;
    end_all_turns(&mut g);
    turn_of(&mut g, 0);
    end_all_turns(&mut g);
    let dead = g.players[0].dead_last_fight.clone();
    assert!(!dead.is_empty(), "the fight's dead are recorded");
    assert!(
        dead.iter().any(|&c| c == id(&g, "Buzzing Vermin")),
        "the other side's dead count too"
    );
    turn_of(&mut g, 0);
    g.players[0].gold = 10;
    assert!(legal_for_0(&mut g, Action::HeroPower));
    act(&mut g, 0, Action::HeroPower);
    let picks: Vec<CardId> = g.players[0].discover.iter().copied().collect();
    assert!(!picks.is_empty());
    for c in picks {
        assert!(dead.contains(&c), "every offer died in that fight");
    }
    // The pick is a plain copy: printed stats, whatever it died with.
    choose(&mut g, 0, 0);
    let got = *g.players[0].hand.last().unwrap();
    let printed = g.data.cards.make(got.card, false);
    assert_eq!((got.attack, got.health), (printed.attack, printed.health));
}

#[test]
fn kelthuzads_kitty_discovers_one_warband_per_dead_hero() {
    let mut g = hero_game(24, "Mr. Bigglesworth");
    // Two heroes die in the same round: 1 fights me, 2 fights 3. 4..7 are out
    // and 3 stays alive, so the game goes on.
    for p in 4..8 {
        g.players[p].alive = false;
        g.players[p].placement = 3 + (p as u8 - 2);
        g.players[p].done = true;
    }
    for (me, them) in [(0usize, 1usize), (1, 0), (2, 3), (3, 2)] {
        g.players[me].opponent = Some(Opponent::Player(them as u8));
    }
    for p in [1, 2] {
        g.players[p].health = 1;
        g.players[p].armor = 0;
    }
    // Their warbands: one minion each, and they lose their fights.
    put_on_board(&mut g, 1, "Cord Puller");
    put_on_board(&mut g, 2, "Lullabot");
    for p in [0, 3] {
        put_on_board(&mut g, p, "Buzzing Vermin");
        for m in &mut g.players[p].board {
            m.attack = 50;
            m.health = 50;
        }
    }
    end_all_turns(&mut g);
    assert!(!g.players[1].alive && !g.players[2].alive, "both heroes died");
    assert!(g.players[0].alive && g.players[3].alive);
    // Both warbands waited for my next turn, and the first Discover is open.
    turn_of(&mut g, 0);
    let first: Vec<CardId> = g.players[0].discover_minions.iter().map(|m| m.card).collect();
    assert_eq!(first.len(), 1, "one minion in that warband");
    assert_eq!(g.players[0].discover_queue.len(), 1, "the other one is queued");
    // An exact copy, no pool copy moved.
    let before: u32 = g.pool.iter().map(|&n| n as u32).sum();
    choose(&mut g, 0, 0);
    let after: u32 = g.pool.iter().map(|&n| n as u32).sum();
    assert_eq!(before, after, "a fresh copy takes no pool copy");
    // The second dead hero's warband comes right after it.
    let second: Vec<CardId> = g.players[0].discover_minions.iter().map(|m| m.card).collect();
    assert_eq!(second.len(), 1, "the second warband");
    let mut seen = first.clone();
    seen.extend(second.iter().copied());
    let want = [id(&g, "Cord Puller"), id(&g, "Lullabot")];
    for c in &want {
        assert!(seen.contains(c), "warband card {c} offered: {seen:?}");
    }
    choose(&mut g, 0, 0);
    assert!(g.players[0].discover_minions.is_empty(), "no third warband");
    assert!(g.players[0].discover_queue.is_empty());
    g.check_invariants().unwrap();
}

#[test]
fn upbeat_harmony_copies_the_leftmost_card_every_three_turns() {
    let mut g = hero_game(25, "Rock Master Voone");
    put_in_hand(&mut g, 0, "Flittering Bat");
    put_in_hand(&mut g, 0, "Cord Puller");
    let hand = g.players[0].hand.len();
    end_all_turns(&mut g);
    turn_of(&mut g, 0);
    end_all_turns(&mut g);
    turn_of(&mut g, 0);
    assert_eq!(g.players[0].hand.len(), hand, "two turns, nothing yet");
    end_all_turns(&mut g);
    turn_of(&mut g, 0);
    assert_eq!(g.players[0].hand.len(), hand + 1, "the third turn copies one");
    assert_eq!(g.players[0].hand[0].card, id(&g, "Flittering Bat"));
    assert_eq!(g.players[0].hand.last().unwrap().card, id(&g, "Flittering Bat"));
}

#[test]
fn choose_your_champion_sits_in_hand_locked_until_sixty_gold() {
    let mut g = hero_game(26, "Thorim, Stormlord");
    // The Discover opens at the pick, three Tier 7 minions.
    assert!(!g.players[0].discover.is_empty(), "a Tier 7 Discover");
    for &c in g.players[0].discover.iter() {
        assert_eq!(g.data.cards.get(c).tier, 7);
    }
    let pick = g.players[0].discover[0];
    choose(&mut g, 0, 0);
    // The pick is a real card in hand, locked until 60 Gold is spent.
    let slot = g.players[0]
        .hand
        .iter()
        .position(|m| m.card == pick)
        .expect("the pick is in hand");
    assert_eq!(g.players[0].hand[slot].locked_gold, 60);
    assert!(g.players[0].hand[slot].locked_in_hand(g.players[0].tier, 0));
    assert!(
        !legal_for_0(&mut g, Action::Play { hand: slot as u8, pos: 0 }),
        "locked in hand"
    );
    // 59 Gold spent is not enough.
    g.players[0].counts.gold_spent = 59;
    assert!(
        !legal_for_0(&mut g, Action::Play { hand: slot as u8, pos: 0 }),
        "59 Gold is not 60"
    );
    // The 60th unlocks it, and it plays like any minion.
    g.players[0].counts.gold_spent = 60;
    assert!(
        legal_for_0(&mut g, Action::Play { hand: slot as u8, pos: 0 }),
        "60 Gold unlocks it"
    );
    act(&mut g, 0, Action::Play { hand: slot as u8, pos: 0 });
    assert_eq!(g.players[0].board.last().map(|m| m.card), Some(pick));
}

#[test]
fn puzzle_box_casts_a_tavern_spell_from_turn_three() {
    let mut g = hero_game(27, "Yogg-Saron, Hope's End");
    clear_board(&mut g, 0);
    g.players[0].hand.clear();
    assert!(g.players[0].last_spell.is_none(), "nothing on turns 1 and 2");
    end_all_turns(&mut g);
    turn_of(&mut g, 0);
    assert_eq!(g.turn, 2);
    assert!(g.players[0].last_spell.is_none(), "the power unlocks on turn 3");
    end_all_turns(&mut g);
    turn_of(&mut g, 0);
    assert_eq!(g.turn, 3);
    let spell = g.players[0].last_spell.expect("Puzzle Box cast a spell");
    assert!(g.data.cards.get(spell).tavern_spell, "a Tavern spell");
    assert!(g.data.cards.get(spell).tier <= g.players[0].tier);
    // It counts as a Tavern spell cast.
    assert!(g.players[0].spells_cast > 0);
}

#[test]
fn lucky_roll_pays_the_die_and_locks_the_power_for_that_many_turns() {
    // A few seeds, so the die is not always the same number.
    for seed in 0..5 {
        let mut g = hero_game(seed + 30, "Snake Eyes");
        g.players[0].gold = 1;
        act(&mut g, 0, Action::HeroPower);
        let paid = g.players[0].gold;
        assert!((1..=6).contains(&paid), "a six-sided die: {paid}");
        assert_eq!(g.players[0].power.unwrap().progress, paid as u32, "locked for as many turns as rolled");
        assert_eq!(g.players[0].power.unwrap().uses_left, 0);
        // The button stays locked for exactly `rolled` turns, then comes
        // back with the Gold for it.
        let mut turns_off = 0u32;
        loop {
            end_all_turns(&mut g);
            turn_of(&mut g, 0);
            g.players[0].gold = 10;
            // The button itself, not `is_legal`: a trinket offer stops that
            // one and has nothing to do with the die.
            if g.power_usable(0) {
                break;
            }
            turns_off += 1;
            assert!(turns_off <= 6, "never locked forever");
        }
        assert_eq!(turns_off, paid as u32, "locked for that many turns: roll {paid}");
    }
}

// ---- the observation ----

#[test]
fn the_observation_shows_each_powers_count_and_what_is_left() {
    let mut out = vec![0f32; bgcore::obs::OBS_SIZE];
    // Sharpen Blades is a growing one: the level it reached and the cards to
    // the next step.
    let mut g = hero_game(70, "Edwin VanCleef");
    g.players[0].counts.cards_bought = 6;
    turn_of(&mut g, 0);
    bgcore::obs::write(&g, 0, &mut out);
    assert_eq!(out[bgcore::obs::SCALAR_POWER_PROGRESS], 1.0, "six cards is level 1");
    assert_eq!(out[bgcore::obs::SCALAR_POWER_LEFT], 2.0, "two cards to the next level");
    // Verdant Spheres counts up to its third minion.
    let mut g = hero_game(71, "Kael'thas Sunstrider");
    g.players[0].power.as_mut().unwrap().progress = 2;
    turn_of(&mut g, 0);
    bgcore::obs::write(&g, 0, &mut out);
    assert_eq!(out[bgcore::obs::SCALAR_POWER_PROGRESS], 2.0);
    assert_eq!(out[bgcore::obs::SCALAR_POWER_LEFT], 1.0);
    // Buried Treasure's digs, and a power with no count reading zero.
    let mut g = hero_game(72, "Captain Eudora");
    g.players[0].power.as_mut().unwrap().progress = 3;
    turn_of(&mut g, 0);
    bgcore::obs::write(&g, 0, &mut out);
    assert_eq!(out[bgcore::obs::SCALAR_POWER_PROGRESS], 3.0);
    assert_eq!(out[bgcore::obs::SCALAR_POWER_LEFT], 1.0, "one dig to the Golden minion");
    let mut g = hero_game(73, "George the Fallen");
    turn_of(&mut g, 0);
    bgcore::obs::write(&g, 0, &mut out);
    assert_eq!(out[bgcore::obs::SCALAR_POWER_PROGRESS], 0.0);
    assert_eq!(out[bgcore::obs::SCALAR_POWER_LEFT], 0.0);
    assert_eq!(out[bgcore::obs::SCALAR_POWER2_PROGRESS], 0.0, "no second power");
    // Choose Your Champion: the Gold spent and the Gold to go, and the Tier 7
    // pick sitting in hand marked as locked until it is all in.
    let mut g = hero_game(74, "Thorim, Stormlord");
    let card = g.players[0].discover[0];
    choose(&mut g, 0, 0);
    // The pick is the first card in hand.
    let slot = g.players[0].hand.iter().position(|m| m.card == card).expect("in hand");
    g.players[0].counts.gold_spent = 20;
    turn_of(&mut g, 0);
    bgcore::obs::write(&g, 0, &mut out);
    assert_eq!(out[bgcore::obs::SCALAR_POWER_PROGRESS], 20.0);
    assert_eq!(out[bgcore::obs::SCALAR_POWER_LEFT], 40.0);
    let at = bgcore::obs::HAND_START + slot * bgcore::obs::SLOT_FEATURES;
    assert_eq!(out[at + bgcore::obs::SLOT_CARD_ID] as usize, card as usize + 1);
    assert_eq!(out[at + bgcore::obs::SLOT_LOCKED], 1.0, "locked in hand");
    // The 60th Gold unlocks it.
    g.players[0].counts.gold_spent = 60;
    bgcore::obs::write(&g, 0, &mut out);
    assert_eq!(out[bgcore::obs::SCALAR_POWER_PROGRESS], 60.0);
    assert_eq!(out[bgcore::obs::SCALAR_POWER_LEFT], 0.0);
    assert_eq!(out[at + bgcore::obs::SLOT_LOCKED], 0.0, "unlocked");
}
