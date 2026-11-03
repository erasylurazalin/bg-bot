//! Repeats: what a repeated Battlecry or spell does. A repeat still counts as
//! a trigger or a cast, but making a minion Golden happens once; a repeated
//! Choose One picks its option at random on the target the player picked; and
//! a repeat that asks for a target lands on the one the first run picked
//! (docs/rules/design.md "Set versus add", Era 2026-11-04).

use std::sync::Arc;

use bgcore::{Action, CardDb, CardId, Data, Game, Minion, Tribe};

fn lobby(ts: &[Tribe]) -> bgcore::Tribes {
    ts.iter().copied().fold(bgcore::Tribes::NONE, |m, t| m.with(t))
}

/// Pirates, Dragons and Murlocs, so every card these tests name is in the
/// pool: Elite Navigator, Kalecgos, Brann, Kelp Keeper, Young Murk-Eye.
fn game(seed: u64) -> Game {
    Game::new_with_lobby(
        Arc::new(Data::bundled()),
        seed,
        lobby(&[Tribe::Pirate, Tribe::Dragon, Tribe::Murloc, Tribe::Beast, Tribe::Mech]),
    )
}

fn id(db: &CardDb, name: &str) -> CardId {
    db.by_name(name).expect(name)
}

fn stats(g: &Game, p: usize, slot: usize) -> (i32, i32) {
    let m = &g.players[p].board[slot];
    (m.attack, m.health)
}

fn golden(g: &Game, p: usize, slot: usize) -> bool {
    g.players[p].board[slot].golden
}

/// Answer whatever the current player has open (a Discover, a target, a
/// discard, or a trinket offer), so a helper that only wants the next turn
/// never stalls on a prompt. False when nothing is open.
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

fn act(g: &mut Game, p: usize, a: Action) {
    turn_of(g, p);
    g.step(a).unwrap_or_else(|_| panic!("{a:?} illegal for player {p}"));
}

/// A card from the pool straight onto the board; golden takes three copies.
fn put_on_board(g: &mut Game, p: usize, name: &str, golden: bool) {
    let c = id(&g.data.cards, name);
    let slot = g.data.cards.get(c).pool_slot as usize;
    let copies = if golden { 3 } else { 1 };
    g.pool[slot] -= copies;
    let m = g.players[p].make_card(&g.data.cards, c, golden, copies);
    g.players[p].board.push(m);
}

/// A minion from the pool into the hand; golden takes three copies.
fn put_in_hand(g: &mut Game, p: usize, name: &str, golden: bool) {
    let c = id(&g.data.cards, name);
    let slot = g.data.cards.get(c).pool_slot as usize;
    let copies = if golden { 3 } else { 1 };
    g.pool[slot] -= copies;
    let m = g.players[p].make_card(&g.data.cards, c, golden, copies);
    g.players[p].hand.push(m);
}

/// The Tavern holds minions that are not Pirates, so a "friendly Pirate"
/// Battlecry only has the board's Pirates to pick from.
fn set_shop(g: &mut Game, p: usize, names: &[&str]) {
    let old: Vec<Minion> = g.players[p].shop.drain(..).collect();
    for m in old {
        let slot = g.data.cards.get(m.card).pool_slot as usize;
        g.pool[slot] += m.pool_copies;
    }
    for n in names {
        let c = id(&g.data.cards, n);
        let slot = g.data.cards.get(c).pool_slot as usize;
        g.pool[slot] -= 1;
        let m = g.players[p].make_card(&g.data.cards, c, false, 1);
        g.players[p].shop.push(m);
    }
}

fn play(g: &mut Game, p: usize, hand: u8, pos: u8) {
    act(g, p, Action::Play { hand, pos });
}

// ---- golden-making happens once ----

#[test]
fn brann_and_elite_navigator_make_one_pirate_golden_and_still_count_two_battlecries() {
    // Brann's "your Battlecries trigger twice" repeats Elite Navigator's
    // "make a friendly Pirate Golden", but a repeat never makes a second
    // minion Golden: it still counts as a Battlecry for Kalecgos and every
    // "Battlecry triggered" counter (docs/rules/design.md "Set versus add").
    let mut g = game(7001);
    put_on_board(&mut g, 0, "Brann Bronzebeard", false);
    put_on_board(&mut g, 0, "Kalecgos, Arcane Aspect", false);
    put_on_board(&mut g, 0, "Lovesick Balladist", false); // a tier 4 Pirate
    put_on_board(&mut g, 0, "Southsea Busker", false); // another one
    set_shop(&mut g, 0, &["Bubble Gunner"]);
    // Kalecgos is a Dragon itself, so its own buff shows twice.
    let kale = stats(&g, 0, 1);
    put_in_hand(&mut g, 0, "Elite Navigator", false);
    g.players[0].gold = 10;
    let hand = g.players[0].hand.len() as u8 - 1;
    let at = g.players[0].board.len() as u8;
    play(&mut g, 0, hand, at);
    // The one pick, then nothing more to answer.
    turn_of(&mut g, 0);
    assert!(g.players[0].pending_target.is_some(), "the first trigger asks for its Pirate");
    let balladist = g.players[0]
        .board
        .iter()
        .position(|m| g.data.cards.get(m.card).name == "Lovesick Balladist")
        .unwrap();
    act(&mut g, 0, Action::Target(balladist as u8));
    assert!(g.players[0].pending_target.is_none(), "the repeat never asks again");
    assert!(golden(&g, 0, balladist), "the picked Pirate is Golden");
    let busker = g.players[0]
        .board
        .iter()
        .position(|m| g.data.cards.get(m.card).name == "Southsea Busker")
        .unwrap();
    assert!(!golden(&g, 0, busker), "the repeat makes no second Pirate Golden");
    assert_eq!(g.players[0].battlecries_triggered, 2, "the Battlecry still counts twice");
    let after = stats(&g, 0, 1);
    assert_eq!(
        (after.0 - kale.0, after.1 - kale.1),
        (4, 4),
        "Kalecgos fires once per Battlecry trigger, so twice"
    );
    g.check_invariants().unwrap();
}

#[test]
fn a_golden_elite_navigator_makes_two_pirates_golden() {
    // The card's own count, not a repeat: a golden Elite Navigator picks two
    // Pirates and each pick makes one Golden.
    let mut g = game(7002);
    put_on_board(&mut g, 0, "Lovesick Balladist", false);
    put_on_board(&mut g, 0, "Southsea Busker", false);
    set_shop(&mut g, 0, &["Bubble Gunner"]);
    put_in_hand(&mut g, 0, "Elite Navigator", true);
    g.players[0].gold = 10;
    let hand = g.players[0].hand.len() as u8 - 1;
    let at = g.players[0].board.len() as u8;
    play(&mut g, 0, hand, at);
    // Two picks: the first leaves a prompt (two Pirates are legal), the
    // second has only the other one left.
    assert!(g.players[0].pending_target.is_some(), "the first of the two picks");
    act(&mut g, 0, Action::Target(0));
    if g.players[0].pending_target.is_some() {
        act(&mut g, 0, Action::Target(1));
    }
    assert!(g.players[0].pending_target.is_none(), "two picks, no more");
    assert!(golden(&g, 0, 0) && golden(&g, 0, 1), "both Pirates are Golden");
    g.check_invariants().unwrap();
}

#[test]
fn eyes_of_the_earth_mother_with_balinda_makes_one_golden_and_counts_two_casts() {
    // Balinda casts the spell twice, both casts count for "cast a spell", and
    // the golden happens once.
    let mut g = game(7003);
    put_on_board(&mut g, 0, "Balinda Stonehearth", false);
    put_on_board(&mut g, 0, "Lovesick Balladist", false);
    put_on_board(&mut g, 0, "Southsea Busker", false);
    set_shop(&mut g, 0, &["Bubble Gunner"]);
    let before = g.players[0].spells_cast;
    g.give(0, "Eyes of the Earth Mother").unwrap();
    let hand = g.players[0].hand.len() as u8 - 1;
    g.players[0].gold = 20;
    play(&mut g, 0, hand, 0); // a spell is played with pos 0
    assert!(g.players[0].pending_target.is_some(), "the spell asks for its minion once");
    act(&mut g, 0, Action::Target(1));
    assert!(g.players[0].pending_target.is_none(), "the second cast does not ask again");
    assert!(golden(&g, 0, 1), "the picked minion is Golden");
    assert!(!golden(&g, 0, 2), "the second cast makes no other minion Golden");
    assert_eq!(g.players[0].spells_cast - before, 2, "both casts count");
    g.check_invariants().unwrap();
}

// ---- a repeated Choose One ----

#[test]
fn brann_repeats_a_taught_choose_one_spell_with_a_random_option_on_the_same_target() {
    // Brann doubles the Magicfin token's Battlecry, which is the Choose One
    // spell it was taught. The player picks the option and the target once;
    // the second cast's option is random and lands on the same minion
    // (docs/rules/design.md "Set versus add", Era 2026-11-04).
    let mut g = game(7004);
    put_on_board(&mut g, 0, "Brann Bronzebeard", false);
    put_on_board(&mut g, 0, "Magicfin Mycologist", false);
    put_on_board(&mut g, 0, "Southsea Busker", false);
    set_shop(&mut g, 0, &["Bubble Gunner"]);
    // Both of Alliance Flag's options target one minion.
    g.players[0].shop_spell = Some(id(&g.data.cards, "Alliance Flag"));
    g.players[0].gold = 20;
    act(&mut g, 0, Action::BuySpell);
    let token = g.players[0]
        .hand
        .iter()
        .position(|m| m.taught_spell.is_some())
        .expect("a taught token") as u8;
    let at = g.players[0].board.len() as u8;
    play(&mut g, 0, token, at);
    let before: Vec<(i32, i32)> =
        (0..g.players[0].board.len()).map(|s| stats(&g, 0, s)).collect();
    assert!(g.players[0].pending_choose_one.is_some(), "the player picks the option once");
    act(&mut g, 0, Action::Choose(0));
    assert!(g.players[0].pending_target.is_some(), "and its target once");
    act(&mut g, 0, Action::Target(2));
    assert!(g.players[0].pending_choose_one.is_none(), "the repeat's option is random, not asked");
    assert!(g.players[0].pending_target.is_none(), "and it does not ask for a target");
    // Alliance Flag's options are +3/+1 and +1/+3, so two casts on one minion
    // give 8 stats in all whichever options the random pick took.
    let after: Vec<(i32, i32)> =
        (0..g.players[0].board.len()).map(|s| stats(&g, 0, s)).collect();
    let mut gained = (0, 0);
    for (s, (a, h)) in after.iter().enumerate() {
        let d = (a - before[s].0, h - before[s].1);
        if d != (0, 0) {
            assert_eq!(s, 2, "only the minion the player picked was touched");
            gained = d;
        }
    }
    assert_eq!(gained.0 + gained.1, 8, "both casts landed on it: {gained:?}");
    g.check_invariants().unwrap();
}

// ---- repeats after a prompt: a Battlecry trigger ----

#[test]
fn a_golden_kelp_keeper_triggers_a_targeting_battlecry_twice_on_the_one_pick() {
    // A golden Kelp Keeper triggers the chosen minion's Battlecry twice. The
    // first trigger asks for the Pirate to buff and the second lands on it
    // (docs/rules/design.md "Set versus add").
    let mut g = game(7005);
    put_on_board(&mut g, 0, "Kelp Keeper", true);
    put_on_board(&mut g, 0, "Lovesick Balladist", false);
    put_on_board(&mut g, 0, "Southsea Busker", false);
    put_on_board(&mut g, 0, "Bigwig Bandit", false);
    set_shop(&mut g, 0, &["Bubble Gunner"]);
    g.players[0].gold = 10;
    g.players[0].gold_spent_turn = 0;
    act(&mut g, 0, Action::Activate(0));
    // The Activate asks which minion's Battlecry to trigger.
    assert!(g.players[0].pending_target.is_some(), "the Activate picks its minion");
    act(&mut g, 0, Action::Target(1));
    // Lovesick Balladist's own Battlecry asks which Pirate to buff.
    assert!(g.players[0].pending_target.is_some(), "its Battlecry picks its Pirate");
    let busker = stats(&g, 0, 2);
    let bandit = stats(&g, 0, 3);
    act(&mut g, 0, Action::Target(2));
    assert!(g.players[0].pending_target.is_none(), "the second trigger asks nothing more");
    let after = stats(&g, 0, 2);
    // +0/+2 a trigger, and Lovesick Balladist adds +0/+1 for each Gold spent
    // this turn (the Activate spent 1), so two triggers give +0/+6.
    assert_eq!(
        (after.0 - busker.0, after.1 - busker.1),
        (0, 6),
        "both triggers landed on the one Pirate picked"
    );
    assert_eq!(stats(&g, 0, 3), bandit, "nothing else moved");
    g.check_invariants().unwrap();
}

#[test]
fn drakkari_young_murk_eye_lands_every_neighbours_battlecry_twice() {
    // Drakkari Enchanter repeats the end of turn, so Young Murk-Eye triggers
    // each neighbour's Battlecry twice. Both neighbours ask for their own
    // target and both repeats land on it: four Battlecry triggers in all.
    let mut g = game(7006);
    put_on_board(&mut g, 0, "Lovesick Balladist", false);
    put_on_board(&mut g, 0, "Young Murk-Eye", false);
    put_on_board(&mut g, 0, "Lovesick Balladist", false);
    put_on_board(&mut g, 0, "Drakkari Enchanter", false);
    put_on_board(&mut g, 0, "Southsea Busker", false);
    put_on_board(&mut g, 0, "Bigwig Bandit", false);
    set_shop(&mut g, 0, &["Bubble Gunner"]);
    g.players[0].gold_spent_turn = 0;
    let before: Vec<(i32, i32)> =
        (0..g.players[0].board.len()).map(|s| stats(&g, 0, s)).collect();
    // The end of the turn answers what it opens by itself: a target no one
    // answered goes to the first legal minion.
    act(&mut g, 0, Action::EndTurn);
    while answer(&mut g) {}
    let after: Vec<(i32, i32)> =
        (0..g.players[0].board.len()).map(|s| stats(&g, 0, s)).collect();
    let health: i32 = after.iter().zip(&before).map(|(a, b)| a.1 - b.1).sum();
    assert_eq!(health, 8, "two neighbours, twice each, +0/+2 a trigger: {after:?}");
    g.check_invariants().unwrap();
}

#[test]
fn a_repeat_of_a_choose_one_minion_picks_its_option_at_random() {
    // A golden Kelp Keeper triggers a Choose One minion's choice twice: the
    // player picks once, the repeat's option is random (docs/rules/design.md
    // "Set versus add"). Sprightly Scarab's options are +1/+1 with Reborn or
    // +4/+0 with Windfury, both on a Tavern Beast.
    let mut g = game(7007);
    put_on_board(&mut g, 0, "Kelp Keeper", true);
    put_on_board(&mut g, 0, "Sprightly Scarab", false);
    // Sprightly Scarab's options buff a Beast, so the Tavern holds one.
    set_shop(&mut g, 0, &["Flittering Bat"]);
    g.players[0].gold = 10;
    act(&mut g, 0, Action::Activate(0));
    // The Activate picks the minion whose Battlecry to trigger: the Scarab is
    // the only one, so it runs straight away.
    if g.players[0].pending_target.is_some() {
        act(&mut g, 0, Action::Target(1));
    }
    assert!(g.players[0].pending_choose_one.is_some(), "the player picks the option");
    let beast = {
        let m = g.players[0].shop[0]; // the Tavern's Beast, the only legal target
        (m.attack, m.health)
    };
    act(&mut g, 0, Action::Choose(0));
    assert!(g.players[0].pending_choose_one.is_none(), "the repeat never asks for an option");
    assert!(g.players[0].pending_target.is_none(), "and never for a target");
    // One choice of the player's option 0 is +1/+1; the random one is either
    // option 0 again (+1/+1) or option 1 (+4/+0 with Windfury).
    let shop_after = g.players[0].shop[0];
    let gained = (shop_after.attack - beast.0, shop_after.health - beast.1);
    assert!(
        [(2, 2), (5, 1)].contains(&gained),
        "both choices landed on the one Tavern Beast: {gained:?}"
    );
    g.check_invariants().unwrap();
}

