//! The Demon minions and spells of the 5-tribe lobby, with the real cards
//! from cards.toml: consuming Tavern minions, Demon Fodder, hero Health as a
//! resource, the damage and spell counters, and the fight-side Hellcaller.

use std::sync::Arc;

use bgcore::fight::{Counters, fight_with};
use bgcore::{Action, CardDb, Data, Game, Keywords, Minion, Opponent, Rng, Tribe};

fn db() -> CardDb {
    Data::bundled().cards
}

fn mk(db: &CardDb, name: &str, golden: bool) -> Minion {
    db.make(db.by_name(name).expect(name), golden)
}

/// A minion with `attack` that takes a long time to kill.
fn fighter(db: &CardDb, attack: i32) -> Minion {
    let mut w = mk(db, "Microbot", false);
    w.attack = attack;
    w.health = 2000;
    w
}

fn lobby(ts: &[Tribe]) -> bgcore::Tribes {
    ts.iter().copied().fold(bgcore::Tribes::NONE, |m, t| m.with(t))
}

fn game(seed: u64) -> Game {
    Game::new_with_lobby(Arc::new(Data::bundled()), seed, lobby(&[Tribe::Beast, Tribe::Mech, Tribe::Murloc, Tribe::Demon, Tribe::Elemental]))
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

fn is_legal(g: &mut Game, p: usize, a: Action) -> bool {
    turn_of(g, p);
    g.is_legal(a)
}

/// Everyone ends their turn until the next turn starts.
fn next_turn(g: &mut Game) {
    let t = g.turn;
    while g.turn == t && !g.over {
        if !answer(g) {
            g.step(Action::EndTurn).unwrap();
        }
    }
}

fn id(g: &Game, name: &str) -> bgcore::CardId {
    g.data.cards.by_name(name).unwrap_or_else(|| panic!("no card {name}"))
}

fn put_on_board(g: &mut Game, p: usize, name: &str) {
    let c = id(g, name);
    let slot = g.data.cards.get(c).pool_slot as usize;
    g.pool[slot] -= 1;
    let mut m = g.players[p].make_card(&g.data.cards, c, false, 1);
    m.uses_left = m.turn_uses(&g.data.cards);
    g.players[p].board.push(m);
}

/// Replace a player's shop with the named cards, through the pool.
fn set_shop(g: &mut Game, p: usize, names: &[&str]) {
    let old: Vec<Minion> = g.players[p].shop.drain(..).collect();
    for m in old {
        if !g.data.cards.get(m.card).token {
            let slot = g.data.cards.get(m.card).pool_slot as usize;
            g.pool[slot] += m.pool_copies;
        }
    }
    for n in names {
        let c = id(g, n);
        let slot = g.data.cards.get(c).pool_slot as usize;
        g.pool[slot] -= 1;
        let m = g.players[p].make_card(&g.data.cards, c, false, 1);
        g.players[p].shop.push(m);
    }
}

/// Empty a player's shop, returning its pool copies.
fn clear_shop(g: &mut Game, p: usize) {
    set_shop(g, p, &[]);
}

/// Put a generated card (a token) in the Tavern; it takes no pool copy.
fn put_token_in_shop(g: &mut Game, p: usize, name: &str, golden: bool) {
    let c = id(g, name);
    let m = g.data.cards.make(c, golden);
    g.players[p].shop.push(m);
}

fn fodder_count(g: &Game, p: usize) -> usize {
    g.players[p]
        .shop
        .iter()
        .filter(|m| g.data.cards.get(m.card).is_fodder)
        .count()
}

fn stats(m: &Minion) -> (i32, i32) {
    (m.attack, m.health)
}

/// Every other player gets one huge minion, so player 0 loses the fight.
fn gang_up_on_player_0(g: &mut Game) {
    let microbot = id(g, "Microbot");
    for q in 1..8 {
        let mut m = g.data.cards.make(microbot, false);
        m.attack = 50;
        m.health = 50;
        g.players[q].board.push(m);
    }
}

fn hand_has(g: &Game, name: &str) -> bool {
    g.players[0]
        .hand
        .iter()
        .any(|m| g.data.cards.get(m.card).name == name)
}

/// Take players out of the game as if they had died on an earlier turn,
/// giving their cards back to the pool, to set up a small lobby.
fn knock_out_players(g: &mut Game, dead: &[usize]) {
    let mut place = 8u8;
    for &q in dead {
        let pl = &mut g.players[q];
        let cards: Vec<Minion> =
            pl.shop.drain(..).chain(pl.hand.drain(..)).chain(pl.board.drain(..)).collect();
        let discovers: Vec<_> = pl.discover.drain(..).collect();
        pl.shop_spell = None;
        pl.pending_target = None;
        pl.pending_shop_target = None;
        pl.pending_choose_one = None;
        pl.discover_queue.clear();
        pl.discover_damage_from = None;
        pl.alive = false;
        pl.done = true;
        pl.placement = place;
        place -= 1;
        for m in cards {
            let c = g.data.cards.get(m.card);
            if m.pool_copies > 0 && !c.token && !c.is_spell() {
                g.pool[c.pool_slot as usize] += m.pool_copies;
            }
        }
        for c in discovers {
            if !g.data.cards.get(c).is_spell() {
                g.pool[g.data.cards.get(c).pool_slot as usize] += 1;
            }
        }
    }
}

// ---- Demon Fodder ----

#[test]
fn a_fodder_feeds_a_demon_and_its_slot_is_refilled() {
    let mut g = game(1);
    g.players[0].gold = 3;
    put_on_board(&mut g, 0, "Wrath Weaver");
    g.players[0].fodder_refreshes.clear();
    g.players[0].fodder_refreshes.push(1);
    act(&mut g, 0, Action::Roll);
    // The Fodder appeared in this Refresh and fed the only Demon at once:
    // Wrath Weaver took its +2/+2 and the Fodder's slot drew a real card.
    assert_eq!(stats(&g.players[0].board[0]), (1 + 2, 3 + 2));
    assert_eq!(g.players[0].shop.len(), 3);
    assert_eq!(fodder_count(&g, 0), 0);
    assert!(g.players[0].shop.iter().all(|m| !g.data.cards.get(m.card).is_fodder));
    // the generated Fodder never had a pool copy and the refill came from
    // the pool, so the totals still balance
    g.check_invariants().unwrap();
}

#[test]
fn the_refresh_buffs_reach_the_fodder_before_it_feeds() {
    // A Fodder feeds once the Refresh is over, so it carries the "after the
    // Tavern is Refreshed" buffs into the Demon it feeds:
    // `Hearthstone_2026_10_27_04_30_04-g3` turn 8 step 2, where the log's
    // Fodder feeds as 8/8, a 2/2 plus the Blood Gem Barrage's 6/6.
    let mut g = game(1);
    g.players[0].gold = 3;
    put_on_board(&mut g, 0, "Wrath Weaver");
    g.players[0].barrage = (2, 2);
    g.players[0].fodder_refreshes.clear();
    g.players[0].fodder_refreshes.push(1);
    act(&mut g, 0, Action::Roll);
    // the Fodder is 2/2 plus the Barrage's 2/2, so the Weaver gains +4/+4
    assert_eq!(stats(&g.players[0].board[0]), (1 + 4, 3 + 4), "fed as 4/4");
    assert_eq!(fodder_count(&g, 0), 0);
    g.check_invariants().unwrap();
}

#[test]
fn a_golden_wrath_weaver_damages_the_hero_twice() {
    // "deal 1 damage to your hero and gain +2/+2, twice" (the golden text):
    // two hits, so a Soul Rewinder takes +2 Health for each and a Devout
    // Hellcaller +2/+2 for each (2026_10_24_15_25_26-g4 turn 5 step 5).
    let mut g = game(7);
    put_on_board(&mut g, 0, "Soul Rewinder");
    put_on_board(&mut g, 0, "Devout Hellcaller");
    let weaver = id(&g, "Wrath Weaver");
    let slot = g.data.cards.get(weaver).pool_slot as usize;
    g.pool[slot] -= 3;
    let mut golden = g.players[0].make_card(&g.data.cards, weaver, true, 3);
    golden.uses_left = golden.turn_uses(&g.data.cards);
    g.players[0].board.push(golden);
    let health = g.players[0].health;
    g.players[0].gold = 10;
    let weaver = g.data.cards.make(id(&g, "Wrath Weaver"), false);
    g.players[0].hand.push(weaver);
    let h = g.players[0].hand.len() as u8 - 1;
    act(&mut g, 0, Action::Play { hand: h, pos: 3 });
    // The two hits of 1 come back through the Rewinder, so the hero pays 2 in
    // all, and each hit is its own: +4 Health, and +2/+2 twice.
    assert_eq!(g.players[0].health, health, "the Rewinder gives the damage back");
    assert_eq!(stats(&g.players[0].board[0]), (4, 2 + 4), "two hits, +2 Health each");
    assert_eq!(stats(&g.players[0].board[1]), (4 + 4, 4 + 4), "two hits, +2/+2 each");
    assert_eq!(stats(&g.players[0].board[2]), (2 + 4, 6 + 4), "the golden one gained +4/+4");
    g.check_invariants().unwrap();
}

#[test]
fn a_waiting_fodder_can_be_bought_and_played() {
    let mut g = game(2);
    g.players[0].gold = 10;
    g.players[0].fodder_refreshes.clear();
    g.players[0].fodder_refreshes.push(1);
    act(&mut g, 0, Action::Roll);
    let slot = (0..g.players[0].shop.len())
        .find(|&i| g.data.cards.get(g.players[0].shop[i].card).is_fodder)
        .expect("the Refresh made a Fodder");
    assert!(is_legal(&mut g, 0, Action::Buy(slot as u8)));
    act(&mut g, 0, Action::Buy(slot as u8));
    let hand = g.players[0]
        .hand
        .iter()
        .position(|m| g.data.cards.get(m.card).is_fodder)
        .expect("the bought Fodder is in hand");
    assert_eq!(stats(&g.players[0].hand[hand]), (2, 2));
    // a bought Fodder is an ordinary 2/2 Demon, and it never feeds
    act(&mut g, 0, Action::Play { hand: hand as u8, pos: 0 });
    assert_eq!(stats(&g.players[0].board[0]), (2, 2));
    assert!(g.data.cards.get(g.players[0].board[0].card).is_fodder);
    g.check_invariants().unwrap();
}

#[test]
fn a_fodder_feeds_on_the_refresh_it_appears_in() {
    let mut g = game(31);
    g.players[0].gold = 3;
    put_on_board(&mut g, 0, "Wrath Weaver");
    g.players[0].fodder_refreshes.clear();
    g.players[0].fodder_refreshes.push(1);
    act(&mut g, 0, Action::Roll);
    // the Fodder appeared and fed the only Demon at once; its slot is a
    // normal minion again
    assert_eq!(fodder_count(&g, 0), 0);
    assert_eq!(stats(&g.players[0].board[0]), (1 + 2, 3 + 2));
    assert_eq!(g.players[0].shop.len(), 3);
    g.check_invariants().unwrap();
}

#[test]
fn the_turn_start_roll_never_takes_queued_fodders() {
    // unfrozen, so the Tavern changed and the roll counts as a Refresh for
    // Waveling and Barrage, but Fodders still wait for an explicit one
    let mut g = game(32);
    g.players[0].gold = 3;
    put_on_board(&mut g, 0, "Wrath Weaver");
    g.players[0].fodder_refreshes.clear();
    g.players[0].fodder_refreshes.push(1);
    next_turn(&mut g);
    assert_eq!(g.players[0].fodder_refreshes.as_slice(), &[1], "still queued");
    assert_eq!(fodder_count(&g, 0), 0, "the roll made no Fodder");
    act(&mut g, 0, Action::Roll);
    assert!(g.players[0].fodder_refreshes.is_empty(), "the explicit one took it");
    // it fed on the Refresh it appeared in
    assert_eq!(fodder_count(&g, 0), 0);
    g.check_invariants().unwrap();
}

#[test]
fn a_waiting_fodder_feeds_as_soon_as_a_demon_lands() {
    let mut g = game(3);
    g.players[0].gold = 10;
    g.players[0].fodder_refreshes.clear();
    g.players[0].fodder_refreshes.push(1);
    act(&mut g, 0, Action::Roll);
    assert_eq!(fodder_count(&g, 0), 1, "no Demon on board, so it waits");
    assert!(g.players[0].fodder_refreshes.is_empty());
    let size = g.players[0].shop.len();
    // a Demon landing later makes it feed at once, and its slot refills
    g.give(0, "Devout Hellcaller").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    assert_eq!(fodder_count(&g, 0), 0);
    assert_eq!(stats(&g.players[0].board[0]), (4 + 2, 4 + 2));
    assert_eq!(g.players[0].shop.len(), size);
    g.check_invariants().unwrap();
}

#[test]
fn the_start_of_turn_roll_is_not_a_refresh_for_fodders() {
    let mut g = game(43);
    g.players[0].gold = 10;
    g.give(0, "Laboratory Assistant").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    act(&mut g, 0, Action::Sell(0));
    assert_eq!(g.players[0].fodder_refreshes.as_slice(), [1, 1, 1]);
    next_turn(&mut g);
    // the automatic roll at the start of the turn left the queue alone and
    // made no Fodder
    assert_eq!(g.players[0].fodder_refreshes.as_slice(), [1, 1, 1]);
    assert_eq!(fodder_count(&g, 0), 0);
    // the next explicit Refresh is the one that takes one
    act(&mut g, 0, Action::Roll);
    assert_eq!(g.players[0].fodder_refreshes.as_slice(), [1, 1]);
    assert_eq!(fodder_count(&g, 0), 1);
    g.check_invariants().unwrap();
}

#[test]
fn snarky_sharks_refresh_leaves_queued_fodders_alone() {
    let mut g = game(44);
    g.players[0].gold = 10;
    put_on_board(&mut g, 0, "Wrath Weaver");
    put_on_board(&mut g, 0, "Snarky Shark");
    g.players[0].fodder_refreshes.clear();
    g.players[0].fodder_refreshes.push(1);
    act(&mut g, 0, Action::Sell(1));
    // the sell's Refresh with a Fishbait is a card effect, not the player's
    // own Refresh: the queued Fodder stays queued and nothing feeds
    assert_eq!(g.players[0].fodder_refreshes.as_slice(), [1]);
    assert_eq!(stats(&g.players[0].board[0]), (1, 3));
    assert_eq!(g.players[0].shop[0].card, id(&g, "Fishbait"));
    g.check_invariants().unwrap();
}

// ---- Refreshes getting Fodders ----

#[test]
fn laboratory_assistant_puts_a_fodder_in_each_of_the_next_three_refreshes() {
    let mut g = game(4);
    g.players[0].gold = 10;
    g.give(0, "Laboratory Assistant").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    assert_eq!(g.players[0].fodder_refreshes.as_slice(), [1, 1, 1]);
    // no Demon once it is sold, so Fodders stay in the Tavern
    act(&mut g, 0, Action::Sell(0));
    for want in [1, 1, 1, 0] {
        act(&mut g, 0, Action::Roll);
        assert_eq!(fodder_count(&g, 0), want);
    }
    g.check_invariants().unwrap();

    // a golden one adds two Fodders to each of the three
    let mut g = game(5);
    g.players[0].gold = 10;
    g.give(0, "Laboratory Assistant").unwrap();
    g.players[0].hand[0].golden = true;
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    assert_eq!(g.players[0].fodder_refreshes.as_slice(), [2, 2, 2]);
    // playing a golden minion puts a Triple Reward spell in hand; nothing is
    // open to answer, so sell right away
    act(&mut g, 0, Action::Sell(0));
    act(&mut g, 0, Action::Roll);
    assert_eq!(fodder_count(&g, 0), 2);
    g.check_invariants().unwrap();
}

#[test]
fn twisted_wrathguard_adds_to_the_very_next_refresh() {
    let mut g = game(6);
    g.players[0].gold = 10;
    put_on_board(&mut g, 0, "Twisted Wrathguard");
    put_on_board(&mut g, 0, "Buzzing Vermin");
    g.players[0].fodder_refreshes.clear();
    g.players[0].fodder_refreshes.extend([1, 1, 1]);
    act(&mut g, 0, Action::Sell(1));
    // the next Refresh (the first entry), not the end of the queue
    assert_eq!(g.players[0].fodder_refreshes.as_slice(), [2, 1, 1]);
    // the two Fodders feed the Wrathguard as soon as they appear
    act(&mut g, 0, Action::Roll);
    assert_eq!(g.players[0].fodder_refreshes.as_slice(), [1, 1]);
    assert_eq!(stats(&g.players[0].board[0]), (8 + 4, 8 + 4));
    g.check_invariants().unwrap();
}

#[test]
fn two_laboratory_assistants_stack_fodders_per_refresh() {
    let mut g = game(33);
    g.players[0].gold = 10;
    g.give(0, "Laboratory Assistant").unwrap();
    g.give(0, "Laboratory Assistant").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    act(&mut g, 0, Action::Play { hand: 0, pos: 1 });
    assert_eq!(g.players[0].fodder_refreshes.as_slice(), [2, 2, 2]);
    // no Demon on the board while the Fodders arrive, so they stay
    act(&mut g, 0, Action::Sell(0));
    act(&mut g, 0, Action::Sell(0));
    for want in [2, 2, 2, 0] {
        act(&mut g, 0, Action::Roll);
        assert_eq!(fodder_count(&g, 0), want);
    }
    g.check_invariants().unwrap();
}

#[test]
fn queued_fodders_can_fill_the_spell_slot() {
    let mut g = game(34);
    g.players[0].tier = 6;
    g.players[0].gold = 3;
    put_on_board(&mut g, 0, "Ominous Seer");
    g.players[0].fodder_refreshes.clear();
    // all six minion slots plus the spell slot
    g.players[0].fodder_refreshes.push(7);
    act(&mut g, 0, Action::Roll);
    // a Demon was on board when they appeared, so every Fodder fed it and
    // its slot was refilled with a minion: the Tavern holds seven minions
    // and no spell
    assert_eq!(fodder_count(&g, 0), 0);
    assert_eq!(g.players[0].shop.len(), 7);
    assert!(g.players[0].shop_spell.is_none(), "the spell slot was taken");
    // the only Demon ate all seven Fodders
    assert_eq!(stats(&g.players[0].board[0]), (2 + 14, 1 + 14));
    g.check_invariants().unwrap();
}

#[test]
fn more_fodders_than_slots_widen_the_tavern() {
    // Tier 4 deals five minions and a spell. Six Fodders take the first six
    // slots and the spell keeps the seventh; seven Fodders fill the Tavern
    // and there is no room for the spell (2026_11_01_04_27_54-g2 turn 11
    // step 26 and step 7, 2026_11_01_17_59_13-g1 turn 10 step 21).
    let mut g = game(36);
    g.players[0].tier = 4;
    g.players[0].gold = 3;
    put_on_board(&mut g, 0, "Ominous Seer");
    g.players[0].fodder_refreshes.clear();
    g.players[0].fodder_refreshes.push(6);
    act(&mut g, 0, Action::Roll);
    assert_eq!(fodder_count(&g, 0), 0, "they all fed the Demon");
    assert_eq!(g.players[0].shop.len(), 6, "the Tavern grew to hold them all");
    assert!(g.players[0].shop_spell.is_some(), "the spell slot is still there");
    g.check_invariants().unwrap();

    let mut g = game(37);
    g.players[0].tier = 4;
    g.players[0].gold = 3;
    put_on_board(&mut g, 0, "Ominous Seer");
    g.players[0].fodder_refreshes.clear();
    g.players[0].fodder_refreshes.push(7);
    act(&mut g, 0, Action::Roll);
    assert_eq!(g.players[0].shop.len(), 7, "seven cards is all there is room for");
    assert!(g.players[0].shop_spell.is_none(), "the spell slot was taken");
    g.check_invariants().unwrap();
}

#[test]
fn urzul_can_consume_a_waiting_fodder() {
    let mut g = game(35);
    clear_shop(&mut g, 0);
    put_token_in_shop(&mut g, 0, "Demon Fodder", false);
    put_on_board(&mut g, 0, "Insatiable Ur'zul");
    g.give(0, "Flaming Enforcer").unwrap();
    let before_pool: u32 = g.pool.iter().map(|&n| n as u32).sum();
    act(&mut g, 0, Action::Play { hand: 0, pos: 1 });
    // a Fodder in the Tavern is a normal Tavern minion: Ur'zul's after-play
    // consume (for the next Demon played, never its own play) eats it for
    // +2/+2, and the generated copy never touches the pool
    assert_eq!(stats(&g.players[0].board[0]), (4 + 2, 6 + 2));
    assert!(g.players[0].shop.is_empty());
    assert_eq!(g.pool.iter().map(|&n| n as u32).sum::<u32>(), before_pool);
    g.check_invariants().unwrap();
}

#[test]
fn a_fodder_next_to_a_demon_feeds_before_the_enforcer_eats() {
    let mut g = game(36);
    put_on_board(&mut g, 0, "Flaming Enforcer");
    set_shop(&mut g, 0, &["Buzzing Vermin"]);
    put_token_in_shop(&mut g, 0, "Demon Fodder", false);
    act(&mut g, 0, Action::EndTurn);
    // a Demon is on board, so the Fodder feeds the Enforcer (+2/+2) as the
    // action ends, before the end of turn: it is never there to be eaten
    assert_eq!(fodder_count(&g, 0), 0);
    let (a, h) = stats(&g.players[0].board[0]);
    assert!(a >= 4 + 2 && h >= 5 + 2, "fed, then ate a Tavern minion: {a}/{h}");
    g.check_invariants().unwrap();
}

#[test]
fn the_enforcer_eats_a_random_one_of_two_on_a_health_tie() {
    // Flaming Enforcer's "consume the highest-Health Tavern minion": on a
    // Health tie the pick is random, as every other highest-Health pick in the
    // Tavern is (docs/rules/design.md: Arcane Absorption and De-volition-ist).
    // `Hearthstone_2026_11_03_13_59_12-g2` turn 4 step 5 eats the second of
    // two 3/4s; ours always ate the left-most one.
    let mut seen = [false; 2];
    for seed in 0..24 {
        let mut g = game(seed);
        put_on_board(&mut g, 0, "Flaming Enforcer");
        set_shop(&mut g, 0, &["Snow Baller", "Intrepid Botanist"]);
        assert_eq!(stats(&g.players[0].shop[0]), stats(&g.players[0].shop[1]), "a tie");
        act(&mut g, 0, Action::EndTurn);
        let left = g.players[0].shop.first().map(|m| g.data.cards.get(m.card).name.clone());
        match left.as_deref() {
            Some("Snow Baller") => seen[1] = true,     // the Botanist was eaten
            Some("Intrepid Botanist") => seen[0] = true, // the Snow Baller was eaten
            other => panic!("seed {seed}: the Tavern kept {other:?}"),
        }
        g.check_invariants().unwrap();
    }
    assert!(seen[0] && seen[1], "both of the tied minions get eaten over the seeds: {seen:?}");
}

// ---- Hero health ----

#[test]
fn malchezaars_health_refreshes_cannot_kill() {
    let mut g = game(7);
    g.players[0].gold = 0;
    g.players[0].health = 2;
    put_on_board(&mut g, 0, "Malchezaar, Prince of Dance");
    act(&mut g, 0, Action::Roll);
    assert_eq!(g.players[0].health, 1);
    // the second one would take the hero to 0
    assert!(!is_legal(&mut g, 0, Action::Roll));

    // with a rewind on the board it is payable at 1 Health, and the rewinder
    // still gains its +2 Health
    let mut g = game(8);
    g.players[0].gold = 0;
    g.players[0].health = 1;
    put_on_board(&mut g, 0, "Malchezaar, Prince of Dance");
    put_on_board(&mut g, 0, "Soul Rewinder");
    let before = g.players[0].board[1].health;
    assert!(is_legal(&mut g, 0, Action::Roll));
    act(&mut g, 0, Action::Roll);
    assert_eq!(g.players[0].health, 1);
    assert_eq!(g.players[0].board[1].health, before + 2);
    g.check_invariants().unwrap();
}

#[test]
fn a_later_rewind_gives_nothing_back_but_still_buffs_the_tavern() {
    let mut g = game(32);
    g.players[0].gold = 0;
    g.players[0].health = 1;
    put_on_board(&mut g, 0, "Malchezaar, Prince of Dance");
    put_on_board(&mut g, 0, "Soul Rewinder");
    put_on_board(&mut g, 0, "Ashen Corruptor");
    let soul = g.players[0].board[1].health;
    act(&mut g, 0, Action::Roll);
    // Soul Rewinder (left) gives the Health back; Ashen Corruptor has nothing
    // left to rewind but still gives the Tavern +2/+2 this turn
    assert_eq!(g.players[0].health, 1);
    assert_eq!(g.players[0].board[1].health, soul + 2);
    for m in &g.players[0].shop {
        let c = g.data.cards.get(m.card);
        assert_eq!((m.attack, m.health), (c.attack + 2, c.health + 2), "{}", c.name);
    }
    g.check_invariants().unwrap();
}

#[test]
fn wrath_weaver_damage_kills_the_hero_on_the_spot() {
    let mut g = game(9);
    g.players[0].health = 1;
    put_on_board(&mut g, 0, "Wrath Weaver");
    g.give(0, "Devout Hellcaller").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 1 });
    assert_eq!(g.players[0].health, 0);
    assert_eq!(g.players[0].last_damage, 1);
    // the hero dies at once, in the middle of its turn: worst place still
    // open, no more actions, no fight this turn
    assert!(!g.players[0].alive);
    assert_eq!(g.players[0].placement, 8);
    assert_eq!(g.players[0].opponent, None);
    assert_ne!(g.current_player(), Some(0));
    assert!(g
        .players
        .iter()
        .all(|p| p.opponent != Some(Opponent::Player(0))));
    assert!(g.players[0].board.is_empty(), "its cards went back to the pool");
    g.check_invariants().unwrap();
    // the rest of the turn and the fights run without it
    next_turn(&mut g);
    assert!(!g.players[0].alive);
    assert_eq!(g.players[0].placement, 8);
    assert_eq!(g.alive(), 7);
    g.check_invariants().unwrap();
}

#[test]
fn a_rewind_saves_a_hero_wrath_weaver_would_kill() {
    let mut g = game(41);
    g.players[0].health = 1;
    put_on_board(&mut g, 0, "Wrath Weaver");
    put_on_board(&mut g, 0, "Soul Rewinder");
    g.give(0, "Ominous Seer").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 2 });
    // the damage took the hero to 0 and the rewind gave it straight back
    assert_eq!(g.players[0].health, 1);
    assert!(g.players[0].alive);
    assert_eq!(g.players[0].placement, 0);
    assert_eq!(stats(&g.players[0].board[1]), (4, 2 + 2));
    g.check_invariants().unwrap();
}

#[test]
fn a_mid_turn_death_leaving_one_player_ends_the_game() {
    let mut g = game(42);
    knock_out_players(&mut g, &[2, 3, 4, 5, 6, 7]);
    turn_of(&mut g, 0);
    g.players[0].health = 1;
    put_on_board(&mut g, 0, "Wrath Weaver");
    g.give(0, "Ominous Seer").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 1 });
    assert_eq!(g.players[0].health, 0);
    // 2 players were left, so the dying hero takes 2nd and the game ends
    assert!(!g.players[0].alive);
    assert_eq!(g.players[0].placement, 2);
    assert_eq!(g.players[1].placement, 1);
    assert_eq!(g.alive(), 1);
    assert!(g.over);
    assert_eq!(g.current_player(), None);
    g.check_invariants().unwrap();
}

#[test]
fn tichondrius_and_eredar_react_to_health_paid_for_a_refresh() {
    let mut g = game(10);
    g.players[0].gold = 0;
    g.players[0].health = 30;
    put_on_board(&mut g, 0, "Malchezaar, Prince of Dance");
    put_on_board(&mut g, 0, "Tichondrius");
    put_on_board(&mut g, 0, "Eredar Escapist");
    // one damage short of the Corrupted Cupcakes
    g.players[0].board[2].improvements = 3;
    let before = stats(&g.players[0].board[1]);
    act(&mut g, 0, Action::Roll);
    assert_eq!(g.players[0].health, 29);
    // Tichondrius gives every Demon +4/+4, the Escapist included
    assert_eq!(stats(&g.players[0].board[1]), (before.0 + 4, before.1 + 4));
    assert_eq!(stats(&g.players[0].board[2]), (6 + 4, 8 + 4));
    assert!(hand_has(&g, "Corrupted Cupcakes"));
    assert_eq!(
        g.players[0].board[2].events_left(&g.data.cards),
        Some(4),
        "the counter starts over"
    );
    g.check_invariants().unwrap();
}

// ---- Sacrificial Wrathguard ----

#[test]
fn sacrificial_wrathguard_buffs_the_tavern_for_the_game() {
    let db = db();
    // two Activates: the Deathrattle gives 2/2 + 2 * 2/2
    let mut w = mk(&db, "Sacrificial Wrathguard", false);
    w.improvements = 2;
    let wall = fighter(&db, 50);
    let out = fight_with(
        &db,
        &[w],
        &[wall],
        [Counters::default(); 2],
        20,
        &mut Rng::new(1),
        &mut (),
    );
    assert_eq!(out.sides[0].tavern_buff_gain, (6, 6));

    // a golden one doubles the whole Deathrattle, so each Activate adds
    // +4/+4 on top of the golden base +4/+4
    let mut w = mk(&db, "Sacrificial Wrathguard", true);
    w.improvements = 2;
    let out = fight_with(
        &db,
        &[w],
        &[fighter(&db, 50)],
        [Counters::default(); 2],
        20,
        &mut Rng::new(1),
        &mut (),
    );
    assert_eq!(out.sides[0].tavern_buff_gain, (12, 12));

    // in a whole game the Activate raises the number and the fight applies
    // the Tavern buff to the next Refresh
    let mut g = game(11);
    g.players[0].gold = 5;
    put_on_board(&mut g, 0, "Sacrificial Wrathguard");
    act(&mut g, 0, Action::Activate(0));
    next_turn(&mut g);
    act(&mut g, 0, Action::Activate(0));
    assert_eq!(g.players[0].board[0].improvements, 2);
    gang_up_on_player_0(&mut g);
    next_turn(&mut g);
    let buff = g
        .players[0]
        .tavern_buffs
        .iter()
        .find(|b| !b.this_turn && b.tribe.is_none())
        .copied()
        .expect("the Deathrattle's Tavern buff is kept");
    assert_eq!((buff.attack, buff.health), (6, 6));
    for m in &g.players[0].shop {
        let c = g.data.cards.get(m.card);
        assert_eq!((m.attack, m.health), (c.attack + 6, c.health + 6));
    }
    g.check_invariants().unwrap();

    // A golden one counts Activates once per Activate, not twice: the
    // doubled number lives in the Deathrattle.
    let mut g = game(46);
    g.players[0].gold = 5;
    let w = id(&g, "Sacrificial Wrathguard");
    let slot = g.data.cards.get(w).pool_slot as usize;
    g.pool[slot] -= 3;
    let m = g.players[0].make_card(&g.data.cards, w, true, 3);
    g.players[0].board.push(m);
    act(&mut g, 0, Action::Activate(0));
    next_turn(&mut g);
    act(&mut g, 0, Action::Activate(0));
    assert_eq!(g.players[0].board[0].improvements, 2);
    g.check_invariants().unwrap();
}

#[test]
fn deathstrider_triggers_the_wrathguard_in_the_shop_through_a_fishbait_attack() {
    let mut g = game(45);
    g.players[0].gold = 10;
    put_on_board(&mut g, 0, "Wolf Pup");
    put_on_board(&mut g, 0, "Deathstrider");
    put_on_board(&mut g, 0, "Sacrificial Wrathguard");
    g.players[0].board[2].improvements = 2;
    put_on_board(&mut g, 0, "Snarky Shark");
    // the Shark's sell refreshes with a Fishbait; the left-most Beast (the
    // Rally Wolf Pup) attacks it, and Deathstrider's after-attack trigger
    // runs the Wrathguard's Deathrattle in the shop
    act(&mut g, 0, Action::Sell(3));
    let buff = g
        .players[0]
        .tavern_buffs
        .iter()
        .find(|b| b.tribe.is_none() && !b.this_turn)
        .copied()
        .expect("the Deathrattle's Tavern buff was applied");
    assert_eq!((buff.attack, buff.health), (6, 6));
    g.check_invariants().unwrap();
}

// ---- Demons consuming ----

#[test]
fn mind_muck_has_a_demon_consume_a_random_tavern_minion() {
    // "Choose a friendly Demon. It consumes a minion in the Tavern to gain
    // its stats." The Demon is the only pick: the game takes the Tavern
    // minion itself, so nothing waits on the player (log
    // 2026_11_01_17_59_13-g1 turn 8 step 26, where one play with Brann took
    // three Tavern minions and the log shows no choice prompt between them).
    let mut g = game(12);
    put_on_board(&mut g, 0, "Devout Hellcaller");
    set_shop(&mut g, 0, &["Buzzing Vermin", "Cord Puller"]);
    g.give(0, "Mind Muck").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 1 });
    assert!(g.players[0].pending_shop_target.is_none(), "the game picks the Tavern minion");
    // either 1/1 is a legal pick
    assert_eq!(stats(&g.players[0].board[0]), (5, 5));
    assert_eq!(g.players[0].shop.len(), 1);
    g.check_invariants().unwrap();

    // a golden Mind Muck takes double its stats
    let mut g = game(13);
    put_on_board(&mut g, 0, "Devout Hellcaller");
    set_shop(&mut g, 0, &["Cord Puller"]);
    g.give(0, "Mind Muck").unwrap();
    g.players[0].hand[0].golden = true;
    act(&mut g, 0, Action::Play { hand: 0, pos: 1 });
    // with one Tavern card there is nothing to ask either way
    assert!(g.players[0].pending_shop_target.is_none());
    assert_eq!(stats(&g.players[0].board[0]), (6, 6));
    g.check_invariants().unwrap();
}

#[test]
fn insatiable_ur_zul_consumes_when_it_is_played_and_returns_the_copy() {
    let mut g = game(14);
    let slot = g.data.cards.get(id(&g, "Buzzing Vermin")).pool_slot as usize;
    set_shop(&mut g, 0, &["Buzzing Vermin"]);
    let before = g.pool[slot];
    put_on_board(&mut g, 0, "Insatiable Ur'zul");
    g.give(0, "Flaming Enforcer").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 1 });
    // 4/6 plus the 1/1 it ate
    assert_eq!(stats(&g.players[0].board[0]), (5, 7));
    assert!(g.players[0].shop.is_empty());
    assert_eq!(g.pool[slot], before + 1, "the consumed copy is back");
    g.check_invariants().unwrap();
}

#[test]
fn after_you_play_a_demon_waits_for_the_battlecry_to_consume() {
    // Mind Muck's Battlecry asks for a Demon, which then eats a Tavern
    // minion. The raw log puts the Wrath Weaver's "after you play a Demon"
    // after the consume (2026_11_01_17_59_13-g1 turn 8 step 26: the eaten
    // minions carry no Ashen buff), so the Tavern is unbuffed when the
    // Demon eats and only what is left takes the +2/+2.
    let mut g = game(46);
    put_on_board(&mut g, 0, "Wrath Weaver");
    put_on_board(&mut g, 0, "Ashen Corruptor");
    put_on_board(&mut g, 0, "Deft Deserter");
    set_shop(&mut g, 0, &["Buzzing Vermin", "Cord Puller"]);
    g.players[0].health = 30;
    g.give(0, "Mind Muck").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 3 });
    // three Demons could eat (Mind Muck itself never counts), so the play
    // waits for the pick, and the Weaver has not fired
    assert!(g.players[0].pending_target.is_some());
    assert_eq!(stats(&g.players[0].board[0]), (1, 3), "nothing fired yet");
    act(&mut g, 0, Action::Target(2));
    // 8/8 plus the 1/1 it ate as it stood: had the Weaver fired first, the
    // Tavern would already be 3/3 and the Deserter 11/11
    assert_eq!(stats(&g.players[0].board[2]), (9, 9));
    assert_eq!(stats(&g.players[0].board[0]), (3, 5), "and then the Weaver's hit");
    // Ashen Corruptor rewinds that damage and gives the Tavern's survivor
    // the +2/+2 once
    assert_eq!(g.players[0].health, 30);
    assert_eq!(g.players[0].shop.len(), 1);
    assert_eq!(stats(&g.players[0].shop[0]), (3, 3), "the survivor takes the buff");
    g.check_invariants().unwrap();
}

#[test]
fn a_golden_wrath_weaver_buffs_the_tavern_only_after_the_consume() {
    // The same play with the golden Weaver: two hits, so the survivor takes
    // +2/+2 twice (+4/+4, the log's survivors) while the eaten minion still
    // carries none of it.
    let mut g = game(47);
    let weaver = id(&g, "Wrath Weaver");
    let slot = g.data.cards.get(weaver).pool_slot as usize;
    g.pool[slot] -= 3;
    let golden = g.players[0].make_card(&g.data.cards, weaver, true, 3);
    g.players[0].board.push(golden);
    put_on_board(&mut g, 0, "Ashen Corruptor");
    put_on_board(&mut g, 0, "Deft Deserter");
    set_shop(&mut g, 0, &["Buzzing Vermin", "Cord Puller"]);
    g.players[0].health = 30;
    g.give(0, "Mind Muck").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 3 });
    act(&mut g, 0, Action::Target(2));
    assert_eq!(stats(&g.players[0].board[2]), (9, 9), "the eaten 1/1 came unbuffed");
    assert_eq!(g.players[0].shop.len(), 1);
    assert_eq!(stats(&g.players[0].shop[0]), (5, 5), "two hits, +2/+2 each");
    assert_eq!(stats(&g.players[0].board[0]), (2 + 4, 6 + 4));
    assert_eq!(g.players[0].health, 30, "both hits are rewound");
    g.check_invariants().unwrap();
}

#[test]
fn after_you_play_a_demon_waits_for_the_discover_pick() {
    // Imposing Percussionist's Battlecry Discovers a Demon and then damages
    // the hero for its tier. The raw log puts the Wrath Weaver's trigger
    // after the pick and its damage (2026_10_24_15_25_26, 16:31:00: the
    // Weaver's blocks follow the whole play block), so the Weaver fires on
    // the answer, not on the play.
    let mut g = game(48);
    put_on_board(&mut g, 0, "Wrath Weaver");
    g.players[0].tier = 3;
    g.players[0].health = 30;
    g.give(0, "Imposing Percussionist").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 1 });
    assert_eq!(g.players[0].discover.len(), 3);
    assert_eq!(stats(&g.players[0].board[0]), (1, 3), "nothing fired yet");
    assert_eq!(g.players[0].health, 30, "the damage follows the pick");
    let tier = g.data.cards.get(g.players[0].discover[0]).tier as i32;
    act(&mut g, 0, Action::Choose(0));
    assert_eq!(stats(&g.players[0].board[0]), (3, 5), "the Weaver fires on the answer");
    assert_eq!(g.players[0].health, 30 - tier - 1, "the pick's damage, then the Weaver's");
    g.check_invariants().unwrap();
}

#[test]
fn a_played_demon_never_counts_for_its_own_after_play_trigger() {
    let mut g = game(49);
    put_on_board(&mut g, 0, "Wrath Weaver");
    g.players[0].health = 30;
    g.give(0, "Wrath Weaver").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 1 });
    assert_eq!(stats(&g.players[0].board[0]), (3, 5), "the one on the board fired");
    assert_eq!(stats(&g.players[0].board[1]), (1, 3), "the one just played did not");
    assert_eq!(g.players[0].health, 29);
    // and the mark is gone: the next Demon played fires the one played here
    g.give(0, "Deft Deserter").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 2 });
    assert_eq!(stats(&g.players[0].board[0]), (5, 7));
    assert_eq!(stats(&g.players[0].board[1]), (3, 5));
    g.check_invariants().unwrap();
}

#[test]
fn soulkeeping_jailer_has_every_demon_consume_one() {
    let mut g = game(15);
    g.players[0].gold = 2;
    put_on_board(&mut g, 0, "Soulkeeping Jailer");
    put_on_board(&mut g, 0, "Devout Hellcaller");
    let slot = g.data.cards.get(id(&g, "Buzzing Vermin")).pool_slot as usize;
    set_shop(&mut g, 0, &["Buzzing Vermin", "Buzzing Vermin"]);
    let before = g.pool[slot];
    act(&mut g, 0, Action::Activate(0));
    assert_eq!(stats(&g.players[0].board[0]), (3 + 1, 5 + 1));
    assert_eq!(stats(&g.players[0].board[1]), (4 + 1, 4 + 1));
    assert!(g.players[0].shop.is_empty());
    assert_eq!(g.pool[slot], before + 2);
    g.check_invariants().unwrap();

    // golden Jailer: each Demon takes double stats
    let mut g = game(16);
    g.players[0].gold = 2;
    put_on_board(&mut g, 0, "Soulkeeping Jailer");
    g.players[0].board[0].golden = true;
    let before = stats(&g.players[0].board[0]);
    set_shop(&mut g, 0, &["Buzzing Vermin"]);
    act(&mut g, 0, Action::Activate(0));
    assert_eq!(
        stats(&g.players[0].board[0]),
        (before.0 + 2, before.1 + 2)
    );
    g.check_invariants().unwrap();
}

#[test]
fn flaming_enforcer_eats_the_highest_health_minion_at_end_of_turn() {
    let mut g = game(17);
    put_on_board(&mut g, 0, "Flaming Enforcer");
    set_shop(&mut g, 0, &["Buzzing Vermin", "Lullabot"]);
    g.players[0].shop[1].health = 8;
    act(&mut g, 0, Action::EndTurn);
    // Lullabot is 2/8, Buzzing Vermin 1/1
    assert_eq!(stats(&g.players[0].board[0]), (4 + 2, 5 + 8));
    assert_eq!(g.players[0].shop.len(), 1);
    g.check_invariants().unwrap();
}

// ---- Tavern buffs made by Demons ----

#[test]
fn deft_deserter_gives_the_tavern_stats_and_one_shared_keyword() {
    let mask = Keywords::TAUNT | Keywords::DIVINE_SHIELD | Keywords::WINDFURY;
    for seed in 0..8 {
        let mut g = game(seed);
        g.players[0].gold = 1;
        put_on_board(&mut g, 0, "Deft Deserter");
        set_shop(&mut g, 0, &["Flittering Bat", "Bubble Gunner", "Lullabot"]);
        let before: Vec<_> = g.players[0].shop.iter().map(|m| (m.attack, m.health)).collect();
        act(&mut g, 0, Action::Activate(0));
        let mut chosen = 0u8;
        for (m, base) in g.players[0].shop.iter().zip(before) {
            assert_eq!((m.attack, m.health), (base.0 + 8, base.1 + 8));
            let k = m.keywords.0 & mask;
            assert_eq!(k.count_ones(), 1, "seed {seed}: exactly one new keyword");
            if chosen == 0 {
                chosen = k;
            }
            assert_eq!(k, chosen, "seed {seed}: the same keyword everywhere");
        }
        g.check_invariants().unwrap();
    }
}

#[test]
fn devilish_distractor_buffs_the_tavern_this_game() {
    let mut g = game(18);
    put_on_board(&mut g, 0, "Devilish Distractor");
    set_shop(&mut g, 0, &["Buzzing Vermin"]);
    g.give(0, "Fortify").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    act(&mut g, 0, Action::Target(0));
    // Fortify's own +0/+3, then the Distractor's +1/+2 for the game
    assert_eq!(stats(&g.players[0].board[0]), (4, 7 + 3));
    assert_eq!(stats(&g.players[0].shop[0]), (1 + 1, 1 + 2));
    // later arrivals get it too
    act(&mut g, 0, Action::Roll);
    for m in &g.players[0].shop {
        let c = g.data.cards.get(m.card);
        assert_eq!((m.attack, m.health), (c.attack + 1, c.health + 2), "{}", c.name);
    }
    g.check_invariants().unwrap();
}

// ---- Counters ----

#[test]
fn ominous_seer_discounts_the_next_tavern_spell() {
    let mut g = game(19);
    g.players[0].tier = 3;
    g.give(0, "Ominous Seer").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    g.players[0].shop_spell = Some(id(&g, "Shiny Ring"));
    assert_eq!(g.players[0].spell_cost(&g.data.cards), Some(1));
    g.players[0].gold = 3;
    act(&mut g, 0, Action::BuySpell);
    assert_eq!(g.players[0].gold, 2);
    assert_eq!(g.players[0].spell_discount, 0, "the discount is used up");

    // golden: (2) less
    let mut g = game(20);
    g.players[0].tier = 3;
    g.give(0, "Ominous Seer").unwrap();
    g.players[0].hand[0].golden = true;
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    g.players[0].shop_spell = Some(id(&g, "Shiny Ring"));
    assert_eq!(g.players[0].spell_cost(&g.data.cards), Some(0));
}

#[test]
fn felboar_consumes_after_three_spells_and_starts_over() {
    let mut g = game(21);
    put_on_board(&mut g, 0, "Felboar");
    set_shop(&mut g, 0, &["Buzzing Vermin"]);
    assert_eq!(g.players[0].board[0].events_left(&g.data.cards), Some(3));
    for _ in 0..2 {
        g.give(0, "Tavern Coin").unwrap();
        act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    }
    assert_eq!(stats(&g.players[0].board[0]), (2, 6));
    assert_eq!(g.players[0].board[0].events_left(&g.data.cards), Some(1));
    g.give(0, "Tavern Coin").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    assert_eq!(stats(&g.players[0].board[0]), (3, 7));
    assert!(g.players[0].shop.is_empty());
    assert_eq!(g.players[0].board[0].events_left(&g.data.cards), Some(3));
    g.check_invariants().unwrap();
}

#[test]
fn trapper_clapper_queues_fodders_after_a_fight() {
    let db = db();
    let out = fight_with(
        &db,
        &[mk(&db, "Trapped Clapper", false)],
        &[fighter(&db, 50)],
        [Counters::default(); 2],
        20,
        &mut Rng::new(1),
        &mut (),
    );
    assert_eq!(out.sides[0].n_fodder_refreshes, 3);
    assert_eq!(out.sides[0].fodder_refreshes[..3], [1, 1, 1]);

    let out = fight_with(
        &db,
        &[mk(&db, "Trapped Clapper", true)],
        &[fighter(&db, 50)],
        [Counters::default(); 2],
        20,
        &mut Rng::new(1),
        &mut (),
    );
    assert_eq!(out.sides[0].fodder_refreshes[..3], [2, 2, 2]);

    // in a game the queued Fodders wait for an explicit Refresh: the roll at
    // the start of the turn leaves them, the next Roll takes one and the
    // Clapper (still the only Demon) eats it
    let mut g = game(22);
    put_on_board(&mut g, 0, "Trapped Clapper");
    gang_up_on_player_0(&mut g);
    next_turn(&mut g);
    assert_eq!(g.players[0].fodder_refreshes.as_slice(), [1, 1, 1]);
    assert_eq!(stats(&g.players[0].board[0]), (2, 2));
    assert_eq!(fodder_count(&g, 0), 0);
    act(&mut g, 0, Action::Roll);
    assert_eq!(g.players[0].board[0].attack, 2 + 2);
    assert_eq!(g.players[0].board[0].health, 2 + 2);
    assert_eq!(g.players[0].fodder_refreshes.as_slice(), [1, 1]);
    g.check_invariants().unwrap();
}

#[test]
fn imposing_percussionist_damages_the_hero_for_the_demons_tier() {
    let mut g = game(23);
    g.players[0].tier = 3;
    g.players[0].health = 30;
    g.give(0, "Imposing Percussionist").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    assert_eq!(g.players[0].discover.len(), 3);
    assert!(g.players[0].discover.iter().all(|&c| {
        let card = g.data.cards.get(c);
        card.tribes.has(Tribe::Demon) && card.tier <= 3
    }));
    let tier = g.data.cards.get(g.players[0].discover[0]).tier as i32;
    act(&mut g, 0, Action::Choose(0));
    assert_eq!(g.players[0].health, 30 - tier);
    assert_eq!(g.players[0].last_damage, tier);
    g.check_invariants().unwrap();

    // golden: two Discovers, damage for both
    let mut g = game(24);
    g.players[0].tier = 3;
    g.players[0].health = 30;
    g.give(0, "Imposing Percussionist").unwrap();
    g.players[0].hand[0].golden = true;
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    // the second Demon Discover waits (the Triple Reward is a spell in hand
    // now, not a Discover)
    assert_eq!(g.players[0].discover_queue.len(), 1);
    assert!(g.players[0]
        .discover
        .iter()
        .all(|&c| g.data.cards.get(c).tribes.has(Tribe::Demon)));
    let t1 = g.data.cards.get(g.players[0].discover[0]).tier as i32;
    act(&mut g, 0, Action::Choose(0));
    assert_eq!(g.players[0].discover.len(), 3, "the second Discover follows");
    assert!(g.players[0]
        .discover
        .iter()
        .all(|&c| g.data.cards.get(c).tribes.has(Tribe::Demon)));
    let t2 = g.data.cards.get(g.players[0].discover[0]).tier as i32;
    act(&mut g, 0, Action::Choose(0));
    assert_eq!(g.players[0].health, 30 - t1 - t2);
    g.check_invariants().unwrap();
}

#[test]
fn eredar_escapist_is_damage_counted_and_carries_over() {
    let mut g = game(25);
    put_on_board(&mut g, 0, "Eredar Escapist");
    g.players[0].board[0].improvements = 3;
    put_on_board(&mut g, 0, "Wrath Weaver");
    g.give(0, "Flaming Enforcer").unwrap();
    g.players[0].health = 30;
    act(&mut g, 0, Action::Play { hand: 0, pos: 2 });
    // the demon play is 1 damage: 3 + 1 = 4, so the Cupcakes arrive
    assert!(hand_has(&g, "Corrupted Cupcakes"));
    assert_eq!(g.players[0].board[0].events_left(&g.data.cards), Some(4));
    g.players[0].board[0].improvements = 2;
    // the counter lives on the minion, so the end of the turn leaves it be
    act(&mut g, 0, Action::EndTurn);
    assert_eq!(g.players[0].board[0].improvements, 2);
    g.check_invariants().unwrap();
}

// ---- Demon spells ----

#[test]
fn imp_lusionist_grants_methodical_madness() {
    let db = db();
    let out = fight_with(
        &db,
        &[mk(&db, "Imp-lusionist", false)],
        &[fighter(&db, 50)],
        [Counters::default(); 2],
        20,
        &mut Rng::new(1),
        &mut (),
    );
    let gained = out.sides[0].cards_gained();
    assert_eq!(gained.len(), 1);
    assert_eq!(db.get(gained[0]).name, "Methodical Madness");
    // golden gets two
    let out = fight_with(
        &db,
        &[mk(&db, "Imp-lusionist", true)],
        &[fighter(&db, 50)],
        [Counters::default(); 2],
        20,
        &mut Rng::new(1),
        &mut (),
    );
    assert_eq!(out.sides[0].cards_gained().len(), 2);

    // the spell consumes two Tavern minions, stats and Bonus Keywords
    let mut g = game(26);
    put_on_board(&mut g, 0, "Devout Hellcaller");
    set_shop(&mut g, 0, &["Buzzing Vermin", "Cord Puller"]);
    g.give(0, "Methodical Madness").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    act(&mut g, 0, Action::Target(0));
    // both 1/1s, Taunt and Divine Shield
    assert_eq!(stats(&g.players[0].board[0]), (6, 6));
    assert!(g.players[0].board[0].keywords.has(Keywords::TAUNT));
    assert!(g.players[0].board[0].keywords.has(Keywords::DIVINE_SHIELD));
    assert!(g.players[0].shop.is_empty());
    g.check_invariants().unwrap();
}

#[test]
fn a_taught_methodical_madness_with_no_demon_fizzles() {
    // Magicfin Mycologist teaches the bought spell to a token; casting it
    // with no friendly Demon must not leave a choice that can never be made.
    let mut g = game(28);
    g.players[0].tier = 4;
    g.players[0].gold = 10;
    put_on_board(&mut g, 0, "Magicfin Mycologist");
    g.players[0].shop_spell = Some(id(&g, "Methodical Madness"));
    act(&mut g, 0, Action::BuySpell);
    let token = g.players[0]
        .hand
        .iter()
        .position(|m| g.data.cards.get(m.card).taught_token)
        .unwrap();
    act(&mut g, 0, Action::Play { hand: token as u8, pos: 1 });
    assert!(g.players[0].pending_target.is_none());
    act(&mut g, 0, Action::EndTurn);
    g.check_invariants().unwrap();
}

#[test]
fn corrupted_cupcakes_consumes_three_for_stats_only() {
    let mut g = game(27);
    put_on_board(&mut g, 0, "Devout Hellcaller");
    set_shop(&mut g, 0, &["Buzzing Vermin", "Cord Puller", "Lullabot"]);
    g.give(0, "Corrupted Cupcakes").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    act(&mut g, 0, Action::Target(0));
    // 1/1 + 1/1 + 2/2
    assert_eq!(stats(&g.players[0].board[0]), (4 + 4, 4 + 4));
    assert!(!g.players[0].board[0].keywords.has(Keywords::TAUNT));
    assert!(g.players[0].shop.is_empty());
    g.check_invariants().unwrap();
}

// ---- Devout Hellcaller in the shop ----

#[test]
fn hellcaller_grows_from_wrath_weavers_hero_damage() {
    let mut g = game(37);
    g.players[0].health = 30;
    put_on_board(&mut g, 0, "Wrath Weaver");
    put_on_board(&mut g, 0, "Devout Hellcaller");
    g.give(0, "Ominous Seer").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 2 });
    // the Demon play made Wrath Weaver damage the hero, and the Hellcaller
    // gained its +2/+2 for good right away
    assert_eq!(g.players[0].health, 29);
    assert_eq!(stats(&g.players[0].board[1]), (6, 6));
    g.check_invariants().unwrap();
}

#[test]
fn hellcaller_grows_from_damage_a_rewind_gives_back() {
    let mut g = game(40);
    g.players[0].health = 30;
    put_on_board(&mut g, 0, "Wrath Weaver");
    put_on_board(&mut g, 0, "Soul Rewinder");
    put_on_board(&mut g, 0, "Devout Hellcaller");
    g.give(0, "Ominous Seer").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 3 });
    // Wrath Weaver's damage counted for the Hellcaller before Soul Rewinder
    // gave the Health back: the hero ends unchanged and the Hellcaller grew
    assert_eq!(g.players[0].health, 30);
    assert_eq!(stats(&g.players[0].board[2]), (6, 6));
    g.check_invariants().unwrap();
}

#[test]
fn hellcaller_grows_from_imposing_percussionists_hero_damage() {
    let mut g = game(38);
    g.players[0].tier = 3;
    g.players[0].health = 30;
    put_on_board(&mut g, 0, "Devout Hellcaller");
    g.give(0, "Imposing Percussionist").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 1 });
    let tier = g.data.cards.get(g.players[0].discover[0]).tier as i32;
    act(&mut g, 0, Action::Choose(0));
    assert_eq!(g.players[0].health, 30 - tier);
    assert_eq!(stats(&g.players[0].board[0]), (6, 6));
    g.check_invariants().unwrap();
}

#[test]
fn a_health_cost_is_not_a_demon_dealing_damage() {
    let mut g = game(39);
    g.players[0].gold = 0;
    g.players[0].health = 30;
    put_on_board(&mut g, 0, "Malchezaar, Prince of Dance");
    put_on_board(&mut g, 0, "Devout Hellcaller");
    act(&mut g, 0, Action::Roll);
    assert_eq!(g.players[0].health, 29);
    assert_eq!(stats(&g.players[0].board[1]), (4, 4));
    g.check_invariants().unwrap();
}

// ---- Devout Hellcaller in combat ----

#[test]
fn devout_hellcaller_grows_when_another_demon_deals_damage() {
    let db = db();
    let wall = fighter(&db, 0);
    // Wrath Weaver attacks first and deals 1: the Hellcaller gains +2/+2
    let a = [mk(&db, "Wrath Weaver", false), mk(&db, "Devout Hellcaller", false)];
    let out = fight_with(
        &db,
        &a,
        &[wall],
        [Counters::default(); 2],
        1,
        &mut Rng::new(1),
        &mut (),
    );
    assert_eq!(out.sides[0].permanent_stats[1], (2, 2));
    // the golden Hellcaller gains +4/+4
    let a = [mk(&db, "Wrath Weaver", false), mk(&db, "Devout Hellcaller", true)];
    let out = fight_with(
        &db,
        &a,
        &[wall],
        [Counters::default(); 2],
        1,
        &mut Rng::new(1),
        &mut (),
    );
    assert_eq!(out.sides[0].permanent_stats[1], (4, 4));
    // its own damage is not "another friendly Demon"
    let a = [mk(&db, "Devout Hellcaller", false), mk(&db, "Wrath Weaver", false)];
    let out = fight_with(
        &db,
        &a,
        &[wall],
        [Counters::default(); 2],
        1,
        &mut Rng::new(1),
        &mut (),
    );
    assert!(out.sides[0].permanent_stats.iter().all(|&s| s == (0, 0)));
}

// ---- The run 3 fix ----

#[test]
fn random_murlocs_and_discovers_stay_within_the_players_tier() {
    // Primalfin Lookout at tier 1: only the two tier 1 Murlocs exist
    let mut g = game(28);
    g.players[0].tier = 1;
    put_on_board(&mut g, 0, "Tad");
    g.give(0, "Primalfin Lookout").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 1 });
    assert_eq!(g.players[0].discover.len(), 2);
    assert!(g.players[0].discover.iter().all(|&c| g.data.cards.get(c).tier == 1));
    act(&mut g, 0, Action::Choose(0));

    // Tad: a random Murloc on sell, never above tier 2
    for seed in 0..20 {
        let mut g = game(seed);
        g.players[0].tier = 2;
        put_on_board(&mut g, 0, "Tad");
        act(&mut g, 0, Action::Sell(0));
        let card = g.data.cards.get(g.players[0].hand[0].card);
        assert!(card.tribes.has(Tribe::Murloc));
        assert!(card.tier <= 2, "seed {seed}: {} is tier {}", card.name, card.tier);
    }

    // Cloning Conch: a random Murloc and its copy, up to tier 2
    for seed in 30..50 {
        let mut g = game(seed);
        g.players[0].tier = 2;
        g.give(0, "Cloning Conch").unwrap();
        act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
        assert_eq!(g.players[0].hand[0].card, g.players[0].hand[1].card);
        let card = g.data.cards.get(g.players[0].hand[0].card);
        assert!(card.tribes.has(Tribe::Murloc));
        assert!(card.tier <= 2, "seed {seed}: {} is tier {}", card.name, card.tier);
    }
}

#[test]
fn a_waiting_tavern_pick_takes_the_first_when_the_turn_runs_out() {
    // A Fishbait asks which Tavern card it replaces when there are two.
    let mut g = game(29);
    put_on_board(&mut g, 0, "Lurking Lionfish");
    set_shop(&mut g, 0, &["Buzzing Vermin", "Cord Puller"]);
    turn_of(&mut g, 0);
    g.players[0].gold = 10;
    g.players[0].actions = g.data.rules.max_actions_per_turn - 1;
    g.step(Action::Activate(0)).unwrap();
    // the turn is over and the waiting pick took the left-most Tavern card
    assert!(g.players[0].done);
    assert!(g.players[0].pending_shop_target.is_none());
    let names: Vec<String> = g.players[0]
        .shop
        .iter()
        .map(|m| g.data.cards.get(m.card).name.clone())
        .collect();
    assert!(!names.contains(&"Buzzing Vermin".to_string()), "the left-most card was replaced: {names:?}");
    g.check_invariants().unwrap();
}

#[test]
fn flaming_enforcer_takes_the_leftmost_of_equal_healths() {
    let mut g = game(30);
    put_on_board(&mut g, 0, "Flaming Enforcer");
    set_shop(&mut g, 0, &["Buzzing Vermin", "Cord Puller"]);
    // equal Health: the left-most one is eaten
    act(&mut g, 0, Action::EndTurn);
    let names: Vec<_> = g.players[0]
        .shop
        .iter()
        .map(|m| g.data.cards.get(m.card).name.clone())
        .collect();
    assert_eq!(names, ["Cord Puller"]);
    assert_eq!(stats(&g.players[0].board[0]), (4 + 1, 5 + 1));
    g.check_invariants().unwrap();
}

#[test]
fn malchezaars_uses_come_back_gold_beats_health_and_golden_doubles() {
    // the two uses come back next turn
    let mut g = game(33);
    g.players[0].gold = 0;
    g.players[0].health = 2;
    put_on_board(&mut g, 0, "Malchezaar, Prince of Dance");
    act(&mut g, 0, Action::Roll);
    assert_eq!(g.players[0].health, 1);
    assert!(!is_legal(&mut g, 0, Action::Roll));
    next_turn(&mut g);
    g.players[0].gold = 0;
    assert_eq!(g.players[0].health_refreshes_left(&g.data.cards), 2);
    // with Gold the next Refresh is paid in Gold and keeps the Health uses
    g.players[0].gold = 1;
    act(&mut g, 0, Action::Roll);
    assert_eq!((g.players[0].health, g.players[0].gold), (1, 0));
    assert_eq!(g.players[0].health_refreshes_left(&g.data.cards), 2);
    g.check_invariants().unwrap();

    // a golden refresher has twice the uses
    let mut g = game(34);
    g.players[0].gold = 0;
    put_on_board(&mut g, 0, "Malchezaar, Prince of Dance");
    g.players[0].board[0].golden = true;
    g.players[0].board[0].uses_left = g.players[0].board[0].turn_uses(&g.data.cards);
    assert_eq!(g.players[0].health_refreshes_left(&g.data.cards), 4);
    for _ in 0..4 {
        act(&mut g, 0, Action::Roll);
    }
    assert!(!is_legal(&mut g, 0, Action::Roll));
    g.check_invariants().unwrap();
}

#[test]
fn a_malchezaar_triple_starts_with_fresh_health_refreshes() {
    // the copy spent one of its two refreshes; the golden from the triple
    // has its own four, and they are real uses
    let mut g = game(36);
    g.players[0].health = 30;
    put_on_board(&mut g, 0, "Malchezaar, Prince of Dance");
    g.players[0].gold = 0;
    act(&mut g, 0, Action::Roll);
    assert_eq!(g.players[0].health_refreshes_left(&g.data.cards), 1);
    set_shop(&mut g, 0, &["Malchezaar, Prince of Dance", "Malchezaar, Prince of Dance"]);
    g.players[0].gold = 20;
    act(&mut g, 0, Action::Buy(0));
    act(&mut g, 0, Action::Buy(0)); // the second one triples
    let gold = g.players[0].hand.iter().position(|m| m.golden).unwrap();
    act(&mut g, 0, Action::Play { hand: gold as u8, pos: 0 });
    assert_eq!(g.players[0].health_refreshes_left(&g.data.cards), 4,
        "the golden starts fresh, the copies' spent use does not count");
    let health = g.players[0].health;
    g.players[0].gold = 0;
    act(&mut g, 0, Action::Roll);
    assert_eq!(g.players[0].health, health - 1, "one of the golden's own uses");
    assert_eq!(g.players[0].health_refreshes_left(&g.data.cards), 3);
    g.check_invariants().unwrap();
}

#[test]
fn a_bought_minion_keeps_a_this_turn_tavern_buff_which_the_tavern_loses() {
    let mut g = game(35);
    g.players[0].gold = 0;
    g.players[0].health = 5;
    put_on_board(&mut g, 0, "Malchezaar, Prince of Dance");
    put_on_board(&mut g, 0, "Ashen Corruptor");
    set_shop(&mut g, 0, &["Buzzing Vermin"]);
    // the Health-paid Refresh is rewound and still fires Ashen Corruptor's
    // Tavern buff, which lasts this turn only
    act(&mut g, 0, Action::Roll);
    assert_eq!(g.players[0].health, 5);
    let base = g.data.cards.get(g.players[0].shop[0].card);
    let (ba, bh) = (base.attack, base.health);
    for m in &g.players[0].shop {
        let c = g.data.cards.get(m.card);
        assert_eq!((m.attack, m.health), (c.attack + 2, c.health + 2), "{}", c.name);
    }
    // the bought minion keeps the buff
    g.players[0].gold = 3;
    act(&mut g, 0, Action::Buy(0));
    assert_eq!(stats(&g.players[0].hand[0]), (ba + 2, bh + 2));
    act(&mut g, 0, Action::EndTurn);
    // the Tavern is plain again, the bought minion is not
    turn_of(&mut g, 0);
    for m in &g.players[0].shop {
        let c = g.data.cards.get(m.card);
        assert_eq!((m.attack, m.health), (c.attack, c.health), "{}", c.name);
    }
    assert_eq!(stats(&g.players[0].hand[0]), (ba + 2, bh + 2));
    g.check_invariants().unwrap();
}

#[test]
fn this_turn_and_this_game_tavern_buffs_stack_and_only_the_turn_one_ends() {
    let mut g = game(36);
    g.players[0].gold = 0;
    g.players[0].health = 5;
    put_on_board(&mut g, 0, "Malchezaar, Prince of Dance");
    put_on_board(&mut g, 0, "Ashen Corruptor");
    put_on_board(&mut g, 0, "Devilish Distractor");
    act(&mut g, 0, Action::Roll);
    // the Corruptor's +2/+2 this turn
    for m in &g.players[0].shop {
        let c = g.data.cards.get(m.card);
        assert_eq!((m.attack, m.health), (c.attack + 2, c.health + 2), "{}", c.name);
    }
    // and the Distractor's +1/+2 this game (Fortify cast on the Demon)
    g.players[0].gold = 9;
    g.give(0, "Fortify").unwrap();
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    act(&mut g, 0, Action::Target(2));
    for m in &g.players[0].shop {
        let c = g.data.cards.get(m.card);
        assert_eq!((m.attack, m.health), (c.attack + 3, c.health + 4), "{}", c.name);
    }
    act(&mut g, 0, Action::EndTurn);
    turn_of(&mut g, 0);
    // only the this-game buff is left on the fresh Tavern
    for m in &g.players[0].shop {
        let c = g.data.cards.get(m.card);
        assert_eq!((m.attack, m.health), (c.attack + 1, c.health + 2), "{}", c.name);
    }
    assert_eq!(g.players[0].tavern_buffs.len(), 1);
    g.check_invariants().unwrap();
}

#[test]
fn combat_damage_is_not_rewound_and_triggers_nothing() {
    let mut g = game(37);
    put_on_board(&mut g, 0, "Soul Rewinder");
    put_on_board(&mut g, 0, "Tichondrius");
    let rewinder = stats(&g.players[0].board[0]);
    let tichondrius = stats(&g.players[0].board[1]);
    gang_up_on_player_0(&mut g);
    next_turn(&mut g);
    // the fight is lost: the damage is taken normally (tier 1 plus the one
    // tier-1 survivor), not rewound, and Tichondrius does not react to it
    assert_eq!(g.players[0].health, 28);
    assert_eq!(g.players[0].last_damage, 0);
    assert_eq!(stats(&g.players[0].board[0]), rewinder);
    assert_eq!(stats(&g.players[0].board[1]), tichondrius);
    // and combat kills from 1 Health, where a Health cost could not
    let mut g = game(38);
    put_on_board(&mut g, 0, "Soul Rewinder");
    g.players[0].health = 1;
    gang_up_on_player_0(&mut g);
    next_turn(&mut g);
    assert!(!g.players[0].alive);
    g.check_invariants().unwrap();
}

/// A Fodder-adding Battlecry that runs twice (Brann) gives two Fodders to each
/// of the same three Refreshes, not one to each of six: "Add a Fodder to your
/// next 3 Refreshes" names the Refreshes, so the doubling doubles what each of
/// them gets. The log writes the queue as such (Hearthstone_2026_10_28_22_11_
/// 50-g4 turn 12 step 47: [2, 2, 2], where a golden Assistant's two a Refresh
/// reads the same way).
#[test]
fn a_doubled_fodder_battlecry_adds_two_to_each_refresh() {
    let mut g = game(71);
    g.players[0].gold = 10;
    put_on_board(&mut g, 0, "Brann Bronzebeard");
    g.give(0, "Laboratory Assistant").unwrap();
    let hand = (g.players[0].hand.len() - 1) as u8;
    act(&mut g, 0, Action::Play { hand, pos: 0 });
    assert_eq!(
        g.players[0].fodder_refreshes.as_slice(),
        [2, 2, 2],
        "two Fodders to each of three Refreshes"
    );
    g.check_invariants().unwrap();
}
