//! The Quilboar tribe, with the real cards from cards.toml: Blood Gems and
//! the gem value, Tavern spells versus plain spells, Balinda and Hot-Air
//! Surveyor gem casts, Roogug, Gem Confiscation, Blood Gem Barrage, the
//! discard, Thorned Trailblazer, the Triple Reward spell, Fandral's Fortune,
//! Jailbird Juggernaut's Golem and the widened tribe mask.

use std::sync::Arc;

use bgcore::cards::Tribes;
use bgcore::fight::{Counters, fight_with};
use bgcore::{Action, CardDb, CardId, Data, Game, Minion, Rng, Tribe};

fn db() -> CardDb {
    Data::bundled().cards
}

fn lobby(ts: &[Tribe]) -> bgcore::Tribes {
    ts.iter().copied().fold(bgcore::Tribes::NONE, |m, t| m.with(t))
}

fn game(seed: u64) -> Game {
    Game::new_with_lobby(Arc::new(Data::bundled()), seed, lobby(&[Tribe::Beast, Tribe::Mech, Tribe::Elemental, Tribe::Demon, Tribe::Quilboar]))
}

fn mk(dbc: &CardDb, name: &str, golden: bool) -> Minion {
    dbc.make(dbc.by_name(name).expect(name), golden)
}

fn id(dbc: &CardDb, name: &str) -> CardId {
    dbc.by_name(name).expect(name)
}

fn stats(m: &Minion) -> (i32, i32) {
    (m.attack, m.health)
}

/// A minion that never attacks and takes a long time to kill.
fn wall(dbc: &CardDb) -> Minion {
    let mut w = mk(dbc, "Microbot", false);
    w.attack = 0;
    w.health = 1000;
    w
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

/// Everyone ends their turn until the next turn starts.
fn end_all_turns(g: &mut Game) {
    let t = g.turn;
    while g.turn == t {
        if !answer(g) {
            g.step(Action::EndTurn).unwrap();
        }
    }
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

fn put_in_hand(g: &mut Game, p: usize, name: &str, golden: bool) {
    let c = id(&g.data.cards, name);
    let slot = g.data.cards.get(c).pool_slot as usize;
    let copies = if golden { 3 } else { 1 };
    g.pool[slot] -= copies;
    let m = g.players[p].make_card(&g.data.cards, c, golden, copies);
    g.players[p].hand.push(m);
}

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

fn name(g: &Game, m: &Minion) -> String {
    g.data.cards.get(m.card).name.clone()
}

fn play(g: &mut Game, p: usize, hand: u8, pos: u8) {
    act(g, p, Action::Play { hand, pos });
}

/// The hand slot of the first Blood Gem, if any.
fn gem_slot(g: &Game, p: usize) -> Option<usize> {
    let gem = id(&g.data.cards, "Blood Gem");
    g.players[p].hand.iter().position(|m| m.card == gem)
}

/// The uses start_turn would hand the board's Trailblazers (put_on_board
/// bypasses the play path).
fn recount_trailblazers(g: &mut Game) {
    let cards = g.data.cards.clone();
    for m in &mut g.players[0].board {
        m.uses_left = m.turn_uses(&cards);
    }
}

fn gems_in_hand(g: &Game, p: usize) -> usize {
    let gem = id(&g.data.cards, "Blood Gem");
    g.players[p].hand.iter().filter(|m| m.card == gem).count()
}

/// Cast one Blood Gem from hand on the board minion in slot `pos`.
fn cast_gem(g: &mut Game, p: usize, pos: u8) {
    let h = gem_slot(g, p).expect("no Blood Gem in hand") as u8;
    play(g, p, h, 0);
    act(g, p, Action::Target(pos));
}

// ================= Blood Gems and the gem value =================

#[test]
fn razorfen_geomancer_gets_gems_and_a_gem_casts_on_board_and_tavern() {
    let mut g = game(2);
    put_in_hand(&mut g, 0, "Razorfen Geomancer", false);
    play(&mut g, 0, 0, 0);
    assert_eq!(gems_in_hand(&g, 0), 2);
    // a Gem casts on a friendly minion with the player's value, and is
    // tracked as Blood Gem stats on it
    let before = stats(&g.players[0].board[0]);
    cast_gem(&mut g, 0, 0);
    assert_eq!(stats(&g.players[0].board[0]), (before.0 + 1, before.1 + 1));
    assert_eq!(g.players[0].board[0].gem_attack, 1);
    assert_eq!(g.players[0].board[0].gem_health, 1);
    // and on a minion in the Tavern ("a minion")
    set_shop(&mut g, 0, &["Wolf Pup"]);
    let shop_before = stats(&g.players[0].shop[0]);
    let h = gem_slot(&g, 0).unwrap() as u8;
    play(&mut g, 0, h, 0);
    act(&mut g, 0, Action::TargetShop(0));
    assert_eq!(stats(&g.players[0].shop[0]), (shop_before.0 + 1, shop_before.1 + 1));
    assert_eq!(g.players[0].shop[0].gem_attack, 1);
    g.check_invariants().unwrap();
}

/// "Blood Gems played from your hand cast an extra time" (Hot-Air Surveyor)
/// has no "friendly minion" in it, so it casts again on a Tavern minion too,
/// where Balinda's does not (docs/rules/design.md: her text says "on a
/// friendly minion"). The log `10_28_19_47_59-g1` turn 8 gives Relentless
/// Deflector +2/+2 for one Gem with a Surveyor on the board.
#[test]
fn a_gem_on_a_tavern_minion_casts_again_per_surveyor() {
    let mut g = game(41);
    put_on_board(&mut g, 0, "Hot-Air Surveyor", false);
    g.give(0, "Blood Gem").unwrap();
    set_shop(&mut g, 0, &["Wolf Pup"]);
    let before = stats(&g.players[0].shop[0]);
    let h = gem_slot(&g, 0).unwrap() as u8;
    play(&mut g, 0, h, 0);
    act(&mut g, 0, Action::TargetShop(0));
    assert_eq!(
        stats(&g.players[0].shop[0]),
        (before.0 + 2, before.1 + 2),
        "two casts of a 1/1 Gem"
    );
    // Balinda keeps her hands off a Tavern minion.
    let mut g = game(42);
    put_on_board(&mut g, 0, "Balinda Stonehearth", false);
    g.give(0, "Blood Gem").unwrap();
    set_shop(&mut g, 0, &["Wolf Pup"]);
    let before = stats(&g.players[0].shop[0]);
    let h = gem_slot(&g, 0).unwrap() as u8;
    play(&mut g, 0, h, 0);
    act(&mut g, 0, Action::TargetShop(0));
    assert_eq!(stats(&g.players[0].shop[0]), (before.0 + 1, before.1 + 1));
    g.check_invariants().unwrap();
}

#[test]
fn the_gem_value_grows_this_game_from_every_source() {
    let mut g = game(3);
    assert_eq!((g.players[0].gem_attack, g.players[0].gem_health), (1, 1));
    // Fearless Foodie: +1/+1 or 4 Gems
    put_in_hand(&mut g, 0, "Fearless Foodie", false);
    play(&mut g, 0, 0, 0);
    act(&mut g, 0, Action::Choose(0));
    assert_eq!((g.players[0].gem_attack, g.players[0].gem_health), (2, 2));
    // Sanguine Champion's Battlecry: +2/+1
    put_in_hand(&mut g, 0, "Sanguine Champion", false);
    play(&mut g, 0, 0, 0);
    assert_eq!((g.players[0].gem_attack, g.players[0].gem_health), (4, 3));
    // a Gem now gives the whole value
    put_on_board(&mut g, 0, "Tusked Camper", false);
    g.give(0, "Blood Gem").unwrap();
    let before = stats(&g.players[0].board[0]);
    cast_gem(&mut g, 0, 0);
    assert_eq!(stats(&g.players[0].board[0]), (before.0 + 4, before.1 + 3));
    // Gem Day: +1 Attack or +1 Health
    g.give(0, "Gem Day").unwrap();
    let h = g.players[0].hand.len() as u8 - 1;
    play(&mut g, 0, h, 0);
    act(&mut g, 0, Action::Choose(0));
    assert_eq!((g.players[0].gem_attack, g.players[0].gem_health), (5, 3));
    g.give(0, "Gem Day").unwrap();
    let h = g.players[0].hand.len() as u8 - 1;
    play(&mut g, 0, h, 0);
    act(&mut g, 0, Action::Choose(1));
    assert_eq!((g.players[0].gem_attack, g.players[0].gem_health), (5, 4));
    g.check_invariants().unwrap();
}

#[test]
fn golden_choose_one_cards_double_their_numbers() {
    let mut g = game(4);
    put_in_hand(&mut g, 0, "Fearless Foodie", true);
    play(&mut g, 0, 0, 0);
    act(&mut g, 0, Action::Choose(0));
    assert_eq!((g.players[0].gem_attack, g.players[0].gem_health), (3, 3));
    let mut g = game(4);
    put_in_hand(&mut g, 0, "Crater Miner", true);
    play(&mut g, 0, 0, 0);
    act(&mut g, 0, Action::Choose(0));
    assert_eq!(gems_in_hand(&g, 0), 4, "golden: 4 gems");
    let mut g = game(4);
    put_in_hand(&mut g, 0, "Crater Miner", true);
    play(&mut g, 0, 0, 0);
    act(&mut g, 0, Action::Choose(1));
    let days = g.players[0].hand.iter().filter(|m| name(&g, m) == "Gem Day").count();
    assert_eq!(days, 2, "golden: 2 Gem Days");
    g.check_invariants().unwrap();
}

#[test]
fn blood_gems_are_plain_spells_no_counters_no_azerite_no_spell_power() {
    // Felboar counts Tavern spells only
    let mut g = game(5);
    put_on_board(&mut g, 0, "Felboar", false);
    set_shop(&mut g, 0, &["Wolf Pup"]);
    g.players[0].gold = 10;
    g.give(0, "Blood Gem").unwrap();
    g.give(0, "Blood Gem").unwrap();
    g.give(0, "Blood Gem").unwrap();
    for _ in 0..3 {
        cast_gem(&mut g, 0, 0);
    }
    assert_eq!(g.players[0].board[0].improvements, 0, "gems do not count as spells");
    g.give(0, "Tavern Coin").unwrap();
    let h = g.players[0].hand.len() as u8 - 1;
    play(&mut g, 0, h, 0);
    assert_eq!(g.players[0].board[0].improvements, 1, "a Tavern spell does");

    // Living Azerite reacts to Tavern spells only
    let mut g = game(6);
    put_on_board(&mut g, 0, "Living Azerite", false);
    set_shop(&mut g, 0, &["Dune Dweller", "Dune Dweller"]);
    let before = stats(&g.players[0].shop[0]);
    g.give(0, "Blood Gem").unwrap();
    cast_gem(&mut g, 0, 0);
    assert_eq!(stats(&g.players[0].shop[0]), before, "a gem is not a Tavern spell");
    g.give(0, "Tavern Coin").unwrap();
    let h = g.players[0].hand.len() as u8 - 1;
    play(&mut g, 0, h, 0);
    assert_eq!(stats(&g.players[0].shop[0]), (before.0 + 4, before.1 + 3), "a Tavern spell does");
    g.check_invariants().unwrap();
}

#[test]
fn a_gem_on_the_amalgam_feeds_felboar_once_per_player_cast() {
    // The known step from Era's log (Hearthstone_2026_10_24_17_59_26-g1,
    // turn 10 steps 59 to 64): one plain Hot-Air Surveyor, a Blood Gem on the
    // Gatekeeper Amalgam. Felboar counts the gem and the Surveyor's extra
    // cast, two spells, and never the Misplaced Tea Set the Amalgam casts for
    // the player (Era 2026-11-02).
    let mut g = game(30);
    put_on_board(&mut g, 0, "Felboar", false);
    put_on_board(&mut g, 0, "Hot-Air Surveyor", false);
    put_on_board(&mut g, 0, "Gatekeeper Amalgam", false);
    let before: i32 = g.players[0].board.iter().map(|m| m.attack).sum();
    g.players[0].gold = 10;
    g.give(0, "Blood Gem").unwrap();
    cast_gem(&mut g, 0, 2);
    assert_eq!(
        g.players[0].board[0].improvements,
        2,
        "the gem and the Surveyor's copy, no Tea Set"
    );
    // The Tea Set itself still happened: it buffed a minion of each type.
    let after: i32 = g.players[0].board.iter().map(|m| m.attack).sum();
    assert!(after >= before + 4, "the Tea Set still buffed the board");
    g.check_invariants().unwrap();
}

#[test]
fn felboar_eats_the_tavern_minion_the_same_gem_buffed() {
    // docs/rules/design.md "Eating a minion with a spell": Felboar's "After
    // you cast 3 spells" fires once the spell has resolved, so it takes the
    // minion the gem itself buffed. The old order fired the trigger on the
    // play and ate the minion as it stood before the gem landed (log
    // 2026_11_02_00_10_22-g4 turn 9 step 39).
    let mut g = game(30);
    put_on_board(&mut g, 0, "Felboar", false);
    set_shop(&mut g, 0, &["Wolf Pup"]);
    g.players[0].board[0].improvements = 2;
    g.players[0].gold = 10;
    g.give(0, "Blood Gem").unwrap();
    let gem = id(&g.data.cards, "Blood Gem");
    let h = g.players[0].hand.iter().position(|m| m.card == gem).unwrap() as u8;
    play(&mut g, 0, h, 0);
    let before = (
        stats(&g.players[0].board[0]),
        stats(&g.players[0].shop[0]),
        (g.players[0].gem_attack, g.players[0].gem_health),
    );
    act(&mut g, 0, Action::TargetShop(0));
    let (felboar, pup, gem) = before;
    assert_eq!(
        stats(&g.players[0].board[0]),
        (felboar.0 + pup.0 + gem.0, felboar.1 + pup.1 + gem.1),
        "the eaten minion had the gem's stats on it"
    );
    g.check_invariants().unwrap();
}

#[test]
fn a_three_cast_gem_feeds_felboar_between_its_casts() {
    // Hearthstone_2026_11_02_00_10_22-g4 turn 9 step 39: a Gem played from
    // hand with Balinda (two casts) and a Hot-Air Surveyor (one more) is a
    // three-cast play, and the log's Felboar counter reaches 3 on the first
    // of the three. So the minion it eats carries the Devilish Distractor's
    // buff from that cast alone (+1/+2), not the play's three (+3/+6): a
    // cast's "after you cast a spell" trigger fires as that cast resolves,
    // before the next one starts (docs/rules/design.md "Eating a minion with
    // a spell"). The old order fired all three after the last cast and ate
    // the minion with all three buffs on it.
    let mut g = game(33);
    put_on_board(&mut g, 0, "Felboar", false);
    put_on_board(&mut g, 0, "Devilish Distractor", false);
    put_on_board(&mut g, 0, "Balinda Stonehearth", false);
    put_on_board(&mut g, 0, "Hot-Air Surveyor", false);
    set_shop(&mut g, 0, &["Wolf Pup"]);
    // Two spells already counted: this play's first cast is the third.
    g.players[0].board[0].improvements = 2;
    g.players[0].gold = 10;
    g.give(0, "Blood Gem").unwrap();
    let before = (stats(&g.players[0].board[0]), stats(&g.players[0].shop[0]));
    let h = gem_slot(&g, 0).expect("a Blood Gem in hand") as u8;
    play(&mut g, 0, h, 0);
    act(&mut g, 0, Action::Target(1)); // the Distractor
    let (felboar, pup) = before;
    assert_eq!(
        stats(&g.players[0].board[0]),
        (felboar.0 + pup.0 + 1, felboar.1 + pup.1 + 2),
        "one cast's Tavern buff on the eaten minion, not the play's three"
    );
    g.check_invariants().unwrap();
}

#[test]
fn a_whenever_tavern_spell_trigger_fires_before_the_spell_resolves() {
    // Living Azerite: "Whenever you cast a Tavern spell, give Elementals in
    // the Tavern +4/+3". "Whenever" is on the cast, so a spell that reads the
    // Tavern (Arcane Absorption) sees the buffed minion.
    let mut g = game(31);
    put_on_board(&mut g, 0, "Living Azerite", false);
    put_on_board(&mut g, 0, "Living Azerite", false);
    set_shop(&mut g, 0, &["Dune Dweller"]);
    g.players[0].tier = 6;
    g.players[0].gold = 10;
    g.give(0, "Arcane Absorption").unwrap();
    let h = g.players[0].hand.len() as u8 - 1;
    play(&mut g, 0, h, 0);
    act(&mut g, 0, Action::Target(1));
    // Dune Dweller 3/3 takes two +4/+3 (two Azerites): 11/9. Half of it,
    // rounded up, is 6/5.
    assert_eq!(stats(&g.players[0].shop[0]), (11, 9), "both Azerites buffed the Tavern");
    assert_eq!(stats(&g.players[0].board[1]), (8 + 6, 6 + 5), "the spell read the buffed Tavern");
    g.check_invariants().unwrap();
}

#[test]
fn a_taught_choose_one_spell_is_the_minions_cast_and_gets_no_option() {
    // A Magicfin Apprentice casts the Tavern spell it was taught: Alliance
    // Flag is a Choose One spell, so its option is picked at random with no
    // prompt (docs/rules/design.md "Set versus add", Era 2026-11-04). Its
    // target is still the player's pick, and the cast is the minion's and
    // counts for nothing.
    let mut g = game(32);
    put_on_board(&mut g, 0, "Felboar", false);
    put_on_board(&mut g, 0, "Gatekeeper Amalgam", false);
    let flag = g.data.cards.by_name("Alliance Flag").unwrap();
    let token = g.data.cards.taught_token.expect("a taught token");
    let mut m = g.players[0].make_card(&g.data.cards, token, false, 0);
    m.taught_spell = Some(flag);
    g.players[0].hand.push(m);
    g.players[0].gold = 10;
    let h = g.players[0].hand.len() as u8 - 1;
    play(&mut g, 0, h, 2);
    assert!(g.players[0].pending_choose_one.is_none(), "no option is offered");
    act(&mut g, 0, Action::Target(1));
    // The Amalgam is 6/6 and the random option is +3/+1 or +1/+3: the cast
    // still lands.
    let (attack, health) = stats(&g.players[0].board[1]);
    assert!(
        [(6 + 3, 6 + 1), (6 + 1, 6 + 3)].contains(&(attack, health)),
        "the spell still buffed: {attack}/{health}"
    );
    assert_eq!(g.players[0].board[0].improvements, 0, "not a spell you cast");
    assert_eq!(g.players[0].spells_cast, 0, "and not a Tavern spell you cast");
    g.check_invariants().unwrap();
}

#[test]
fn a_tavern_spell_on_the_amalgam_counts_once() {
    // Hearthstone_2026_10_28_02_28_32-g4, turn 11 step 33: a Fortify cast on
    // the Gatekeeper Amalgam counts 1, not 2.
    let mut g = game(31);
    put_on_board(&mut g, 0, "Felboar", false);
    put_on_board(&mut g, 0, "Gatekeeper Amalgam", false);
    g.players[0].gold = 10;
    g.give(0, "Fortify").unwrap();
    let h = g.players[0].hand.len() as u8 - 1;
    play(&mut g, 0, h, 0);
    act(&mut g, 0, Action::Target(1));
    assert_eq!(g.players[0].board[0].improvements, 1, "one Tavern spell, one count");
    assert_eq!(g.players[0].spells_cast, 1, "the Tea Set is not a spell you cast");
    assert_eq!(g.players[0].spells_cast_turn, 1, "and not one this turn either");
    g.check_invariants().unwrap();
}

#[test]
fn balinda_sets_and_each_surveyor_adds_gem_casts() {
    let mut g = game(7);
    put_on_board(&mut g, 0, "Tusked Camper", false);
    g.give(0, "Blood Gem").unwrap();
    cast_gem(&mut g, 0, 0);
    assert_eq!(g.players[0].board[0].gem_attack, 1, "one cast alone");

    let mut g = game(8);
    put_on_board(&mut g, 0, "Balinda Stonehearth", false);
    put_on_board(&mut g, 0, "Tusked Camper", false);
    g.give(0, "Blood Gem").unwrap();
    cast_gem(&mut g, 0, 1);
    assert_eq!(g.players[0].board[1].gem_attack, 2, "Balinda: 2 casts");

    let mut g = game(9);
    put_on_board(&mut g, 0, "Balinda Stonehearth", false);
    put_on_board(&mut g, 0, "Hot-Air Surveyor", false);
    put_on_board(&mut g, 0, "Tusked Camper", false);
    g.give(0, "Blood Gem").unwrap();
    cast_gem(&mut g, 0, 2);
    assert_eq!(g.players[0].board[2].gem_attack, 3, "Balinda plus one Surveyor: 3 casts");

    // two Surveyors stack, golden ones add two
    let mut g = game(10);
    put_on_board(&mut g, 0, "Hot-Air Surveyor", false);
    put_on_board(&mut g, 0, "Hot-Air Surveyor", false);
    put_on_board(&mut g, 0, "Tusked Camper", false);
    g.give(0, "Blood Gem").unwrap();
    cast_gem(&mut g, 0, 2);
    assert_eq!(g.players[0].board[2].gem_attack, 3, "two adders stack");
    g.check_invariants().unwrap();
}

#[test]
fn a_roogug_reacts_to_a_gem_from_hand_and_two_roogugs_never_loop() {
    // docs/rules/design.md "Geomagus Roogug": every Gem played on it makes it
    // play its own on a different friendly minion, and a Gem another Roogug
    // played is the one exception, so two Roogugs never loop.
    let mut g = game(11);
    put_on_board(&mut g, 0, "Geomagus Roogug", false);
    put_on_board(&mut g, 0, "Tusked Camper", false);
    g.give(0, "Blood Gem").unwrap();
    cast_gem(&mut g, 0, 0);
    // the cast gem lands on Roogug, which plays its own on the Camper
    assert_eq!(g.players[0].board[0].gem_attack, 1, "the cast gem");
    assert_eq!(g.players[0].board[1].gem_attack, 1, "Roogug's gem on a different minion");

    // a gem on another minion does nothing to Roogug
    let mut g = game(12);
    put_on_board(&mut g, 0, "Geomagus Roogug", false);
    put_on_board(&mut g, 0, "Tusked Camper", false);
    g.give(0, "Blood Gem").unwrap();
    cast_gem(&mut g, 0, 1);
    assert_eq!(g.players[0].board[0].gem_attack, 0);
    assert_eq!(g.players[0].board[1].gem_attack, 1);

    // two Roogugs: one from-hand gem moves one gem across, no loop
    let mut g = game(13);
    put_on_board(&mut g, 0, "Geomagus Roogug", false);
    put_on_board(&mut g, 0, "Geomagus Roogug", false);
    g.give(0, "Blood Gem").unwrap();
    cast_gem(&mut g, 0, 0);
    let total: i32 = g.players[0].board.iter().map(|m| m.gem_attack).sum();
    assert_eq!(total, 2, "the cast gem and one reaction, never more");
    g.check_invariants().unwrap();
}

#[test]
fn a_hogriders_gem_on_a_roogug_makes_it_play_its_own() {
    // docs/rules/design.md "Geomagus Roogug": it reacts to Gems a minion or
    // a spell plays, Turbo Hogrider's included. The Power log
    // Hearthstone_2026_10_28_02_28_32 shows each of the Hogrider's Gems on a
    // Roogug followed by its trigger.
    let mut g = game(41);
    put_on_board(&mut g, 0, "Turbo Hogrider", false);
    put_on_board(&mut g, 0, "Geomagus Roogug", false);
    put_on_board(&mut g, 0, "Tusked Camper", false);
    g.give(0, "Gem Day").unwrap();
    let h = g.players[0].hand.len() as u8 - 1;
    play(&mut g, 0, h, 0);
    act(&mut g, 0, Action::Choose(0));
    // Gem Day makes a Gem +2/+1: the Hogrider's two Gems land on it (and on
    // the two Quilboar beside it), and each one makes the Roogug play a Gem
    // of its own.
    assert_eq!(g.players[0].board[1].gem_attack, 4, "the Hogrider's two Gems");
    let all: i32 = g.players[0].board.iter().map(|m| m.gem_attack).sum();
    assert_eq!(all, 16, "2 Gems on each of the three Quilboar, and 2 reactions");
    g.check_invariants().unwrap();
}

#[test]
fn a_gem_confiscation_on_a_roogug_makes_it_play_a_gem_per_gem() {
    // Three Gems go on the target, so a Roogug there plays three of its own,
    // each as the Gem before it lands. The Power log
    // Hearthstone_2026_10_28_02_28_32 (line 1715147 onward, game 4 turn 13)
    // shows the Roogug's Gems on the neighbours before the steal: they lose
    // the stats those Gems gave them to the Roogug. So the Roogug keeps its
    // 3 Gems plus the 3 the neighbours lost, and the neighbours are back to
    // where they started.
    let mut g = game(42);
    put_on_board(&mut g, 0, "Tusked Camper", false);
    put_on_board(&mut g, 0, "Geomagus Roogug", false);
    put_on_board(&mut g, 0, "Roadboar", false);
    let camp = stats(&g.players[0].board[0]);
    let boar = stats(&g.players[0].board[2]);
    g.give(0, "Gem Confiscation").unwrap();
    let h = g.players[0].hand.len() as u8 - 1;
    play(&mut g, 0, h, 0);
    act(&mut g, 0, Action::Target(1));
    assert_eq!(
        g.players[0].board[1].gem_attack, 6,
        "the three Confiscation Gems, then the three the Roogug's own Gems left on the neighbours"
    );
    let others: i32 = [0, 2].into_iter().map(|i| g.players[0].board[i].gem_attack).sum();
    assert_eq!(others, 0, "the steal took the Gems the Roogug played on them");
    assert_eq!(stats(&g.players[0].board[0]), camp, "a neighbour ends where it started");
    assert_eq!(stats(&g.players[0].board[2]), boar);
    g.check_invariants().unwrap();
}

#[test]
fn a_barrage_gem_on_a_tavern_roogug_does_nothing_to_the_board() {
    // Blood Gem Barrage gives its Gem to every Tavern minion on a Refresh. A
    // Roogug in the Tavern doesn't react from there until we know it does
    // (docs/notes/UNSURE.md, 2026-11-04): the board gets nothing.
    let mut g = game(43);
    set_shop(&mut g, 0, &["Geomagus Roogug", "Cord Puller", "Wolf Pup"]);
    put_on_board(&mut g, 0, "Tusked Camper", false);
    g.players[0].barrage = (2, 2);
    g.players[0].gold = 10;
    // buy one of the Roogug's neighbours and freeze: the next turn's roll
    // fills that slot, which makes it a Refresh, and the Roogug stays
    act(&mut g, 0, Action::Buy(1));
    act(&mut g, 0, Action::Freeze);
    end_all_turns(&mut g);
    assert_eq!(name(&g, &g.players[0].shop[0]), "Geomagus Roogug", "still in the Tavern");
    assert_eq!(g.players[0].shop[0].gem_attack, 2, "Barrage's Gem on it");
    let board: i32 = g.players[0].board.iter().map(|m| m.gem_attack).sum();
    assert_eq!(board, 0, "a Tavern Roogug plays no Gem on the board");
    g.check_invariants().unwrap();
}

#[test]
fn a_golden_roogug_reacts_once_per_extra_gem_cast() {
    // docs/rules/design.md "Geomagus Roogug": every Gem played on it makes it
    // react. A golden Hot-Air Surveyor makes one gem play three casts, each
    // cast lands on the Roogug and makes it react, so a golden Roogug's two
    // Gems fire three times. Its own Gems carry the from-Roogug mark, so
    // nothing chains: 3 Gems on the Roogug itself and 6 over the rest of the
    // board, and no more.
    let mut g = game(19);
    put_on_board(&mut g, 0, "Geomagus Roogug", true);
    put_on_board(&mut g, 0, "Hot-Air Surveyor", true);
    put_on_board(&mut g, 0, "Tusked Camper", false);
    g.give(0, "Blood Gem").unwrap();
    cast_gem(&mut g, 0, 0);
    assert_eq!(g.players[0].board[0].gem_attack, 3, "three casts on the Roogug");
    let spread: i32 = g.players[0].board[1..].iter().map(|m| m.gem_attack).sum();
    assert_eq!(spread, 6, "two gems from each of the three casts, and no chain");
    g.check_invariants().unwrap();
}

#[test]
fn every_extra_gem_cast_makes_the_gatekeeper_cast_its_tea_set_again() {
    // A gem that lands on a Gatekeeper Amalgam makes it cast its Misplaced Tea
    // Set, and one gem play with a golden Hot-Air Surveyor is three casts, so
    // three Tea Sets: the Gems give the Amalgam +1/+1 three times and its own
    // Tea Sets +4/+4 three times, and the Tea Set's one-pick-per-type rule
    // gives the three Quilboar hosts +4/+4 a cast between them.
    let mut g = game(20);
    put_on_board(&mut g, 0, "Hot-Air Surveyor", true);
    put_on_board(&mut g, 0, "Gatekeeper Amalgam", false);
    put_on_board(&mut g, 0, "Tusked Camper", false);
    put_on_board(&mut g, 0, "Tusked Camper", false);
    let before: Vec<(i32, i32)> = g.players[0].board.iter().map(stats).collect();
    g.give(0, "Blood Gem").unwrap();
    cast_gem(&mut g, 0, 1);
    let delta = |i: usize| {
        let now = stats(&g.players[0].board[i]);
        (now.0 - before[i].0, now.1 - before[i].1)
    };
    assert_eq!(delta(1), (15, 15), "three Gems and three Tea Sets on the Amalgam");
    let quilboar: (i32, i32) = [0, 2, 3]
        .into_iter()
        .map(delta)
        .fold((0, 0), |a, b| (a.0 + b.0, a.1 + b.1));
    assert_eq!(quilboar, (12, 12), "three +4/+4 Tea Set picks over the Quilboars");
    g.check_invariants().unwrap();
}

// ================= Gem Confiscation and Barrage =================

#[test]
fn gem_confiscation_plays_gems_and_steals_the_neighbours_gem_stats() {
    let mut g = game(14);
    put_on_board(&mut g, 0, "Tusked Camper", false);
    put_on_board(&mut g, 0, "Prodigious Tusker", false);
    put_on_board(&mut g, 0, "Roadboar", false);
    // buffed gem stats on both neighbours: two gems each
    for _ in 0..2 {
        g.give(0, "Blood Gem").unwrap();
        cast_gem(&mut g, 0, 0);
        g.give(0, "Blood Gem").unwrap();
        cast_gem(&mut g, 0, 2);
    }
    assert_eq!(g.players[0].board[0].gem_attack, 2);
    assert_eq!(g.players[0].board[2].gem_attack, 2);
    let mid = stats(&g.players[0].board[1]);
    g.give(0, "Gem Confiscation").unwrap();
    let h = g.players[0].hand.len() as u8 - 1;
    play(&mut g, 0, h, 0);
    act(&mut g, 0, Action::Target(1));
    // 3 gems of the player's +1/+1, plus 2/2 of gems from each neighbour
    assert_eq!(stats(&g.players[0].board[1]), (mid.0 + 3 + 4, mid.1 + 3 + 4));
    assert_eq!(g.players[0].board[1].gem_attack, 7);
    assert_eq!(g.players[0].board[1].gem_health, 7);
    // the neighbours lost exactly their gem stats
    let left = stats(&g.players[0].board[0]);
    assert_eq!(g.players[0].board[0].gem_attack, 0);
    let right = stats(&g.players[0].board[2]);
    assert_eq!(g.players[0].board[2].gem_health, 0);
    assert_eq!(left.0, 2); // the Camper's base 2 attack, gems gone
    assert_eq!(right.1, 4); // the Roadboar's base 4 health, gems gone
    g.check_invariants().unwrap();
}

#[test]
fn blood_gem_barrage_buffs_every_tavern_minion_on_every_explicit_refresh() {
    let mut g = game(15);
    set_shop(&mut g, 0, &["Wolf Pup", "Cord Puller"]);
    let base: Vec<(i32, i32)> = g.players[0].shop.iter().map(stats).collect();
    g.give(0, "Blood Gem Barrage").unwrap();
    let h = g.players[0].hand.len() as u8 - 1;
    play(&mut g, 0, h, 0);
    // gems at their base +1/+1 add no bonus: the cast queues +2/+2
    assert_eq!(g.players[0].barrage, (2, 2));
    g.players[0].gold = 10;
    act(&mut g, 0, Action::Roll);
    for m in &g.players[0].shop {
        let c = g.data.cards.get(m.card);
        assert_eq!(stats(m), (c.attack + 2, c.health + 2), "every Tavern minion, from its card base");
        assert_eq!(m.gem_attack, 2, "counted as Blood Gem stats");
        assert_eq!(m.gem_health, 2);
    }
    drop(base);
    // each cast adds on top
    g.give(0, "Blood Gem Barrage").unwrap();
    let h = g.players[0].hand.len() as u8 - 1;
    play(&mut g, 0, h, 0);
    assert_eq!(g.players[0].barrage, (4, 4));
    g.check_invariants().unwrap();
}

#[test]
fn barrage_hits_the_tavern_on_the_turn_start_roll_only_when_it_changed() {
    // unfrozen: the roll is a Refresh, every new Tavern minion gets it
    let mut g = game(18);
    g.players[0].barrage = (2, 2);
    end_all_turns(&mut g);
    for m in &g.players[0].shop {
        let c = g.data.cards.get(m.card);
        if !c.is_spell() {
            assert_eq!(stats(m), (c.attack + 2, c.health + 2), "{}", c.name);
            assert_eq!((m.gem_attack, m.gem_health), (2, 2));
        }
    }

    // a fully frozen, full Tavern: neither the spell slot nor a minion slot
    // was filled, no Barrage
    let mut g = game(18);
    g.players[0].barrage = (2, 2);
    act(&mut g, 0, Action::Freeze);
    end_all_turns(&mut g);
    for m in &g.players[0].shop {
        let c = g.data.cards.get(m.card);
        if !c.is_spell() {
            assert_eq!(stats(m), (c.attack, c.health), "{}", c.name);
            assert_eq!((m.gem_attack, m.gem_health), (0, 0));
        }
    }

    // frozen with one card bought: the roll fills that slot and Barrage hits
    // every Tavern minion, the frozen ones included; buying one each turn,
    // the frozen minions stack it twice over two turns
    let mut g = game(18);
    g.players[0].barrage = (2, 2);
    g.players[0].gold = 10;
    act(&mut g, 0, Action::Buy(0));
    act(&mut g, 0, Action::Freeze);
    end_all_turns(&mut g);
    assert!(!g.players[0].shop[1..].is_empty(), "cards froze over");
    for m in &g.players[0].shop {
        let c = g.data.cards.get(m.card);
        if !c.is_spell() {
            assert_eq!(stats(m), (c.attack + 2, c.health + 2), "{}", c.name);
        }
    }
    // second turn: buy one of the cards frozen over, freeze again
    g.players[0].gold = 10;
    act(&mut g, 0, Action::Buy(1));
    act(&mut g, 0, Action::Freeze);
    end_all_turns(&mut g);
    // what froze over from turn 1 stacked Barrage twice (+4/+4); the card
    // drawn into the bought slot this turn got it once (+2/+2)
    let over = |gain: (i32, i32)| {
        g.players[0]
            .shop
            .iter()
            .filter(|m| {
                let c = g.data.cards.get(m.card);
                !c.is_spell() && (m.attack - c.attack, m.health - c.health) == gain
            })
            .count()
    };
    assert!(over((4, 4)) > 0, "the frozen minions stack Barrage twice");
    assert!(over((2, 2)) > 0, "the new slot's card got it once");
    g.check_invariants().unwrap();
}

#[test]
fn barrage_adds_the_gem_bonus_and_spell_power_once_per_cast() {
    let mut g = game(16);
    // gems at +2/+2
    put_in_hand(&mut g, 0, "Fearless Foodie", false);
    play(&mut g, 0, 0, 0);
    act(&mut g, 0, Action::Choose(0));
    // spell power +1/+0 (Intrepid Botanist)
    put_in_hand(&mut g, 0, "Intrepid Botanist", false);
    play(&mut g, 0, 0, 0);
    act(&mut g, 0, Action::Choose(0));
    assert_eq!(g.players[0].total_spell_bonus(&g.data.cards), (1, 0));
    g.give(0, "Blood Gem Barrage").unwrap();
    let h = g.players[0].hand.len() as u8 - 1;
    play(&mut g, 0, h, 0);
    // 2 + gem bonus 1 (gems at +2/+2) + spell power 1 = 4 attack,
    // 2 + 1 + 0 = 3 health
    assert_eq!(g.players[0].barrage, (4, 3));
    g.check_invariants().unwrap();
}

#[test]
fn veteran_brigand_casts_barrage_three_times_or_gems_the_board() {
    let mut g = game(17);
    put_on_board(&mut g, 0, "Tusked Camper", false);
    put_on_board(&mut g, 0, "Roadboar", false);
    put_in_hand(&mut g, 0, "Veteran Brigand", false);
    play(&mut g, 0, 0, 0);
    act(&mut g, 0, Action::Choose(1)); // cast Barrage 3 times
    assert_eq!(g.players[0].barrage, (6, 6), "three casts of +2/+2 add on top");
    // each Barrage cast is a Tavern spell cast: after the 3 casts Felboar
    // has consumed a Tavern minion (its "every 3 spells" counter cycled once)
    let mut g = game(19);
    put_on_board(&mut g, 0, "Felboar", false);
    set_shop(&mut g, 0, &["Wolf Pup"]);
    let wolf = stats(&g.players[0].shop[0]);
    let before = stats(&g.players[0].board[0]);
    put_in_hand(&mut g, 0, "Veteran Brigand", false);
    play(&mut g, 0, 0, 0);
    act(&mut g, 0, Action::Choose(1));
    let felboar = g.players[0].board.iter().position(|m| name(&g, m) == "Felboar").unwrap();
    assert_eq!(stats(&g.players[0].board[felboar]), (before.0 + wolf.0, before.1 + wolf.1),
        "three real casts fed Felboar once");
    assert_eq!(g.players[0].board[felboar].improvements, 0, "the count kept no remainder");
    // option 0: 3 gems on all your minions
    let mut g = game(18);
    put_on_board(&mut g, 0, "Tusked Camper", false);
    put_on_board(&mut g, 0, "Roadboar", false);
    put_in_hand(&mut g, 0, "Veteran Brigand", false);
    play(&mut g, 0, 0, 0);
    act(&mut g, 0, Action::Choose(0));
    assert_eq!(g.players[0].board[0].gem_attack, 3);
    assert_eq!(g.players[0].board[1].gem_attack, 3);
    g.check_invariants().unwrap();
}

// ================= Discard =================

#[test]
fn mangled_bandit_discards_a_playable_card_and_the_pool_copy_goes_back() {
    let mut g = game(19);
    put_on_board(&mut g, 0, "Mangled Bandit", false);
    put_in_hand(&mut g, 0, "Wolf Pup", false);
    g.give(0, "Blood Gem").unwrap();
    g.give(0, "Lockbox").unwrap();
    let pup = id(&g.data.cards, "Wolf Pup");
    let pool_before = g.pool[g.data.cards.get(pup).pool_slot as usize];
    act(&mut g, 0, Action::Activate(0));
    assert!(g.players[0].pending_discard.is_some());
    // the Lockbox is unplayable: it is not a legal discard
    let lb = g.players[0].hand.iter().position(|m| name(&g, m) == "Lockbox").unwrap();
    turn_of(&mut g, 0);
    assert!(!g.is_legal(Action::Discard(lb as u8)));
    let gem = gem_slot(&g, 0).unwrap();
    assert!(g.is_legal(Action::Discard(gem as u8)), "a Blood Gem is playable");
    // discard the pool minion: its copy goes back and 3 gems arrive
    act(&mut g, 0, Action::Discard(0));
    assert_eq!(g.pool[g.data.cards.get(pup).pool_slot as usize], pool_before + 1);
    assert!(g.players[0].hand.iter().all(|m| name(&g, m) != "Wolf Pup"));
    assert_eq!(gems_in_hand(&g, 0), 4, "the held gem and the 3 reward gems");
    g.check_invariants().unwrap();

    // discarding a generated card (a Blood Gem) gives nothing back
    let mut g = game(20);
    put_on_board(&mut g, 0, "Mangled Bandit", false);
    g.give(0, "Blood Gem").unwrap();
    act(&mut g, 0, Action::Activate(0));
    let gem = gem_slot(&g, 0).unwrap() as u8;
    act(&mut g, 0, Action::Discard(gem));
    assert_eq!(gems_in_hand(&g, 0), 3, "the reward gems, the discarded one gone");
    g.check_invariants().unwrap();
}

#[test]
fn a_bandit_with_nothing_discardable_cannot_activate() {
    let mut g = game(21);
    put_on_board(&mut g, 0, "Mangled Bandit", false);
    g.give(0, "Lockbox").unwrap();
    turn_of(&mut g, 0);
    assert!(!g.is_legal(Action::Activate(0)), "the only card is unplayable");
    g.give(0, "Blood Gem").unwrap();
    turn_of(&mut g, 0);
    assert!(g.is_legal(Action::Activate(0)));
}

// ================= Trailblazer, Fandral, both effects =================

#[test]
fn trailblazer_runs_both_effects_of_the_first_choose_one_each_turn() {
    let mut g = game(22);
    put_on_board(&mut g, 0, "Thorned Trailblazer", false);
    recount_trailblazers(&mut g);
    put_in_hand(&mut g, 0, "Crater Miner", false);
    play(&mut g, 0, 0, 0);
    // covered: both effects ran, no Choose One prompt opened
    assert!(g.players[0].pending_choose_one.is_none());
    assert_eq!(gems_in_hand(&g, 0), 2);
    assert!(g.players[0].hand.iter().any(|m| name(&g, m) == "Gem Day"));
    // a second Choose One card the same turn asks as usual (hand 0 holds the
    // Triple Reward spell; the Infiltrator went in at the end)
    put_in_hand(&mut g, 0, "Sly Infiltrator", false);
    let h = g.players[0].hand.len() as u8 - 1;
    play(&mut g, 0, h, 0);
    assert!(g.players[0].pending_choose_one.is_some());
    act(&mut g, 0, Action::Choose(0));
    assert_eq!(g.players[0].free_rolls, 2);
    // the next turn covers one again
    end_all_turns(&mut g);
    put_in_hand(&mut g, 0, "Crater Miner", false);
    let h = g.players[0].hand.len() as u8 - 1;
    play(&mut g, 0, h, 0);
    assert!(g.players[0].pending_choose_one.is_none());
    g.check_invariants().unwrap();
}

#[test]
fn two_trailblazers_cover_two_and_a_golden_one_covers_two() {
    let mut g = game(23);
    put_on_board(&mut g, 0, "Thorned Trailblazer", false);
    put_on_board(&mut g, 0, "Thorned Trailblazer", false);
    recount_trailblazers(&mut g);
    for k in 0..2 {
        put_in_hand(&mut g, 0, "Crater Miner", false);
        let h = g.players[0].hand.len() as u8 - 1;
        play(&mut g, 0, h, 0);
        assert!(g.players[0].pending_choose_one.is_none(), "covered play {k}");
    }
    put_in_hand(&mut g, 0, "Crater Miner", false);
    let h = g.players[0].hand.len() as u8 - 1;
    play(&mut g, 0, h, 0);
    assert!(g.players[0].pending_choose_one.is_some(), "the third asks");
    act(&mut g, 0, Action::Choose(0));

    let mut g = game(24);
    put_on_board(&mut g, 0, "Thorned Trailblazer", true);
    recount_trailblazers(&mut g);
    for k in 0..2 {
        put_in_hand(&mut g, 0, "Crater Miner", false);
        let h = g.players[0].hand.len() as u8 - 1;
        play(&mut g, 0, h, 0);
        assert!(g.players[0].pending_choose_one.is_none(), "golden covers two: play {k}");
    }
    put_in_hand(&mut g, 0, "Crater Miner", false);
    let h = g.players[0].hand.len() as u8 - 1;
    play(&mut g, 0, h, 0);
    assert!(g.players[0].pending_choose_one.is_some(), "the third asks");
    g.check_invariants().unwrap();
}

#[test]
fn a_trailblazer_triples_into_a_golden_with_its_own_fresh_combines() {
    // an unused Trailblazer tripled: the golden has exactly its own 2
    // combines, not 3 (the board copies' leftovers leave with them)
    let mut g = game(26);
    put_on_board(&mut g, 0, "Thorned Trailblazer", false);
    put_on_board(&mut g, 0, "Thorned Trailblazer", false);
    recount_trailblazers(&mut g);
    set_shop(&mut g, 0, &["Thorned Trailblazer"]);
    g.players[0].gold = 10;
    act(&mut g, 0, Action::Buy(0)); // the third copy triples
    assert_eq!(g.players[0].board.len(), 0, "the copies went into the golden");
    let gold = g.players[0].hand.iter().position(|m| m.golden).unwrap();
    play(&mut g, 0, gold as u8, 0);
    for k in 0..2 {
        put_in_hand(&mut g, 0, "Crater Miner", false);
        let h = g.players[0].hand.len() as u8 - 1;
        play(&mut g, 0, h, 0);
        assert!(g.players[0].pending_choose_one.is_none(), "golden covers two: play {k}");
    }
    put_in_hand(&mut g, 0, "Crater Miner", false);
    let h = g.players[0].hand.len() as u8 - 1;
    play(&mut g, 0, h, 0);
    assert!(g.players[0].pending_choose_one.is_some(), "the third asks");
    act(&mut g, 0, Action::Choose(0));

    // a used-up Trailblazer tripled gives a fresh 2 as well
    let mut g = game(27);
    put_on_board(&mut g, 0, "Thorned Trailblazer", false);
    put_on_board(&mut g, 0, "Thorned Trailblazer", false);
    recount_trailblazers(&mut g);
    put_in_hand(&mut g, 0, "Crater Miner", false);
    play(&mut g, 0, 0, 0);
    assert!(g.players[0].pending_choose_one.is_none(), "one combine is spent");
    set_shop(&mut g, 0, &["Thorned Trailblazer"]);
    g.players[0].gold = 10;
    act(&mut g, 0, Action::Buy(0));
    let gold = g.players[0].hand.iter().position(|m| m.golden).unwrap();
    play(&mut g, 0, gold as u8, 0);
    for k in 0..2 {
        put_in_hand(&mut g, 0, "Crater Miner", false);
        let h = g.players[0].hand.len() as u8 - 1;
        play(&mut g, 0, h, 0);
        assert!(g.players[0].pending_choose_one.is_none(), "fresh two: play {k}");
    }
    g.check_invariants().unwrap();
}

#[test]
fn selling_a_trailblazer_takes_its_leftover_combines_along() {
    let mut g = game(28);
    put_on_board(&mut g, 0, "Thorned Trailblazer", false);
    put_on_board(&mut g, 0, "Thorned Trailblazer", false);
    recount_trailblazers(&mut g);
    act(&mut g, 0, Action::Sell(0));
    put_in_hand(&mut g, 0, "Crater Miner", false);
    play(&mut g, 0, 0, 0);
    assert!(g.players[0].pending_choose_one.is_none(), "the remaining one covers it");
    put_in_hand(&mut g, 0, "Crater Miner", false);
    let h = g.players[0].hand.len() as u8 - 1;
    play(&mut g, 0, h, 0);
    assert!(g.players[0].pending_choose_one.is_some(), "the sold one's combine is gone");
    g.check_invariants().unwrap();
}

#[test]
fn fandral_discovers_a_choose_one_with_both_effects() {
    let mut g = game(25);
    g.players[0].tier = 6;
    g.players[0].gold = 10;
    g.give(0, "Fandral's Fortune").unwrap();
    play(&mut g, 0, 0, 0);
    assert_eq!(g.players[0].discover.len(), 3);
    for &c in &g.players[0].discover {
        assert!(g.data.cards.get(c).choose_one.is_some(), "only Choose One cards");
        assert!(g.data.cards.get(c).tier <= 6);
    }
    let pick = g.players[0].discover[0];
    act(&mut g, 0, Action::Choose(0));
    let m = g.players[0].hand[0];
    assert_eq!(m.card, pick);
    assert!(m.both_effects);
    // playing it runs both options with no prompt
    g.players[0].board.clear();
    play(&mut g, 0, 0, 0);
    assert!(g.players[0].pending_choose_one.is_none());
    g.check_invariants().unwrap();
}

#[test]
fn turbo_hogrider_plays_gems_on_all_quilboar_after_a_choose_one_play() {
    let mut g = game(26);
    put_on_board(&mut g, 0, "Turbo Hogrider", false);
    put_on_board(&mut g, 0, "Tusked Camper", false);
    put_on_board(&mut g, 0, "Wolf Pup", false);
    g.give(0, "Gem Day").unwrap();
    let h = g.players[0].hand.len() as u8 - 1;
    play(&mut g, 0, h, 0);
    // nothing happens while the choice is open
    assert_eq!(g.players[0].board[1].gem_attack, 0);
    act(&mut g, 0, Action::Choose(0));
    // Gem Day's +1 Attack lands first, so the 2 gems are +2/+1 each
    assert_eq!(g.players[0].board[1].gem_attack, 4, "2 gems on the Quilboar");
    assert_eq!(g.players[0].board[1].gem_health, 2);
    assert_eq!(g.players[0].board[2].gem_attack, 0, "a Beast gets none");
    // the Hogrider itself is a Quilboar and counts
    assert_eq!(g.players[0].board[0].gem_attack, 4);
    g.check_invariants().unwrap();
}

#[test]
fn turbo_hogrider_waits_until_the_choice_is_made_so_the_played_quilboar_gets_gems() {
    let mut g = game(26);
    put_on_board(&mut g, 0, "Turbo Hogrider", false);
    put_in_hand(&mut g, 0, "Sly Infiltrator", false);
    play(&mut g, 0, 0, 1);
    assert!(g.players[0].pending_choose_one.is_some());
    assert_eq!(g.players[0].board[1].gem_attack, 0, "not yet: the choice is open");
    act(&mut g, 0, Action::Choose(0)); // the free Refreshes
    assert_eq!(name(&g, &g.players[0].board[1]), "Sly Infiltrator");
    assert_eq!(g.players[0].board[1].gem_attack, 2, "the played Quilboar gets the 2 gems");
    assert_eq!(g.players[0].board[0].gem_attack, 2);
    g.check_invariants().unwrap();
}

// ================= Triple Reward as a spell =================

#[test]
fn the_triple_reward_tier_is_fixed_when_it_is_given() {
    let mut g = game(27);
    g.players[0].tier = 3;
    put_in_hand(&mut g, 0, "Tusked Camper", true);
    play(&mut g, 0, 0, 0);
    let reward = &g.players[0].hand[0];
    assert_eq!(name(&g, reward), "Triple Reward");
    assert_eq!(reward.improvements, 4, "tier + 1 when it was given");
    // leveling before casting changes nothing
    g.players[0].tier = 6;
    play(&mut g, 0, 0, 0);
    assert_eq!(g.players[0].discover.len(), 3);
    assert!(g.players[0].discover.iter().all(|&c| g.data.cards.get(c).tier == 4));
    act(&mut g, 0, Action::Choose(0));
    let picked = g.players[0].hand.last().unwrap();
    assert_eq!(g.data.cards.get(picked.card).tier, 4);
    g.check_invariants().unwrap();
}

#[test]
fn the_reward_comes_before_battlecry_cards_and_is_lost_on_a_full_hand() {
    // Razorfen Flapper's Battlecry adds a Blood Gem Barrage to the hand
    let mut g = game(28);
    put_in_hand(&mut g, 0, "Razorfen Flapper", true);
    play(&mut g, 0, 0, 0);
    let names: Vec<String> = g.players[0].hand.iter().map(|m| name(&g, m)).collect();
    assert_eq!(names[0], "Triple Reward", "the reward is first");
    assert_eq!(names[1], "Blood Gem Barrage");

    // a full hand loses it (one slot freed by the play goes to the reward)
    let mut g = game(29);
    put_in_hand(&mut g, 0, "Razorfen Flapper", true);
    while g.players[0].hand.len() < 10 {
        g.give(0, "Fortify").unwrap();
    }
    play(&mut g, 0, 0, 0);
    assert!(g.players[0].hand.iter().any(|m| name(&g, m) == "Triple Reward"));
    assert!(g.players[0].hand.iter().all(|m| name(&g, m) != "Blood Gem Barrage"));
    g.check_invariants().unwrap();
}

// ================= Max gold, end of turn, misc cards =================

#[test]
fn snare_trapper_raises_the_maximum_gold_past_ten() {
    let mut g = game(30);
    put_in_hand(&mut g, 0, "Snare Trapper", false);
    play(&mut g, 0, 0, 0);
    act(&mut g, 0, Action::Choose(1));
    assert_eq!(g.players[0].extra_max_gold, 1);
    // Pin both trinket slots with cards that do nothing on their own, so the
    // turn 6 and 9 offers do not change the Gold this test counts.
    let lesser = g.data.trinkets.by_name("Explorer's Binoculars").unwrap();
    let greater = g.data.trinkets.by_name("Artisanal Urn").unwrap();
    g.set_trinket(0, 0, Some(lesser));
    g.set_trinket(0, 1, Some(greater));
    // reach a late turn: the cap is the turn gold plus the extra
    while g.turn < 15 {
        end_all_turns(&mut g);
    }
    assert_eq!(g.players[0].gold, 11, "10 turn gold plus 1");

    let mut g = game(31);
    put_in_hand(&mut g, 0, "Snare Trapper", true);
    play(&mut g, 0, 0, 0);
    act(&mut g, 0, Action::Choose(1));
    assert_eq!(g.players[0].extra_max_gold, 2, "golden: 2");
    g.check_invariants().unwrap();
}

#[test]
fn gem_rat_and_trench_fighter_get_their_spells_at_end_of_turn() {
    let mut g = game(32);
    put_on_board(&mut g, 0, "Gem Rat", false);
    put_on_board(&mut g, 0, "Trench Fighter", false);
    end_all_turns(&mut g);
    let names: Vec<String> = g.players[0].hand.iter().map(|m| name(&g, m)).collect();
    assert!(names.contains(&"Gem Day".to_string()));
    assert!(names.contains(&"Gem Confiscation".to_string()));
    assert_eq!(g.players[0].gem_attack, 1, "nothing else fired");
    g.check_invariants().unwrap();
}

#[test]
fn sly_infiltrator_and_roadboar_and_bramble_tunneler() {
    let mut g = game(33);
    put_in_hand(&mut g, 0, "Sly Infiltrator", false);
    play(&mut g, 0, 0, 0);
    act(&mut g, 0, Action::Choose(0));
    assert_eq!(g.players[0].free_rolls, 2);

    // Roadboar's Rally gets a Blood Gem into the hand, in a fight
    let dbc = db();
    let out = fight_with(
        &dbc,
        &[mk(&dbc, "Roadboar", false)],
        &[wall(&dbc)],
        [Counters::default(); 2],
        1,
        &mut Rng::new(1),
        &mut (),
    );
    assert_eq!(out.sides[0].cards_gained(), &[id(&dbc, "Blood Gem")]);

    // Bramble Tunneler's Rally gets a random Choose One card up to the tier
    let out = fight_with(
        &dbc,
        &[mk(&dbc, "Bramble Tunneler", false)],
        &[wall(&dbc)],
        [Counters { tier: 6, ..Counters::default() }, Counters::default()],
        1,
        &mut Rng::new(2),
        &mut (),
    );
    let got = out.sides[0].cards_gained();
    assert_eq!(got.len(), 1);
    let c = dbc.get(got[0]);
    assert!(c.choose_one.is_some(), "a Choose One card: {}", c.name);
    assert!(c.tier <= 6);
}

#[test]
fn victorious_geomant_overflows_gems_onto_the_leftmost_minion() {
    let mut g = game(34);
    put_on_board(&mut g, 0, "Tusked Camper", false);
    put_on_board(&mut g, 0, "Victorious Geomant", false);
    g.players[0].gold = 10;
    while g.players[0].hand.len() < 8 {
        g.give(0, "Fortify").unwrap();
    }
    let bandit = g.players[0].board.iter().position(|m| name(&g, m) == "Victorious Geomant").unwrap();
    act(&mut g, 0, Action::Activate(bandit as u8));
    // 6 gems: 2 fit in hand, 4 are cast on the left-most minion
    assert_eq!(gems_in_hand(&g, 0), 2);
    assert_eq!(g.players[0].board[0].gem_attack, 4, "the left-most, not the Geomant");
    assert_eq!(g.players[0].board[1].gem_attack, 0);
    g.check_invariants().unwrap();
}

// ================= Gems in fights =================

#[test]
fn rally_gems_in_a_fight_use_the_gem_value_and_refiner_raises_it_for_good() {
    let dbc = db();
    // Tusked Camper's Rally plays a gem on itself, at the side's gem value
    let out = fight_with(
        &dbc,
        &[mk(&dbc, "Tusked Camper", false)],
        &[wall(&dbc)],
        [Counters { tier: 6, ..Counters::default() }, Counters::default()],
        1,
        &mut Rng::new(3),
        &mut (),
    );
    assert_eq!(out.sides[0].permanent_stats[0], (0, 0), "fight gems are temporary");

    // Sanguine Refiner's Rally raises the value inside the fight, and the
    // gain is for good (it leaves through the outcome)
    let out = fight_with(
        &dbc,
        &[mk(&dbc, "Sanguine Refiner", false), mk(&dbc, "Tusked Camper", false)],
        &[wall(&dbc), wall(&dbc)],
        [Counters::default(); 2],
        1,
        &mut Rng::new(4),
        &mut (),
    );
    assert_eq!(out.sides[0].gem_gain, (1, 2), "one Rally");
}

#[test]
fn vineweaver_gems_are_permanent_and_carry_out_of_the_fight() {
    let dbc = db();
    let out = fight_with(
        &dbc,
        &[mk(&dbc, "Razorfen Vineweaver", false)],
        &[wall(&dbc)],
        [Counters { gem_attack: 2, gem_health: 1, ..Counters::default() }, Counters::default()],
        1,
        &mut Rng::new(5),
        &mut (),
    );
    // 4 permanent gems at +2/+1: +8/+4, kept after the fight as stats and
    // as Blood Gem stats
    assert_eq!(out.sides[0].permanent_stats[0], (8, 4));
    assert_eq!(out.sides[0].permanent_gems[0], (8, 4));
}

#[test]
fn tusker_gems_the_attacker_and_bonker_gems_all_others() {
    let dbc = db();
    // Prodigious Tusker: whenever ANOTHER friendly minion attacks. The
    // Tusker itself never attacks here (0 attack), so its own slot never
    // gains anything and nothing is permanent.
    let mut camper = mk(&dbc, "Tusked Camper", false);
    camper.attack = 3;
    let mut tusker = mk(&dbc, "Prodigious Tusker", false);
    tusker.attack = 0;
    let out = fight_with(
        &dbc,
        &[camper, tusker],
        &[wall(&dbc)],
        [Counters::default(); 2],
        1,
        &mut Rng::new(6),
        &mut (),
    );
    assert_eq!(out.sides[0].permanent_stats[1], (0, 0), "the Tusker only reacts");

    // Bonker's Rally gems every OTHER friendly minion: the Bonker swings
    // first (left-most), its Rally gems the Camper, and the Camper's own
    // swing right after carries the gem.
    struct Saw(Vec<(String, i32)>);
    impl bgcore::fight::Watch for Saw {
        fn attack(&mut self, _side: usize, attacker: &Minion, _defender: &Minion) {
            let db2 = db();
            self.0.push((db2.get(attacker.card).name.clone(), attacker.attack));
        }
    }
    let mut w = Saw(Vec::new());
    let mut bonker = mk(&dbc, "Bonker", false);
    bonker.attack = 2;
    let mut camper = mk(&dbc, "Tusked Camper", false);
    camper.attack = 2;
    let _ = fight_with(
        &dbc,
        &[bonker, camper],
        &[wall(&dbc)],
        [Counters::default(); 2],
        2,
        &mut Rng::new(7),
        &mut w,
    );
    // Windfury: the Bonker swings twice (no gem on itself either time), then
    // the Camper swings carrying Bonker's Rally gem (+1) and its own (+1)
    assert_eq!(w.0[0], ("Bonker".to_string(), 2), "no gem on itself");
    assert_eq!(w.0[1], ("Bonker".to_string(), 2), "still none on the second swing");
    // 2 base + one Rally gem per Windfury swing of Bonker (2) + its own
    assert_eq!(w.0[2], ("Tusked Camper".to_string(), 5), "two of Bonker's gems plus its own Rally");
}

#[test]
fn tusked_camper_and_bonker_gem_themselves_and_others_in_a_fight() {
    // A direct look at the fight board through the Watch: the Camper attacks
    // and shows its gem in its health when it next swings.
    let dbc = db();
    struct Saw(Vec<(String, i32, i32)>);
    impl bgcore::fight::Watch for Saw {
        fn attack(&mut self, _side: usize, attacker: &Minion, _defender: &Minion) {
            let db2 = db();
            self.0.push((
                db2.get(attacker.card).name.clone(),
                attacker.attack,
                attacker.health,
            ));
        }
    }
    let mut w = Saw(Vec::new());
    let mut camper = mk(&dbc, "Tusked Camper", false);
    camper.attack = 3;
    fight_with(
        &dbc,
        &[camper, mk(&dbc, "Prodigious Tusker", false)],
        &[wall(&dbc)],
        [Counters::default(); 2],
        2,
        &mut Rng::new(8),
        &mut w,
    );
    // the first swing is the Camper at 3 attack; after it (Rally gem +1/+1
    // and the Tusker's reaction gem +1/+1) its later swings show 5 attack
    // the Camper swung once, already carrying its Rally gem (+1/+1) and the
    // Tusker's reaction gem (+1/+1): 3+2 attack, 3+2 health. The Tusker
    // swung at its base stats: nothing gems the attacker for its own attack.
    assert_eq!(w.0.len(), 2, "{:?}", w.0);
    assert_eq!(w.0[0], ("Tusked Camper".to_string(), 5, 5));
    assert_eq!(w.0[1], ("Prodigious Tusker".to_string(), 2, 5));
}

// ================= Jailbird Juggernaut =================

#[test]
fn the_golem_attacks_first_with_the_juggernauts_stats_and_stays() {
    let dbc = db();
    let mut w = SawAttacks(Vec::new());
    let out = fight_with(
        &dbc,
        &[mk(&dbc, "Jailbird Juggernaut", false)],
        &[wall(&dbc)],
        [Counters::default(); 2],
        1,
        &mut Rng::new(9),
        &mut w,
    );
    // the Golem (6/15, the Juggernaut's stats) swung first
    assert_eq!(w.0[0], ("Golem".to_string(), 6, 15));
    assert!(w.0.iter().any(|(n, _, _)| n == "Jailbird Juggernaut"));
    // both survive: the wall has 0 attack
    assert_eq!(out.sides[0].permanent_stats[0], (0, 0));
    // the wall took both hits
    assert_eq!(w.0.len(), 2);
}

struct SawAttacks(Vec<(String, i32, i32)>);

impl bgcore::fight::Watch for SawAttacks {
    fn attack(&mut self, _side: usize, attacker: &Minion, _defender: &Minion) {
        let db2 = db();
        self.0.push((db2.get(attacker.card).name.clone(), attacker.attack, attacker.health));
    }
}

#[test]
fn a_golem_kill_fizzles_the_juggernauts_swing() {
    let dbc = db();
    let mut w = SawAttacks(Vec::new());
    let mut pup = mk(&dbc, "Wolf Pup", false);
    pup.health = 1;
    let out = fight_with(
        &dbc,
        &[mk(&dbc, "Jailbird Juggernaut", false)],
        &[pup],
        [Counters::default(); 2],
        1,
        &mut Rng::new(10),
        &mut w,
    );
    // only the Golem swung: the pup died to it, so the Juggernaut's own
    // attack fizzled and it took nothing back
    assert_eq!(w.0.len(), 1);
    assert_eq!(w.0[0].0, "Golem");
    assert_eq!(out.winner, Some(0));
}

#[test]
fn a_golden_juggernaut_makes_a_double_stat_golem_and_a_full_board_makes_none() {
    let dbc = db();
    let mut w = SawAttacks(Vec::new());
    fight_with(
        &dbc,
        &[mk(&dbc, "Jailbird Juggernaut", true)],
        &[wall(&dbc)],
        [Counters::default(); 2],
        1,
        &mut Rng::new(11),
        &mut w,
    );
    assert_eq!(w.0[0], ("Golem".to_string(), 24, 60), "double the golden stats");

    // a full board: no Golem, the Juggernaut attacks normally
    let mut w = SawAttacks(Vec::new());
    fight_with(
        &dbc,
        &[mk(&dbc, "Jailbird Juggernaut", false), wall(&dbc), wall(&dbc), wall(&dbc), wall(&dbc), wall(&dbc), wall(&dbc)],
        &[wall(&dbc)],
        [Counters::default(); 2],
        1,
        &mut Rng::new(12),
        &mut w,
    );
    assert_eq!(w.0.len(), 1, "just the Juggernaut's own swing: {:?}", w.0);
    assert_eq!(w.0[0].0, "Jailbird Juggernaut");
}

// ================= The tribe mask =================

#[test]
fn the_tribe_mask_is_sixteen_bits_wide_with_quilboar_ninth() {
    for t in Tribe::EVERY {
        assert!(Tribes::ALL.has(t), "{t:?} missing from the all-tribes mask");
    }
    assert_eq!(Tribe::EVERY.len(), 10);
    let dbc = db();
    // Felboar is Demon and Quilboar in the game
    let felboar = dbc.get(id(&dbc, "Felboar"));
    assert!(felboar.tribes.has(Tribe::Demon));
    assert!(felboar.tribes.has(Tribe::Quilboar));
    assert!(!felboar.tribes.has(Tribe::Beast));
    // an all-type minion is a member of the new tribe too
    let all = Tribes::ALL;
    assert!(all.has(Tribe::Quilboar));
    // the parser knows "quilboar" and rejects an unknown tribe
    let ok = CardDb::from_toml(
        "[[minion]]\nname = \"Q\"\ntier = 1\ntribe = \"quilboar\"\nstats = [1, 1]\n",
    )
    .unwrap();
    assert!(ok.get(0).tribes.has(Tribe::Quilboar));
    assert!(CardDb::from_toml(
        "[[minion]]\nname = \"Q\"\ntier = 1\ntribe = \"ogre\"\nstats = [1, 1]\n"
    )
    .is_err());
    // a dual-tribe parse
    let ok = CardDb::from_toml(
        "[[minion]]\nname = \"Q\"\ntier = 1\ntribes = [\"demon\", \"quilboar\"]\nstats = [1, 1]\n",
    )
    .unwrap();
    assert!(ok.get(0).tribes.has(Tribe::Demon) && ok.get(0).tribes.has(Tribe::Quilboar));
}

#[test]
fn every_quilboar_card_is_in_the_data_with_its_real_numbers() {
    let dbc = db();
    let expect: &[(&str, u8, i32, i32)] = &[
        ("Razorfen Geomancer", 1, 2, 1),
        ("Tusked Camper", 1, 2, 3),
        ("Crater Miner", 2, 2, 2),
        ("Prodigious Tusker", 2, 2, 5),
        ("Roadboar", 2, 2, 4),
        ("Fearless Foodie", 3, 2, 4),
        ("Gem Rat", 3, 4, 4),
        ("Mangled Bandit", 3, 3, 3),
        ("Sly Infiltrator", 3, 4, 5),
        ("Thorned Trailblazer", 3, 4, 5),
        ("Trench Fighter", 3, 3, 3),
        ("Bonker", 4, 2, 7),
        ("Bramble Tunneler", 4, 3, 6),
        ("Geomagus Roogug", 4, 4, 6),
        ("Hot-Air Surveyor", 4, 3, 7),
        ("Razorfen Flapper", 4, 5, 3),
        ("Snare Trapper", 4, 4, 4),
        ("Razorfen Vineweaver", 5, 4, 4),
        ("Sanguine Refiner", 5, 3, 8),
        ("Sanguine Champion", 6, 9, 4),
        ("Turbo Hogrider", 6, 6, 8),
        ("Veteran Brigand", 6, 8, 8),
        ("Victorious Geomant", 6, 10, 10),
        ("Jailbird Juggernaut", 7, 6, 15),
    ];
    for &(n, tier, a, h) in expect {
        let c = dbc.get(id(&dbc, n));
        assert_eq!(c.tier, tier, "{n}");
        assert_eq!((c.attack, c.health), (a, h), "{n}");
        assert!(c.tribes.has(Tribe::Quilboar), "{n}");
        assert!(!c.token, "{n} is a pool minion");
    }
    // the pool spells and the generated ones
    for (n, tier, cost, tavern) in [
        ("Blood Gem Barrage", 4, 1, true),
        ("Gem Confiscation", 4, 1, true),
        ("Fandral's Fortune", 6, 3, true),
        ("Blood Gem", 0, 0, false),
        ("Gem Day", 0, 0, false),
        ("Triple Reward", 0, 0, false),
    ] {
        let c = dbc.get(id(&dbc, n));
        assert!(c.is_spell(), "{n}");
        assert_eq!(c.tier, tier, "{n}");
        assert_eq!(c.cost, cost, "{n}");
        assert_eq!(c.tavern_spell, tavern, "{n}");
    }
    // the Golem token: neutral tier 1, no keywords
    let golem = dbc.get(id(&dbc, "Golem"));
    assert!(golem.token);
    assert_eq!(golem.tier, 1);
    assert!(golem.tribes.is_neutral());
    assert_eq!(golem.keywords.0, 0);
    // the Blood Gem variants stay out
    assert!(dbc.by_name("Divine Shield").is_none());
}

#[test]
fn a_gem_gets_no_spell_power_and_a_golden_roogug_plays_two() {
    let mut g = game(35);
    // spell power +1/+0 on board
    put_on_board(&mut g, 0, "Intrepid Botanist", false);
    put_on_board(&mut g, 0, "Tusked Camper", false);
    g.give(0, "Blood Gem").unwrap();
    let before = stats(&g.players[0].board[1]);
    cast_gem(&mut g, 0, 1);
    assert_eq!(stats(&g.players[0].board[1]), (before.0 + 1, before.1 + 1),
        "the gem value only, no Tavern spell bonus");

    // a golden Roogug plays two gems for one from hand
    let mut g = game(36);
    put_on_board(&mut g, 0, "Geomagus Roogug", true);
    put_on_board(&mut g, 0, "Tusked Camper", false);
    g.give(0, "Blood Gem").unwrap();
    cast_gem(&mut g, 0, 0);
    assert_eq!(g.players[0].board[0].gem_attack, 1, "the cast gem");
    assert_eq!(g.players[0].board[1].gem_attack, 2, "two from the golden Roogug");
    g.check_invariants().unwrap();
}

#[test]
fn gem_confiscation_on_a_tavern_minion_steals_from_its_tavern_neighbours() {
    let mut g = game(37);
    set_shop(&mut g, 0, &["Wolf Pup", "Cord Puller", "Lullabot"]);
    // two gems on the left neighbour
    g.players[0].shop[0].attack += 2;
    g.players[0].shop[0].health += 2;
    g.players[0].shop[0].gem_attack = 2;
    g.players[0].shop[0].gem_health = 2;
    let mid = stats(&g.players[0].shop[1]);
    g.give(0, "Gem Confiscation").unwrap();
    let h = g.players[0].hand.len() as u8 - 1;
    play(&mut g, 0, h, 0);
    act(&mut g, 0, Action::TargetShop(1));
    // 3 gems on the Cord Puller plus the Wolf Pup's 2/2 of gems; the Microbot
    // (right neighbour) had none
    assert_eq!(stats(&g.players[0].shop[1]), (mid.0 + 3 + 2, mid.1 + 3 + 2));
    assert_eq!(g.players[0].shop[1].gem_attack, 5);
    assert_eq!(g.players[0].shop[0].gem_attack, 0);
    let pup_base = stats(&g.players[0].shop[0]);
    let pup = g.data.cards.get(id(&g.data.cards, "Wolf Pup"));
    assert_eq!((pup_base.0, pup_base.1), (pup.attack, pup.health), "back to its card stats");
    g.check_invariants().unwrap();
}

#[test]
fn sanguine_champion_deathrattle_raises_the_gem_value_for_good() {
    let dbc = db();
    let mut champ = mk(&dbc, "Sanguine Champion", false);
    champ.health = 1;
    champ.keywords.set(bgcore::Keywords::TAUNT);
    let mut killer = wall(&dbc);
    killer.attack = 50;
    let out = fight_with(
        &dbc,
        &[champ],
        &[killer],
        [Counters::default(); 2],
        1,
        &mut Rng::new(12),
        &mut (),
    );
    assert_eq!(out.sides[0].gem_gain, (2, 1), "the Deathrattle, for good");
}

#[test]
fn razorfen_flappers_deathrattle_gets_a_barrage_spell() {
    let dbc = db();
    let mut flapper = mk(&dbc, "Razorfen Flapper", false);
    flapper.health = 1;
    flapper.keywords.set(bgcore::Keywords::TAUNT);
    let mut killer = wall(&dbc);
    killer.attack = 50;
    let out = fight_with(
        &dbc,
        &[flapper],
        &[killer],
        [Counters::default(); 2],
        1,
        &mut Rng::new(13),
        &mut (),
    );
    assert!(out.sides[0]
        .cards_gained()
        .iter()
        .any(|&c| dbc.get(c).name == "Blood Gem Barrage"));
}

// ================= Victorious Geomant's leftover Gems =================

/// Victorious Geomant's Activate hands Gems over and casts the ones that do
/// not fit on the left-most minion. Each of those is a real cast, so the
/// minion it lands on answers with its own "whenever you cast a spell on
/// this": the log's golden Geomant casts four Gems on a Gatekeeper Amalgam and
/// the Amalgam's four Misplaced Tea Sets run
/// (`Hearthstone_2026_10_24_17_59_26-g1` turn 12 step 40, where the Amalgam
/// gains +76/+124 and not the Gems' +56/+104; triage group E10). A gem the
/// Activate casts is not one played from the hand, so a Hot-Air Surveyor adds
/// no cast to it.
#[test]
fn a_geomants_leftover_gems_are_casts_the_target_sees() {
    let mut g = game(21);
    put_on_board(&mut g, 0, "Gatekeeper Amalgam", false);
    put_on_board(&mut g, 0, "Victorious Geomant", false);
    // The hand is full, so every Gem goes on the left-most minion.
    for _ in 0..10 {
        g.give(0, "Blood Gem").unwrap();
    }
    let before = stats(&g.players[0].board[0]);
    act(&mut g, 0, Action::Activate(1));
    let now = stats(&g.players[0].board[0]);
    // Six Gems of +1/+1, and a Misplaced Tea Set for each of them: the Amalgam
    // is every type and the only minion, so it takes one +4/+4 a cast.
    assert_eq!(
        (now.0 - before.0, now.1 - before.1),
        (6 + 6 * 4, 6 + 6 * 4),
        "six Gems and their six Tea Sets"
    );
    g.check_invariants().unwrap();
}

/// The same cast with a golden Hot-Air Surveyor up: the extra cast is for
/// Blood Gems played from the hand, so the Activate's leftover Gems still go
/// once each.
#[test]
fn a_surveyor_adds_no_cast_to_a_geomants_leftover_gems() {
    let mut g = game(22);
    put_on_board(&mut g, 0, "Gatekeeper Amalgam", false);
    put_on_board(&mut g, 0, "Hot-Air Surveyor", true);
    put_on_board(&mut g, 0, "Victorious Geomant", false);
    for _ in 0..10 {
        g.give(0, "Blood Gem").unwrap();
    }
    let before = stats(&g.players[0].board[0]);
    act(&mut g, 0, Action::Activate(2));
    let now = stats(&g.players[0].board[0]);
    // Six Gems, each with its own Tea Set (the Surveyor's three casts are for
    // the hand and do not reach them).
    assert_eq!(
        (now.0 - before.0, now.1 - before.1),
        (6 + 6 * 4, 6 + 6 * 4),
        "the Surveyor's extra casts are for Gems played from the hand"
    );
    g.check_invariants().unwrap();
}

/// Balinda Stonehearth's "Your spells that target friendly minions cast
/// twice" reaches the leftover Gem, which does target a friendly minion. The
/// log's plain Geomant with one Gem more than the hand holds puts it on the
/// left-most minion twice: +2/+4 on a golden Bramble Tunneler whose Gems are
/// worth +1/+2 (`Hearthstone_2026_11_03_15_22_36-g4` turn 12 step 8), and the
/// `Hearthstone_2026_10_24_17_59_26-g1` turn 13 step 28 Gatekeeper Amalgam
/// takes +114/+186 where its three leftover Gems alone give +57/+93. A golden
/// Balinda casts it three times.
#[test]
fn a_balinda_casts_a_geomants_leftover_gems_twice() {
    for (golden_balinda, casts) in [(false, 2), (true, 3)] {
        let mut g = game(23);
        put_on_board(&mut g, 0, "Bramble Tunneler", false);
        put_on_board(&mut g, 0, "Balinda Stonehearth", golden_balinda);
        put_on_board(&mut g, 0, "Victorious Geomant", false);
        // The hand is full, so every one of the six Gems is cast on the
        // left-most minion.
        for _ in 0..10 {
            g.give(0, "Blood Gem").unwrap();
        }
        let before = stats(&g.players[0].board[0]);
        act(&mut g, 0, Action::Activate(2));
        let now = stats(&g.players[0].board[0]);
        // The Gems are +1/+1: six of them, each cast `casts` times.
        assert_eq!(
            (now.0 - before.0, now.1 - before.1),
            (6 * casts, 6 * casts),
            "Balinda casts each leftover Gem {casts} times"
        );
        g.check_invariants().unwrap();
    }
}
