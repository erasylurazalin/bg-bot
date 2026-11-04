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

/// A full hand still takes the third copy off the board: the merge is one
/// action that takes all three copies and puts the Golden one in the hand, so
/// the slots it frees are free before the Golden card needs one (log
/// 2026_10_24_04_08_00-g1 turn 10 step 25: nine cards in hand, one copy in
/// hand and one on the board, the bought third one makes the hand ten, and
/// the log ends with the Golden The Shadow of Doubt 21/169 in the hand and
/// the board copy gone).
#[test]
fn a_full_hand_takes_the_board_copy_the_merge_makes_room_for() {
    let mut g = sandbox(7);
    in_hand(&mut g, "Mummifier");
    for _ in 0..8 {
        in_hand(&mut g, "Butchering");
    }
    on_board(&mut g, "Mummifier");
    assert_eq!(g.players[0].hand.len(), 9);
    let bought = card(&mut g, "Mummifier");
    let slot = g.players[0].shop.len() as u8;
    g.players[0].shop.push(bought);
    g.step(Action::Buy(slot)).unwrap();
    while answer(&mut g) {}
    assert_eq!(board_names(&g), Vec::<String>::new(), "the board copy went into the Golden one");
    assert!(
        g.players[0].hand.iter().filter(|m| m.golden).count() == 1,
        "the three Mummifiers made one Golden one in the hand: {:?}",
        hand_names(&g)
    );
    assert_eq!(g.players[0].hand.len(), 9, "ten cards, two copies in, one Golden out");
}

/// Three copies all on the board need a free hand slot for the Golden card:
/// with a full hand they stay where they are, and the check as an action ends
/// merges them once an action takes a card out of the hand (Era, 2026-11-04,
/// docs/rules/design.md "A full hand"). A copy in hand would free the slot
/// itself; a board copy does not.
#[test]
fn three_board_copies_wait_for_a_hand_slot() {
    let mut g = sandbox(9);
    for _ in 0..3 {
        on_board(&mut g, "Mummifier");
    }
    for _ in 0..9 {
        in_hand(&mut g, "Butchering");
    }
    in_hand(&mut g, "Deadly Spore");
    g.players[0].gold = 10;
    assert_eq!(g.players[0].hand.len(), 10);
    // The summon that left the three copies asks for the check, and the full
    // hand refuses it: the copies wait.
    g.note_triples_due();
    g.step(Action::Freeze).unwrap();
    assert_eq!(
        board_names(&g),
        ["Mummifier", "Mummifier", "Mummifier"],
        "the full hand holds the merge back"
    );
    assert!(g.players[0].hand.iter().all(|m| !m.golden));
    // Playing the Spore frees the hand slot the Golden card needs, and the
    // game's own check at the end of the action takes the three board copies.
    g.step(Action::Play { hand: 9, pos: 3 }).unwrap();
    assert_eq!(board_names(&g), ["Deadly Spore"], "the three board copies went into the merge");
    assert_eq!(
        g.players[0].hand.iter().filter(|m| m.golden).count(),
        1,
        "one Golden Mummifier in the hand: {:?}",
        hand_names(&g)
    );
    assert_eq!(g.players[0].hand.len(), 10, "nine spells, the Golden one, the Spore is on the board");
}

/// The same for a triple a fight left on the board: nothing asked for its
/// check during the fight, and the turn's start cannot merge it for want of a
/// hand slot, so the copies must wait rather than stay apart for good
/// (night-note question 19, Era 2026-11-04). A check refused at the start of
/// the turn is asked again as each action ends.
#[test]
fn a_triple_held_from_a_fight_waits_for_a_hand_slot() {
    let mut g = sandbox(10);
    for _ in 0..3 {
        on_board(&mut g, "Mummifier");
    }
    for _ in 0..9 {
        in_hand(&mut g, "Butchering");
    }
    in_hand(&mut g, "Deadly Spore");
    g.players[0].gold = 10;
    g.dark_gift_start_of_turn(0);
    assert_eq!(
        board_names(&g),
        ["Mummifier", "Mummifier", "Mummifier"],
        "a full hand holds the merge back"
    );
    assert!(g.players[0].hand.iter().all(|m| !m.golden), "no Golden card has a slot yet");
    // A hand slot opens, and the wait is over.
    g.step(Action::Play { hand: 9, pos: 3 }).unwrap();
    assert_eq!(board_names(&g), ["Deadly Spore"], "the three board copies went into the merge");
    assert_eq!(
        g.players[0].hand.iter().filter(|m| m.golden).count(),
        1,
        "one Golden Mummifier in the hand: {:?}",
        hand_names(&g)
    );
}

/// A copy's damage never comes off the Golden card's Health: the game sums the
/// copies and then heals the Golden one, so what a copy has left below its
/// card's own Health is damage, not a buff (log 2026_10_24_00_56_06-g1 turn 8
/// step 21: the merged Plaguerunner 7051 takes ATK 25 and DAMAGE 1, then
/// DAMAGE 0, so a board copy at 21/1 gives a 25/4 Golden one and not a 25/3).
#[test]
fn a_damaged_copy_keeps_its_damage_off_the_golden_card() {
    let mut g = sandbox(8);
    in_hand(&mut g, "Suspicious Prisonguard");
    on_board(&mut g, "Suspicious Prisonguard");
    // 3/3 with two damage, as a board copy can carry it.
    g.players[0].board[0].health = 1;
    let bought = card(&mut g, "Suspicious Prisonguard");
    let slot = g.players[0].shop.len() as u8;
    g.players[0].shop.push(bought);
    g.step(Action::Buy(slot)).unwrap();
    while answer(&mut g) {}
    let gold = g.players[0]
        .hand
        .iter()
        .find(|m| m.golden)
        .expect("the three Prisonguards make a Golden one");
    assert_eq!(
        (gold.attack, gold.health),
        (6, 6),
        "a Prisonguard is 3/3, double that is 6/6, and the damage is gone"
    );
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
