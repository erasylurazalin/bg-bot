//! Where a card a shop action gives goes, and when the game's own triple
//! check runs (the shop replay's E13 and E14 groups): a Health price is paid
//! before the spell it buys, a Graverobber's copy joins the hand before the
//! Skeletons its destroy leaves, those Skeletons wait on the board while a
//! Discover is open, a triple still forms in a full hand, and the Fodder
//! queue stops at a full Tavern.

use std::sync::Arc;

use bgcore::{Action, Data, Game, Minion, Tribe, Tribes};

fn data() -> Arc<Data> {
    Arc::new(Data::bundled())
}

/// A one-player sandbox game with every tribe in its lobby, so the cards
/// these tests hold are pool cards.
fn sandbox(seed: u64) -> Game {
    let all = Tribe::EVERY.iter().copied().fold(Tribes::NONE, |m, t| m.with(t));
    Game::new_sandbox(data(), seed, Some(all), 0)
}

/// A card as the game builds it, out of the pool when it has a copy there.
fn card(g: &mut Game, name: &str) -> Minion {
    let c = g.data.cards.by_name(name).unwrap();
    let card = g.data.cards.get(c);
    let slot = card.pool_slot as usize;
    let take = !card.token && !card.is_spell() && slot < g.pool.len() && g.pool[slot] > 0;
    if take {
        g.pool[slot] -= 1;
    }
    g.players[0].make_card(&g.data.cards, c, false, u8::from(take))
}

fn on_board(g: &mut Game, name: &str) {
    let m = card(g, name);
    g.players[0].board.push(m);
}

fn in_hand(g: &mut Game, name: &str) {
    let m = card(g, name);
    g.players[0].hand.push(m);
}

fn board_names(g: &Game) -> Vec<String> {
    g.players[0].board.iter().map(|m| g.data.cards.get(m.card).name.clone()).collect()
}

fn hand_names(g: &Game) -> Vec<String> {
    g.players[0].hand.iter().map(|m| g.data.cards.get(m.card).name.clone()).collect()
}

/// Answer the prompt a play's Battlecry opened, if it opened one.
fn answer(g: &mut Game) -> bool {
    let pl = &g.players[0];
    let a = if !pl.discover.is_empty() {
        Action::Choose(0)
    } else if pl.pending_target.is_some() {
        Action::Target(0)
    } else {
        return false;
    };
    if !g.is_legal(a) {
        return false;
    }
    g.step(a).unwrap();
    true
}

/// Disguised Graverobber: "Destroy a friendly Undead to get a plain copy of
/// it." The copy joins the hand first, and the Golden Skeleton the destroyed
/// Cadaver Caretaker's three Skeletons make comes after it, because the
/// game's own triple check runs as the action ends (log
/// 2026_10_30_19_45_15-g2 turn 5 step 4).
#[test]
fn a_graverobbers_copy_joins_the_hand_before_the_skeletons_golden_one() {
    let mut g = sandbox(1);
    on_board(&mut g, "Cadaver Caretaker");
    in_hand(&mut g, "Disguised Graverobber");
    g.step(Action::Play { hand: 0, pos: 1 }).unwrap();
    while answer(&mut g) {}
    assert_eq!(hand_names(&g), ["Cadaver Caretaker", "Skeleton"]);
    assert!(g.players[0].hand[1].golden, "the Skeleton that joins is the Golden one");
    assert_eq!(board_names(&g), ["Disguised Graverobber"]);
}

/// The Skeletons a Maw Caster's destroy summons stay on the board while the
/// Discover its Battlecry opened is unanswered: the triple check waits for
/// the action to settle (log 2026_10_24_15_25_26-g6 turn 8 step 14).
#[test]
fn three_skeletons_wait_on_the_board_while_the_maw_casters_discover_is_open() {
    let mut g = sandbox(2);
    on_board(&mut g, "Cadaver Caretaker");
    in_hand(&mut g, "Maw Caster");
    g.step(Action::Play { hand: 0, pos: 1 }).unwrap();
    assert!(g.players[0].pending_target.is_none(), "one Undead is no prompt");
    assert!(!g.players[0].discover.is_empty(), "the Battlecry's Discover is open");
    assert_eq!(board_names(&g), ["Skeleton", "Skeleton", "Skeleton", "Maw Caster"]);
    assert_eq!(hand_names(&g), Vec::<String>::new());
    // The pick answers the Discover, and only then do they make their Golden
    // one in the hand.
    g.step(Action::Choose(0)).unwrap();
    assert_eq!(board_names(&g), ["Maw Caster"]);
    // The pick joins the hand first (whatever Undead it was), and the Golden
    // Skeleton the three of them make comes after it.
    assert_eq!(g.players[0].hand.len(), 2);
    let last = g.players[0].hand[1];
    assert_eq!(g.data.cards.get(last.card).name, "Skeleton");
    assert!(last.golden);
}

/// Three plain copies make their Golden card even when the hand is full: the
/// copies the hand gives up free their slots first (log
/// 2026_10_24_15_25_26-g6 turn 8 step 16, a Discover pick into a hand of
/// nine).
#[test]
fn a_triple_forms_in_a_full_hand() {
    let mut g = sandbox(4);
    on_board(&mut g, "Mummifier");
    in_hand(&mut g, "Mummifier");
    in_hand(&mut g, "Mummifier");
    // A full hand: the Graverobber and seven spells, which never triple.
    in_hand(&mut g, "Disguised Graverobber");
    for _ in 0..7 {
        in_hand(&mut g, "Butchering");
    }
    assert_eq!(g.players[0].hand.len(), 10);
    g.step(Action::Play { hand: 2, pos: 1 }).unwrap();
    while answer(&mut g) {}
    let hand = hand_names(&g);
    assert!(
        hand.iter().any(|n| n == "Mummifier") && g.players[0].hand.iter().any(|m| m.golden),
        "the third Mummifier makes a Golden one in the full hand: {hand:?}"
    );
    assert_eq!(g.players[0].hand.len(), 8);
}

/// A Tavern spell paid in Health lands after the card its damage gives:
/// Eredar Escapist's two Corrupted Cupcakes take the lower hand slot (log
/// 2026_10_24_15_25_26-g4 turn 11 step 10).
#[test]
fn a_tavern_spell_paid_in_health_lands_after_the_card_its_damage_gives() {
    let mut g = sandbox(5);
    // A Golden Eredar Escapist gives two Corrupted Cupcakes, not one.
    let c = g.data.cards.by_name("Eredar Escapist").unwrap();
    let slot = g.data.cards.get(c).pool_slot as usize;
    g.pool[slot] -= 1;
    let escapist = g.players[0].make_card(&g.data.cards, c, true, 1);
    g.players[0].board.push(escapist);
    // Three of the four hero damage it counts are behind it.
    g.players[0].board[0].improvements = 3;
    let hasty = g.data.cards.by_name("Hasty Excavation").unwrap();
    g.players[0].shop_spell = Some(hasty);
    g.step(Action::BuySpell).unwrap();
    assert_eq!(
        hand_names(&g),
        ["Corrupted Cupcakes", "Corrupted Cupcakes", "Hasty Excavation"]
    );
}

/// Fodders queued for a Refresh stop at a full Tavern's worth of cards: six
/// queued and three more show as 7, not 9 (log 2026_11_01_17_59_13-g1 turn 9
/// step 16).
#[test]
fn fodders_queued_for_a_refresh_stop_at_seven() {
    let mut g = sandbox(6);
    // A full queue for the next Refresh, and Laboratory Assistant's Battlecry
    // queues one for each of the next three Refreshes.
    g.players[0].fodder_refreshes.clear();
    g.players[0].fodder_refreshes.push(7);
    in_hand(&mut g, "Laboratory Assistant");
    g.step(Action::Play { hand: 0, pos: 0 }).unwrap();
    let queued: Vec<u8> = g.players[0].fodder_refreshes.iter().copied().collect();
    assert_eq!(queued, vec![7, 1, 1]);
    // Selling a minion next to a Twisted Wrathguard queues one more, and a
    // full queue stays at seven (log's turn 9 step 17).
    g.players[0].fodder_refreshes.clear();
    g.players[0].fodder_refreshes.push(7);
    on_board(&mut g, "Twisted Wrathguard");
    on_board(&mut g, "Handless Forsaken");
    g.step(Action::Sell(1)).unwrap();
    let queued: Vec<u8> = g.players[0].fodder_refreshes.iter().copied().collect();
    assert_eq!(queued, vec![7]);
}

/// A Golden Cadaver Caretaker's Deathrattle, triggered in the Tavern, brings
/// six 1/1 Skeletons: they triple twice in the one action, and both Golden
/// Skeletons join the hand, leaving the board empty of them (log
/// `Hearthstone_2026_10_26_00_48_15-g2` turn 7 step 6, the Floating Candle
/// Set the trinket hands out).
#[test]
fn a_golden_cadaver_caretakers_six_skeletons_triple_twice() {
    let mut g = sandbox(7);
    let c = g.data.cards.by_name("Cadaver Caretaker").unwrap();
    let slot = g.data.cards.get(c).pool_slot as usize;
    g.pool[slot] -= 1;
    let caretaker = g.players[0].make_card(&g.data.cards, c, true, 1);
    g.players[0].board.push(caretaker);
    in_hand(&mut g, "Floating Candle");
    g.players[0].gold = 10;
    g.step(Action::Play { hand: 0, pos: 0 }).unwrap();
    while answer(&mut g) {}
    assert_eq!(board_names(&g), ["Cadaver Caretaker"], "no Skeleton stays on the board");
    assert_eq!(hand_names(&g), ["Skeleton", "Skeleton"], "two Golden Skeletons in the hand");
    for m in &g.players[0].hand {
        assert!(m.golden);
        assert_eq!((m.attack, m.health), (2, 2));
    }
    g.check_invariants().unwrap();
}
