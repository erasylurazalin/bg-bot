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

fn put_on_board(g: &mut Game, p: usize, name: &str) {
    let c = id(g, name);
    let slot = g.data.cards.get(c).pool_slot as usize;
    g.pool[slot] -= 1;
    let m = g.players[p].make_card(&g.data.cards, c, false, 1);
    g.players[p].board.push(m);
}

/// Put a pool minion in hand (a second Murloc for the guarantee, say).
fn put_in_hand(g: &mut Game, p: usize, name: &str) {
    let c = id(g, name);
    let slot = g.data.cards.get(c).pool_slot as usize;
    g.pool[slot] -= 1;
    let m = g.players[p].make_card(&g.data.cards, c, false, 1);
    g.players[p].hand.push(m);
}

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
    // The price is paid at the pick.
    let (of, price) = offer[1];
    let gold = g.players[0].gold;
    act(&mut g, 0, Action::ChooseTrinket(1));
    assert_eq!(g.players[0].gold, gold.saturating_sub(price), "the price was paid");
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
    let cheapest = g.players[0]
        .trinket_offer
        .iter()
        .map(|&(of, price)| (price, of))
        .min()
        .unwrap()
        .1;
    let gold = g.players[0].gold;
    let price = g.players[0].trinket_offer.iter().find(|&&(of, _)| of == cheapest).unwrap().1;
    act(&mut g, 0, Action::EndTurn);
    assert_eq!(g.players[0].trinkets[0].of, cheapest + 1, "the cheapest one, not declined");
    assert_eq!(g.players[0].gold, gold.saturating_sub(price));
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
    // Reborn". None of the built ones has a Start of Combat yet.
    let toml = r#"
[[trinket]]
name = "Test Banner"
card_id = "TEST_Trinket_2"
slot = "lesser"
soc = { tribe = "undead", keywords = ["taunt", "reborn"] }
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
    let kw = counters[0].soc_tribe.expect("the trinket's Start of Combat reaches the fight");
    assert_eq!(kw.1, bgcore::Keywords::TAUNT | bgcore::Keywords::REBORN);
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
                + t.on_gold_spent.len()
                + t.soc.iter().count()
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
    let pick = g.players[0].trinket_offer[0].0;
    act(&mut g, 0, Action::ChooseTrinket(0));
    assert_eq!(g.players[0].trinkets[0].of, pick + 1, "the cube was replaced");
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
