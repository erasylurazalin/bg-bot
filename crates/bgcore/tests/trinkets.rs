//! The trinket frame and the built trinkets: the two offers (with Marin's and
//! Buttons' power offers), the guarantee and the discount, the slots, the
//! hooks (start of turn, every N turns, events, Start of Combat) and the
//! cards themselves, grouped by the mechanic they use.

use std::sync::Arc;

use bgcore::fight::fight_with;
use bgcore::trinkets::TrinketDb;
use bgcore::{Action, CardId, Data, Game, Minion, Rng, Tribe, Tribes};

fn data() -> Arc<Data> {
    Arc::new(Data::bundled())
}

fn all_tribes() -> Tribes {
    Tribe::EVERY.into_iter().fold(Tribes::NONE, |m, t| m.with(t))
}

/// A hero-less game with every tribe in the lobby.
fn game(seed: u64) -> Game {
    Game::new_with_lobby(data(), seed, all_tribes())
}

fn game_lobby(seed: u64, tribes: &[Tribe]) -> Game {
    let lobby = tribes.iter().copied().fold(Tribes::NONE, |m, t| m.with(t));
    Game::new_with_lobby(data(), seed, lobby)
}

fn id(g: &Game, name: &str) -> CardId {
    g.data.cards.by_name(name).unwrap_or_else(|| panic!("{name}"))
}

fn trinket(g: &Game, name: &str) -> u8 {
    g.data.trinkets.by_name(name).unwrap_or_else(|| panic!("{name} is not a built trinket"))
}

/// Answer whatever the current player has open, so a helper that only wants
/// "the next turn" never stalls on a prompt (a trinket of theirs can open a
/// Discover, and a trinket offer blocks the shopping actions). Returns false
/// when nothing is open.
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
    g.step(a).unwrap_or_else(|_| panic!("{a:?} illegal for player {p}"));
}

fn next_turn(g: &mut Game) {
    let t = g.turn;
    while g.turn == t && !g.over {
        if !answer(g) {
            g.step(Action::EndTurn).unwrap();
        }
    }
}

/// End turns (keeping everyone alive) until `turn` starts.
fn to_turn(g: &mut Game, turn: u32) {
    while g.turn < turn && !g.over {
        for pl in &mut g.players {
            pl.health = pl.health.max(30);
        }
        next_turn(g);
    }
}

/// The pool copies a card taken by a test holds: one, or none for a token
/// (a token is generated, so the pool is not touched).
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

/// Put a pool minion in hand (a second Murloc for the guarantee, say).
fn put_in_hand(g: &mut Game, p: usize, name: &str) {
    let c = id(g, name);
    let n = pool_copies(g, c);
    let m = g.players[p].make_card(&g.data.cards, c, false, n);
    g.players[p].hand.push(m);
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

/// `equip` by card id, for the pairs that share a name.
fn equip_id(g: &mut Game, p: usize, slot: usize, card_id: &str) {
    let i = g.data.trinkets.by_card_id(card_id).unwrap_or_else(|| panic!("{card_id}"));
    equip_index(g, p, slot, i);
}

fn equip_index(g: &mut Game, p: usize, slot: usize, i: u8) {
    g.set_trinket(p, slot, Some(i));
}

/// Put a trinket in a slot as if it had been picked (its on-pick effects do
/// not run, so a test can hold a card and see what it does later).
fn equip(g: &mut Game, p: usize, slot: usize, name: &str) {
    let i = trinket(g, name);
    equip_index(g, p, slot, i);
}

/// The slot a trinket belongs in (0 lesser, 1 greater).
fn slot_of(g: &Game, name: &str) -> usize {
    match g.data.trinkets.get(trinket(g, name)).slot {
        bgcore::trinkets::Slot::Lesser => 0,
        bgcore::trinkets::Slot::Greater => 1,
    }
}

/// Pick the named trinket through the real path: its offer is overwritten
/// (free) and answered, so the pick runs what the card does on the pick.
fn take(g: &mut Game, p: usize, name: &str) {
    let i = trinket(g, name);
    turn_of(g, p);
    let slot = slot_of(g, name);
    g.players[p].trinket_offer.clear();
    g.players[p].trinket_offer_slot = slot as u8 + 1;
    let _ = g.players[p].trinket_offer.try_push((i, 0));
    g.step(Action::ChooseTrinket(0)).unwrap_or_else(|_| panic!("could not take {name}"));
}

/// The Gold a trinket's pick pays out (its on-pick `gold` acts).
fn pick_gold(g: &Game, of: u8) -> u8 {
    g.data
        .trinkets
        .get(of)
        .on_pick
        .iter()
        .map(|a| match a {
            bgcore::trinkets::Act::Gold(n) => *n,
            _ => 0,
        })
        .sum()
}

/// Whether a trinket replaces itself on the pick (Orb of the Unknown), so
/// the slot ends up holding a different one.
fn replaces_itself(g: &Game, of: u8) -> bool {
    g.data
        .trinkets
        .get(of)
        .on_pick
        .iter()
        .any(|a| matches!(a, bgcore::trinkets::Act::ReplaceWithRandom { .. }))
}

fn hand_of(g: &Game, p: usize, name: &str) -> usize {
    let c = id(g, name);
    g.players[p].hand.iter().filter(|m| m.card == c).count()
}

// ---- the offer ----

#[test]
fn the_offers_come_on_turn_6_and_9_with_four_distinct_options() {
    let mut g = game(1);
    to_turn(&mut g, 5);
    for p in 0..8 {
        assert!(g.players[p].trinket_offer.is_empty(), "nothing before turn 6");
    }
    to_turn(&mut g, 6);
    turn_of(&mut g, 0);
    let offer: Vec<(u8, u8)> = g.players[0].trinket_offer.to_vec();
    assert_eq!(offer.len(), 4, "four options");
    assert_eq!(g.players[0].trinket_offer_slot, 1, "the lesser slot");
    let mut ids: Vec<u8> = offer.iter().map(|&(of, _)| of).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 4, "four different trinkets");
    for &(of, _) in &offer {
        assert_eq!(g.data.trinkets.get(of).slot, bgcore::trinkets::Slot::Lesser);
    }
    // The shopping actions wait for the answer; passing does not.
    assert!(!g.is_legal(Action::Roll));
    assert!(!g.is_legal(Action::Buy(0)));
    assert!(g.is_legal(Action::Freeze));
    assert!(g.is_legal(Action::EndTurn));
    // The price is paid at the pick (an option that pays Gold back, or one
    // that replaces itself, is left for the counts above).
    let k = (0..offer.len())
        .find(|&k| !replaces_itself(&g, offer[k].0))
        .expect("an option that keeps the slot");
    let (of, price) = offer[k];
    let gold = g.players[0].gold;
    act(&mut g, 0, Action::ChooseTrinket(k as u8));
    assert_eq!(
        g.players[0].gold,
        gold.saturating_sub(price).saturating_add(pick_gold(&g, of)),
        "the price was paid"
    );
    assert_eq!(g.players[0].trinkets[0].of, of + 1, "the lesser slot holds it");
    assert!(g.players[0].trinket_offer.is_empty());
    // Not offered again on turn 7.
    to_turn(&mut g, 7);
    turn_of(&mut g, 0);
    assert!(g.players[0].trinket_offer.is_empty());
    // The greater one on turn 9.
    to_turn(&mut g, 9);
    turn_of(&mut g, 0);
    assert_eq!(g.players[0].trinket_offer.len(), 4);
    assert_eq!(g.players[0].trinket_offer_slot, 2);
    for &(of, _) in &g.players[0].trinket_offer {
        assert_eq!(g.data.trinkets.get(of).slot, bgcore::trinkets::Slot::Greater);
    }
    g.check_invariants().unwrap();
}

#[test]
fn an_unanswered_offer_is_bought_on_the_way_out() {
    let mut g = game(2);
    to_turn(&mut g, 6);
    turn_of(&mut g, 0);
    // Two known options, so the cheapest is not a matter of the draw.
    let dear = trinket(&g, "Frigid Blossom");
    let cheap = trinket(&g, "Bob's Tip Jar");
    g.players[0].trinket_offer.clear();
    g.players[0].trinket_offer_slot = 1;
    let _ = g.players[0].trinket_offer.try_push((dear, 2));
    let _ = g.players[0].trinket_offer.try_push((cheap, 1));
    g.players[0].gold = 10;
    act(&mut g, 0, Action::EndTurn);
    assert_eq!(g.players[0].trinkets[0].of, cheap + 1, "the cheapest, bought");
    assert_eq!(
        g.players[0].gold,
        10 - 1 + pick_gold(&g, cheap),
        "its price, and the 4 Gold it pays"
    );
}

#[test]
fn marin_offers_a_lesser_trinket_on_turn_5_and_buttons_a_greater_one_on_turn_8() {
    // Marin: a power offer one turn before the normal lesser one, and the
    // pick turns the power into the trinket.
    let d = data();
    let mut g = Game::new_with_lobby(d.clone(), 3, all_tribes());
    let marin = d.heroes.by_name("Marin the Manager").unwrap();
    g.assign_hero(0, marin);
    to_turn(&mut g, 5);
    turn_of(&mut g, 0);
    assert_eq!(g.players[0].trinket_offer.len(), 4);
    assert_eq!(g.players[0].trinket_offer_slot, 3, "the hero power's offer");
    let power_before = g.players[0].power.unwrap();
    act(&mut g, 0, Action::ChooseTrinket(0));
    assert_ne!(g.players[0].trinkets[2].of, 0, "the power became a trinket");
    assert!(!g.power_usable(0), "the power is spent");
    assert_eq!(g.players[0].power.unwrap().of, power_before.of, "the hero is still Marin");
    // The normal turn 6 lesser offer still comes.
    to_turn(&mut g, 6);
    turn_of(&mut g, 0);
    assert_eq!(g.players[0].trinket_offer_slot, 1);
    assert_eq!(g.players[0].trinket_offer.len(), 4);

    // Buttons: the same at turn 8 with a greater offer.
    let mut g = Game::new_with_lobby(data(), 4, all_tribes());
    let buttons = g.data.heroes.by_name("Buttons").unwrap();
    g.assign_hero(0, buttons);
    to_turn(&mut g, 8);
    turn_of(&mut g, 0);
    assert_eq!(g.players[0].trinket_offer_slot, 3);
    for &(of, _) in &g.players[0].trinket_offer {
        assert_eq!(g.data.trinkets.get(of).slot, bgcore::trinkets::Slot::Greater);
    }
    g.check_invariants().unwrap();
}

#[test]
fn the_guarantee_holds_for_the_board_and_for_murlocs_in_hand() {
    // Two Pirates on the board: the offer holds a Pirate trinket.
    for seed in 0..12 {
        let mut g = game(seed);
        put_on_board(&mut g, 0, "Southsea Busker");
        put_on_board(&mut g, 0, "Bilgewater Breakout");
        to_turn(&mut g, 6);
        turn_of(&mut g, 0);
        assert!(
            g.players[0].trinket_offer.iter().any(|&(of, _)| {
                g.data.trinkets.get(of).tribes.has(Tribe::Pirate)
            }),
            "seed {seed}: no Pirate trinket in the offer"
        );
    }
    // One Murloc on the board and one in hand is 2: the offer holds a Murloc
    // trinket (the only lesser one built is Nerglish Phrasebook).
    for seed in 0..12 {
        let mut g = game_lobby(seed, &[Tribe::Murloc, Tribe::Beast]);
        put_on_board(&mut g, 0, "Flighty Scout");
        put_in_hand(&mut g, 0, "Bream Counter");
        to_turn(&mut g, 6);
        turn_of(&mut g, 0);
        assert!(
            g.players[0].trinket_offer.iter().any(|&(of, _)| {
                g.data.trinkets.get(of).tribes.has(Tribe::Murloc)
            }),
            "seed {seed}: a Murloc in hand did not count"
        );
        g.check_invariants().unwrap();
    }
    // One Murloc only (the hand is empty): no guarantee, just the weights.
    let mut g = game_lobby(5, &[Tribe::Murloc, Tribe::Beast]);
    put_on_board(&mut g, 0, "Flighty Scout");
    to_turn(&mut g, 6);
    turn_of(&mut g, 0);
    assert_eq!(g.players[0].trinket_offer.len(), 4);
}

#[test]
fn one_of_two_qualifying_tribes_is_guaranteed() {
    for seed in 0..12 {
        let mut g = game(seed);
        put_on_board(&mut g, 0, "Southsea Busker");
        put_on_board(&mut g, 0, "Bilgewater Breakout");
        put_on_board(&mut g, 0, "Scarlet Survivor");
        put_on_board(&mut g, 0, "Bronze Warden");
        to_turn(&mut g, 6);
        turn_of(&mut g, 0);
        let hit = g.players[0].trinket_offer.iter().any(|&(of, _)| {
            let t = g.data.trinkets.get(of);
            t.tribes.has(Tribe::Pirate) || t.tribes.has(Tribe::Dragon)
        });
        assert!(hit, "seed {seed}: neither qualifying tribe was guaranteed");
    }
}

#[test]
fn a_tribe_trinket_whose_tribe_falls_short_costs_two_less() {
    for seed in 0..12 {
        let mut g = game(seed);
        // No minions: every tribe trinket is filler, every neutral one keeps
        // its price.
        to_turn(&mut g, 6);
        turn_of(&mut g, 0);
        for &(of, price) in &g.players[0].trinket_offer {
            let t = g.data.trinkets.get(of);
            let want = if t.tribes.is_empty() { t.cost } else { t.cost.saturating_sub(2) };
            assert_eq!(price, want, "seed {seed}: {} costs {price}", t.name);
        }
    }
    // Two Pirates: a Pirate trinket keeps its price, the rest are cut.
    for seed in 0..12 {
        let mut g = game(seed);
        put_on_board(&mut g, 0, "Southsea Busker");
        put_on_board(&mut g, 0, "Bilgewater Breakout");
        to_turn(&mut g, 6);
        turn_of(&mut g, 0);
        for &(of, price) in &g.players[0].trinket_offer {
            let t = g.data.trinkets.get(of);
            let qualified = t.tribes.has(Tribe::Pirate);
            let want = if t.tribes.is_empty() || qualified { t.cost } else { t.cost.saturating_sub(2) };
            assert_eq!(price, want, "seed {seed}: {} costs {price}", t.name);
        }
    }
}

#[test]
fn only_trinkets_of_a_lobby_tribe_are_offered() {
    // No Pirate in the lobby: never a Pirate-tagged option.
    for seed in 0..20 {
        let mut g = game_lobby(seed, &[Tribe::Beast, Tribe::Mech, Tribe::Dragon]);
        to_turn(&mut g, 6);
        turn_of(&mut g, 0);
        for &(of, _) in &g.players[0].trinket_offer {
            let t = g.data.trinkets.get(of);
            assert!(
                t.tribes.is_empty() || !t.tribes.has(Tribe::Pirate),
                "seed {seed}: {} offered without Pirates",
                t.name
            );
        }
    }
}

// ---- the hooks ----

#[test]
fn a_start_of_turn_trinket_fires_the_turn_after_the_pick() {
    // Sellemental Portrait: get one on the pick, then one at each start of
    // turn (never on the pick turn itself).
    let mut g = game(6);
    to_turn(&mut g, 5);
    turn_of(&mut g, 0);
    // The pick happens on turn 5 here, so turn 6 is the first start of turn.
    equip(&mut g, 0, 0, "Sellemental Portrait");
    let before = hand_of(&g, 0, "Sellemental");
    to_turn(&mut g, 6);
    turn_of(&mut g, 0);
    assert_eq!(hand_of(&g, 0, "Sellemental"), before + 1, "one at the start of the next turn");
    to_turn(&mut g, 7);
    turn_of(&mut g, 0);
    assert_eq!(hand_of(&g, 0, "Sellemental"), before + 2, "and one more each turn");
    g.check_invariants().unwrap();
}

#[test]
fn an_every_n_turns_rule_counts_the_pick_turn_as_one() {
    // A synthetic trinket: the built ones have no "every N turns" rule.
    let toml = r#"
[[trinket]]
name = "Test Hourglass"
card_id = "TEST_Trinket_1"
slot = "lesser"
rule = { event = "turns", every = 3, acts = [{ get = "Tavern Coin" }] }
"#;
    let d = Arc::new(
        Data::from_toml_all(
            include_str!("../../../cards.toml"),
            include_str!("../../../rules.toml"),
            include_str!("../../../heroes.toml"),
            toml,
        )
        .unwrap(),
    );
    assert_eq!(d.trinkets.len(), 1);
    let mut g = Game::new_with_lobby(d, 7, all_tribes());
    to_turn(&mut g, 5);
    turn_of(&mut g, 0);
    // Picked on turn 5: that turn counts as 1, so it fires at the end of 7.
    g.set_trinket(0, 0, Some(0));
    let coins = |g: &Game| hand_of(g, 0, "Tavern Coin");
    to_turn(&mut g, 6);
    assert_eq!(coins(&g), 0, "turn 6: counted, no fire yet");
    to_turn(&mut g, 7);
    assert_eq!(coins(&g), 0, "turn 7: counted (3), the fire is at its end");
    // End turn 7: the third turn fires on the way out.
    for pl in &mut g.players {
        pl.health = 30;
    }
    let t = g.turn;
    while g.turn == t {
        g.step(Action::EndTurn).unwrap();
    }
    assert_eq!(g.turn, 8);
    assert_eq!(coins(&g), 1, "the third turn fired it at the end of turn 7");
    // It counts up again: the next fire is at the end of turn 10.
    to_turn(&mut g, 10);
    let t = g.turn;
    while g.turn == t {
        for pl in &mut g.players {
            pl.health = 30;
        }
        g.step(Action::EndTurn).unwrap();
    }
    assert_eq!(coins(&g), 2, "then every 3 turns");
}

#[test]
fn a_start_of_combat_trinket_runs_in_a_fight() {
    // A synthetic trinket: "Start of Combat: your Undead gain Taunt and
    // Reborn".
    let toml = r#"
[[trinket]]
name = "Test Banner"
card_id = "TEST_Trinket_2"
slot = "lesser"
fight = [{ when = "soc", tribe = "undead", keywords = ["taunt", "reborn"] }]
"#;
    let d = Arc::new(
        Data::from_toml_all(
            include_str!("../../../cards.toml"),
            include_str!("../../../rules.toml"),
            include_str!("../../../heroes.toml"),
            toml,
        )
        .unwrap(),
    );
    let mut g = Game::new_with_lobby(d, 8, all_tribes());
    g.set_trinket(0, 0, Some(0));
    put_on_board(&mut g, 0, "Eternal Knight");
    put_on_board(&mut g, 0, "Scarlet Survivor");
    let counters = [g.players[0].counters(&g.data, g.lobby), bgcore::Counters::default()];
    assert_eq!(counters[0].trinkets.len(), 1, "the trinket's Start of Combat reaches the fight");
    #[derive(Default)]
    struct Seen(Vec<(CardId, u8)>);
    impl bgcore::fight::Watch for Seen {
        fn attack(&mut self, side: usize, attacker: &Minion, _: &Minion) {
            if side == 0 {
                self.0.push((attacker.card, attacker.keywords.0));
            }
        }
    }
    let mut watch = Seen::default();
    let board = g.players[0].board.clone();
    let mut wall = g.data.cards.make(id(&g, "Lullabot"), false);
    wall.attack = 0;
    wall.health = 1000;
    let _ = fight_with(
        &g.data.cards,
        &board,
        &[wall, wall],
        counters,
        60,
        &mut Rng::new(1),
        &mut watch,
    );
    let undead = id(&g, "Eternal Knight");
    let dragon = id(&g, "Scarlet Survivor");
    let kw_of = |c: CardId| watch.0.iter().find(|&&(card, _)| card == c).map(|&(_, k)| k);
    if let Some(k) = kw_of(undead) {
        assert_eq!(k & (bgcore::Keywords::TAUNT | bgcore::Keywords::REBORN),
                   bgcore::Keywords::TAUNT | bgcore::Keywords::REBORN, "the Undead got both");
    }
    if let Some(k) = kw_of(dragon) {
        assert_eq!(k & bgcore::Keywords::REBORN, 0, "the Dragon got nothing");
    }
}

#[test]
fn a_trinket_event_counter_fires_every_n_casts() {
    // Bloodbound Earrings: after every 4 spells, a Blood Gem on all minions.
    let mut g = game(9);
    put_on_board(&mut g, 0, "Flittering Bat");
    equip(&mut g, 0, 0, "Bloodbound Earrings");
    let before = g.players[0].board[0].attack;
    let coin = id(&g, "Tavern Coin");
    g.players[0].gold = 10;
    for _ in 0..4 {
        g.players[0].hand.push(g.players[0].make_card(&g.data.cards, coin, false, 0));
        g.players[0].gold = 10;
        act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    }
    assert_eq!(g.players[0].board[0].attack, before + 1, "the fourth spell played a Gem");
    assert_eq!(g.players[0].trinkets[0].count, 0, "the counter reset");
}

// ---- the built trinkets, by mechanic ----

#[test]
fn the_start_of_turn_trinkets_hand_out_their_card_each_turn() {
    let cases: [(&str, &str, usize); 6] = [
        ("Sellemental Portrait", "Sellemental", 1),
        ("Chromatic Tear", "", 0),
        ("Volumizer Portrait", "", 0),
        ("Lockbox Portrait", "Lockbox", 1),
        ("Tableware Portrait", "Menagerie Tableware", 1),
        ("Butcher's Sickle", "Butchering", 0),
    ];
    for (name, card, want) in cases {
        let mut g = game(10);
        to_turn(&mut g, 4);
        let slot = slot_of(&g, name);
        equip(&mut g, 0, slot, name);
        let has = |g: &Game| -> usize {
            if card.is_empty() {
                g.players[0].hand.len()
            } else {
                hand_of(g, 0, card)
            }
        };
        let before = has(&g);
        to_turn(&mut g, 5);
        turn_of(&mut g, 0);
        // A Chromadrake is one of five, so any card count above zero does.
        assert!(has(&g) > before, "{name} did nothing at the start of the turn");
        if want > 0 {
            assert_eq!(has(&g), before + want, "{name}");
        }
        g.check_invariants().unwrap();
    }
}

#[test]
fn the_one_time_gifts_pay_on_the_pick() {
    // Explorer's Binoculars: three random tier 4 minions.
    let mut g = game(11);
    take(&mut g, 0, "Explorer's Binoculars");
    assert_eq!(g.players[0].hand.len(), 3);
    for m in &g.players[0].hand {
        assert_eq!(g.data.cards.get(m.card).tier, 4);
    }
    // Magician's Top Hat: two each from tiers 1, 2 and 3.
    let mut g = game(12);
    take(&mut g, 0, "Magician's Top Hat");
    assert_eq!(g.players[0].hand.len(), 6);
    // Corrupted Coin Portrait: two Corrupted Coins.
    let mut g = game(13);
    take(&mut g, 0, "Corrupted Coin Portrait");
    assert_eq!(hand_of(&g, 0, "Corrupted Coin"), 2);
    // Great Boar Sticker: three Gems and +2/+1 to every future Gem.
    let mut g = game(14);
    take(&mut g, 0, "Great Boar Sticker");
    assert_eq!(hand_of(&g, 0, "Blood Gem"), 3);
    assert_eq!((g.players[0].gem_attack, g.players[0].gem_health), (3, 2));
    // Artisanal Urn: your Undead have +20 Attack, wherever they are.
    let mut g = game(15);
    take(&mut g, 0, "Artisanal Urn");
    assert_eq!(g.players[0].undead_bonus, (20, 0));
    // Bob's Tip Jar: 4 Gold and 4 maximum Gold.
    let mut g = game(16);
    let gold = g.players[0].gold;
    take(&mut g, 0, "Bob's Tip Jar");
    assert_eq!(g.players[0].gold, gold + 4);
    assert_eq!(g.players[0].extra_max_gold, 4);
    // Pagle's Fishing Rod: a random tier 7 minion.
    let mut g = game(17);
    take(&mut g, 0, "Pagle's Fishing Rod");
    assert_eq!(g.players[0].hand.len(), 1);
    assert_eq!(g.data.cards.get(g.players[0].hand[0].card).tier, 7);
    // Surprise Portrait and Beatboxer Portrait: the named minion.
    for (name, card) in [
        ("Surprise Portrait", "Elemental of Surprise"),
        ("Beatboxer Portrait", "Polarizing Beatboxer"),
    ] {
        let mut g = game(18);
        take(&mut g, 0, name);
        assert_eq!(hand_of(&g, 0, card), 1, "{name}");
    }
    // Heart of the Forest: +1/+1 for your Tavern spells.
    let mut g = game(19);
    take(&mut g, 0, "Heart of the Forest");
    assert_eq!(g.players[0].spell_bonus, (1, 1));
}

#[test]
fn the_cast_triggers_feed_the_player_bonuses() {
    // Comfy Coffin: your Undead have +1 Attack per Tavern spell.
    let mut g = game(20);
    equip(&mut g, 0, 0, "Comfy Coffin");
    cast_tavern_spell(&mut g, 0);
    assert_eq!(g.players[0].undead_bonus, (1, 0));
    cast_tavern_spell(&mut g, 0);
    assert_eq!(g.players[0].undead_bonus, (2, 0));
    // Demonic Bloodletter: minions in the Tavern have +1/+1 this game.
    let mut g = game(21);
    equip(&mut g, 0, 0, "Demonic Bloodletter");
    set_shop(&mut g, 0, &["Flittering Bat"]);
    let before = g.players[0].shop[0].attack;
    cast_tavern_spell(&mut g, 0);
    assert_eq!(g.players[0].shop[0].attack, before + 1);
    // Corrupted Baton: give your Deity +4/+4.
    let mut g = game_lobby(22, &[Tribe::Aberration, Tribe::Beast]);
    equip(&mut g, 0, 0, "Corrupted Baton");
    let deity = g.players[0].deity.expect("an Aberration lobby has a Deity");
    cast_tavern_spell(&mut g, 0);
    assert_eq!(g.players[0].deity.unwrap().attack, deity.attack + 4);
    // Miniature Ship: give your Pirates +3/+3.
    let mut g = game(23);
    equip(&mut g, 0, 1, "Miniature Ship");
    put_on_board(&mut g, 0, "Southsea Busker");
    let before = g.players[0].board[0].attack;
    cast_tavern_spell(&mut g, 0);
    assert_eq!(g.players[0].board[0].attack, before + 3);
    // Bloodfury Shield: add a Fodder to the next Refresh.
    let mut g = game(24);
    equip(&mut g, 0, 1, "Bloodfury Shield");
    cast_tavern_spell(&mut g, 0);
    assert_eq!(g.players[0].fodder_refreshes.first().copied(), Some(1));
    g.check_invariants().unwrap();
}

#[test]
fn the_play_and_buy_triggers() {
    // Nomi Sticker: after you play an Elemental, Tavern Elementals +3/+2.
    let mut g = game(25);
    equip(&mut g, 0, 0, "Nomi Sticker");
    set_shop(&mut g, 0, &["Sellemental", "Flittering Bat"]);
    let before = g.players[0].shop[0].attack;
    play_from_hand(&mut g, 0, "Sellemental");
    assert_eq!(g.players[0].shop[0].attack, before + 3, "the Tavern Elemental grew");
    // Nerglish Phrasebook: after you play a minion, the left-most card in
    // hand gets +3/+3.
    let mut g = game(26);
    equip(&mut g, 0, 0, "Nerglish Phrasebook");
    put_in_hand(&mut g, 0, "Flittering Bat");
    let before = g.players[0].hand[0].attack;
    play_from_hand(&mut g, 0, "Southsea Busker");
    assert_eq!(g.players[0].hand[0].attack, before + 3);
    // Recycling Sticker: after you play an Elemental, a free Refresh.
    let mut g = game(27);
    equip(&mut g, 0, 1, "Recycling Sticker");
    play_from_hand(&mut g, 0, "Sellemental");
    assert_eq!(g.players[0].free_rolls, 1);
    // Secret Schematic: after you buy a Mech, get a random Tavern spell.
    let mut g = game(28);
    equip(&mut g, 0, 1, "Secret Schematic");
    g.players[0].gold = 20;
    set_shop(&mut g, 0, &["Lullabot"]);
    act(&mut g, 0, Action::Buy(0));
    assert_eq!(g.players[0].hand.len(), 2, "the Mech and a Tavern spell");
    assert!(g.data.cards.get(g.players[0].hand[1].card).tavern_spell);
    // Ur'zul Sticker: after you play a Demon, another friendly Demon
    // consumes a Tavern minion.
    let mut g = game(29);
    equip(&mut g, 0, 1, "Ur'zul Sticker");
    put_on_board(&mut g, 0, "Wrath Weaver");
    let before = g.players[0].board[0].attack;
    set_shop(&mut g, 0, &["Lullabot"]);
    play_from_hand(&mut g, 0, "Ominous Seer");
    assert!(g.players[0].board[0].attack > before, "the Demon ate");
    g.check_invariants().unwrap();
}

#[test]
fn the_gold_and_upgrade_trinkets() {
    // Goblin Wallet: +1 maximum Gold at the end of each turn.
    let mut g = game(30);
    to_turn(&mut g, 4);
    equip(&mut g, 0, 0, "Goblin Wallet");
    to_turn(&mut g, 5);
    assert_eq!(g.players[0].extra_max_gold, 1, "one end of turn has passed");
    // Bartend-o-Tron's Oilcan: -3 on the pick and again at each start.
    let mut g = game(31);
    g.players[0].upgrade_cost = 9;
    take(&mut g, 0, "Bartend-o-Tron's Oilcan");
    assert_eq!(g.players[0].upgrade_cost, 6);
    let before = g.players[0].upgrade_cost;
    to_turn(&mut g, 2);
    // The turn start also drops the countdown by 1, and the trinket takes 3.
    assert!(g.players[0].upgrade_cost <= before - 3, "the start of the turn took 3 more");
    // Frigid Blossom: -1 upgrade cost after a Refresh.
    let mut g = game(32);
    equip(&mut g, 0, 0, "Frigid Blossom");
    g.players[0].upgrade_cost = 7;
    g.players[0].gold = 3;
    act(&mut g, 0, Action::Roll);
    assert_eq!(g.players[0].upgrade_cost, 6);
    g.check_invariants().unwrap();
}

#[test]
fn the_tavern_shape_trinkets() {
    // Cursed Crystal: +3/+3 to the Tavern this turn, and it must not stack
    // when you Refresh again.
    let mut g = game(33);
    equip(&mut g, 0, 0, "Cursed Crystal");
    g.players[0].gold = 5;
    let bonus = |g: &Game| -> Vec<i32> {
        g.players[0]
            .shop
            .iter()
            .map(|m| m.attack - g.data.cards.get(m.card).attack)
            .collect()
    };
    act(&mut g, 0, Action::Roll);
    assert!(!g.players[0].shop.is_empty());
    assert!(bonus(&g).iter().all(|&b| b == 3), "every Tavern minion got +3 Attack");
    act(&mut g, 0, Action::Roll);
    assert!(bonus(&g).iter().all(|&b| b == 3), "a second Refresh must not double it");
    // It is this turn only.
    to_turn(&mut g, 2);
    turn_of(&mut g, 0);
    assert!(g.players[0].shop.iter().all(|m| m.attack == g.data.cards.get(m.card).attack));
    g.check_invariants().unwrap();
}

#[test]
fn the_end_of_turn_and_discard_trinkets() {
    // Enigmatic Headstone: at the end of each turn, your Undead have +2
    // Attack.
    let mut g = game(34);
    to_turn(&mut g, 4);
    equip(&mut g, 0, 0, "Enigmatic Headstone");
    to_turn(&mut g, 5);
    assert_eq!(g.players[0].undead_bonus, (2, 0));
    // Sphere of Memory: at the end of your turn, three copies of the last
    // Tavern spell you cast.
    let mut g = game(35);
    equip(&mut g, 0, 1, "Sphere of Memory");
    let coin = id(&g, "Tavern Coin");
    g.players[0].hand.push(g.players[0].make_card(&g.data.cards, coin, false, 0));
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    let t = g.turn;
    while g.turn == t {
        for pl in &mut g.players {
            pl.health = 30;
        }
        g.step(Action::EndTurn).unwrap();
    }
    assert_eq!(hand_of(&g, 0, "Tavern Coin"), 3, "three copies at the end of the turn");
    // Shath'Yar Shrine: after you discard a spell, a random Aberration.
    let mut g = game_lobby(36, &[Tribe::Aberration, Tribe::Beast]);
    equip(&mut g, 0, 1, "Shath'Yar Shrine");
    let coin = id(&g, "Tavern Coin");
    g.players[0].hand.push(g.players[0].make_card(&g.data.cards, coin, false, 0));
    g.players[0].pending_discard = Some(bgcore::game::PendingDiscard {
        source: None,
        reward: bgcore::Effect::GoldNextTurn(0),
        golden: false,
    });
    act(&mut g, 0, Action::Discard(0));
    assert_eq!(g.players[0].hand.len(), 1);
    assert!(g.data.cards.get(g.players[0].hand[0].card).tribes.has(Tribe::Aberration));
    // Tome of the Ancients: after you discard 3 cards, Tavern spells +1/+1.
    let mut g = game(37);
    equip(&mut g, 0, 1, "Tome of the Ancients");
    for _ in 0..3 {
        let coin = id(&g, "Tavern Coin");
        g.players[0].hand.push(g.players[0].make_card(&g.data.cards, coin, false, 0));
        g.players[0].pending_discard = Some(bgcore::game::PendingDiscard {
            source: None,
            reward: bgcore::Effect::GoldNextTurn(0),
            golden: false,
        });
        act(&mut g, 0, Action::Discard(0));
    }
    assert_eq!(g.players[0].spell_bonus, (1, 1));
    g.check_invariants().unwrap();
}

#[test]
fn gold_pendant_makes_a_small_minion_golden_each_turn() {
    let mut g = game(38);
    put_on_board(&mut g, 0, "Lullabot");
    take(&mut g, 0, "Gold Pendant");
    assert!(g.players[0].board[0].golden, "a tier 4 or below minion became Golden");
    assert_eq!(g.players[0].golden_played, 0, "not a Golden played");
    // A board with no small minion: nothing happens, no panic.
    let mut g = game(39);
    put_on_board(&mut g, 0, "Captain Sanders");
    take(&mut g, 0, "Gold Pendant");
    assert!(!g.players[0].board[0].golden);
}

// ---- helpers for the card tests ----


fn cast_tavern_spell(g: &mut Game, p: usize) {
    let coin = id(g, "Tavern Coin");
    g.players[p].hand.push(g.players[p].make_card(&g.data.cards, coin, false, 0));
    act(g, p, Action::Play { hand: 0, pos: 0 });
}

fn play_from_hand(g: &mut Game, p: usize, name: &str) {
    // Take it from the pool, play it, then let the pool come back right.
    put_in_hand(g, p, name);
    let slot = g.players[p].hand.len() - 1;
    act(g, p, Action::Play { hand: slot as u8, pos: g.players[p].board.len() as u8 });
}

/// The trinket table as the tests see it (a compile-time check that the
/// bundled file parses and holds the built cards).
#[test]
fn the_trinket_table_parses() {
    let d = Data::bundled();
    assert!(d.trinkets.len() >= 76, "only {} trinkets", d.trinkets.len());
    for i in 0..d.trinkets.len() as u8 {
        let t = d.trinkets.get(i);
        assert!(!t.name.is_empty());
        assert!(t.card_id.starts_with("BG"), "{}", t.card_id);
        assert!(
            t.on_pick.len()
                + t.start_of_turn.len()
                + t.end_of_turn.len()
                + t.on_buy.len()
                + t.on_buy_spell.len()
                + t.on_play.len()
                + t.on_sell.len()
                + t.on_refresh.len()
                + t.on_cast_spell.len()
                + t.on_cast_spell_on_minion.len()
                + t.on_discard.len()
                + t.on_get.len()
                + t.on_gold_spent.len()
                + t.fight.len()
                + t.rule.iter().count()
                > 0,
            "{} does nothing",
            t.name
        );
    }
    // A bad file is refused, and a misspelt card is a load error.
    assert!(TrinketDb::from_toml("nonsense", &d.cards).is_err());
    let bad = r#"
[[trinket]]
name = "Broken"
card_id = "TEST_x"
slot = "lesser"
on_pick = [{ get = "No Such Card" }]
"#;
    assert!(TrinketDb::from_toml(bad, &d.cards).unwrap_err().contains("unknown card"));
    let bad_slot = r#"
[[trinket]]
name = "Broken"
card_id = "TEST_y"
slot = "huge"
"#;
    assert!(TrinketDb::from_toml(bad_slot, &d.cards).unwrap_err().contains("slot"));
}

#[test]
fn the_discover_bounty_and_link_portraits() {
    // Book of Medivh: Discover a Tavern spell (twice for the greater form).
    for (id, wants) in [("BG30_MagicItem_420", 1usize), ("BG30_MagicItem_420t", 2)] {
        let mut g = game(40);
        take_id(&mut g, 0, id);
        for k in 0..wants {
            assert_eq!(g.players[0].discover.len(), 3, "a Tavern spell Discover is open");
            for &c in &g.players[0].discover {
                assert!(g.data.cards.get(c).tavern_spell);
            }
            act(&mut g, 0, Action::Choose(0));
            assert_eq!(
                g.players[0].hand.len(),
                k + 1,
                "hand {:?}, last_spell {:?}, discarded {}",
                g.players[0].hand.iter().map(|m| g.data.cards.get(m.card).name.clone()).collect::<Vec<_>>(),
                g.players[0].last_spell.map(|c| g.data.cards.get(c).name.clone()),
                g.players[0].discarded,
            );
        }
    }
    // Glass of Perspective: a random Choose One card (two for the greater).
    for (id, wants) in [("BG36_MagicItem_303", 1usize), ("BG36_MagicItem_303t", 2)] {
        let mut g = game(41);
        g.players[0].tier = 4; // a Choose One minion up to the player's tier
        take_id(&mut g, 0, id);
        assert_eq!(g.players[0].hand.len(), wants);
        for m in &g.players[0].hand {
            assert!(g.data.cards.get(m.card).choose_one.is_some());
        }
    }
    // Sunken Anchor: two random Bounties.
    let mut g = game(42);
    take(&mut g, 0, "Sunken Anchor");
    assert_eq!(g.players[0].hand.len(), 2);
    // Privateer Portrait: a Proud Privateer and two Bounties, then two more
    // Bounties at each start of turn.
    let mut g = game(43);
    take(&mut g, 0, "Privateer Portrait");
    assert_eq!(hand_of(&g, 0, "Proud Privateer"), 1);
    assert_eq!(g.players[0].hand.len(), 3);
    to_turn(&mut g, 2);
    turn_of(&mut g, 0);
    assert_eq!(g.players[0].hand.len(), 5, "two more Bounties on the next turn");
    // Willbreaker Sticker: two linked Tavern spells, and playing one
    // discards the other.
    let mut g = game(44);
    take(&mut g, 0, "Willbreaker Sticker");
    assert_eq!(g.players[0].hand.len(), 2);
    let (a, b) = (g.players[0].hand[0], g.players[0].hand[1]);
    assert_ne!(a.linked, 0);
    assert_eq!(a.linked & 0x3fff, b.linked & 0x3fff);
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    assert_eq!(g.players[0].hand.len(), 0, "the other one was discarded");
    assert_eq!(g.players[0].discarded, 1);
    // Surveyor Portrait: a Hot-Air Surveyor and +6/+6 to the Gems you play
    // from hand.
    let mut g = game(45);
    take(&mut g, 0, "Surveyor Portrait");
    assert_eq!(hand_of(&g, 0, "Hot-Air Surveyor"), 1);
    assert_eq!((g.players[0].gem_attack, g.players[0].gem_health), (7, 7));
    // Cookie's Stirring Rod: after five Murlocs, two random Tavern spells.
    let mut g = game_lobby(46, &[Tribe::Murloc, Tribe::Beast]);
    equip(&mut g, 0, 1, "Cookie's Stirring Rod");
    for name in [
        "Bubble Gunner",
        "Flighty Scout",
        "Expert Aviator",
        "Tad",
        "Very Hungry Winterfinner",
    ] {
        play_from_hand(&mut g, 0, name);
    }
    assert_eq!(g.players[0].hand.len(), 2, "two Tavern spells");
    for m in &g.players[0].hand {
        assert!(g.data.cards.get(m.card).tavern_spell);
    }
    g.check_invariants().unwrap();
}

#[test]
fn the_greater_forms_are_stronger() {
    // Nerglish Phrasebook: +6/+6 instead of +3/+3.
    let mut g = game(47);
    take_id(&mut g, 0, "BG30_MagicItem_914t");
    put_in_hand(&mut g, 0, "Flittering Bat");
    let before = g.players[0].hand[0].attack;
    play_from_hand(&mut g, 0, "Southsea Busker");
    assert_eq!(g.players[0].hand[0].attack, before + 6);
    // Comfy Coffin: +2 Attack instead of +1 per Tavern spell.
    let mut g = game(48);
    take_id(&mut g, 0, "BG30_MagicItem_547t");
    cast_tavern_spell(&mut g, 0);
    assert_eq!(g.players[0].undead_bonus, (2, 0));
    // Great Boar Sticker: five Gems and +3/+3.
    let mut g = game(49);
    take_id(&mut g, 0, "BG30_MagicItem_988t");
    assert_eq!(hand_of(&g, 0, "Blood Gem"), 5);
    assert_eq!((g.players[0].gem_attack, g.players[0].gem_health), (4, 4));
    // Bloodbound Earrings: every five spells, two Gems on all minions.
    let mut g = game(50);
    put_on_board(&mut g, 0, "Flittering Bat");
    equip_id(&mut g, 0, 1, "BG32_MagicItem_808t");
    let before = g.players[0].board[0].attack;
    let coin = id(&g, "Tavern Coin");
    for _ in 0..5 {
        g.players[0].gold = 10;
        g.players[0].hand.push(g.players[0].make_card(&g.data.cards, coin, false, 0));
        act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    }
    assert_eq!(g.players[0].board[0].attack, before + 2, "two Gems");
    // Heart of the Forest: +1/+1 on the pick and +1/+1 more every six casts.
    let mut g = game(51);
    take(&mut g, 0, "Heart of the Forest");
    assert_eq!(g.players[0].spell_bonus, (1, 1));
    for _ in 0..6 {
        cast_tavern_spell(&mut g, 0);
    }
    assert_eq!(g.players[0].spell_bonus, (2, 2), "six casts improved it");
}

/// Pick a trinket by card id (the pairs share a name, so tests use the id).
fn take_id(g: &mut Game, p: usize, card_id: &str) {
    let i = g.data.trinkets.by_card_id(card_id).unwrap_or_else(|| panic!("{card_id}"));
    turn_of(g, p);
    let slot = match g.data.trinkets.get(i).slot {
        bgcore::trinkets::Slot::Lesser => 0,
        bgcore::trinkets::Slot::Greater => 1,
    };
    g.players[p].trinket_offer.clear();
    g.players[p].trinket_offer_slot = slot as u8 + 1;
    let _ = g.players[p].trinket_offer.try_push((i, 0));
    g.step(Action::ChooseTrinket(0)).unwrap_or_else(|_| panic!("could not take {card_id}"));
}


// ---- the second round: prices, Gold and the trinket slots ----

#[test]
fn a_health_price_is_self_damage_and_not_gold_spent() {
    // Pilgrimp Sticker: one Demon a turn costs Health.
    let mut g = game_lobby(60, &[Tribe::Demon, Tribe::Beast]);
    to_turn(&mut g, 5);
    take(&mut g, 0, "Pilgrimp Sticker");
    to_turn(&mut g, 6);
    turn_of(&mut g, 0);
    set_shop(&mut g, 0, &["Wrath Weaver"]);
    g.players[0].gold = 10;
    g.players[0].health = 30;
    g.players[0].armor = 0;
    let gold = g.players[0].gold;
    let hp = g.players[0].health;
    act(&mut g, 0, Action::Buy(0));
    assert_eq!(g.players[0].gold, gold, "Health, not Gold");
    assert_eq!(g.players[0].health, hp - 3, "three Health of self damage");
    assert_eq!(g.players[0].gold_spent_turn, 0, "not Gold spent");
    assert_eq!(hand_of(&g, 0, "Wrath Weaver"), 1, "it is still a buy");
    // The use is gone: the next Demon is bought with Gold.
    set_shop(&mut g, 0, &["Ominous Seer"]);
    g.players[0].gold = 10;
    act(&mut g, 0, Action::Buy(0));
    assert_eq!(g.players[0].gold, 7);
    g.check_invariants().unwrap();
}

#[test]
fn a_free_buy_spends_no_gold_but_is_a_buy() {
    // Warcry Totem: two Battlecry minions a turn are free.
    let mut g = game(61);
    to_turn(&mut g, 5);
    take(&mut g, 0, "Warcry Totem");
    to_turn(&mut g, 6);
    turn_of(&mut g, 0);
    set_shop(&mut g, 0, &["Ominous Seer"]);
    g.players[0].gold = 0;
    act(&mut g, 0, Action::Buy(0));
    assert_eq!(g.players[0].gold, 0);
    assert_eq!(g.players[0].gold_spent_turn, 0, "a free buy spends no Gold");
    assert_eq!(g.players[0].tp.free_buys, 1, "one left");
    assert_eq!(hand_of(&g, 0, "Ominous Seer"), 1, "but it is a buy");
    g.check_invariants().unwrap();
}

#[test]
fn the_gold_trinkets_pay_out() {
    // Glowing Crystal: 1 Gold per different friendly type at the start of
    // the turn.
    let mut g = game(62);
    put_on_board(&mut g, 0, "Southsea Busker");
    put_on_board(&mut g, 0, "Scarlet Survivor");
    equip(&mut g, 0, 0, "Glowing Crystal");
    let before = g.players[0].gold;
    to_turn(&mut g, 2);
    assert!(g.players[0].gold >= before + 2, "two types paid out");
    // Splinter of Aurum: with 15 Gold, a Golden Tier 5 minion, once.
    let mut g = game(63);
    take(&mut g, 0, "Splinter of Aurum");
    g.players[0].gold = 15;
    g.check_golden_when_rich(0);
    assert_eq!(g.players[0].hand.len(), 1);
    assert!(g.players[0].hand[0].golden);
    assert_eq!(g.data.cards.get(g.players[0].hand[0].card).tier, 5);
    g.players[0].gold = 20;
    g.check_golden_when_rich(0);
    assert_eq!(g.players[0].hand.len(), 1, "once per game");
    g.check_invariants().unwrap();
}

#[test]
fn the_trinket_slot_trinkets() {
    // Orb of the Unknown: a random Lesser trinket replaces the slot.
    let mut g = game(64);
    take(&mut g, 0, "Orb of the Unknown");
    assert_ne!(g.players[0].trinkets[0].of, 0, "the slot still holds one");
    let orb = g.data.trinkets.by_name("Orb of the Unknown").unwrap() + 1;
    assert_ne!(g.players[0].trinkets[0].of, orb, "it replaced itself");
    // Mysterious Orb: 10 Gold and the next offer is Lesser.
    let mut g = game(65);
    take(&mut g, 0, "Mysterious Orb");
    assert!(g.players[0].tp.next_lesser);
    // Trip Vouchers: a greater offer into the lesser slot two turns later,
    // and the normal turn 9 offer still comes.
    let mut g = game(66);
    to_turn(&mut g, 6);
    turn_of(&mut g, 0);
    g.players[0].trinket_offer.clear();
    g.players[0].trinket_offer_slot = 1;
    let tv = g.data.trinkets.by_card_id("BG30_MagicItem_891").unwrap();
    let _ = g.players[0].trinket_offer.try_push((tv, 0));
    g.step(Action::ChooseTrinket(0)).unwrap();
    assert_eq!(g.players[0].tp.greater_into_lesser_turn, 8);
    to_turn(&mut g, 8);
    turn_of(&mut g, 0);
    assert_eq!(g.players[0].trinket_offer_slot, 1, "into the lesser slot");
    for &(of, _) in &g.players[0].trinket_offer {
        assert_eq!(g.data.trinkets.get(of).slot, bgcore::trinkets::Slot::Greater);
    }
    act(&mut g, 0, Action::ChooseTrinket(0));
    to_turn(&mut g, 9);
    turn_of(&mut g, 0);
    assert_eq!(g.players[0].trinket_offer_slot, 2, "the normal turn 9 offer still comes");
    g.check_invariants().unwrap();
}

#[test]
fn mystery_cube_offers_a_free_replacement() {
    let mut g = game(67);
    to_turn(&mut g, 6);
    turn_of(&mut g, 0);
    let mc = g.data.trinkets.by_card_id("BG30_MagicItem_703").unwrap();
    g.players[0].trinket_offer.clear();
    g.players[0].trinket_offer_slot = 1;
    let _ = g.players[0].trinket_offer.try_push((mc, 0));
    g.players[0].gold = 10;
    g.step(Action::ChooseTrinket(0)).unwrap();
    // Its on-pick deals two free Lesser options; picking one replaces it.
    assert_eq!(g.players[0].trinket_offer.len(), 2, "two options");
    assert!(g.players[0].trinket_offer.iter().all(|&(_, p)| p == 0), "free");
    for &(of, _) in &g.players[0].trinket_offer {
        assert_eq!(g.data.trinkets.get(of).slot, bgcore::trinkets::Slot::Lesser);
    }
    // The cube is gone afterwards. (An option that replaces itself on the
    // pick, Orb of the Unknown, is replaced again, so only the cube leaving
    // is checked.)
    let k = (0..g.players[0].trinket_offer.len())
        .find(|&k| g.players[0].trinket_offer[k].0 != mc)
        .expect("an option that is not the cube again");
    act(&mut g, 0, Action::ChooseTrinket(k as u8));
    assert_ne!(g.players[0].trinkets[0].of, mc + 1, "the cube was replaced");
    assert_ne!(g.players[0].trinkets[0].of, 0, "and the slot is not empty");
    g.check_invariants().unwrap();
}

#[test]
fn the_mystery_cube_keeps_offering_every_turn() {
    let mut g = game(167);
    to_turn(&mut g, 6);
    turn_of(&mut g, 0);
    let mc = g.data.trinkets.by_card_id("BG30_MagicItem_703").unwrap();
    g.players[0].trinket_offer.clear();
    g.players[0].trinket_offer_slot = 1;
    let _ = g.players[0].trinket_offer.try_push((mc, 0));
    g.players[0].gold = 10;
    g.step(Action::ChooseTrinket(0)).unwrap();
    let k = (0..g.players[0].trinket_offer.len())
        .find(|&k| g.players[0].trinket_offer[k].0 != mc)
        .expect("an option");
    let first = g.players[0].trinket_offer[k].0;
    act(&mut g, 0, Action::ChooseTrinket(k as u8));
    assert!(g.players[0].trinkets[0].cube, "the slot is a cube slot");
    // Every turn after it: a fresh free offer of two Lesser trinkets, never
    // the one the slot holds.
    let mut held_before = vec![first];
    for turn in 7..9 {
        to_turn(&mut g, turn);
        turn_of(&mut g, 0);
        let held = g.players[0].trinkets[0].of - 1;
        assert!(g.players[0].trinkets[0].cube, "still a cube slot on turn {turn}");
        let offer: Vec<(u8, u8)> = g.players[0].trinket_offer.to_vec();
        assert_eq!(offer.len(), 2, "two options on turn {turn}");
        assert!(offer.iter().all(|&(_, p)| p == 0), "free on turn {turn}");
        assert!(offer.iter().all(|&(of, _)| of != held), "never the held one");
        assert!(
            offer.iter().all(|&(of, _)| g.data.trinkets.get(of).slot
                == bgcore::trinkets::Slot::Lesser),
            "Lesser only"
        );
        assert!(
            offer.iter().all(|&(of, _)| g.data.trinkets.get(of).cube),
            "a one-time effect is never offered"
        );
        act(&mut g, 0, Action::ChooseTrinket(0));
        assert!(g.players[0].trinkets[0].cube, "the flag stays with the slot");
        held_before.push(g.players[0].trinkets[0].of - 1);
        g.check_invariants().unwrap();
    }
    // Turn 9: the normal Greater offer goes first, and the cube's offer comes
    // straight after it is answered.
    to_turn(&mut g, 9);
    turn_of(&mut g, 0);
    assert_eq!(g.players[0].trinket_offer.len(), 4, "the turn 9 offer");
    assert_eq!(g.players[0].trinket_offer_slot, 2, "into the Greater slot");
    act(&mut g, 0, Action::ChooseTrinket(0));
    let held = g.players[0].trinkets[0].of - 1;
    assert_eq!(g.players[0].trinket_offer.len(), 2, "then the cube's two");
    assert!(g.players[0].trinket_offer.iter().all(|&(of, _)| of != held));
    assert_ne!(held, held_before[1], "the slot moved on every turn");
    g.check_invariants().unwrap();
}

#[test]
fn the_mystery_cube_offers_after_the_held_trinkets_own_start_of_turn() {
    // A trinket with a start-of-turn effect in a cube slot does its own work
    // first, then the cube replaces it (DESIGN.md).
    let mut g = game(168);
    let cube = trinket(&g, "Mystery Cube");
    g.set_trinket(0, 0, Some(cube));
    g.players[0].trinkets[0].cube = true;
    g.players[0].trinkets[0].cube_choices = 2;
    // Gold Pendant: makes a random Tier 4 or below minion Golden each turn.
    put_on_board(&mut g, 0, "Sellemental");
    let pendant = trinket(&g, "Gold Pendant");
    g.set_trinket(0, 0, Some(pendant));
    assert!(g.players[0].trinkets[0].cube, "the slot kept the cube flag");
    g.players[0].gold = 10;
    // End the turn with nothing answered, so the next start of turn is what
    // the test sees.
    next_turn(&mut g);
    turn_of(&mut g, 0);
    assert!(g.players[0].board[0].golden, "the Pendant fired before the swap");
    assert_eq!(g.players[0].trinket_offer.len(), 2, "then the cube offered");
    g.check_invariants().unwrap();
}

#[test]
fn booty_bay_brew_fires_once_per_spending() {
    let mut g = game(68);
    put_on_board(&mut g, 0, "Southsea Busker");
    take(&mut g, 0, "Booty Bay Brew");
    let before = g.players[0].board[0].attack;
    // One spending of 3 Gold (a buy) is one firing, not three.
    set_shop(&mut g, 0, &["Ominous Seer"]);
    g.players[0].gold = 10;
    act(&mut g, 0, Action::Buy(0));
    eprintln!("mask {:#x} before {before} after {} board {:?}", g.players[0].trinket_events,
        g.players[0].board[0].attack,
        g.players[0].board.iter().map(|m| (g.data.cards.get(m.card).name.clone(), m.attack)).collect::<Vec<_>>());
    assert_eq!(g.players[0].board[0].attack, before + 3, "once, whatever the amount");
    g.check_invariants().unwrap();
}

// ---- the fight-side trinkets ----
//
// A trinket has no host minion, so its fight effects leave through
// `Counters.trinkets` (a small `Copy` bundle built from the slots) and come
// back through `SideOutcome`. These tests fight player 0's board against a
// wall with the real `fight_with`, so the counters they build are the ones a
// real combat builds.

/// A minion with `attack` and `health` that soaks a fight (Lullabot's card).
fn wall(g: &Game, attack: i32, health: i32) -> Minion {
    let mut m = g.data.cards.make(id(g, "Lullabot"), false);
    m.attack = attack;
    m.health = health;
    m
}

/// Fight player 0's board against one wall, with player 0's trinkets.
fn fight_wall(g: &Game, seed: u64, attack: i32, health: i32, max_attacks: u32) -> bgcore::fight::Outcome {
    fight_wall_watched(g, seed, attack, health, max_attacks, &mut ())
}

fn fight_wall_watched(
    g: &Game,
    seed: u64,
    attack: i32,
    health: i32,
    max_attacks: u32,
    watch: &mut impl bgcore::fight::Watch,
) -> bgcore::fight::Outcome {
    let board = g.players[0].board.clone();
    let counters = [g.players[0].counters(&g.data, g.lobby), bgcore::Counters::default()];
    fight_with(
        &g.data.cards,
        &board,
        &[wall(g, attack, health)],
        counters,
        max_attacks,
        &mut Rng::new(seed),
        watch,
    )
}

/// What a fight did to the minions, for the tests that cannot read the board
/// afterwards through `Outcome`.
#[derive(Default)]
struct Seen {
    attacks: Vec<(usize, Minion)>,
    deaths: Vec<(usize, Minion)>,
    summons: Vec<(usize, Minion)>,
}

impl bgcore::fight::Watch for Seen {
    fn attack(&mut self, side: usize, attacker: &Minion, _defender: &Minion) {
        self.attacks.push((side, *attacker));
    }
    fn death(&mut self, side: usize, _pos: usize, m: &Minion) {
        self.deaths.push((side, *m));
    }
    fn summon(&mut self, side: usize, _pos: usize, m: &Minion) {
        self.summons.push((side, *m));
    }
}

impl Seen {
    /// The first time player 0 attacked with a minion whose card is `name`.
    fn attacker(&self, g: &Game, name: &str) -> Option<Minion> {
        self.attacks
            .iter()
            .find(|(s, m)| *s == 0 && g.data.cards.get(m.card).name == name)
            .map(|&(_, m)| m)
    }

    /// The first minion player 0 summoned with this card.
    fn summoned(&self, g: &Game, name: &str) -> Option<Minion> {
        self.summons
            .iter()
            .find(|(s, m)| *s == 0 && g.data.cards.get(m.card).name == name)
            .map(|&(_, m)| m)
    }

    /// The first minion of player 0 that died with this card.
    fn dead(&self, g: &Game, name: &str) -> Option<Minion> {
        self.deaths
            .iter()
            .find(|(s, m)| *s == 0 && g.data.cards.get(m.card).name == name)
            .map(|&(_, m)| m)
    }

    /// Every minion player 0 summoned with this card.
    fn summoned_all(&self, g: &Game, name: &str) -> Vec<Minion> {
        self.summons
            .iter()
            .filter(|(s, m)| *s == 0 && g.data.cards.get(m.card).name == name)
            .map(|&(_, m)| m)
            .collect()
    }

    /// Every minion of player 0 that died with this card.
    fn dead_all(&self, g: &Game, name: &str) -> Vec<Minion> {
        self.deaths
            .iter()
            .filter(|(s, m)| *s == 0 && g.data.cards.get(m.card).name == name)
            .map(|&(_, m)| m)
            .collect()
    }
}

/// Put `count` copies of a minion on player 0's board at 1 Health, so a fight
/// kills them one attack at a time.
fn weak_board(g: &mut Game, p: usize, name: &str, count: usize) {
    for _ in 0..count {
        put_on_board(g, p, name);
        let m = g.players[p].board.last_mut().unwrap();
        m.health = 1;
    }
}

#[test]
fn an_avenge_trinket_fires_after_its_third_friendly_death() {
    // Quilligraphy Set: Avenge (3): your Blood Gems give an extra +1 Health.
    let mut g = game(11);
    equip(&mut g, 0, 0, "Quilligraphy Set");
    assert_eq!(g.players[0].counters(&g.data, g.lobby).trinkets.len(), 1, "the bundle holds it");
    // Two deaths are not three.
    weak_board(&mut g, 0, "Southsea Busker", 2);
    let out = fight_wall(&g, 1, 10, 500, 60);
    assert_eq!(out.sides[0].gem_gain, (0, 0), "two deaths: no fire");
    // The count starts again in the next fight (DESIGN.md: Avenge resets).
    g.players[0].board.clear();
    weak_board(&mut g, 0, "Southsea Busker", 2);
    let out = fight_wall(&g, 2, 10, 500, 60);
    assert_eq!(out.sides[0].gem_gain, (0, 0), "two more, still no fire");
    // Three in one fight does fire, once.
    g.players[0].board.clear();
    weak_board(&mut g, 0, "Southsea Busker", 3);
    let out = fight_wall(&g, 3, 10, 500, 60);
    assert_eq!(out.sides[0].gem_gain, (0, 1), "Avenge (3): +1 Health on the Gems");
}

#[test]
fn a_start_of_combat_trinket_shields_the_two_ends() {
    // Holy Mallet: Start of Combat: give your left and right-most minions
    // Divine Shield.
    let mut g = game(12);
    equip(&mut g, 0, 0, "Holy Mallet");
    put_on_board(&mut g, 0, "Eternal Knight");
    put_on_board(&mut g, 0, "Southsea Busker");
    put_on_board(&mut g, 0, "Scarlet Survivor");
    let mut seen = Seen::default();
    fight_wall_watched(&g, 1, 0, 500, 60, &mut seen);
    let ds = |name: &str| {
        seen.attacker(&g, name).map(|m| m.keywords.has(bgcore::Keywords::DIVINE_SHIELD))
    };
    assert_eq!(ds("Eternal Knight"), Some(true), "the left end is shielded");
    assert_eq!(ds("Southsea Busker"), Some(false), "the middle one is not");
    assert_eq!(ds("Scarlet Survivor"), Some(true), "the right end is shielded");
    // One minion is both ends, and gets one shield.
    g.players[0].board.clear();
    put_on_board(&mut g, 0, "Eternal Knight");
    let mut seen = Seen::default();
    fight_wall_watched(&g, 1, 0, 500, 60, &mut seen);
    assert_eq!(
        seen.attacker(&g, "Eternal Knight").map(|m| m.keywords.has(bgcore::Keywords::DIVINE_SHIELD)),
        Some(true)
    );
}

#[test]
fn summoning_a_beast_buffs_it() {
    // Mama Bear Sticker: whenever you summon a Beast, give it +6/+6.
    let mut g = game(13);
    equip(&mut g, 0, 0, "Mama Bear Sticker");
    // Buzzing Vermin's Deathrattle summons a 2/2 Beetle, a Beast.
    weak_board(&mut g, 0, "Buzzing Vermin", 1);
    let mut seen = Seen::default();
    fight_wall_watched(&g, 1, 10, 500, 60, &mut seen);
    let beetle = seen.summoned(&g, "Beetle").expect("the Vermin's Beast was summoned");
    assert_eq!((beetle.attack, beetle.health), (2 + 6, 2 + 6), "the sticker buffed it");
    // The same fight with no trinket leaves the Beetle alone.
    g.players[0].board.clear();
    g.players[0].trinkets[0] = Default::default();
    weak_board(&mut g, 0, "Buzzing Vermin", 1);
    let mut seen = Seen::default();
    fight_wall_watched(&g, 1, 10, 500, 60, &mut seen);
    let beetle = seen.summoned(&g, "Beetle").expect("summoned without the trinket");
    assert_eq!((beetle.attack, beetle.health), (2, 2), "no trinket, no buff");
}

#[test]
fn an_avenge_trinket_gives_a_random_undead_reborn() {
    // Staff of the Scourge: Avenge (5): give a random friendly Undead Reborn.
    let mut g = game(14);
    equip(&mut g, 0, 0, "Staff of the Scourge");
    weak_board(&mut g, 0, "Southsea Busker", 5);
    put_on_board(&mut g, 0, "Eternal Knight");
    g.players[0].board.last_mut().unwrap().health = 100;
    let mut seen = Seen::default();
    fight_wall_watched(&g, 1, 10, 500, 60, &mut seen);
    let knight = seen.attacker(&g, "Eternal Knight").expect("the Knight attacked");
    assert!(
        knight.keywords.has(bgcore::Keywords::REBORN),
        "five deaths gave the Undead Reborn"
    );
}

#[test]
fn beetle_band_summons_taunt_beetles_at_the_right_end() {
    // Beetle Band: Avenge (5): summon a 2/2 Beetle, give it Taunt.
    let mut g = game(15);
    equip(&mut g, 0, 0, "Beetle Band");
    weak_board(&mut g, 0, "Southsea Busker", 5);
    let mut seen = Seen::default();
    fight_wall_watched(&g, 1, 10, 500, 60, &mut seen);
    assert_eq!(seen.summoned(&g, "Beetle").map(|m| (m.attack, m.health)), Some((2, 2)));
    // The Beetle is the side's only minion by then, so it attacks and dies:
    // its death shows the Taunt it was given.
    let dead = seen.dead(&g, "Beetle").expect("the Beetle fought and died");
    assert!(dead.keywords.has(bgcore::Keywords::TAUNT), "it has Taunt");
    // The greater one is Avenge (7) and summons two.
    let mut g = game(16);
    equip_id(&mut g, 0, 1, "BG32_MagicItem_860t");
    weak_board(&mut g, 0, "Southsea Busker", 7);
    let mut seen = Seen::default();
    fight_wall_watched(&g, 1, 10, 500, 60, &mut seen);
    let summoned: Vec<Minion> = seen.summoned_all(&g, "Beetle");
    assert_eq!(summoned.len(), 2, "two Beetles");
    for m in &summoned {
        assert_eq!((m.attack, m.health), (2, 2));
    }
    let dead: Vec<Minion> = seen.dead_all(&g, "Beetle");
    assert_eq!(dead.len(), 2, "both fought");
    for m in dead {
        assert!(m.keywords.has(bgcore::Keywords::TAUNT), "both got Taunt");
    }
}

#[test]
fn fridge_magnet_gets_a_random_magnetic_minion_after_three_deaths() {
    let mut g = game(17);
    equip(&mut g, 0, 1, "Fridge Magnet");
    weak_board(&mut g, 0, "Southsea Busker", 3);
    let out = fight_wall(&g, 1, 10, 500, 60);
    let gained = out.sides[0].cards_gained();
    assert_eq!(gained.len(), 1, "one card");
    assert!(
        g.data.cards.get(gained[0]).keywords.has(bgcore::Keywords::MAGNETIC),
        "a Magnetic minion"
    );
}

#[test]
fn wicked_tome_raises_the_tavern_spell_bonus() {
    // Wicked Tome (greater): Avenge (4): your Tavern spells give +1/+1.
    let mut g = game(19);
    equip_id(&mut g, 0, 1, "BG32_MagicItem_270t");
    weak_board(&mut g, 0, "Southsea Busker", 4);
    let out = fight_wall(&g, 1, 10, 500, 60);
    assert_eq!(out.sides[0].spell_bonus_gain, (1, 1));
    // Three deaths are not enough for Avenge (4).
    let mut g = game(18);
    equip_id(&mut g, 0, 1, "BG32_MagicItem_270t");
    weak_board(&mut g, 0, "Southsea Busker", 3);
    let out = fight_wall(&g, 1, 10, 500, 60);
    assert_eq!(out.sides[0].spell_bonus_gain, (0, 0));
}

#[test]
fn the_eye_of_dalaran_gives_a_spell_when_a_typeless_minion_dies() {
    let mut g = game(20);
    equip(&mut g, 0, 0, "The Eye of Dalaran");
    weak_board(&mut g, 0, "Patient Scout", 1);
    let out = fight_wall(&g, 1, 10, 500, 60);
    let gained = out.sides[0].cards_gained();
    assert_eq!(gained.len(), 1, "a Tavern spell");
    assert!(g.data.cards.get(gained[0]).is_spell(), "and it is a spell");
    // A minion with a type dying gives nothing.
    let mut g = game(21);
    equip(&mut g, 0, 0, "The Eye of Dalaran");
    weak_board(&mut g, 0, "Southsea Busker", 1);
    let out = fight_wall(&g, 1, 10, 500, 60);
    assert!(out.sides[0].cards_gained().is_empty(), "a Pirate is not typeless");
}

/// A one-player game that fights a copy of its own board at the end of every
/// turn, so a fight's gains can be read from the player afterwards.
fn sandbox(seed: u64, lobby: &[Tribe]) -> Game {
    let tribes = lobby.iter().copied().fold(Tribes::NONE, |m, t| m.with(t));
    Game::new_sandbox(data(), seed, Some(tribes), 0)
}

/// Put `count` copies of a minion on player 0's board without pool copies
/// (a sandbox fixture, cleared at the end of every turn), at 1 Health.
fn fixture(g: &mut Game, p: usize, name: &str, count: usize) {
    let c = id(g, name);
    for _ in 0..count {
        let mut m = g.data.cards.make(c, false);
        m.health = 1;
        g.players[p].board.push(m);
    }
}

/// End the sandbox's turn once, answering nothing (its board is the fixture).
fn sandbox_turn(g: &mut Game) {
    g.step(Action::EndTurn).unwrap();
}

fn count_in_hand(g: &Game, p: usize, tribe: Tribe) -> usize {
    g.players[p]
        .hand
        .iter()
        .filter(|m| g.data.cards.get(m.card).tribes.has(tribe))
        .count()
}

#[test]
fn a_death_counter_takes_shop_and_fight_deaths_and_carries_across_fights() {
    // Bleeding Heart: after 8 friendly minions die, get a random Undead.
    let mut g = sandbox(31, &[Tribe::Undead, Tribe::Beast]);
    equip(&mut g, 0, 0, "Bleeding Heart");
    // Three 1/1s on both sides: every fight kills all three of ours.
    for _ in 0..2 {
        g.players[0].board.clear();
        fixture(&mut g, 0, "Patient Scout", 3);
        sandbox_turn(&mut g);
    }
    assert_eq!(g.players[0].trinkets[0].count, 6, "two fights of three deaths");
    assert_eq!(count_in_hand(&g, 0, Tribe::Undead), 0, "not eight yet");
    g.players[0].board.clear();
    fixture(&mut g, 0, "Patient Scout", 3);
    sandbox_turn(&mut g);
    assert_eq!(g.players[0].trinkets[0].count, 1, "nine deaths: fired at eight, one over");
    assert_eq!(count_in_hand(&g, 0, Tribe::Undead), 1, "a random Undead");
    // A shop death counts too (DESIGN.md: shop deaths count for death
    // counters). Butchering destroys an Undead in the shop; the counter was
    // at 1 after nine deaths and goes to 2.
    let before = g.players[0].trinkets[0].count;
    put_on_board(&mut g, 0, "Eternal Knight");
    let knight = g.players[0].board.len() as u8 - 1;
    g.give(0, "Butchering").unwrap();
    let h = g.players[0].hand.len() as u8 - 1;
    act(&mut g, 0, Action::Play { hand: h, pos: 0 });
    act(&mut g, 0, Action::Target(knight));
    assert_eq!(g.players[0].trinkets[0].count, before + 1, "the shop death counted");
}

#[test]
fn a_summon_counter_counts_the_beasts_a_fight_summons() {
    // Wildfeather Duster: after 6 Beasts are summoned, get a random Beast.
    let mut g = sandbox(32, &[Tribe::Beast, Tribe::Undead]);
    equip_id(&mut g, 0, 1, "BG35_MagicItem_700");
    // Six 1/1 Beasts whose Deathrattles each summon a Beetle: six summons.
    for _ in 0..6 {
        put_on_board(&mut g, 0, "Buzzing Vermin");
    }
    sandbox_turn(&mut g);
    assert_eq!(count_in_hand(&g, 0, Tribe::Beast), 1, "a random Beast after six summons");
}

#[test]
fn the_death_counters_name_their_tribe() {
    // The three "after N friendly minions die" trinkets, from the table.
    let g = game(33);
    for (name, tribe, every) in [
        ("Bleeding Heart", Tribe::Undead, 8),
        ("Lucky Tabby", Tribe::Beast, 7),
        ("Stormcoil Sticker", Tribe::Mech, 6),
    ] {
        let t = g.data.trinkets.get(trinket(&g, name));
        let rule = t.rule.as_ref().unwrap_or_else(|| panic!("{name}: no rule"));
        assert_eq!(rule.event, bgcore::trinkets::Event::Death, "{name}");
        assert_eq!(rule.every, every, "{name}");
        assert_eq!(
            rule.acts,
            vec![bgcore::trinkets::Act::RandomMinion { tribe, count: 1 }],
            "{name}"
        );
    }
}

#[test]
fn the_first_deathrattle_of_a_fight_fires_an_extra_time() {
    // Deathly Phylactery: your first Deathrattle each combat triggers twice.
    let mut g = game(34);
    equip(&mut g, 0, 0, "Deathly Phylactery");
    put_on_board(&mut g, 0, "Buzzing Vermin");
    let mut seen = Seen::default();
    fight_wall_watched(&g, 1, 10, 500, 60, &mut seen);
    assert_eq!(seen.summoned_all(&g, "Beetle").len(), 2, "the first one fired twice");
    // Only the first: two Vermin give three Beetles, not four.
    let mut g = game(35);
    equip(&mut g, 0, 0, "Deathly Phylactery");
    put_on_board(&mut g, 0, "Buzzing Vermin");
    put_on_board(&mut g, 0, "Buzzing Vermin");
    let mut seen = Seen::default();
    fight_wall_watched(&g, 1, 10, 500, 60, &mut seen);
    assert_eq!(seen.summoned_all(&g, "Beetle").len(), 3, "one extra, once");
    // Without it the same board summons two.
    let mut g = game(36);
    put_on_board(&mut g, 0, "Buzzing Vermin");
    put_on_board(&mut g, 0, "Buzzing Vermin");
    let mut seen = Seen::default();
    fight_wall_watched(&g, 1, 10, 500, 60, &mut seen);
    assert_eq!(seen.summoned_all(&g, "Beetle").len(), 2);
}

/// The stats of player 0's minion with this card, the first time it attacked.
fn attack_stats(seen: &Seen, g: &Game, name: &str) -> (i32, i32) {
    let m = seen.attacker(g, name).unwrap_or_else(|| panic!("{name} never attacked"));
    (m.attack, m.health)
}

#[test]
fn training_certificate_doubles_the_two_lowest_attack_minions() {
    let mut g = game(41);
    equip(&mut g, 0, 0, "Training Certificate");
    put_on_board(&mut g, 0, "Eternal Knight"); // 4/2, the highest
    put_on_board(&mut g, 0, "Patient Scout"); // 1/1
    put_on_board(&mut g, 0, "Deadly Spore"); // 1/1
    let mut seen = Seen::default();
    // A wall that hits nothing back: the shields and stats stay as they were.
    fight_wall_watched(&g, 1, 0, 500, 60, &mut seen);
    assert_eq!(attack_stats(&seen, &g, "Patient Scout"), (2, 2), "doubled");
    assert_eq!(attack_stats(&seen, &g, "Deadly Spore"), (2, 2), "doubled");
    assert_eq!(attack_stats(&seen, &g, "Eternal Knight"), (4, 2), "the highest is left alone");
}

#[test]
fn ironforge_anvil_triples_the_minions_with_no_type() {
    let mut g = game(42);
    equip_id(&mut g, 0, 1, "BG30_MagicItem_403");
    put_on_board(&mut g, 0, "Patient Scout"); // neutral 1/1
    put_on_board(&mut g, 0, "Southsea Busker"); // pirate 3/1
    let mut seen = Seen::default();
    fight_wall_watched(&g, 1, 0, 500, 60, &mut seen);
    assert_eq!(attack_stats(&seen, &g, "Patient Scout"), (3, 3), "tripled");
    assert_eq!(attack_stats(&seen, &g, "Southsea Busker"), (3, 1), "a Pirate keeps its stats");
}

#[test]
fn bronze_timepiece_adds_half_the_attack_as_health() {
    let mut g = game(43);
    equip_id(&mut g, 0, 1, "BG30_MagicItem_995");
    put_on_board(&mut g, 0, "Eternal Knight"); // 4/2: +2 Health
    put_on_board(&mut g, 0, "Southsea Busker"); // 3/1: +2 Health (3 halves up)
    let mut seen = Seen::default();
    fight_wall_watched(&g, 1, 0, 500, 60, &mut seen);
    assert_eq!(attack_stats(&seen, &g, "Eternal Knight"), (4, 4));
    assert_eq!(attack_stats(&seen, &g, "Southsea Busker"), (3, 3), "half of 3 rounds up");
}

#[test]
fn protective_ring_shields_four_random_pirates() {
    let mut g = game(44);
    equip_id(&mut g, 0, 1, "BG35_MagicItem_711");
    for _ in 0..5 {
        put_on_board(&mut g, 0, "Southsea Busker");
    }
    let mut seen = Seen::default();
    // Five swings, so each Pirate attacks exactly once and shows the shield
    // it got (nothing takes it away: the wall has no Attack).
    fight_wall_watched(&g, 1, 0, 500, 5, &mut seen);
    assert_eq!(seen.attacks.len(), 5);
    let shielded = seen
        .attacks
        .iter()
        .filter(|(s, _)| *s == 0)
        .filter(|(_, m)| m.keywords.has(bgcore::Keywords::DIVINE_SHIELD))
        .count();
    assert_eq!(shielded, 4, "four of the five Pirates, each once");
}

#[test]
fn lightfeather_sticker_shields_the_rally_minions() {
    let mut g = game(45);
    equip_id(&mut g, 0, 1, "BG36_MagicItem_213");
    put_on_board(&mut g, 0, "Flittering Bat"); // a Rally minion
    put_on_board(&mut g, 0, "Southsea Busker"); // no Rally
    let mut seen = Seen::default();
    fight_wall_watched(&g, 1, 0, 500, 60, &mut seen);
    let bat = seen.attacker(&g, "Flittering Bat").expect("the Bat attacked");
    assert!(bat.keywords.has(bgcore::Keywords::DIVINE_SHIELD), "a Rally minion is shielded");
    let pirate = seen.attacker(&g, "Southsea Busker").expect("the Pirate attacked");
    assert!(!pirate.keywords.has(bgcore::Keywords::DIVINE_SHIELD), "and a plain one is not");
}

#[test]
fn baleful_incense_gives_the_end_undead_reborn() {
    let mut g = game(46);
    equip(&mut g, 0, 0, "Baleful Incense");
    put_on_board(&mut g, 0, "Eternal Knight"); // Undead, the left end
    put_on_board(&mut g, 0, "Southsea Busker"); // a Pirate in the middle
    put_on_board(&mut g, 0, "Scarlet Survivor"); // a Dragon, the right end
    let mut seen = Seen::default();
    fight_wall_watched(&g, 1, 0, 500, 60, &mut seen);
    let knight = seen.attacker(&g, "Eternal Knight").expect("the Knight attacked");
    assert!(knight.keywords.has(bgcore::Keywords::REBORN), "the left-most Undead has it");
    let dragon = seen.attacker(&g, "Scarlet Survivor").expect("the Dragon attacked");
    assert!(!dragon.keywords.has(bgcore::Keywords::REBORN), "the right end is not an Undead");
}

#[test]
fn eternal_and_converter_portraits_give_their_own_cards_keywords() {
    // Eternal Portrait: Taunt and Reborn on your Eternal Knights.
    let mut g = game(47);
    equip(&mut g, 0, 0, "Eternal Portrait");
    put_on_board(&mut g, 0, "Eternal Knight");
    put_on_board(&mut g, 0, "Flittering Bat");
    let mut seen = Seen::default();
    fight_wall_watched(&g, 1, 0, 500, 60, &mut seen);
    let knight = seen.attacker(&g, "Eternal Knight").expect("the Knight attacked");
    assert!(knight.keywords.has(bgcore::Keywords::TAUNT | bgcore::Keywords::REBORN));
    let bat = seen.attacker(&g, "Flittering Bat").expect("the Bat attacked");
    assert!(!bat.keywords.has(bgcore::Keywords::REBORN), "another card gets nothing");
    // Converter Portrait: Reborn on your Faceless Converters.
    let mut g = game(48);
    equip(&mut g, 0, 1, "Converter Portrait");
    put_on_board(&mut g, 0, "Faceless Converter");
    put_on_board(&mut g, 0, "Flittering Bat");
    let mut seen = Seen::default();
    fight_wall_watched(&g, 1, 0, 500, 60, &mut seen);
    let conv = seen.attacker(&g, "Faceless Converter").expect("the Converter attacked");
    assert!(conv.keywords.has(bgcore::Keywords::REBORN));
}

#[test]
fn rivendare_portrait_doubles_the_health_of_your_titus() {
    let mut g = game(49);
    equip_id(&mut g, 0, 1, "BG30_MagicItem_310");
    put_on_board(&mut g, 0, "Titus Rivendare"); // 1/7
    put_on_board(&mut g, 0, "Southsea Busker"); // 3/1
    let mut seen = Seen::default();
    fight_wall_watched(&g, 1, 0, 500, 60, &mut seen);
    assert_eq!(attack_stats(&seen, &g, "Titus Rivendare"), (1, 14), "Health doubled");
    assert_eq!(attack_stats(&seen, &g, "Southsea Busker"), (3, 1), "nothing else changed");
}

#[test]
fn hoggy_bank_gives_your_quilboar_a_deathrattle() {
    let mut g = game(50);
    equip(&mut g, 0, 0, "Hoggy Bank");
    weak_board(&mut g, 0, "Crater Miner", 1); // a Quilboar at 1 Health
    put_on_board(&mut g, 0, "Southsea Busker");
    let out = fight_wall(&g, 1, 10, 500, 60);
    let gems = out.sides[0].cards_gained();
    assert_eq!(gems.len(), 2, "two Blood Gems");
    assert_eq!(Some(gems[0]), g.data.cards.blood_gem, "and they are Blood Gems");
    // A minion that is not a Quilboar gets no Deathrattle.
    let mut g = game(51);
    equip(&mut g, 0, 0, "Hoggy Bank");
    weak_board(&mut g, 0, "Southsea Busker", 1);
    let out = fight_wall(&g, 1, 10, 500, 60);
    assert!(out.sides[0].cards_gained().is_empty(), "no Quilboar, no Gems");
}

#[test]
fn herald_sticker_triggers_every_friendly_deathrattle() {
    let mut g = game(52);
    equip_id(&mut g, 0, 1, "BG32_MagicItem_306");
    put_on_board(&mut g, 0, "Buzzing Vermin");
    put_on_board(&mut g, 0, "Buzzing Vermin");
    // Both Deathrattles fire at the start of combat, with no death at all.
    let mut seen = Seen::default();
    let out = fight_wall_watched(&g, 1, 0, 500, 60, &mut seen);
    assert_eq!(seen.summoned_all(&g, "Beetle").len(), 2, "both, at Start of Combat");
    assert_eq!(out.sides[0].deathrattles_triggered, 2, "and they count as triggers");
    // And each Vermin's own Deathrattle still fires when it dies: two more.
    let out = fight_wall(&g, 1, 10, 500, 60);
    assert_eq!(out.sides[0].deathrattles_triggered, 4);
}

#[test]
fn sky_golem_portrait_grants_a_deathrattle_that_buffs_the_rest_for_good() {
    let mut g = game(53);
    equip_id(&mut g, 0, 1, "BG35_MagicItem_740");
    weak_board(&mut g, 0, "Southsea Busker", 1); // dies first
    put_on_board(&mut g, 0, "Eternal Knight"); // survives
    g.players[0].board.last_mut().unwrap().health = 100;
    // Five swings: one death, and the Knight is still there at the end.
    let out = fight_wall(&g, 1, 3, 500, 5);
    // Every minion got "Deathrattle: give your minions +2/+2 permanently",
    // so the Knight's stats are kept through the outcome.
    assert_eq!(out.sides[0].permanent_stats[1], (2, 2), "the survivor keeps +2/+2");
    assert_eq!(out.sides[0].deathrattles_triggered, 1, "the dead one's Deathrattle fired");
}

#[test]
fn fang_anklet_buffs_your_beasts_and_improves_with_every_beast_summoned() {
    let mut g = game(54);
    equip_id(&mut g, 0, 1, "BG35_MagicItem_701");
    put_on_board(&mut g, 0, "Flittering Bat"); // a Beast 1/4
    put_on_board(&mut g, 0, "Southsea Busker"); // a Pirate, untouched
    let mut seen = Seen::default();
    let out = fight_wall_watched(&g, 1, 0, 500, 60, &mut seen);
    assert_eq!(attack_stats(&seen, &g, "Flittering Bat"), (2, 5), "Beasts have +1/+1");
    assert_eq!(attack_stats(&seen, &g, "Southsea Busker"), (3, 1), "Pirates do not");
    assert_eq!(out.sides[0].summon_tribes[0], 0, "no Beast was summoned");
    // A Beast summoned during the fight improves the anklet for good.
    let mut g = game(55);
    equip_id(&mut g, 0, 1, "BG35_MagicItem_701");
    weak_board(&mut g, 0, "Buzzing Vermin", 1); // its Beetle is a summon
    let out = fight_wall(&g, 1, 10, 500, 60);
    assert_eq!(out.sides[0].trinket_improve[1], 1, "one Beast summon");
}

#[test]
fn dramaloc_sticker_gives_your_murlocs_the_hand_attack() {
    let mut g = game(56);
    equip_id(&mut g, 0, 1, "BG35_MagicItem_754");
    put_on_board(&mut g, 0, "Bubble Gunner"); // a Murloc 2/3
    put_on_board(&mut g, 0, "Southsea Busker"); // a Pirate
    // The hand: the game only passes it to a fight that reads it.
    g.players[0].board.clear();
    let mut murloc = g.data.cards.make(id(&g, "Bubble Gunner"), false);
    murloc.attack = 2;
    g.players[0].board.push(murloc);
    let mut strong = g.data.cards.make(id(&g, "Eternal Knight"), false);
    strong.attack = 9;
    g.players[0].hand.push(strong);
    assert!(g.players[0].trinket_reads_hand(&g.data), "the fight is given the hand");
    let board = g.players[0].board.clone();
    let hand = g.players[0].hand.clone();
    let counters = [g.players[0].counters(&g.data, g.lobby), bgcore::Counters::default()];
    let mut seen = Seen::default();
    bgcore::fight::fight_with_hands(
        &g.data.cards,
        &board,
        &[wall(&g, 0, 500)],
        [&hand, &[]],
        counters,
        60,
        &mut Rng::new(1),
        &mut seen,
    );
    let bubble = seen.attacker(&g, "Bubble Gunner").expect("the Murloc attacked");
    assert_eq!(bubble.attack, 2 + 9, "it gained the hand minion's Attack");
}

/// Every attack a minion of player 0 made with this card, in order, as
/// `(attack, health)` at the moment it swung.
fn swings(seen: &Seen, g: &Game, name: &str) -> Vec<(i32, i32)> {
    seen.attacks
        .iter()
        .filter(|(s, m)| *s == 0 && g.data.cards.get(m.card).name == name)
        .map(|(_, m)| (m.attack, m.health))
        .collect()
}

#[test]
fn herding_horn_gives_a_free_refresh_for_every_rally_swing() {
    let mut g = game(61);
    equip(&mut g, 0, 0, "Herding Horn");
    put_on_board(&mut g, 0, "Flittering Bat"); // has a Rally
    put_on_board(&mut g, 0, "Southsea Busker"); // and one without
    // Two swings: the Bat's and the Pirate's.
    let out = fight_wall(&g, 1, 0, 500, 2);
    assert_eq!(out.sides[0].free_refresh_gain, 1, "one free Refresh per Rally swing");
    // A Windfury Rally minion swings twice and gives two.
    let mut g = game(62);
    equip(&mut g, 0, 0, "Herding Horn");
    put_on_board(&mut g, 0, "Flittering Bat");
    g.players[0].board[0].keywords.set(bgcore::Keywords::WINDFURY);
    // Two attacks, each a Windfury pair of swings: four.
    let out = fight_wall(&g, 1, 0, 500, 2);
    assert_eq!(out.sides[0].free_refresh_gain, 4, "every swing counts");
}

#[test]
fn lionfish_portrait_buffs_every_attacking_beast() {
    let mut g = game(63);
    take(&mut g, 0, "Lionfish Portrait");
    assert_eq!(hand_of(&g, 0, "Lurking Lionfish"), 1, "the pick hands one over");
    put_on_board(&mut g, 0, "Flittering Bat"); // Beast 1/4
    put_on_board(&mut g, 0, "Southsea Busker"); // Pirate 3/1
    let mut seen = Seen::default();
    let out = fight_wall_watched(&g, 1, 0, 500, 2, &mut seen);
    assert_eq!(swings(&seen, &g, "Flittering Bat"), [(5, 6)], "+4/+2 on the Beast");
    assert_eq!(swings(&seen, &g, "Southsea Busker"), [(3, 1)], "not on the Pirate");
    assert!(out.sides[0].cards_gained().is_empty(), "the fight itself gives no card");
}

#[test]
fn all_purpose_kibble_grows_with_every_beast_attack() {
    let mut g = game(64);
    equip_id(&mut g, 0, 1, "BG32_MagicItem_200");
    put_on_board(&mut g, 0, "Flittering Bat"); // 1/4
    let mut seen = Seen::default();
    let out = fight_wall_watched(&g, 1, 0, 500, 2, &mut seen);
    assert_eq!(
        swings(&seen, &g, "Flittering Bat"),
        [(1 + 3, 4), (1 + 3 + 4, 4)],
        "+3, then +4: it improved"
    );
    assert_eq!(out.sides[0].trinket_improve[1], 2, "two attacks improved it twice");
}

#[test]
fn slamma_sticker_doubles_the_attack_of_a_summoned_beast() {
    let mut g = game(65);
    equip_id(&mut g, 0, 1, "BG30_MagicItem_540");
    weak_board(&mut g, 0, "Buzzing Vermin", 1);
    let mut seen = Seen::default();
    fight_wall_watched(&g, 1, 10, 500, 20, &mut seen);
    let beetle = seen.summoned(&g, "Beetle").expect("the Beetle was summoned");
    assert_eq!((beetle.attack, beetle.health), (4, 2), "Attack doubled");
}

#[test]
fn reinforced_shield_shields_the_first_five_summons() {
    let mut g = game(66);
    equip_id(&mut g, 0, 1, "BG30_MagicItem_886");
    weak_board(&mut g, 0, "Buzzing Vermin", 6);
    let mut seen = Seen::default();
    fight_wall_watched(&g, 1, 10, 500, 40, &mut seen);
    let beetles = seen.summoned_all(&g, "Beetle");
    assert_eq!(beetles.len(), 6, "six Vermin, six Beetles");
    let shielded = beetles
        .iter()
        .filter(|m| m.keywords.has(bgcore::Keywords::DIVINE_SHIELD))
        .count();
    assert_eq!(shielded, 5, "five times per combat, then no more");
}

#[test]
fn faerie_dragon_scale_shields_three_attacks() {
    let mut g = game(67);
    equip_id(&mut g, 0, 1, "BG32_MagicItem_363");
    put_on_board(&mut g, 0, "Scarlet Survivor"); // a Dragon
    g.players[0].board[0].health = 100;
    let mut seen = Seen::default();
    // The wall hits back for 1, which takes each new shield off, so every
    // swing shows whether it got a fresh one.
    fight_wall_watched(&g, 1, 1, 500, 12, &mut seen);
    let ds: Vec<bool> = seen
        .attacks
        .iter()
        .filter(|(s, m)| *s == 0 && g.data.cards.get(m.card).name == "Scarlet Survivor")
        .map(|(_, m)| m.keywords.has(bgcore::Keywords::DIVINE_SHIELD))
        .collect();
    assert!(ds.len() >= 4, "the Dragon swung enough times");
    assert_eq!(&ds[..4], [true, true, true, false], "three times per combat, then no more");
}

#[test]
fn funeral_wreath_gives_a_plain_copy_of_a_reborn_minion() {
    let mut g = game(68);
    equip_id(&mut g, 0, 1, "BG36_MagicItem_217");
    weak_board(&mut g, 0, "Risen Rider", 4); // 2/1, Reborn
    let out = fight_wall(&g, 1, 10, 500, 40);
    let gained = out.sides[0].cards_gained();
    assert_eq!(gained.len(), 3, "three times per combat");
    for &c in gained {
        assert_eq!(c, id(&g, "Risen Rider"), "a plain copy of the card");
    }
}

#[test]
fn jar_o_gems_plays_a_blood_gem_on_your_quilboar_after_two_attacks() {
    let mut g = game(69);
    equip_id(&mut g, 0, 1, "BG30_MagicItem_546");
    put_on_board(&mut g, 0, "Crater Miner"); // Quilboar 2/2
    put_on_board(&mut g, 0, "Southsea Busker"); // Pirate
    let mut seen = Seen::default();
    fight_wall_watched(&g, 1, 0, 500, 4, &mut seen);
    // The Gem lands after the second friendly attack: from then on the
    // Quilboar swings at +1/+1.
    assert_eq!(
        swings(&seen, &g, "Crater Miner"),
        [(2, 2), (3, 3)],
        "a Gem after the second attack, once"
    );
    assert_eq!(swings(&seen, &g, "Southsea Busker"), [(3, 1), (3, 1)], "Pirates get nothing");
}

#[test]
fn evil_experiment_gives_the_deity_reborn_when_it_awakens() {
    let mut g = game(70);
    equip_id(&mut g, 0, 1, "BG36_MagicItem_416");
    // Four Aberrations die, and the fifth death... the fourth does it: the
    // Deity awakens with `deity_left` at 0.
    weak_board(&mut g, 0, "Joyous", 4);
    let mut seen = Seen::default();
    let mut counters = g.players[0].counters(&g.data, g.lobby);
    counters.deity = Some(bgcore::fight::Deity {
        card: id(&g, "C'Thun"),
        attack: 10,
        health: 10,
        golden: false,
    });
    let board = g.players[0].board.clone();
    bgcore::fight::fight_with(
        &g.data.cards,
        &board,
        &[wall(&g, 10, 500)],
        [counters, bgcore::Counters::default()],
        60,
        &mut Rng::new(1),
        &mut seen,
    );
    let god = seen.attacker(&g, "C'Thun").expect("the Deity woke up and swung");
    assert!(god.keywords.has(bgcore::Keywords::REBORN), "and it has Reborn");
}

#[test]
fn a_death_counter_can_name_a_tribe() {
    // The vocabulary a later "after N friendly Beasts die" trinket needs: a
    // tribe on the death event. No built card uses it yet.
    let toml = r#"
[[trinket]]
name = "Test Muster"
card_id = "TEST_Trinket_3"
slot = "lesser"
tribes = ["beast"]
rule = { event = "death_beast", every = 2, acts = [{ gold = 1 }] }
"#;
    let d = Arc::new(
        Data::from_toml_all(
            include_str!("../../../cards.toml"),
            include_str!("../../../rules.toml"),
            include_str!("../../../heroes.toml"),
            toml,
        )
        .unwrap(),
    );
    let mut g = Game::new_with_lobby(d, 71, all_tribes());
    g.set_trinket(0, 0, Some(0));
    weak_board(&mut g, 0, "Buzzing Vermin", 1); // a Beast that leaves a Beast behind
    weak_board(&mut g, 0, "Patient Scout", 1); // no type
    let out = fight_wall(&g, 1, 10, 500, 60);
    let beast = Tribe::Beast as usize;
    assert_eq!(out.sides[0].death_tribes[beast], 2, "the Vermin and its Beetle");
    assert_eq!(out.sides[0].friendly_deaths, 3, "and the neutral one counts overall");
    let neutral = out.sides[0].friendly_deaths - out.sides[0].death_tribes[beast];
    assert_eq!(neutral, 1, "one death that is not a Beast");
}

// ---- block 2: the minions only trinkets hand out, and getting cards ----

#[test]
fn the_minion_portraits_hand_out_their_token() {
    let cases: [(&str, &str); 7] = [
        ("Morgl Portrait", "Tide Oracle Morgl"),
        ("Behemoth Portrait", "Arcane Behemoth"),
        ("Boom's Monster Portrait", "Dr. Boom's Monster"),
        ("Grifter Portrait", "Doubloon Grifter"),
        ("Bassgill Portrait", "Bassgill"),
        ("Manipulator Portrait", "Faceless Manipulator"),
        ("Fish Portrait", "Fish of N'Zoth"),
    ];
    for (name, card) in cases {
        let mut g = game(80);
        take(&mut g, 0, name);
        assert_eq!(hand_of(&g, 0, card), 1, "{name} hands out a {card}");
        // A token: no pool copy, so selling it gives nothing back.
        let m = g.players[0].hand[hand_of(&g, 0, card) - 1];
        assert_eq!(m.pool_copies, 0, "{card} is generated, not a pool card");
    }
}

#[test]
fn booms_monster_portrait_hands_out_another_copy_each_turn() {
    let mut g = game(81);
    take(&mut g, 0, "Boom's Monster Portrait");
    assert_eq!(hand_of(&g, 0, "Dr. Boom's Monster"), 1, "one on the pick");
    to_turn(&mut g, 6);
    assert_eq!(hand_of(&g, 0, "Dr. Boom's Monster"), 2, "another at the start of turn 6");
}

#[test]
fn grifter_portrait_makes_the_first_pirate_buy_of_the_turn_free() {
    let mut g = game(82);
    take(&mut g, 0, "Grifter Portrait");
    assert_eq!(g.players[0].tp.free_tribe_left, 1);
    set_shop(&mut g, 0, &["Southsea Busker", "Microbot"]);
    g.players[0].gold = 10;
    let gold = g.players[0].gold;
    act(&mut g, 0, Action::Buy(0));
    assert_eq!(g.players[0].gold, gold, "the first Pirate was free");
    set_shop(&mut g, 0, &["Southsea Busker"]);
    act(&mut g, 0, Action::Buy(0));
    assert_eq!(g.players[0].gold, gold - 3, "the second Pirate costs Gold");
}

#[test]
fn accord_o_tron_portrait_magnetizes_both_ends_of_the_mechs() {
    let mut g = game(83);
    put_on_board(&mut g, 0, "Auto Accelerator");
    put_on_board(&mut g, 0, "Microbot");
    equip(&mut g, 0, 1, "Accord-o-Tron Portrait");
    let before: Vec<(i32, i32)> =
        g.players[0].board.iter().map(|m| (m.attack, m.health)).collect();
    let next = g.turn + 1;
    to_turn(&mut g, next);
    let after: Vec<(i32, i32)> =
        g.players[0].board.iter().map(|m| (m.attack, m.health)).collect();
    let accord = id(&g, "Accord-o-Tron");
    for i in 0..2 {
        assert_eq!(
            after[i],
            (before[i].0 + 3, before[i].1 + 3),
            "minion {i} took the Accord-o-Tron's 3/3"
        );
        assert!(
            g.players[0].board[i].parts().any(|(c, _)| c == accord),
            "and as a Magnetized card, not a plain buff"
        );
    }
    assert_eq!(g.players[0].board.len(), 2, "Magnetizing adds no minion");
}

#[test]
fn bassgill_portrait_shields_a_murloc_summoned_in_combat() {
    let mut g = game(84);
    equip(&mut g, 0, 1, "Bassgill Portrait");
    let murloc = id(&g, "Bassgill");
    let m = g.players[0].make_card(&g.data.cards, murloc, false, 0);
    g.players[0].hand.push(m);
    weak_board(&mut g, 0, "Bassgill", 1);
    let mut w = Seen::default();
    let board = g.players[0].board.clone();
    let hand = g.players[0].hand.clone();
    let counters = [g.players[0].counters(&g.data, g.lobby), bgcore::Counters::default()];
    let _ = bgcore::fight::fight_with_hands(
        &g.data.cards,
        &board,
        &[wall(&g, 10, 900)],
        [&hand, &[]],
        counters,
        60,
        &mut Rng::new(1),
        &mut w,
    );
    let out = w.summoned_all(&g, "Bassgill");
    assert_eq!(out.len(), 1, "the hand's Bassgill came out on its death");
    assert_eq!(
        out[0].keywords.0 & bgcore::Keywords::DIVINE_SHIELD,
        bgcore::Keywords::DIVINE_SHIELD,
        "the summoned Murloc got Divine Shield"
    );
}

#[test]
fn portable_factory_remembers_its_discover_and_copies_it_each_turn() {
    let mut g = game(85);
    take(&mut g, 0, "Portable Factory");
    assert!(!g.players[0].discover.is_empty(), "a Discover is open on the pick");
    for &c in &g.players[0].discover {
        assert_eq!(g.data.cards.get(c).tier, 4, "a tier 4 minion");
        assert!(!g.data.cards.get(c).tribes.is_neutral(), "with a type");
    }
    let picked = g.players[0].discover[0];
    act(&mut g, 0, Action::Choose(0));
    assert_eq!(g.players[0].trinkets[0].card, Some(picked), "the trinket remembers it");
    to_turn(&mut g, 6);
    let same = g.players[0].hand.iter().filter(|m| m.card == picked).count();
    assert_eq!(same, 2, "another copy at the start of the next turn");
}

#[test]
fn azeroth_model_globe_pays_every_two_turns() {
    let mut g = game(86);
    to_turn(&mut g, 5);
    turn_of(&mut g, 0);
    equip(&mut g, 0, 0, "Azeroth Model Globe");
    let hands = g.players[0].hand.len();
    to_turn(&mut g, 6);
    assert_eq!(g.players[0].hand.len(), hands, "turn 6 is the first turn counted: no pay");
    to_turn(&mut g, 7);
    assert_eq!(g.players[0].gold, g.data.rules.gold(7) + 2, "2 Gold on turn 7");
    assert!(!g.players[0].discover.is_empty(), "and a Discover is open");
    for &c in &g.players[0].discover {
        assert_eq!(g.data.cards.get(c).tier, 6, "of tier 6 minions");
    }
    turn_of(&mut g, 0);
    let hands = g.players[0].hand.len();
    act(&mut g, 0, Action::Choose(0));
    assert_eq!(g.players[0].hand.len(), hands + 1, "the pick is in hand");
}

#[test]
fn rockin_music_box_gets_a_battlecry_minion_each_turn() {
    let mut g = game(87);
    to_turn(&mut g, 5);
    let before = g.players[0].hand.len();
    take(&mut g, 0, "Rockin' Music Box");
    assert_eq!(g.players[0].hand.len(), before + 1, "one on the pick");
    for m in &g.players[0].hand {
        assert!(g.data.cards.get(m.card).battlecry.is_some(), "a Battlecry minion");
    }
    to_turn(&mut g, 6);
    assert_eq!(g.players[0].hand.len(), before + 2, "another at the start of the turn");
}

#[test]
fn reflective_pendant_copies_a_friendly_minion_plain() {
    let mut g = game(88);
    put_on_board(&mut g, 0, "Lullabot");
    g.players[0].board[0].attack += 5;
    take(&mut g, 0, "Reflective Pendant");
    let copy = g.players[0].hand[0];
    assert_eq!(copy.card, id(&g, "Lullabot"));
    let base = g.data.cards.get(copy.card);
    assert_eq!((copy.attack, copy.health), (base.attack, base.health), "plain: no buffs");
    assert_eq!(copy.pool_copies, 0, "a generated copy");
}

#[test]
fn transcribing_typewriter_copies_the_next_two_minions_bought() {
    let mut g = game(89);
    take(&mut g, 0, "Transcribing Typewriter");
    assert_eq!(g.players[0].tp.copy_buys, 2);
    set_shop(&mut g, 0, &["Lullabot", "Southsea Busker"]);
    g.players[0].gold = 10;
    act(&mut g, 0, Action::Buy(0));
    assert_eq!(hand_of(&g, 0, "Lullabot"), 2, "an extra copy of the first buy");
    act(&mut g, 0, Action::Buy(0));
    assert_eq!(hand_of(&g, 0, "Southsea Busker"), 2, "and of the second");
    assert_eq!(g.players[0].tp.copy_buys, 0, "both uses spent");
    set_shop(&mut g, 0, &["Clever Castaway"]);
    act(&mut g, 0, Action::Buy(0));
    assert_eq!(hand_of(&g, 0, "Clever Castaway"), 1, "no third copy");
}

#[test]
fn writhing_tentacles_copies_the_first_discarded_minion_doubled() {
    let mut g = game(90);
    equip(&mut g, 0, 1, "Writhing Tentacles");
    put_in_hand(&mut g, 0, "Lullabot");
    let m = g.players[0].hand[0];
    g.players[0].pending_discard = Some(bgcore::game::PendingDiscard {
        source: None,
        reward: bgcore::Effect::GoldNextTurn(0),
        golden: false,
    });
    act(&mut g, 0, Action::Discard(0));
    assert_eq!(g.players[0].hand.len(), 1, "the copy arrived");
    let copy = g.players[0].hand[0];
    assert_eq!(copy.card, m.card);
    assert_eq!((copy.attack, copy.health), (m.attack * 2, m.health * 2));
    // Only the first minion each turn.
    put_in_hand(&mut g, 0, "Microbot");
    let slot = g.players[0].hand.len() - 1;
    g.players[0].pending_discard = Some(bgcore::game::PendingDiscard {
        source: None,
        reward: bgcore::Effect::GoldNextTurn(0),
        golden: false,
    });
    act(&mut g, 0, Action::Discard(slot as u8));
    assert_eq!(g.players[0].hand.len(), 1, "no second copy this turn");
}

#[test]
fn aggem_sticker_plays_gems_on_one_minion_of_each_type() {
    let mut g = game(91);
    put_on_board(&mut g, 0, "Accord-o-Tron"); // mech
    put_on_board(&mut g, 0, "Southsea Busker"); // pirate
    put_on_board(&mut g, 0, "Accord-o-Tron"); // mech again
    equip(&mut g, 0, 1, "Aggem Sticker");
    let before: Vec<(i32, i32)> =
        g.players[0].board.iter().map(|m| (m.attack, m.health)).collect();
    let next = g.turn + 1;
    to_turn(&mut g, next);
    let after: Vec<(i32, i32)> =
        g.players[0].board.iter().map(|m| (m.attack, m.health)).collect();
    let (gem_a, gem_h) = (g.players[0].gem_attack, g.players[0].gem_health);
    let grown: Vec<bool> = (0..3).map(|i| after[i] != before[i]).collect();
    assert_eq!(
        grown.iter().filter(|&&b| b).count(),
        2,
        "the Pirate and one of the two Mechs: one minion per type"
    );
    for i in 0..3 {
        if grown[i] {
            assert_eq!(
                (after[i].0 - before[i].0, after[i].1 - before[i].1),
                (gem_a * 7, gem_h * 7),
                "seven Gems on minion {i}"
            );
        }
    }
}

#[test]
fn sinstone_sticker_copies_the_first_two_discovers_each_turn() {
    let mut g = game(92);
    take(&mut g, 0, "Sinstone Sticker");
    assert_eq!(g.players[0].tp.discover_copies, 2);
    for n in 0..2 {
        let book = id(&g, "Boundless Potential");
        g.players[0].hand.push(g.players[0].make_card(&g.data.cards, book, false, 0));
        let slot = g.players[0].hand.len() - 1;
        act(&mut g, 0, Action::Play { hand: slot as u8, pos: 0 });
        // Boundless Potential is a Choose One: the minion half first.
        if g.players[0].pending_choose_one.is_some() {
            act(&mut g, 0, Action::Choose(0));
        }
        assert!(!g.players[0].discover.is_empty(), "discover {n}");
        let picked = g.players[0].discover[0];
        act(&mut g, 0, Action::Choose(0));
        let got = g.players[0].hand.iter().filter(|m| m.card == picked).count();
        assert_eq!(got, 2, "the pick and its copy ({n})");
    }
    assert_eq!(g.players[0].tp.discover_copies, 0, "two is the cap");
}

#[test]
fn trusty_crowbar_buffs_the_left_most_on_every_pirate() {
    let mut g = game(93);
    put_on_board(&mut g, 0, "Lullabot");
    put_on_board(&mut g, 0, "Microbot");
    equip(&mut g, 0, 1, "Trusty Crowbar");
    let first = g.players[0].board[0];
    let second = g.players[0].board[1];
    assert!(g.give(0, "Southsea Busker").is_some());
    let now = g.players[0].board[0];
    assert_eq!(
        (now.attack, now.health),
        (first.attack + 16, first.health + 16),
        "the left-most minion got +16/+16"
    );
    assert_eq!(g.players[0].board[1].attack, second.attack, "nobody else did");
    // A non-Pirate does nothing.
    assert!(g.give(0, "Microbot").is_some());
    assert_eq!(g.players[0].board[0].attack, now.attack);
}

#[test]
fn warband_whistle_gives_a_free_refresh_of_the_warband() {
    let mut g = game(94);
    put_on_board(&mut g, 0, "Lullabot");
    put_on_board(&mut g, 0, "Microbot");
    let free = g.players[0].free_rolls;
    take(&mut g, 0, "Warband Whistle");
    assert_eq!(g.players[0].free_rolls, free + 1, "a free Refresh");
    act(&mut g, 0, Action::Roll);
    let warband: Vec<CardId> = g.players[0].board.iter().map(|m| m.card).collect();
    assert!(!g.players[0].shop.is_empty());
    for m in &g.players[0].shop {
        assert!(warband.contains(&m.card), "the Tavern holds warband copies");
        assert_eq!(m.pool_copies, 0, "plain copies, no pool copy");
    }
}

#[test]
fn designer_eyepatch_makes_two_pirates_golden() {
    let mut g = game(95);
    take(&mut g, 0, "Designer Eyepatch");
    put_in_hand(&mut g, 0, "Southsea Busker");
    set_shop(&mut g, 0, &["Southsea Busker"]);
    g.players[0].gold = 10;
    act(&mut g, 0, Action::Buy(0));
    assert_eq!(hand_of(&g, 0, "Southsea Busker"), 1, "two copies became one");
    let busker = *g.players[0].hand.iter().find(|m| m.card == id(&g, "Southsea Busker")).unwrap();
    assert!(busker.golden, "and it is Golden");
    // Two copies of something that is not a Pirate do not triple.
    put_in_hand(&mut g, 0, "Lullabot");
    set_shop(&mut g, 0, &["Lullabot"]);
    act(&mut g, 0, Action::Buy(0));
    assert_eq!(hand_of(&g, 0, "Lullabot"), 2, "still two Lullabots");
}

#[test]
fn goldenizer_supply_hands_out_the_spell_every_three_turns() {
    let mut g = game(96);
    to_turn(&mut g, 5);
    turn_of(&mut g, 0);
    equip(&mut g, 0, 0, "Goldenizer Supply");
    let start = g.turn;
    for _ in 0..3 {
        let next = g.turn + 1;
    to_turn(&mut g, next);
    }
    assert_eq!(hand_of(&g, 0, "The Goldenizer"), 1, "one after three turns (from {start})");
    // The Goldenizer makes a minion Golden without a Triple Reward.
    put_on_board(&mut g, 0, "Lullabot");
    let slot = g.players[0].hand.iter().position(|m| m.card == id(&g, "The Goldenizer")).unwrap();
    act(&mut g, 0, Action::Play { hand: slot as u8, pos: 0 });
    act(&mut g, 0, Action::Target(0));
    assert!(g.players[0].board[0].golden);
    assert_eq!(g.players[0].golden_played, 0, "not a Golden played");
}

#[test]
fn the_two_turn_portraits_hand_out_their_spell_again() {
    for (name, card) in [("Conch Portrait", "Cloning Conch"), ("Lens Case", "Duplicating Lens")] {
        let mut g = game(97);
        to_turn(&mut g, 5);
        turn_of(&mut g, 0);
        take(&mut g, 0, name);
        assert_eq!(hand_of(&g, 0, card), 1, "{name} on the pick");
        to_turn(&mut g, 6);
        assert_eq!(hand_of(&g, 0, card), 1, "{name}: not on the first turn after");
        to_turn(&mut g, 7);
        assert_eq!(hand_of(&g, 0, card), 2, "{name}: again two turns later");
    }
}

#[test]
fn lens_case_spell_copies_a_small_minion_plain() {
    let mut g = game(98);
    put_on_board(&mut g, 0, "Lullabot");
    g.players[0].board[0].attack += 4;
    put_on_board(&mut g, 0, "Captain Sanders"); // tier 5: too big to copy
    take(&mut g, 0, "Lens Case");
    let slot = g.players[0].hand.iter().position(|m| m.card == id(&g, "Duplicating Lens")).unwrap();
    act(&mut g, 0, Action::Play { hand: slot as u8, pos: 0 });
    act(&mut g, 0, Action::Target(0));
    let copy = *g.players[0].hand.last().unwrap();
    assert_eq!(copy.card, id(&g, "Lullabot"));
    let base = g.data.cards.get(copy.card);
    assert_eq!((copy.attack, copy.health), (base.attack, base.health), "plain");
}

#[test]
fn the_lesser_baller_portrait_hands_out_one_baller_each_turn() {
    let mut g = game(99);
    to_turn(&mut g, 5);
    take(&mut g, 0, "Baller Portrait");
    let ballers = |g: &Game| hand_of(g, 0, "Fire Baller") + hand_of(g, 0, "Snow Baller");
    assert_eq!(ballers(&g), 1, "one of the two");
    to_turn(&mut g, 6);
    assert_eq!(ballers(&g), 2, "another at the start of the turn");
}

#[test]
fn the_greater_baller_portrait_repeats_after_eleven_elementals() {
    let mut g = game(100);
    let i = g.data.trinkets.by_card_id("BG35_MagicItem_861").unwrap();
    g.set_trinket(0, 1, Some(i));
    // Its pick effect: a Temperature Shift. (Equipped, so run it by hand.)
    let shift = id(&g, "Temperature Shift");
    g.players[0].hand.push(g.players[0].make_card(&g.data.cards, shift, false, 0));
    let elementals: Vec<CardId> = ["Water Droplet", "Maelstrom Portal"]
        .iter()
        .filter_map(|n| g.data.cards.by_name(n))
        .collect();
    assert!(!elementals.is_empty(), "an Elemental token to play");
    for _ in 0..11 {
        let m = g.players[0].make_card(&g.data.cards, elementals[0], false, 0);
        g.players[0].hand.push(m);
        let slot = g.players[0].hand.len() - 1;
        let pos = g.players[0].board.len();
        act(&mut g, 0, Action::Play { hand: slot as u8, pos: pos as u8 });
        // Keep the board from filling up: the Elemental is the effect's
        // concern, not the board's.
        if g.players[0].board.len() >= 7 {
            g.players[0].board.clear();
        }
    }
    assert_eq!(hand_of(&g, 0, "Temperature Shift"), 2, "eleven Elementals repeated it");
}

#[test]
fn escapee_portrait_gives_a_golden_escapee_and_earlier_lockboxes() {
    let mut g = game(102);
    take(&mut g, 0, "Escapee Portrait");
    let escapee = id(&g, "Enterprising Escapee");
    let m = *g.players[0].hand.iter().find(|m| m.card == escapee).expect("the Escapee");
    assert!(m.golden, "Golden");
    assert_eq!((m.attack, m.health), (12, 12), "the golden stats");
    assert_eq!(g.players[0].tp.lockbox_sooner, 1);
    // A Lockbox this player gets opens one turn sooner: Lockbox Portrait
    // hands one out at the start of the turn.
    let lockbox_portrait = trinket(&g, "Lockbox Portrait");
    g.set_trinket(0, 0, Some(lockbox_portrait));
    to_turn(&mut g, 6);
    let lockbox = g
        .players[0]
        .hand
        .iter()
        .find(|m| m.card == id(&g, "Lockbox"))
        .expect("the Lockbox Portrait gave one");
    assert_eq!(lockbox.improvements, 4, "5 turns minus the 1");
}

// ---- the hero power trinkets ----

#[test]
fn the_ancient_wishbone_runs_the_hero_power_twice() {
    // Bloodbound: 2 Blood Gems a press, twice a turn. With the Wishbone one
    // press gives 4, and it still costs its 1 Gold once.
    let mut g = game(170);
    let h = g.data.heroes.by_name("Death Speaker Blackthorn").unwrap();
    g.assign_hero(0, h);
    take(&mut g, 0, "Ancient Wishbone");
    g.players[0].gold = 10;
    let before = g.players[0].hand.iter().filter(|m| g.data.cards.get(m.card).blood_gem).count();
    act(&mut g, 0, Action::HeroPower);
    let after = g.players[0].hand.iter().filter(|m| g.data.cards.get(m.card).blood_gem).count();
    assert_eq!(after - before, 4, "twice the Gems");
    assert_eq!(g.players[0].gold, 9, "the press costs once");
    // A passive power fires twice too: Clairvoyance's free Refresh.
    let mut g = game(171);
    let h = g.data.heroes.by_name("Nozdormu").unwrap();
    g.assign_hero(0, h);
    take(&mut g, 0, "Ancient Wishbone");
    let rolls = g.players[0].free_rolls;
    next_turn(&mut g);
    assert_eq!(g.players[0].free_rolls, rolls + 2, "two free Refreshes");
    g.check_invariants().unwrap();
}

#[test]
fn the_sous_chef_sticker_gives_a_second_press_and_a_gold_back() {
    let mut g = game(172);
    let h = g.data.heroes.by_name("Death Speaker Blackthorn").unwrap();
    g.assign_hero(0, h);
    take(&mut g, 0, "Sous Chef Sticker");
    assert_eq!(g.players[0].power.unwrap().uses_left, 3, "two a turn plus one");
    g.players[0].gold = 10;
    act(&mut g, 0, Action::HeroPower);
    assert_eq!(g.players[0].gold, 10, "1 Gold paid, 1 back");
    turn_of(&mut g, 0);
    assert!(g.is_legal(Action::HeroPower), "a third press");
    act(&mut g, 0, Action::HeroPower);
    assert!(!g.is_legal(Action::HeroPower), "and no fourth");
    // The next turn starts with the extra use again.
    next_turn(&mut g);
    turn_of(&mut g, 0);
    assert_eq!(g.players[0].power.unwrap().uses_left, 3);
    g.check_invariants().unwrap();
}

// ---- the Tavern's shape ----

#[test]
fn the_glowing_gauntlet_makes_six_minions_and_a_spell_that_refill() {
    let mut g = game(180);
    take(&mut g, 0, "Glowing Gauntlet");
    g.players[0].tier = 1;
    next_turn(&mut g);
    turn_of(&mut g, 0);
    assert_eq!(g.players[0].shop.len(), 6, "six minions at tier 1");
    assert!(g.players[0].shop_spell.is_some(), "and the spell");
    for m in &g.players[0].shop {
        // +3/+3 from the Gauntlet's Tavern buff.
        let c = g.data.cards.get(m.card);
        assert_eq!((m.attack, m.health), (c.attack + 3, c.health + 3), "{}", c.name);
    }
    // A bought minion is replaced by a minion.
    g.players[0].gold = 10;
    let bought = g.players[0].shop[0].card;
    act(&mut g, 0, Action::Buy(0));
    assert_eq!(g.players[0].shop.len(), 6, "a minion took the slot");
    assert!(g.players[0].shop_spell.is_some(), "the spell slot keeps its spell");
    g.check_invariants().unwrap();
    // A bought spell is replaced by a spell.
    let spell = g.players[0].shop_spell;
    act(&mut g, 0, Action::BuySpell);
    assert!(g.players[0].shop_spell.is_some(), "another spell");
    assert_ne!(g.players[0].shop_spell, spell, "a different one is fine too");
    assert_eq!(g.players[0].shop.len(), 6);
    assert!(!g.players[0].hand.is_empty(), "the bought cards are in hand");
    let _ = bought;
    g.check_invariants().unwrap();
}

#[test]
fn the_innkeepers_stein_offers_a_minion_of_a_higher_tier() {
    let mut g = game(181);
    g.players[0].tier = 2;
    take(&mut g, 0, "Innkeeper's Stein");
    next_turn(&mut g);
    turn_of(&mut g, 0);
    let size = g.data.rules.shop_size(2);
    assert_eq!(g.players[0].shop.len(), size + 1, "one extra slot");
    let high = g.players[0]
        .shop
        .iter()
        .filter(|m| g.data.cards.get(m.card).tier == 3)
        .count();
    assert_eq!(high, 1, "and it is one tier above");
    // At tier 6 there is no higher tier, so the slot does not appear.
    g.players[0].tier = 6;
    next_turn(&mut g);
    turn_of(&mut g, 0);
    assert_eq!(g.players[0].shop.len(), 6, "six is all a Tavern has");
    g.check_invariants().unwrap();
}

#[test]
fn the_electrode_attractor_offers_a_magnetic_mech_and_prices_them_at_two() {
    let mut g = game(182);
    g.players[0].tier = 3;
    take(&mut g, 0, "Electrode Attractor");
    next_turn(&mut g);
    turn_of(&mut g, 0);
    let size = g.data.rules.shop_size(3);
    assert_eq!(g.players[0].shop.len(), size + 1, "one extra slot");
    let mechs = g.players[0]
        .shop
        .iter()
        .filter(|m| {
            let c = g.data.cards.get(m.card);
            c.tribes.has(Tribe::Mech) && c.keywords.has(bgcore::Keywords::MAGNETIC)
        })
        .count();
    assert_eq!(mechs, 1, "a Magnetic Mech");
    let slot = (0..g.players[0].shop.len())
        .find(|&s| {
            let c = g.data.cards.get(g.players[0].shop[s].card);
            c.tribes.has(Tribe::Mech) && c.keywords.has(bgcore::Keywords::MAGNETIC)
        })
        .unwrap();
    g.players[0].gold = 2;
    act(&mut g, 0, Action::Buy(slot as u8));
    assert_eq!(g.players[0].gold, 0, "2 Gold, not 3");
    g.check_invariants().unwrap();
}

#[test]
fn the_guiding_candle_draws_tier_six_for_two_refreshes() {
    let mut g = game(183);
    g.players[0].tier = 2;
    take(&mut g, 0, "Guiding Candle");
    g.players[0].gold = 30;
    g.players[0].free_rolls = 10;
    next_turn(&mut g);
    turn_of(&mut g, 0);
    // The roll at the start of the turn is the first of the two.
    assert!(
        g.players[0].shop.iter().all(|m| g.data.cards.get(m.card).tier == 6),
        "the turn-start roll holds Tier 6"
    );
    act(&mut g, 0, Action::Roll);
    assert!(
        g.players[0].shop.iter().all(|m| g.data.cards.get(m.card).tier == 6),
        "the second Refresh too: {:?}",
        g.players[0].shop.iter().map(|m| g.data.cards.get(m.card).tier).collect::<Vec<_>>()
    );
    act(&mut g, 0, Action::Roll);
    assert!(
        g.players[0].shop.iter().any(|m| g.data.cards.get(m.card).tier <= 2),
        "the third Refresh draws normally again"
    );
    g.check_invariants().unwrap();
}

#[test]
fn bob_blehead_takes_tier_one_and_two_out_of_the_tavern() {
    let mut g = game(184);
    g.players[0].tier = 4;
    let gold = g.players[0].gold;
    take(&mut g, 0, "Bob-blehead");
    assert_eq!(g.players[0].gold, gold.saturating_add(2), "2 Gold");
    g.players[0].free_rolls = 10;
    act(&mut g, 0, Action::Roll);
    assert!(
        g.players[0].shop.iter().all(|m| g.data.cards.get(m.card).tier >= 3),
        "no Tier 1 or 2"
    );
    g.check_invariants().unwrap();
}

// ---- Spellcraft ----
//
// A Spellcraft trinket hands its spell out at the pick and again at the start
// of every turn; the spell is a Tavern spell, and one that is not cast leaves
// the hand at the end of the turn.

fn cast_from_hand(g: &mut Game, p: usize, name: &str) {
    turn_of(g, p);
    let c = id(g, name);
    let h = (0..g.players[p].hand.len())
        .find(|&h| g.players[p].hand[h].card == c)
        .unwrap_or_else(|| panic!("{name} is not in hand"));
    g.step(Action::Play { hand: h as u8, pos: 0 })
        .unwrap_or_else(|_| panic!("could not cast {name}"));
}

#[test]
fn a_spellcraft_spell_leaves_the_hand_at_the_end_of_the_turn() {
    let mut g = game(190);
    take(&mut g, 0, "Ophidian Staff");
    assert_eq!(hand_of(&g, 0, "Ophidian Staff"), 1, "it comes with the pick");
    // Not cast: gone with the turn. Cast: gone too, and back next turn.
    next_turn(&mut g);
    turn_of(&mut g, 0);
    assert_eq!(hand_of(&g, 0, "Ophidian Staff"), 1, "a new one each turn");
    put_on_board(&mut g, 0, "Forest Rover");
    let before = (g.players[0].board[0].attack, g.players[0].board[0].health);
    cast_from_hand(&mut g, 0, "Ophidian Staff");
    act(&mut g, 0, Action::Target(0));
    let m = g.players[0].board[0];
    assert_eq!((m.attack, m.health), (before.0 + 2, before.1 + 2), "+2/+2");
    assert!(m.keywords.has(bgcore::Keywords::REBORN), "and Reborn");
    assert_eq!(hand_of(&g, 0, "Ophidian Staff"), 0, "the cast one is used up");
    assert_eq!(g.players[0].spells_cast, 1, "a Spellcraft cast is a spell cast");
    next_turn(&mut g);
    turn_of(&mut g, 0);
    assert_eq!(hand_of(&g, 0, "Ophidian Staff"), 1, "and comes back");
    // Leaving the hand is not a discard. Only player 0's turn ends here, so
    // the next turn's new copy is not in hand yet.
    let discarded = g.players[0].discarded;
    act(&mut g, 0, Action::EndTurn);
    assert_eq!(hand_of(&g, 0, "Ophidian Staff"), 0, "gone with the turn");
    assert_eq!(g.players[0].discarded, discarded, "not a discard");
    g.check_invariants().unwrap();
}

#[test]
fn the_vibrant_bubble_gives_a_murloc_a_bonus_keyword() {
    let mut g = game(191);
    take(&mut g, 0, "Vibrant Bubble");
    put_on_board(&mut g, 0, "Bubble Gunner");
    let before = g.players[0].board[0].keywords.0;
    cast_from_hand(&mut g, 0, "Vibrant Bubble");
    act(&mut g, 0, Action::Target(0));
    let mask = g.data.rules.bonus_mask;
    assert_ne!(g.players[0].board[0].keywords.0 & mask, before & mask, "a keyword");
}

#[test]
fn the_jailer_sticker_trades_an_undead_for_a_random_one() {
    let mut g = game(192);
    take(&mut g, 0, "Jailer Sticker");
    put_on_board(&mut g, 0, "Harmless Bonehead");
    cast_from_hand(&mut g, 0, "Jailer Sticker");
    act(&mut g, 0, Action::Target(0));
    let undead = id(&g, "Harmless Bonehead");
    assert!(
        !g.players[0].board.iter().any(|m| m.card == undead),
        "the Undead is destroyed"
    );
    assert!(
        g.players[0]
            .hand
            .iter()
            .any(|m| g.data.cards.get(m.card).tribes.has(Tribe::Undead)),
        "a random Undead is in hand"
    );
    g.check_invariants().unwrap();
}

#[test]
fn the_token_of_the_old_gods_transforms_into_a_higher_tier() {
    let mut g = game(193);
    take(&mut g, 0, "Token of the Old Gods");
    put_on_board(&mut g, 0, "Bubble Gunner");
    cast_from_hand(&mut g, 0, "Token of the Old Gods");
    act(&mut g, 0, Action::Target(0));
    assert_eq!(g.players[0].discover.len(), 3, "three cards of the tier above");
    for &c in &g.players[0].discover {
        assert_eq!(g.data.cards.get(c).tier, 2, "one tier above the tier 1 minion");
    }
    act(&mut g, 0, Action::Choose(0));
    let m = g.players[0].board[0];
    assert_ne!(m.card, id(&g, "Bubble Gunner"), "the old card is gone");
    
    let c = g.data.cards.get(m.card);
    assert_eq!(c.tier, 2, "the new minion is the one picked");
    assert_eq!((m.attack, m.health), (c.attack, c.health), "at its printed stats");
    assert_eq!(g.players[0].hand.len(), 0, "and not in hand");
    g.check_invariants().unwrap();
}

#[test]
fn the_chillmere_mosaic_refreshes_with_battlecry_minions_at_one_gold() {
    let mut g = game(194);
    take(&mut g, 0, "Chillmere Mosaic");
    g.players[0].gold = 10;
    cast_from_hand(&mut g, 0, "Chillmere Mosaic");
    assert!(!g.players[0].shop.is_empty(), "the Tavern was refilled");
    for m in &g.players[0].shop {
        assert!(g.data.cards.get(m.card).battlecry.is_some(), "Battlecry minions only");
    }
    let slot = 0u8;
    act(&mut g, 0, Action::Buy(slot));
    assert_eq!(g.players[0].gold, 9, "they cost 1");
    g.check_invariants().unwrap();
}

#[test]
fn the_makeshift_master_gives_the_deity_what_its_minion_gains() {
    let mut g = game(195);
    assert!(g.players[0].deity.is_some(), "an Aberration lobby has a Deity");
    take(&mut g, 0, "Makeshift Master");
    put_on_board(&mut g, 0, "Lullabot");
    cast_from_hand(&mut g, 0, "Makeshift Master");
    act(&mut g, 0, Action::Target(0));
    let before = g.players[0].deity.unwrap();
    // A Tavern spell cast on the watched minion (+2/+2).
    let c = id(&g, "Tavern Dish Banana");
    let m = g.players[0].make_card(&g.data.cards, c, false, 0);
    g.players[0].hand.push(m);
    turn_of(&mut g, 0);
    let h = g.players[0].hand.len() - 1;
    act(&mut g, 0, Action::Play { hand: h as u8, pos: 0 });
    act(&mut g, 0, Action::Target(0));
    let after = g.players[0].deity.unwrap();
    assert_eq!(
        (after.attack - before.attack, after.health - before.health),
        (2, 2),
        "the Deity gained what the watched minion gained"
    );
    g.check_invariants().unwrap();
}

// ---- the leftovers ----

#[test]
fn the_lava_lamp_pays_after_six_sells() {
    let mut g = game(200);
    take(&mut g, 0, "Lava Lamp");
    for i in 0..6 {
        put_on_board(&mut g, 0, "Buzzing Vermin");
        let _ = i;
        act(&mut g, 0, Action::Sell(0));
    }
    let elementals = g.players[0]
        .hand
        .iter()
        .filter(|m| g.data.cards.get(m.card).tribes.has(Tribe::Elemental))
        .count();
    assert_eq!(elementals, 1, "one random Elemental after 6 sells");
    g.check_invariants().unwrap();
}

#[test]
fn a_fight_reports_the_minions_it_summoned() {
    // The counter Soul Battery reads: every friendly summon, the neutral ones
    // included (a Beetle is a Beast, a Skeleton is neutral).
    let mut g = game(204);
    equip(&mut g, 0, 0, "Soul Battery");
    put_on_board(&mut g, 0, "Buzzing Vermin");
    put_on_board(&mut g, 0, "Harmless Bonehead");
    let out = fight_wall_watched(&g, 7, 50, 500, 40, &mut ());
    eprintln!("summons {} tribes {:?} deaths {}", out.sides[0].friendly_summons, out.sides[0].summon_tribes, out.sides[0].friendly_deaths);
    assert!(out.sides[0].friendly_summons >= 2, "a Beetle and a Skeleton");
}

#[test]
fn the_soul_battery_swaps_itself_after_a_fights_summons() {
    // The counter is the minions a fight summons, which come home with the
    // fight's outcome (docs/ENGINE.md).
    let mut g = game(201);
    to_turn(&mut g, 4);
    take(&mut g, 0, "Soul Battery");
    assert_eq!(g.players[0].trinkets[0].of - 1, trinket(&g, "Soul Battery"));
    // One summon short: a fight's Deathrattle summons finish it. The other
    // seats hold a wall, so player 0's minions die and summon.
    let battery = trinket(&g, "Soul Battery") + 1;
    let _ = battery;
    g.players[0].trinkets[0].count = 24;
    put_on_board(&mut g, 0, "Buzzing Vermin");
    put_on_board(&mut g, 0, "Harmless Bonehead");
    for q in 1..bgcore::PLAYERS {
        let mut m = g.data.cards.make(id(&g, "Lullabot"), false);
        m.attack = 50;
        m.health = 500;
        let _ = g.players[q].board.try_push(m);
    }
    next_turn(&mut g);
    let of = g.players[0].trinkets[0].of;
    assert_ne!(of, 0, "something holds the slot");
    assert_ne!(of - 1, trinket(&g, "Soul Battery"), "the Battery replaced itself");
    let t = g.data.trinkets.get(of - 1);
    assert_eq!(t.slot, bgcore::trinkets::Slot::Greater, "a Greater trinket");
    assert!(t.tribes.has(Tribe::Undead), "of the Undead");
    g.check_invariants().unwrap();
}

#[test]
fn the_mask_of_ancient_ones_makes_the_deity_golden() {
    let mut g = game(202);
    let before = g.players[0].deity.unwrap();
    take(&mut g, 0, "Mask of Ancient Ones");
    let after = g.players[0].deity.unwrap();
    assert!(after.golden, "Golden");
    assert_eq!((after.attack, after.health), (before.attack + 1, before.health + 1));
    g.check_invariants().unwrap();
}

#[test]
fn the_innkeepers_hearth_discovers_two_tier_six_minions_at_thirty() {
    let mut g = game(203);
    g.players[0].tier = 6;
    take(&mut g, 0, "Innkeeper's Hearth");
    assert_eq!(g.players[0].discover.len(), 3, "a Discover of Tier 6");
    for &c in &g.players[0].discover {
        assert_eq!(g.data.cards.get(c).tier, 6);
    }
    act(&mut g, 0, Action::Choose(0));
    assert_eq!(g.players[0].discover.len(), 3, "and a second one");
    act(&mut g, 0, Action::Choose(0));
    let got: Vec<(i32, i32)> = g.players[0]
        .hand
        .iter()
        .filter(|m| g.data.cards.get(m.card).tier == 6)
        .map(|m| (m.attack, m.health))
        .collect();
    assert_eq!(got, vec![(30, 30), (30, 30)], "both set to 30/30");
    g.check_invariants().unwrap();
}

#[test]
fn the_cheap_leftovers_do_their_part() {
    // Worn Treasure Map: 10 Gold two turns after the pick, once. The Gold is
    // read at the end of the turn it lands in, before the next one resets it.
    let mut g = game(210);
    take(&mut g, 0, "Worn Treasure Map");
    act(&mut g, 0, Action::EndTurn);
    next_turn(&mut g);
    turn_of(&mut g, 0);
    g.players[0].gold = 0;
    act(&mut g, 0, Action::EndTurn);
    assert_eq!(g.players[0].gold, 10, "10 Gold two turns in");
    assert!(g.players[0].trinkets[0].done, "and it is spent");
    for _ in 0..2 {
        next_turn(&mut g);
        turn_of(&mut g, 0);
        g.players[0].gold = 0;
        act(&mut g, 0, Action::EndTurn);
        assert_eq!(g.players[0].gold, 0, "once per game");
    }
    // Scraper Sticker: a Magnetic Mech now and each turn.
    let mut g = game(211);
    let magnets = |g: &Game| {
        g.players[0]
            .hand
            .iter()
            .filter(|m| {
                let c = g.data.cards.get(m.card);
                c.tribes.has(Tribe::Mech) && c.keywords.has(bgcore::Keywords::MAGNETIC)
            })
            .count()
    };
    take(&mut g, 0, "Scraper Sticker");
    assert_eq!(magnets(&g), 1, "one on the pick");
    next_turn(&mut g);
    assert_eq!(magnets(&g), 2, "and another each turn");
    // Gold Mallet: +1/+1 at the end of the turn, once more per Golden minion
    // played.
    let mut g = game(212);
    put_on_board(&mut g, 0, "Lullabot");
    equip(&mut g, 0, 0, "Gold Mallet");
    g.players[0].golden_played = 2;
    let before = (g.players[0].board[0].attack, g.players[0].board[0].health);
    act(&mut g, 0, Action::EndTurn);
    assert_eq!(
        (g.players[0].board[0].attack, g.players[0].board[0].health),
        (before.0 + 3, before.1 + 4),
        "+1/+1 plus one more for each of the two Golden minions, and Lullabot's own +0/+1"
    );
    // Defensive Shell: the first minion played each turn gets Divine Shield.
    let mut g = game(213);
    take(&mut g, 0, "Defensive Shell");
    put_in_hand(&mut g, 0, "Buzzing Vermin");
    put_in_hand(&mut g, 0, "Harmless Bonehead");
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    act(&mut g, 0, Action::Play { hand: 0, pos: 1 });
    let shielded: Vec<bool> = g.players[0]
        .board
        .iter()
        .map(|m| m.keywords.has(bgcore::Keywords::DIVINE_SHIELD))
        .collect();
    assert_eq!(shielded.iter().filter(|&&b| b).count(), 1, "one shield a turn");
    g.check_invariants().unwrap();
}

#[test]
fn rendle_steals_the_dearest_card_and_ghastly_doubles_the_end_of_turn() {
    // Rendle Sticker: the highest-Tier card in the Tavern, on the pick and at
    // the end of each turn.
    let mut g = game(214);
    g.players[0].tier = 4;
    let dear = "Banana Slamma";
    set_shop(&mut g, 0, &["Buzzing Vermin", dear, "Flittering Bat"]);
    take(&mut g, 0, "Rendle Sticker");
    assert_eq!(hand_of(&g, 0, dear), 1, "the dearest card is in hand");
    assert_eq!(g.players[0].shop.len(), 2, "and left the Tavern");
    g.check_invariants().unwrap();
    // Ghastly Sticker: the end of turn runs twice.
    let mut g = game(215);
    put_on_board(&mut g, 0, "Lullabot");
    take(&mut g, 0, "Ghastly Sticker");
    let before = (g.players[0].board[0].attack, g.players[0].board[0].health);
    act(&mut g, 0, Action::EndTurn);
    assert_eq!(
        (g.players[0].board[0].attack, g.players[0].board[0].health),
        (before.0, before.1 + 2),
        "Lullabot's +0/+1 twice"
    );
}

#[test]
fn the_upstart_embers_double_the_healthiest_tavern_minion() {
    let mut g = game(216);
    g.players[0].tier = 3;
    g.players[0].free_rolls = 4;
    take(&mut g, 0, "Upstart Embers");
    // Every Refresh doubles the highest-Health minion it dealt, so exactly
    // one card of the new Tavern has twice its printed stats.
    for _ in 0..3 {
        act(&mut g, 0, Action::Roll);
        let doubled = g.players[0]
            .shop
            .iter()
            .filter(|m| {
                let c = g.data.cards.get(m.card);
                !c.is_spell() && m.attack == c.attack * 2 && m.health == c.health * 2
            })
            .count();
        assert_eq!(doubled, 1, "one doubled minion");
    }
    g.check_invariants().unwrap();
}

#[test]
fn the_replica_cathedral_casts_the_first_spell_twice() {
    let mut g = game(217);
    put_on_board(&mut g, 0, "Buzzing Vermin");
    take(&mut g, 0, "Replica Cathedral");
    let before = (g.players[0].board[0].attack, g.players[0].board[0].health);
    let c = id(&g, "Tavern Dish Banana");
    let m = g.players[0].make_card(&g.data.cards, c, false, 0);
    g.players[0].hand.push(m);
    turn_of(&mut g, 0);
    let h = g.players[0].hand.len() - 1;
    act(&mut g, 0, Action::Play { hand: h as u8, pos: 0 });
    act(&mut g, 0, Action::Target(0));
    assert_eq!(
        (g.players[0].board[0].attack, g.players[0].board[0].health),
        (before.0 + 4, before.1 + 4),
        "+2/+2 twice"
    );
    // The second spell of the turn casts once.
    let m = g.players[0].make_card(&g.data.cards, c, false, 0);
    g.players[0].hand.push(m);
    turn_of(&mut g, 0);
    let h = g.players[0].hand.len() - 1;
    let before = (g.players[0].board[0].attack, g.players[0].board[0].health);
    act(&mut g, 0, Action::Play { hand: h as u8, pos: 0 });
    act(&mut g, 0, Action::Target(0));
    assert_eq!(
        (g.players[0].board[0].attack, g.players[0].board[0].health),
        (before.0 + 2, before.1 + 2),
        "+2/+2 once"
    );
    g.check_invariants().unwrap();
}

#[test]
fn the_replica_cathedral_doubles_a_spell_with_no_target() {
    // Tavern Coin: 1 Gold a cast, twice on the first spell of the turn.
    let mut g = game(218);
    take(&mut g, 0, "Replica Cathedral");
    let c = id(&g, "Tavern Coin");
    for (expected, cast) in [(2u8, 1u8), (1, 2)] {
        let m = g.players[0].make_card(&g.data.cards, c, false, 0);
        g.players[0].hand.push(m);
        turn_of(&mut g, 0);
        let h = g.players[0].hand.len() - 1;
        let gold = g.players[0].gold;
        act(&mut g, 0, Action::Play { hand: h as u8, pos: 0 });
        assert_eq!(g.players[0].gold, gold.saturating_add(expected), "cast {cast}");
    }
    g.check_invariants().unwrap();
}
