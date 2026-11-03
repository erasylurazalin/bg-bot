//! Repeats: what a repeated Battlecry or spell does. A repeat still counts as
//! a trigger or a cast, but making a minion Golden happens once; a repeated
//! Choose One takes the option the player picked on the target the player
//! picked; a repeat that asks for a target lands on the one the first run
//! picked; a spell a minion casts gets no Choose One choice (its option is
//! random) but a Discover it opens is a normal Discover the player answers,
//! Hooktusk included; and a Discover a Battlecry an Activate triggers opens
//! gives a random card with no Discover offered, which Hooktusk does not see
//! (docs/rules/design.md "Set versus add" and "Shop replay answers", Era
//! 2026-11-04 and 2026-11-03).

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

fn stats_shop(g: &Game, p: usize, slot: usize) -> (i32, i32) {
    let m = &g.players[p].shop[slot];
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

/// The Magicfin token in hand, taught `spell`, ready to be played.
fn taught_token(g: &mut Game, spell: &str) -> u8 {
    let card = id(&g.data.cards, spell);
    // A token has no pool copy to take.
    g.give(0, "Magicfin Apprentice").expect("a taught token");
    let h = g.players[0].hand.len() - 1;
    g.players[0].hand[h].taught_spell = Some(card);
    h as u8
}

#[test]
fn a_taught_choose_one_spell_gets_no_option_and_a_random_one_lands_on_the_target() {
    // A Magicfin token casts the Choose One spell it was taught: its option is
    // picked at random with no prompt, and the target the spell needs is still
    // the player's pick (docs/rules/design.md "Set versus add", Era
    // 2026-11-04).
    let mut g = game(7004);
    put_on_board(&mut g, 0, "Southsea Busker", false);
    put_on_board(&mut g, 0, "Bigwig Bandit", false);
    let token = taught_token(&mut g, "Alliance Flag");
    g.players[0].gold = 10;
    let at = g.players[0].board.len() as u8;
    play(&mut g, 0, token, at);
    assert!(g.players[0].pending_choose_one.is_none(), "no option is offered");
    assert!(g.players[0].pending_target.is_some(), "the target still waits for the player");
    let busy = stats(&g, 0, 0);
    let bandit = stats(&g, 0, 1);
    act(&mut g, 0, Action::Target(0));
    assert!(g.players[0].pending_target.is_none(), "one cast, one pick");
    // Alliance Flag's options are +3/+1 and +1/+3; either may come up.
    let after = stats(&g, 0, 0);
    let gained = (after.0 - busy.0, after.1 - busy.1);
    assert!([(3, 1), (1, 3)].contains(&gained), "one random option ran: {gained:?}");
    assert_eq!(stats(&g, 0, 1), bandit, "nothing else was touched");
    assert_eq!(g.players[0].spells_cast, 0, "a minion's cast is not one of yours");
    g.check_invariants().unwrap();
}

#[test]
fn brann_casts_a_taught_choose_one_spell_twice_each_with_its_own_random_option() {
    // Brann doubles the token's Battlecry, so the taught Choose One casts
    // twice: each cast picks its own random option, both on the one target the
    // player picked, and neither asks anything of the player (docs/rules/
    // design.md "Set versus add", Era 2026-11-04).
    let mut g = game(7008);
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
    assert!(g.players[0].pending_choose_one.is_none(), "no option is offered");
    assert!(g.players[0].pending_target.is_some(), "and its target once");
    let before: Vec<(i32, i32)> =
        (0..g.players[0].board.len()).map(|s| stats(&g, 0, s)).collect();
    let busker = g.players[0]
        .board
        .iter()
        .position(|m| g.data.cards.get(m.card).name == "Southsea Busker")
        .unwrap();
    act(&mut g, 0, Action::Target(busker as u8));
    assert!(g.players[0].pending_choose_one.is_none(), "the second cast picks at random too");
    assert!(g.players[0].pending_target.is_none(), "and does not ask for a target again");
    // +3/+1 and +1/+3 in either order: two casts give 8 stats in all, and only
    // on the minion the player picked.
    let after: Vec<(i32, i32)> =
        (0..g.players[0].board.len()).map(|s| stats(&g, 0, s)).collect();
    for (s, (a, h)) in after.iter().enumerate() {
        let d = (a - before[s].0, h - before[s].1);
        if d != (0, 0) {
            assert_eq!(s, busker, "only the minion the player picked was touched");
            assert_eq!(d.0 + d.1, 8, "both casts landed on it: {d:?}");
        }
    }
    assert_eq!(g.players[0].spells_cast, 0, "a minion's cast is not one of yours");
    g.check_invariants().unwrap();
}

#[test]
fn balinda_casts_a_choose_one_spell_twice_with_the_one_option_and_target() {
    // A repeated Choose One remembers the choice: Balinda casts the spell
    // twice with the same option on the same target (docs/rules/design.md "Set
    // versus add", Era 2026-11-04; log Hearthstone_2026_11_02_00_10_22-g4 turn
    // 12 step 0).
    let mut g = game(7009);
    put_on_board(&mut g, 0, "Balinda Stonehearth", false);
    put_on_board(&mut g, 0, "Southsea Busker", false);
    put_on_board(&mut g, 0, "Bigwig Bandit", false);
    g.players[0].gold = 20;
    g.give(0, "Alliance Flag").unwrap();
    let hand = g.players[0].hand.len() as u8 - 1;
    let before: Vec<(i32, i32)> =
        (0..g.players[0].board.len()).map(|s| stats(&g, 0, s)).collect();
    play(&mut g, 0, hand, 0);
    assert!(g.players[0].pending_choose_one.is_some(), "the player picks the option");
    act(&mut g, 0, Action::Choose(1)); // +1/+3
    assert!(g.players[0].pending_target.is_some(), "and its target, once");
    act(&mut g, 0, Action::Target(1));
    assert!(g.players[0].pending_choose_one.is_none(), "the second cast asks nothing");
    assert!(g.players[0].pending_target.is_none(), "and needs no other target");
    assert_eq!(
        (stats(&g, 0, 1).0 - before[1].0, stats(&g, 0, 1).1 - before[1].1),
        (2, 6),
        "the option the player picked, twice, on the one minion"
    );
    assert_eq!(stats(&g, 0, 2), before[2], "nothing else changed");
    g.check_invariants().unwrap();
}

// ---- a repeated Choose One minion ----

#[test]
fn a_golden_kelp_keeper_repeats_a_choose_one_minion_with_the_same_option_and_target() {
    // A golden Kelp Keeper triggers a Choose One minion's choice twice: the
    // player picks the option and the target once, and the repeat uses the
    // SAME option on the SAME target (docs/rules/design.md "Set versus add",
    // Era 2026-11-04). Sprightly Scarab's options are +1/+1 with Reborn or
    // +4/+0 with Windfury, both on a Tavern Beast.
    let mut g = game(7007);
    put_on_board(&mut g, 0, "Kelp Keeper", true);
    put_on_board(&mut g, 0, "Sprightly Scarab", false);
    // Two Beasts in the Tavern, so the target is a real pick.
    set_shop(&mut g, 0, &["Flittering Bat", "Flittering Bat"]);
    g.players[0].gold = 10;
    act(&mut g, 0, Action::Activate(0));
    // The Activate picks the minion whose Battlecry to trigger: the Scarab is
    // the only one with a Battlecry or a Choose One.
    if g.players[0].pending_target.is_some() {
        act(&mut g, 0, Action::Target(1));
    }
    assert!(g.players[0].pending_choose_one.is_some(), "the player picks the option");
    act(&mut g, 0, Action::Choose(1)); // +4/+0 with Windfury
    assert!(g.players[0].pending_target.is_some(), "and the Tavern Beast once");
    let beast = {
        let m = g.players[0].shop[1];
        (m.attack, m.health)
    };
    let other = {
        let m = g.players[0].shop[0];
        (m.attack, m.health)
    };
    act(&mut g, 0, Action::TargetShop(1));
    assert!(g.players[0].pending_choose_one.is_none(), "the repeat asks no option");
    assert!(g.players[0].pending_target.is_none(), "and no target");
    let shop_after = g.players[0].shop[1];
    let gained = (shop_after.attack - beast.0, shop_after.health - beast.1);
    assert_eq!(gained, (8, 0), "the same option twice, on the one Beast picked");
    assert_eq!(stats_shop(&g, 0, 0), other, "the other Beast is untouched");
    g.check_invariants().unwrap();
}

// ---- a Discover a spell a minion casts opens ----

#[test]
fn a_taught_discover_spell_offers_its_discover_and_hooktusk_fires() {
    // A Magicfin token casts the Discover spell it was taught: the player
    // answers that Discover like any other and it counts for "after you
    // Discover" cards (log Hearthstone_2026_10_28_02_28_32 at 02:41, where
    // the token's Boundless Potential offers three Tavern spells; docs/rules/
    // design.md "Set versus add", Era 2026-11-04). Hooktusk gives the other
    // Pirates +1/+1 for each golden minion played this game.
    let mut g = game(7010);
    put_on_board(&mut g, 0, "Hooktusk, Master Marauder", false);
    put_on_board(&mut g, 0, "Southsea Busker", false);
    g.players[0].golden_played = 1;
    let token = taught_token(&mut g, "A New Sprout");
    g.players[0].gold = 10;
    let at = g.players[0].board.len() as u8;
    let busker = stats(&g, 0, 1);
    let hand = g.players[0].hand.len();
    play(&mut g, 0, token, at);
    assert!(!g.players[0].discover.is_empty(), "a normal Discover is offered");
    assert_eq!(g.players[0].repeat_discover, 0, "no repeat's Discover is waiting here");
    act(&mut g, 0, Action::Choose(0));
    assert!(g.players[0].discover.is_empty(), "the pick landed");
    assert_eq!(g.players[0].hand.len(), hand, "the token left and one card arrived");
    assert_eq!(
        stats(&g, 0, 1),
        (busker.0 + 2, busker.1 + 2),
        "Hooktusk fired for the pick (+1/+1 for the one golden minion played)"
    );
    g.check_invariants().unwrap();
}

#[test]
fn brann_sets_off_two_discovers_from_a_taught_spell() {
    // Brann's two Battlecry triggers are two separate casts, each opening its
    // own Discover for the player to answer (docs/rules/design.md "Set versus
    // add", Era 2026-11-04).
    let mut g = game(7011);
    put_on_board(&mut g, 0, "Brann Bronzebeard", false);
    put_on_board(&mut g, 0, "Magicfin Mycologist", false);
    set_shop(&mut g, 0, &["Bubble Gunner"]);
    g.players[0].shop_spell = Some(id(&g.data.cards, "A New Sprout"));
    g.players[0].gold = 20;
    act(&mut g, 0, Action::BuySpell);
    let token = g.players[0]
        .hand
        .iter()
        .position(|m| m.taught_spell.is_some())
        .expect("a taught token") as u8;
    let at = g.players[0].board.len() as u8;
    let hand = g.players[0].hand.len();
    play(&mut g, 0, token, at);
    assert!(!g.players[0].discover.is_empty(), "the first Discover is offered");
    act(&mut g, 0, Action::Choose(0));
    assert!(!g.players[0].discover.is_empty(), "the second cast offers its own");
    act(&mut g, 0, Action::Choose(0));
    assert!(g.players[0].discover.is_empty(), "both picks landed");
    // The token left the hand and two Discoveries arrived.
    assert_eq!(g.players[0].hand.len(), hand + 1, "two casts, two cards");
    g.check_invariants().unwrap();
}

// ---- a Discover a Kelp Keeper's Activate opens ----

#[test]
fn a_kelp_keeper_answers_a_battlecrys_discover_with_a_random_silent_card() {
    // Kelp Keeper's Activate triggers Rodeo Performer's "Discover a Tavern
    // spell": the way the game answers that is a random card of the Discover's
    // pool with no Discover offered, and it is not a Discover for "after you
    // Discover" cards (docs/rules/design.md "Shop replay answers" and "Set
    // versus add", Era 2026-11-03 and 2026-11-04). A golden Kelp Keeper
    // triggers it twice, so two cards arrive and Hooktusk still sees none.
    let mut g = game(7012);
    put_on_board(&mut g, 0, "Kelp Keeper", true);
    put_on_board(&mut g, 0, "Rodeo Performer", false);
    put_on_board(&mut g, 0, "Hooktusk, Master Marauder", false);
    put_on_board(&mut g, 0, "Southsea Busker", false);
    g.players[0].golden_played = 1;
    g.players[0].gold = 10;
    let busker = stats(&g, 0, 3);
    let hand = g.players[0].hand.len();
    act(&mut g, 0, Action::Activate(0));
    // The Activate asks which minion's Battlecry to trigger: Rodeo Performer
    // and Southsea Busker have one, and the Rodeo Performer is slot 1.
    assert!(g.players[0].pending_target.is_some(), "the Activate picks its minion");
    act(&mut g, 0, Action::Target(1));
    assert!(g.players[0].discover.is_empty(), "no Discover is offered");
    assert!(g.players[0].discover_queue.is_empty(), "and none is left waiting");
    assert_eq!(g.players[0].repeat_discover, 0, "the repeat counter is spent");
    assert_eq!(g.players[0].hand.len(), hand + 2, "both triggers gave a random card");
    for m in &g.players[0].hand {
        assert!(g.data.cards.get(m.card).is_spell(), "a Tavern spell: {:?}", m.card);
    }
    assert_eq!(stats(&g, 0, 3), busker, "Hooktusk did not fire");
    g.check_invariants().unwrap();
}
