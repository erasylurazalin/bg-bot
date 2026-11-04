//! The Pirate tribe, with the real cards from cards.toml: Golden minions and
//! Triple Rewards, gold spent counters, Lockbox, the Bounties, Blade
//! Collector, Ship Master Eudora, Greedy Conniver, Silent Deliverer and the
//! rest of the Pirates.

use std::sync::Arc;

use bgcore::fight::{Watch, fight_with};
use bgcore::{Action, CardDb, CardId, Counters, Data, Game, Keywords, Minion, Rng, Tribe};

fn db() -> CardDb {
    Data::bundled().cards
}

fn lobby(ts: &[Tribe]) -> bgcore::Tribes {
    ts.iter().copied().fold(bgcore::Tribes::NONE, |m, t| m.with(t))
}

fn game(seed: u64) -> Game {
    Game::new_with_lobby(Arc::new(Data::bundled()), seed, lobby(&[Tribe::Beast, Tribe::Mech, Tribe::Murloc, Tribe::Demon, Tribe::Pirate]))
}

fn mk(db: &CardDb, name: &str, golden: bool) -> Minion {
    db.make(db.by_name(name).expect(name), golden)
}

fn id(db: &CardDb, name: &str) -> CardId {
    db.by_name(name).expect(name)
}

fn stats(m: &Minion) -> (i32, i32) {
    (m.attack, m.health)
}

/// A minion that never attacks and takes a long time to kill.
fn wall(db: &CardDb) -> Minion {
    let mut w = mk(db, "Microbot", false);
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

fn lockbox_slot(g: &Game, p: usize) -> Option<usize> {
    let lb = id(&g.data.cards, "Lockbox");
    g.players[p].hand.iter().position(|m| m.card == lb)
}

// ---- fights with a list of deaths ----

fn fight_deaths(
    db: &CardDb,
    a: &[Minion],
    b: &[Minion],
    max_attacks: u32,
    seed: u64,
) -> (bgcore::Outcome, Vec<(usize, CardId)>) {
    struct W(Vec<(usize, CardId)>);
    impl Watch for W {
        fn death(&mut self, side: usize, _pos: usize, m: &Minion) {
            self.0.push((side, m.card));
        }
    }
    let mut w = W(Vec::new());
    let out = fight_with(db, a, b, [Counters::default(); 2], max_attacks, &mut Rng::new(seed), &mut w);
    (out, w.0)
}

// ================= the tribe and the cards =================

#[test]
fn all_the_pirates_are_in_the_pool_with_their_tribes() {
    let db = db();
    let pirates = [
        ("Aureate Laureate", 1),
        ("Southsea Busker", 1),
        ("Bilgewater Breakout", 2),
        ("Clever Castaway", 2),
        ("Surfing Sylvar", 2),
        ("Azsharan Cutlassier", 3),
        ("Greedy Conniver", 3),
        ("Locked-up Mutineer", 3),
        ("Timecap'n Hooktail", 3),
        ("Treasure Parrot", 3),
        ("Bigwig Bandit", 4),
        ("Blade Collector", 4),
        ("Gunpowder Courier", 4),
        ("Lovesick Balladist", 4),
        ("Maritime Extortionist", 4),
        ("Elite Navigator", 5),
        ("Enterprising Escapee", 5),
        ("Proud Privateer", 5),
        ("Ship Master Eudora", 5),
        ("Shipwrecked Rascal", 5),
        ("Hooktusk, Master Marauder", 6),
        ("Silent Deliverer", 6),
        ("Sky Admiral Rogers", 6),
        ("Captain Sanders", 7),
    ];
    for (n, tier) in pirates {
        let c = id(&db, n);
        assert!((c as usize) < db.pool_cards, "{n} is a pool minion");
        assert_eq!(db.get(c).tier, tier, "{n}");
        assert!(db.get(c).tribes.has(Tribe::Pirate), "{n} is a Pirate");
    }
    // the dual tribes keep both
    let hooktail = db.get(id(&db, "Timecap'n Hooktail")).tribes;
    assert!(hooktail.has(Tribe::Dragon) && hooktail.has(Tribe::Pirate));
    let parrot = db.get(id(&db, "Treasure Parrot")).tribes;
    assert!(parrot.has(Tribe::Beast) && parrot.has(Tribe::Pirate));
    // every other card of the old tribes is still not a Pirate
    assert!(!db.get(id(&db, "Wolf Pup")).tribes.has(Tribe::Pirate) || db.get(id(&db, "Wolf Pup")).tribes.is_neutral());
    // an all-type minion is a Pirate too
    assert!(db.get(id(&db, "The Last One Standing")).tribes.has(Tribe::Pirate));
    // the generated and spell cards
    for n in ["Lockbox", "Friendly Bounty", "Healthy Bounty", "Hostile Bounty", "Selfish Bounty", "Wealthy Bounty", "Wave of Gold"] {
        assert!(db.get(id(&db, n)).is_spell(), "{n}");
    }
    let g = game(1);
    g.check_invariants().unwrap();
}

#[test]
fn pirates_in_the_tavern_never_include_tier_7() {
    for seed in 0..50 {
        let g = game(seed);
        for pl in &g.players {
            for m in &pl.shop {
                assert!(g.data.cards.get(m.card).tier <= 6);
            }
        }
    }
}

// ================= Golden minions and Triple Rewards =================

#[test]
fn a_golden_minion_played_from_hand_gives_a_reward_and_counts_for_the_extortionist() {
    let mut g = game(1);
    put_on_board(&mut g, 0, "Maritime Extortionist", false);
    put_in_hand(&mut g, 0, "Cord Puller", true);
    assert_eq!(stats(&g.players[0].board[0]), (7, 7));
    play(&mut g, 0, 0, 1);
    let pl = &g.players[0];
    assert_eq!(pl.golden_played, 1);
    // +7/+7 for the Golden minion played
    assert_eq!(stats(&pl.board[0]), (14, 14));
    // the Triple Reward: a spell in hand whose cast Discovers tier + 1
    assert!(pl.discover.is_empty());
    assert_eq!(pl.hand.len(), 1);
    let reward = pl.hand[0];
    assert_eq!(g.data.cards.get(reward.card).name, "Triple Reward");
    play(&mut g, 0, 0, 0);
    let pl = &g.players[0];
    assert_eq!(pl.discover.len(), 3);
    assert!(pl.discover.iter().all(|&c| g.data.cards.get(c).tier == pl.tier + 1));
    g.check_invariants().unwrap();
}

#[test]
fn the_extortionist_grows_wherever_it_is() {
    let mut g = game(2);
    put_on_board(&mut g, 0, "Maritime Extortionist", false);
    put_in_hand(&mut g, 0, "Maritime Extortionist", false);
    put_in_hand(&mut g, 0, "Cord Puller", true);
    put_in_hand(&mut g, 0, "Cord Puller", true);
    play(&mut g, 0, 2, 1);
    // the Triple Reward spell sits in hand next to the Extortionist
    assert_eq!(stats(&g.players[0].board[0]), (14, 14));
    assert_eq!(stats(&g.players[0].hand[0]), (14, 14));
    play(&mut g, 0, 1, 2);
    assert_eq!(stats(&g.players[0].board[0]), (21, 21));
    assert_eq!(stats(&g.players[0].hand[0]), (21, 21));
    // one drawn later starts with the count
    let c = id(&g.data.cards, "Maritime Extortionist");
    let m = g.players[0].make_card(&g.data.cards, c, false, 0);
    assert_eq!(stats(&m), (21, 21));
    let m = g.players[0].make_card(&g.data.cards, c, true, 0);
    assert_eq!(stats(&m), (14 + 2 * 14, 14 + 2 * 14));
    g.check_invariants().unwrap();
}

#[test]
fn a_minion_made_golden_on_the_board_gives_no_reward_and_does_not_count() {
    let mut g = game(3);
    put_on_board(&mut g, 0, "Maritime Extortionist", false);
    put_on_board(&mut g, 0, "Southsea Busker", false);
    put_in_hand(&mut g, 0, "Elite Navigator", false);
    play(&mut g, 0, 0, 2);
    // two Pirates of tier 4 or below to pick: the Extortionist and the Busker
    assert!(g.players[0].pending_target.is_some());
    act(&mut g, 0, Action::Target(1));
    let pl = &g.players[0];
    assert!(pl.board[1].golden);
    assert_eq!(stats(&pl.board[1]), (6, 2));
    assert!(pl.discover.is_empty(), "no Triple Reward");
    assert_eq!(pl.golden_played, 0, "not played Golden");
    assert_eq!(stats(&pl.board[0]), (7, 7), "the Extortionist did not grow");
    assert!(!pl.board[0].golden);
    g.check_invariants().unwrap();
}

#[test]
fn elite_navigator_only_offers_pirates_of_tier_4_or_below_and_keeps_buffs() {
    let mut g = game(4);
    put_on_board(&mut g, 0, "Southsea Busker", false);
    g.players[0].board[0].attack += 5;
    put_on_board(&mut g, 0, "Shipwrecked Rascal", false); // tier 5: too high
    put_on_board(&mut g, 0, "Cord Puller", false); // not a Pirate
    put_in_hand(&mut g, 0, "Elite Navigator", false);
    play(&mut g, 0, 0, 3);
    // one legal target: it is picked at once
    let pl = &g.players[0];
    assert!(pl.pending_target.is_none());
    assert!(pl.board[0].golden && !pl.board[1].golden && !pl.board[2].golden);
    // printed 3/1 doubled to 6/2 on top of the +5
    assert_eq!(stats(&pl.board[0]), (11, 2));
}

#[test]
fn a_golden_elite_navigator_makes_two_golden() {
    let mut g = game(5);
    put_on_board(&mut g, 0, "Southsea Busker", false);
    put_on_board(&mut g, 0, "Surfing Sylvar", false);
    put_on_board(&mut g, 0, "Blade Collector", false);
    put_in_hand(&mut g, 0, "Elite Navigator", true);
    play(&mut g, 0, 0, 3);
    act(&mut g, 0, Action::Target(0));
    // a Golden minion played gives its Discover; the second pick comes first
    if g.players[0].pending_target.is_some() {
        act(&mut g, 0, Action::Target(1));
    }
    let golden = g.players[0].board.iter().filter(|m| m.golden).count();
    assert_eq!(golden, 3, "two made Golden plus the Navigator itself");
    assert_eq!(g.players[0].golden_played, 1, "only the Navigator was played Golden");
}

#[test]
fn captain_sanders_makes_a_minion_of_tier_6_or_below_golden() {
    let mut g = game(6);
    put_on_board(&mut g, 0, "Cord Puller", false);
    put_on_board(&mut g, 0, "Hooktusk, Master Marauder", false);
    put_in_hand(&mut g, 0, "Captain Sanders", false);
    play(&mut g, 0, 0, 2);
    act(&mut g, 0, Action::Target(1));
    assert!(g.players[0].board[1].golden);
    assert_eq!(stats(&g.players[0].board[1]), (8, 8));
    assert!(g.players[0].discover.is_empty());
    assert!(!g.players[0].board[2].golden);
    g.check_invariants().unwrap();
}

#[test]
fn aureate_laureate_is_always_golden_never_triples_and_gives_no_reward_but_counts() {
    let mut g = game(7);
    put_on_board(&mut g, 0, "Maritime Extortionist", false);
    set_shop(&mut g, 0, &["Aureate Laureate", "Aureate Laureate"]);
    assert!(g.players[0].shop.iter().all(|m| m.golden && m.no_reward));
    assert_eq!(stats(&g.players[0].shop[0]), (2, 2));
    g.players[0].gold = 10;
    act(&mut g, 0, Action::Buy(0));
    act(&mut g, 0, Action::Buy(0));
    put_in_hand(&mut g, 0, "Aureate Laureate", false);
    // three copies in hand: no triple, they stay as they are
    assert_eq!(g.players[0].hand.len(), 3);
    assert!(g.players[0].hand.iter().all(|m| m.golden && stats(m) == (2, 2)));
    play(&mut g, 0, 0, 1);
    let pl = &g.players[0];
    assert!(pl.discover.is_empty(), "no Triple Reward");
    assert_eq!(pl.golden_played, 1, "it was played Golden");
    assert_eq!(stats(&pl.board[0]), (14, 14));
    assert!(pl.board[1].keywords.has(Keywords::DIVINE_SHIELD));
    g.check_invariants().unwrap();
    // selling gives its one copy back
    let before = g.pool[id(&g.data.cards, "Aureate Laureate") as usize];
    act(&mut g, 0, Action::Sell(1));
    assert_eq!(g.pool[id(&g.data.cards, "Aureate Laureate") as usize], before + 1);
    g.check_invariants().unwrap();
}

#[test]
fn silent_deliverer_gets_a_golden_tier_4_minion_that_gives_no_reward() {
    let mut g = game(8);
    put_in_hand(&mut g, 0, "Silent Deliverer", false);
    let before: u32 = g.pool.iter().map(|&n| n as u32).sum();
    play(&mut g, 0, 0, 0);
    let pl = &g.players[0];
    assert_eq!(pl.hand.len(), 1);
    let m = pl.hand[0];
    assert!(m.golden && m.no_reward);
    assert_eq!(g.data.cards.get(m.card).tier, 4);
    assert_eq!(m.pool_copies, 3, "a Golden minion stands for three copies");
    let after: u32 = g.pool.iter().map(|&n| n as u32).sum();
    assert_eq!(before - after, 3);
    g.check_invariants().unwrap();
    // playing it gives no Triple Reward, but counts as a Golden minion played
    // (the flag survives the hand)
    play(&mut g, 0, 0, 1);
    assert!(g.players[0].discover.is_empty());
    assert_eq!(g.players[0].golden_played, 1);
    g.check_invariants().unwrap();
    // a plain Golden minion from hand does give one, as a castable spell
    put_in_hand(&mut g, 0, "Cord Puller", true);
    let last = g.players[0].hand.len() as u8 - 1;
    play(&mut g, 0, last, 0);
    assert!(g.players[0].discover.is_empty());
    assert!(g.players[0]
        .hand
        .iter()
        .any(|m| g.data.cards.get(m.card).name == "Triple Reward"));
}

#[test]
fn a_golden_silent_deliverer_gets_two() {
    let mut g = game(9);
    put_in_hand(&mut g, 0, "Silent Deliverer", true);
    play(&mut g, 0, 0, 0);
    // the Deliverer's own play put a Triple Reward spell in hand (a spell,
    // so it is not one of the no_reward minions counted below)
    let golds: Vec<&Minion> = g.players[0].hand.iter().filter(|m| m.no_reward).collect();
    assert_eq!(golds.len(), 2);
    assert!(golds.iter().all(|m| m.golden && g.data.cards.get(m.card).tier == 4));
    g.check_invariants().unwrap();
}

// ================= gold spent =================

#[test]
fn gold_spent_counters_carry_over_turns_and_ignore_health_and_free_refreshes() {
    let mut g = game(10);
    put_on_board(&mut g, 0, "Gunpowder Courier", false);
    put_on_board(&mut g, 0, "Southsea Busker", false);
    g.players[0].gold = 10;
    let pirate_before = stats(&g.players[0].board[1]);
    // a free Refresh and a Health payment are no gold
    g.players[0].free_rolls = 1;
    act(&mut g, 0, Action::Roll);
    assert_eq!(g.players[0].gold_spent_turn, 0);
    assert_eq!(g.players[0].board[0].improvements, 0);
    let hasty = id(&g.data.cards, "Hasty Excavation");
    g.players[0].tier = 6;
    g.players[0].shop_spell = Some(hasty);
    let health = g.players[0].health;
    act(&mut g, 0, Action::BuySpell);
    assert!(g.players[0].health < health, "paid with Health");
    assert_eq!(g.players[0].gold, 10);
    assert_eq!(g.players[0].gold_spent_turn, 0);
    assert_eq!(g.players[0].board[0].improvements, 0);
    // 3 gold for a minion
    act(&mut g, 0, Action::Buy(0));
    assert_eq!(g.players[0].gold_spent_turn, 3);
    assert_eq!(g.players[0].board[0].improvements, 3);
    assert_eq!(g.players[0].board[0].events_left(&g.data.cards), Some(2));
    // the count carries over the turn
    end_all_turns(&mut g);
    assert_eq!(g.players[0].gold_spent_turn, 0, "this turn's gold resets");
    assert_eq!(g.players[0].board[0].improvements, 3);
    // 2 more gold: 5 spent in all, the Pirates get +3/+1
    act(&mut g, 0, Action::Roll);
    assert_eq!(stats(&g.players[0].board[1]), (pirate_before.0, pirate_before.1));
    act(&mut g, 0, Action::Roll);
    assert_eq!(g.players[0].board[0].improvements, 0);
    assert_eq!(stats(&g.players[0].board[1]), (pirate_before.0 + 3, pirate_before.1 + 1));
    // the Courier is a Pirate itself and gets it too
    assert_eq!(stats(&g.players[0].board[0]), (2 + 3, 5 + 1));
    g.check_invariants().unwrap();
}

#[test]
fn one_big_payment_fires_a_threshold_more_than_once() {
    let mut g = game(11);
    put_on_board(&mut g, 0, "Gunpowder Courier", false);
    // one payment of 12 Gold (a Level) at 5 per trigger: twice, and the
    // countdown takes the overshoot (Era, 2026-11-04): 3 counted, 2 left
    g.players[0].gold = 12;
    g.players[0].upgrade_cost = 12;
    act(&mut g, 0, Action::Level);
    assert_eq!(g.players[0].gold, 0);
    assert_eq!(stats(&g.players[0].board[0]), (2 + 6, 5 + 2));
    assert_eq!(g.players[0].board[0].improvements, 3);
}

#[test]
fn the_thresholds_are_five_gold_for_the_courier_six_for_the_escapee_nine_for_rogers() {
    let db = db();
    let ev = |n: &str| db.get(id(&db, n)).on_event.unwrap().every;
    assert_eq!(ev("Gunpowder Courier"), 5);
    assert_eq!(ev("Enterprising Escapee"), 6);
    assert_eq!(ev("Sky Admiral Rogers"), 9);
}

#[test]
fn a_golden_gunpowder_courier_gives_twice_as_much() {
    let mut g = game(12);
    put_on_board(&mut g, 0, "Gunpowder Courier", true);
    g.players[0].gold = 10;
    for _ in 0..4 {
        act(&mut g, 0, Action::Roll);
    }
    assert_eq!(stats(&g.players[0].board[0]), (4, 10), "not at 4 Gold");
    act(&mut g, 0, Action::Roll);
    assert_eq!(stats(&g.players[0].board[0]), (4 + 6, 10 + 2), "fires at 5 Gold");
}

#[test]
fn lovesick_balladist_reads_the_gold_spent_this_turn() {
    let mut g = game(13);
    put_on_board(&mut g, 0, "Southsea Busker", false);
    put_in_hand(&mut g, 0, "Lovesick Balladist", false);
    g.players[0].gold = 10;
    act(&mut g, 0, Action::Roll);
    act(&mut g, 0, Action::Roll);
    assert_eq!(g.players[0].gold_spent_turn, 2);
    play(&mut g, 0, 0, 1);
    // +2 Health plus 2 for each Gold spent, which is the log's own number:
    // 19 Gold behind a +0/+40 buff and 18 behind a golden +0/+76 (see the
    // card's note in cards.toml)
    assert_eq!(stats(&g.players[0].board[0]), (3, 1 + 2 + 2 * 2));
    // the next turn it starts again from nothing
    end_all_turns(&mut g);
    put_in_hand(&mut g, 0, "Lovesick Balladist", false);
    play(&mut g, 0, 0, 2);
    act(&mut g, 0, Action::Target(0));
    assert_eq!(stats(&g.players[0].board[0]), (3, 7 + 2));
}

#[test]
fn sky_admiral_rogers_and_the_escapee_use_their_own_thresholds() {
    let mut g = game(14);
    put_on_board(&mut g, 0, "Sky Admiral Rogers", false);
    put_on_board(&mut g, 0, "Enterprising Escapee", false);
    g.players[0].gold = 10;
    for _ in 0..6 {
        act(&mut g, 0, Action::Roll);
    }
    // 6 gold: the Escapee gave a Lockbox, Rogers has not fired
    assert!(lockbox_slot(&g, 0).is_some());
    assert_eq!(g.players[0].hand.len(), 1);
    for _ in 0..3 {
        act(&mut g, 0, Action::Roll);
    }
    // 9 gold: a Bounty from Rogers
    assert_eq!(g.players[0].hand.len(), 2);
    let bounty = g.players[0].hand.iter().find(|m| m.card != id(&g.data.cards, "Lockbox")).unwrap();
    assert!(g.data.cards.bounties.contains(&bounty.card));
    g.check_invariants().unwrap();
}

// ================= Lockbox =================

#[test]
fn a_lockbox_counts_down_at_the_end_of_the_owners_turn() {
    let mut g = game(20);
    g.give(0, "Lockbox").unwrap();
    assert_eq!(g.players[0].hand[0].improvements, 5);
    // unplayable and no target
    assert!(!g.is_legal_for_test(0, Action::Play { hand: 0, pos: 0 }));
    end_all_turns(&mut g);
    assert_eq!(g.players[0].hand[0].improvements, 4);
    end_all_turns(&mut g);
    assert_eq!(g.players[0].hand[0].improvements, 3);
    g.check_invariants().unwrap();
}

trait LegalForTest {
    fn is_legal_for_test(&mut self, p: usize, a: Action) -> bool;
}

impl LegalForTest for Game {
    fn is_legal_for_test(&mut self, p: usize, a: Action) -> bool {
        turn_of(self, p);
        self.is_legal(a)
    }
}

#[test]
fn a_second_lockbox_opens_the_first_sooner_instead() {
    let mut g = game(21);
    put_in_hand(&mut g, 0, "Bilgewater Breakout", false);
    play(&mut g, 0, 0, 0);
    // the first Breakout gave a Lockbox
    let i = lockbox_slot(&g, 0).unwrap();
    assert_eq!(g.players[0].hand[i].improvements, 5);
    put_in_hand(&mut g, 0, "Bilgewater Breakout", false);
    let h = g.players[0].hand.len() - 1;
    play(&mut g, 0, h as u8, 1);
    let lockboxes = g.players[0].hand.iter().filter(|m| m.card == id(&g.data.cards, "Lockbox")).count();
    assert_eq!(lockboxes, 1, "still one Lockbox");
    let i = lockbox_slot(&g, 0).unwrap();
    assert_eq!(g.players[0].hand[i].improvements, 4, "opens 1 turn sooner");
    // a golden one opens 2 turns sooner
    put_in_hand(&mut g, 0, "Bilgewater Breakout", true);
    let h = g.players[0].hand.len() - 1;
    play(&mut g, 0, h as u8, 2);
    let i = lockbox_slot(&g, 0).unwrap();
    assert_eq!(g.players[0].hand[i].improvements, 2);
    g.check_invariants().unwrap();
}

#[test]
fn drakkari_enchanter_doubles_the_lockbox_countdown() {
    let mut g = game(22);
    put_on_board(&mut g, 0, "Drakkari Enchanter", false);
    g.give(0, "Lockbox").unwrap();
    end_all_turns(&mut g);
    assert_eq!(g.players[0].hand[0].improvements, 3);
    g.check_invariants().unwrap();
}

#[test]
fn a_lockbox_opens_into_a_golden_minion_of_your_tier_with_a_triple_reward() {
    for seed in 0..40 {
        let mut g = game(100 + seed);
        g.give(0, "Lockbox").unwrap();
        g.players[0].hand[0].improvements = 1;
        end_all_turns(&mut g);
        let pl = &g.players[0];
        assert_eq!(pl.hand.len(), 1, "seed {seed}");
        let m = pl.hand[0];
        let c = g.data.cards.get(m.card);
        assert!(!c.is_spell(), "seed {seed}: it opened");
        // Aureate Laureate's own text says it gives no reward
        assert!(m.golden && (!m.no_reward || c.always_golden), "seed {seed}");
        assert!(!c.tribes.is_neutral(), "seed {seed}: a minion with a type");
        assert!(c.tier <= pl.tier, "seed {seed}: up to your tier");
        assert_eq!(m.pool_copies, 3, "seed {seed}: three copies from the pool");
        g.check_invariants().unwrap();
    }
    // played, it gives a Triple Reward
    let mut g = game(7);
    g.give(0, "Lockbox").unwrap();
    g.players[0].hand[0].improvements = 1;
    end_all_turns(&mut g);
    play(&mut g, 0, 0, 0);
    assert!(g.players[0]
        .hand
        .iter()
        .any(|m| g.data.cards.get(m.card).name == "Triple Reward"));
    assert_eq!(g.players[0].golden_played, 1);
    g.check_invariants().unwrap();
}

#[test]
fn a_lockbox_takes_a_hand_slot_and_a_new_one_is_lost_with_a_full_hand() {
    let mut g = game(24);
    put_on_board(&mut g, 0, "Enterprising Escapee", false);
    for _ in 0..10 {
        g.give(0, "Microbot").unwrap();
    }
    g.players[0].gold = 10;
    for _ in 0..6 {
        act(&mut g, 0, Action::Roll);
    }
    assert_eq!(g.players[0].hand.len(), 10);
    assert!(lockbox_slot(&g, 0).is_none(), "lost");
    // with room it arrives
    g.players[0].hand.pop();
    g.players[0].gold = 10;
    for _ in 0..6 {
        act(&mut g, 0, Action::Roll);
    }
    assert!(lockbox_slot(&g, 0).is_some());
    g.check_invariants().unwrap();
}

#[test]
fn the_mutineer_deathrattle_gets_a_lockbox_or_opens_the_held_one_sooner() {
    let db = db();
    let mut a = mk(&db, "Locked-up Mutineer", false);
    a.attack = 0;
    let mut killer = mk(&db, "Microbot", false);
    killer.attack = 50;
    killer.health = 100;
    let (out, _) = fight_deaths(&db, &[a], &[killer], 1, 1);
    assert_eq!(out.sides[0].lockboxes_gained(), &[1]);
    // golden: 2 turns sooner
    let mut a = mk(&db, "Locked-up Mutineer", true);
    a.attack = 0;
    let mut killer = mk(&db, "Microbot", false);
    killer.attack = 50;
    killer.health = 100;
    let (out, _) = fight_deaths(&db, &[a], &[killer], 1, 1);
    assert_eq!(out.sides[0].lockboxes_gained(), &[2]);

    // in a game: the first Mutineer's death gives the box, the second one's
    // opens it 1 turn sooner
    let mut g = game(25);
    let q = match g.players[0].opponent {
        Some(bgcore::Opponent::Player(q)) => q as usize,
        other => panic!("paired with {other:?}"),
    };
    for p in 0..8 {
        g.players[p].board.clear();
    }
    put_on_board(&mut g, 0, "Locked-up Mutineer", false);
    put_on_board(&mut g, 0, "Locked-up Mutineer", false);
    g.players[0].board[0].attack = 0;
    g.players[0].board[1].attack = 0;
    let mut big = g.players[q].make_card(&g.data.cards, id(&g.data.cards, "Microbot"), false, 0);
    big.attack = 100;
    big.health = 10000;
    g.players[q].board.push(big);
    end_all_turns(&mut g);
    let i = lockbox_slot(&g, 0).expect("a Lockbox");
    assert_eq!(g.players[0].hand[i].improvements, 4);
    assert_eq!(g.players[0].hand.len(), 1);
    g.check_invariants().unwrap();
}

// ================= Bounties =================

#[test]
fn the_five_bounties_are_one_slot_of_the_spell_draw() {
    let db = db();
    assert_eq!(db.bounties.len(), 5);
    assert_eq!(db.spells.iter().filter(|&&s| db.get(s).is_bounty).count(), 1);
    assert!(!db.spells.contains(&id(&db, "Lockbox")), "Lockbox is never drawn");
    let mut rng = Rng::new(5);
    let mut seen = [0u32; 5];
    let mut bounty_draws = 0u32;
    let n = 20000;
    for _ in 0..n {
        let s = db.random_spell(6, bgcore::Tribes::ALL, &mut rng).unwrap();
        if let Some(k) = db.bounties.iter().position(|&b| b == s) {
            seen[k] += 1;
            bounty_draws += 1;
        }
    }
    assert!(seen.iter().all(|&c| c > 0), "all five appear: {seen:?}");
    let spells_up_to_6 = db.spells.len() as f64;
    let expect = n as f64 / spells_up_to_6;
    assert!((bounty_draws as f64 - expect).abs() < expect * 0.3, "one slot: {bounty_draws} vs {expect}");
    // never in the Tavern below tier 3
    for _ in 0..500 {
        let s = db.random_spell(2, bgcore::Tribes::ALL, &mut rng).unwrap();
        assert!(!db.get(s).is_bounty);
    }
    // a Discover never offers the same Bounty twice
    for seed in 0..200 {
        let mut g = game(seed);
        g.players[0].tier = 6;
        let mut offered = Vec::new();
        let mut rng = Rng::new(seed);
        for _ in 0..3 {
            let s = db.random_spell_up_to(6, &offered, bgcore::Tribes::ALL, &mut rng).unwrap();
            assert!(!offered.contains(&s));
            offered.push(s);
        }
    }
}

#[test]
fn bigwig_bandit_and_the_rascal_get_random_bounties_from_the_fight() {
    let db = db();
    let mut killer = mk(&db, "Microbot", false);
    killer.attack = 50;
    killer.health = 100;
    // Rally: the Bandit attacks a wall
    let out = fight_with(
        &db,
        &[mk(&db, "Bigwig Bandit", false)],
        &[wall(&db)],
        [Counters::default(); 2],
        1,
        &mut Rng::new(1),
        &mut (),
    );
    let got = out.sides[0].cards_gained();
    assert_eq!(got.len(), 1);
    assert!(db.bounties.contains(&got[0]));
    // golden: two
    let out = fight_with(
        &db,
        &[mk(&db, "Bigwig Bandit", true)],
        &[wall(&db)],
        [Counters::default(); 2],
        1,
        &mut Rng::new(1),
        &mut (),
    );
    assert_eq!(out.sides[0].cards_gained().len(), 2);
    // Deathrattle of the Rascal
    let mut r = mk(&db, "Shipwrecked Rascal", false);
    r.attack = 0;
    let (out, _) = fight_deaths(&db, &[r], &[killer], 1, 1);
    assert_eq!(out.sides[0].cards_gained().len(), 1);
    assert!(db.bounties.contains(&out.sides[0].cards_gained()[0]));
    // every Bounty can come up
    let mut seen = [false; 5];
    for seed in 0..100 {
        let out = fight_with(
            &db,
            &[mk(&db, "Bigwig Bandit", false)],
            &[wall(&db)],
            [Counters::default(); 2],
            1,
            &mut Rng::new(seed),
            &mut (),
        );
        let k = db.bounties.iter().position(|&b| b == out.sides[0].cards_gained()[0]).unwrap();
        seen[k] = true;
    }
    assert!(seen.iter().all(|&s| s));
}

#[test]
fn the_rascal_battlecry_and_golden_get_bounties_in_the_shop() {
    let mut g = game(30);
    put_in_hand(&mut g, 0, "Shipwrecked Rascal", true);
    play(&mut g, 0, 0, 0);
    let bounties = g.players[0].hand.iter().filter(|m| g.data.cards.bounties.contains(&m.card)).count();
    assert_eq!(bounties, 2);
}

fn cast(g: &mut Game, name: &str) {
    let h = g.give(0, name).map(|_| g.players[0].hand.len() - 1).unwrap();
    play(g, 0, h as u8, 0);
}

#[test]
fn the_bounties_do_what_they_say() {
    // Wealthy: 2 Gold
    let mut g = game(31);
    g.players[0].gold = 0;
    cast(&mut g, "Wealthy Bounty");
    assert_eq!(g.players[0].gold, 2);
    // Selfish: the left-most minion +6/+6
    let mut g = game(32);
    put_on_board(&mut g, 0, "Cord Puller", false);
    put_on_board(&mut g, 0, "Southsea Busker", false);
    cast(&mut g, "Selfish Bounty");
    assert_eq!(stats(&g.players[0].board[0]), (7, 7));
    assert_eq!(stats(&g.players[0].board[1]), (3, 1));
    // Healthy and Hostile: four different minions
    for (n, d) in [("Healthy Bounty", (0, 4)), ("Hostile Bounty", (4, 0))] {
        for seed in 0..20 {
            let mut g = game(200 + seed);
            for _ in 0..6 {
                put_on_board(&mut g, 0, "Southsea Busker", false);
            }
            cast(&mut g, n);
            let hit = g.players[0].board.iter().filter(|m| stats(m) == (3 + d.0, 1 + d.1)).count();
            assert_eq!(hit, 4, "{n} seed {seed}");
            let plain = g.players[0].board.iter().filter(|m| stats(m) == (3, 1)).count();
            assert_eq!(plain, 2);
        }
        // fewer than four minions: all of them, once each
        let mut g = game(33);
        put_on_board(&mut g, 0, "Southsea Busker", false);
        put_on_board(&mut g, 0, "Southsea Busker", false);
        cast(&mut g, n);
        assert!(g.players[0].board.iter().all(|m| stats(m) == (3 + d.0, 1 + d.1)));
    }
    // Wave of Gold: +3/+2, Golden ones another +3/+2
    let mut g = game(34);
    put_on_board(&mut g, 0, "Cord Puller", false);
    put_on_board(&mut g, 0, "Cord Puller", true);
    cast(&mut g, "Wave of Gold");
    assert_eq!(stats(&g.players[0].board[0]), (4, 3));
    assert_eq!(stats(&g.players[0].board[1]), (2 + 6, 2 + 4));
    // the Tavern spell bonus applies (Azsharan Cutlassier's battlecry)
    let mut g = game(35);
    put_on_board(&mut g, 0, "Azsharan Cutlassier", false);
    g.players[0].spell_bonus = (1, 0);
    cast(&mut g, "Selfish Bounty");
    assert_eq!(stats(&g.players[0].board[0]), (6 + 6 + 1, 4 + 6));
}

#[test]
fn friendly_bounty_gets_a_minion_of_the_most_common_type() {
    for seed in 0..30 {
        let mut g = game(300 + seed);
        g.players[0].tier = 6;
        for n in ["Buzzing Vermin", "Flittering Bat", "Cord Puller"] {
            put_on_board(&mut g, 0, n, false);
        }
        cast(&mut g, "Friendly Bounty");
        let m = g.players[0].hand[0];
        assert!(g.data.cards.get(m.card).tribes.has(Tribe::Beast), "seed {seed}");
        assert_eq!(m.pool_copies, 0, "generated, not from the pool");
        g.check_invariants().unwrap();
    }
    // nothing without a typed minion
    let mut g = game(40);
    put_on_board(&mut g, 0, "Suspicious Prisonguard", false);
    cast(&mut g, "Friendly Bounty");
    assert!(g.players[0].hand.is_empty());
    // a Pirate board gets a Pirate
    let mut g = game(41);
    g.players[0].tier = 6;
    put_on_board(&mut g, 0, "Southsea Busker", false);
    cast(&mut g, "Friendly Bounty");
    assert!(g.data.cards.get(g.players[0].hand[0].card).tribes.has(Tribe::Pirate));
}

#[test]
fn proud_privateer_doubles_bounties_and_two_do_not_triple_them() {
    let gold_after = |privateers: &[bool]| {
        let mut g = game(50);
        for &gold in privateers {
            put_on_board(&mut g, 0, "Proud Privateer", gold);
        }
        g.players[0].gold = 0;
        cast(&mut g, "Wealthy Bounty");
        g.players[0].gold
    };
    assert_eq!(gold_after(&[]), 2);
    assert_eq!(gold_after(&[false]), 4);
    assert_eq!(gold_after(&[false, false]), 4, "twice is a fixed count");
    assert_eq!(gold_after(&[true]), 6, "golden: three times");
    assert_eq!(gold_after(&[true, false]), 6);
    // Selfish Bounty twice
    let mut g = game(51);
    put_on_board(&mut g, 0, "Cord Puller", false);
    put_on_board(&mut g, 0, "Proud Privateer", false);
    cast(&mut g, "Selfish Bounty");
    assert_eq!(stats(&g.players[0].board[0]), (13, 13));
    // not other spells
    let mut g = game(52);
    put_on_board(&mut g, 0, "Proud Privateer", false);
    put_on_board(&mut g, 0, "Cord Puller", false);
    cast(&mut g, "Wave of Gold");
    assert_eq!(stats(&g.players[0].board[1]), (4, 3));
}

// ================= fights =================

#[test]
fn blade_collector_also_damages_the_neighbours_of_whomever_it_attacks() {
    let db = db();
    let enemy = |h: i32| {
        let mut m = mk(&db, "Microbot", false);
        m.attack = 0;
        m.health = h;
        m
    };
    // two enemies: whichever is attacked, the other is next to it
    for seed in 0..10 {
        let (out, deaths) = fight_deaths(&db, &[mk(&db, "Blade Collector", false)], &[enemy(3), enemy(3)], 1, seed);
        assert_eq!(deaths.len(), 2, "seed {seed}");
        assert_eq!(out.winner, Some(0));
    }
    // a minion without Cleave kills only the one it attacks
    let mut plain = mk(&db, "Cord Puller", false);
    plain.attack = 3;
    plain.keywords = Keywords::default();
    let (_, deaths) = fight_deaths(&db, &[plain], &[enemy(3), enemy(3)], 1, 1);
    assert_eq!(deaths.len(), 1);
    // Divine Shields on a neighbour pop and no more
    for seed in 0..10 {
        let mut shielded = enemy(3);
        shielded.keywords.set(Keywords::DIVINE_SHIELD);
        let (_, deaths) = fight_deaths(&db, &[mk(&db, "Blade Collector", false)], &[shielded, enemy(3)], 1, seed);
        assert_eq!(deaths.len(), 1, "seed {seed}: only the unshielded one dies");
        assert_eq!(deaths[0].0, 1);
    }
}

#[test]
fn a_venomous_blade_collector_kills_every_neighbour_it_damages() {
    let db = db();
    let wall = |_: usize| wall(&db);
    let mut counts = std::collections::BTreeSet::new();
    for seed in 0..40 {
        let mut c = mk(&db, "Blade Collector", false);
        c.keywords.set(Keywords::VENOMOUS);
        let (_, deaths) = fight_deaths(&db, &[c], &[wall(0), wall(1), wall(2)], 1, seed);
        counts.insert(deaths.len());
        assert!(deaths.len() == 2 || deaths.len() == 3, "seed {seed}: {}", deaths.len());
    }
    assert_eq!(counts.len(), 2, "an edge attack kills two, a middle one all three");
    // a shielded neighbour only loses its shield: Venomous does nothing there
    for seed in 0..20 {
        let mut c = mk(&db, "Blade Collector", false);
        c.keywords.set(Keywords::VENOMOUS);
        let mut shielded = wall(0);
        shielded.keywords.set(Keywords::DIVINE_SHIELD);
        let (_, deaths) = fight_deaths(&db, &[c], &[shielded, wall(1)], 1, seed);
        assert_eq!(deaths.len(), 1, "seed {seed}");
        assert_eq!(deaths[0].0, 1);
    }
}

#[test]
fn blade_collector_deaths_run_deathrattles_and_the_neighbours_do_not_hit_back() {
    let db = db();
    let mut rat = mk(&db, "Cord Puller", false); // Deathrattle: a Microbot
    rat.keywords = Keywords::default();
    rat.attack = 0;
    rat.health = 3;
    let mut other = rat;
    other.health = 1000;
    other.attack = 40; // would kill the Collector if it hit back
    let mut c = mk(&db, "Blade Collector", false);
    c.health = 10;
    // the Collector attacks one of the two; the one that is not the target
    // is a neighbour and must not deal damage to it
    for seed in 0..10 {
        let mut hits_on_collector = 0;
        struct W(Vec<i32>);
        impl Watch for W {
            fn attack(&mut self, _s: usize, _a: &Minion, d: &Minion) {
                self.0.push(d.attack);
            }
        }
        let mut w = W(Vec::new());
        let _ = fight_with(&db, &[c], &[rat, other], [Counters::default(); 2], 1, &mut Rng::new(seed), &mut w);
        hits_on_collector += w.0.len();
        assert_eq!(hits_on_collector, 1);
    }
    let (_, deaths) = fight_deaths(&db, &[c], &[rat, rat], 1, 1);
    assert_eq!(deaths.len(), 2);
}

#[test]
fn ship_master_eudora_buffs_all_for_the_fight_and_golden_minions_keep_it() {
    let db = db();
    // it attacks first and dies to the 50 Attack minion
    let mut eudora = mk(&db, "Ship Master Eudora", false);
    eudora.attack = 1;
    eudora.health = 1;
    let plain = mk(&db, "Southsea Busker", false);
    let golden = mk(&db, "Southsea Busker", true);
    let mut killer = mk(&db, "Microbot", false);
    killer.attack = 50;
    killer.health = 1000;
    let (out, _) = fight_deaths(&db, &[eudora, plain, golden], &[killer], 1, 3);
    let perm = &out.sides[0].permanent_stats;
    assert_eq!(perm[1], (0, 0), "the plain minion keeps nothing");
    assert_eq!(perm[2], (6, 6), "the Golden one keeps +6/+6");
    // a Golden Eudora does it twice
    let mut eudora = mk(&db, "Ship Master Eudora", true);
    eudora.attack = 1;
    eudora.health = 1;
    let (out, _) = fight_deaths(&db, &[eudora, plain, golden], &[killer], 1, 3);
    let perm = &out.sides[0].permanent_stats;
    assert_eq!(perm[1], (0, 0));
    assert_eq!(perm[2], (12, 12));

}

#[test]
fn eudoras_permanent_buff_reaches_the_real_board_after_the_fight() {
    let mut g = game(60);
    let q = match g.players[0].opponent {
        Some(bgcore::Opponent::Player(q)) => q as usize,
        other => panic!("paired with {other:?}"),
    };
    for p in 0..8 {
        g.players[p].board.clear();
    }
    put_on_board(&mut g, 0, "Ship Master Eudora", false);
    g.players[0].board[0].attack = 1;
    g.players[0].board[0].health = 1;
    put_on_board(&mut g, 0, "Southsea Busker", true);
    put_on_board(&mut g, 0, "Southsea Busker", false);
    let mut killer = g.players[q].make_card(&g.data.cards, id(&g.data.cards, "Microbot"), false, 0);
    killer.attack = 100;
    killer.health = 10000;
    g.players[q].board.push(killer);
    end_all_turns(&mut g);
    let b = &g.players[0].board;
    assert_eq!(name(&g, &b[0]), "Ship Master Eudora");
    assert_eq!(stats(&b[1]), (6 + 6, 2 + 6), "the Golden Busker keeps +6/+6");
    assert_eq!(stats(&b[2]), (3, 1), "the plain one keeps nothing");
    g.check_invariants().unwrap();
}

#[test]
fn greedy_conniver_sold_golden_discovers_a_tier_7_minion() {
    let mut g = game(70);
    put_on_board(&mut g, 0, "Greedy Conniver", true);
    act(&mut g, 0, Action::Sell(0));
    let pl = &g.players[0];
    assert_eq!(pl.discover.len(), 3);
    assert!(pl.discover.iter().all(|&c| g.data.cards.get(c).tier == 7));
    g.check_invariants().unwrap();
    act(&mut g, 0, Action::Choose(1));
    assert_eq!(g.players[0].hand.len(), 1);
    assert_eq!(g.data.cards.get(g.players[0].hand[0].card).tier, 7);
    g.check_invariants().unwrap();

    // not golden: nothing
    let mut g = game(71);
    put_on_board(&mut g, 0, "Greedy Conniver", false);
    act(&mut g, 0, Action::Sell(0));
    assert!(g.players[0].discover.is_empty());
    g.check_invariants().unwrap();
}

#[test]
fn hooktusk_buffs_other_pirates_after_a_discover_improved_by_golden_minions_played() {
    let mut g = game(80);
    put_on_board(&mut g, 0, "Hooktusk, Master Marauder", false);
    put_on_board(&mut g, 0, "Southsea Busker", false);
    put_on_board(&mut g, 0, "Cord Puller", false);
    g.discover_tier7_minion(0);
    act(&mut g, 0, Action::Choose(0));
    assert_eq!(stats(&g.players[0].board[0]), (4, 4), "not itself");
    assert_eq!(stats(&g.players[0].board[1]), (4, 2), "+1/+1");
    assert_eq!(stats(&g.players[0].board[2]), (1, 1), "not a Pirate");
    // two Golden minions played: +3/+3
    g.players[0].golden_played = 2;
    g.discover_tier7_minion(0);
    act(&mut g, 0, Action::Choose(0));
    assert_eq!(stats(&g.players[0].board[1]), (7, 5));
    g.check_invariants().unwrap();
}

#[test]
fn surfing_sylvar_repeats_for_each_friendly_golden_minion() {
    let mut g = game(90);
    put_on_board(&mut g, 0, "Cord Puller", false);
    put_on_board(&mut g, 0, "Surfing Sylvar", false);
    put_on_board(&mut g, 0, "Cord Puller", false);
    put_on_board(&mut g, 0, "Cord Puller", true);
    let before: Vec<(i32, i32)> = g.players[0].board.iter().map(stats).collect();
    end_all_turns(&mut g);
    let after: Vec<(i32, i32)> = g.players[0].board.iter().map(stats).collect();
    // once, and once more for the one Golden minion
    assert_eq!(after[0].0, before[0].0 + 2);
    assert_eq!(after[2].0, before[2].0 + 2);
    assert_eq!(after[3], before[3], "not adjacent");
    assert_eq!(after[1], before[1], "not itself");
    // hand Golden minions do not count
    let mut g = game(91);
    put_on_board(&mut g, 0, "Cord Puller", false);
    put_on_board(&mut g, 0, "Surfing Sylvar", false);
    put_in_hand(&mut g, 0, "Cord Puller", true);
    end_all_turns(&mut g);
    assert_eq!(g.players[0].board[0].attack, 1 + 1);
}

#[test]
fn azsharan_cutlassier_raises_the_tavern_spell_bonus() {
    let mut g = game(95);
    put_in_hand(&mut g, 0, "Azsharan Cutlassier", false);
    play(&mut g, 0, 0, 0);
    assert_eq!(g.players[0].spell_bonus, (1, 0));
    put_in_hand(&mut g, 0, "Azsharan Cutlassier", true);
    play(&mut g, 0, 0, 1);
    assert_eq!(g.players[0].spell_bonus, (3, 0));
}

#[test]
fn southsea_busker_and_clever_castaway() {
    let mut g = game(96);
    put_in_hand(&mut g, 0, "Southsea Busker", false);
    play(&mut g, 0, 0, 0);
    assert_eq!(g.players[0].gold_next_turn, 1);
    put_in_hand(&mut g, 0, "Clever Castaway", false);
    g.players[0].tier = 3;
    play(&mut g, 0, 0, 1);
    g.players[0].gold = 5;
    act(&mut g, 0, Action::Activate(1));
    assert_eq!(g.players[0].discover.len(), 3);
    assert!(g.players[0].discover.iter().all(|&c| g.data.cards.get(c).is_spell()));
    act(&mut g, 0, Action::Choose(0));
    assert_eq!(g.players[0].gold, 3);
    assert_eq!(g.players[0].gold_spent_turn, 2);
}

#[test]
fn treasure_parrot_and_hooktail_keep_their_first_tribe_rules() {
    let g = game(97);
    let hooktail = g.data.cards.get(id(&g.data.cards, "Timecap'n Hooktail"));
    assert!(hooktail.on_cast_spell.is_some());
    let parrot = g.data.cards.get(id(&g.data.cards, "Treasure Parrot"));
    assert!(parrot.damage_milestone.is_some());
}

#[test]
fn a_whole_game_with_pirates_keeps_the_pool_exact() {
    use bgcore::bots::random_action;
    for seed in 0..20 {
        let mut g = game(500 + seed);
        let mut rng = Rng::new(seed);
        let mut steps = 0;
        while !g.over && steps < 20000 {
            let a = random_action(&g, &mut rng);
            g.step(a).unwrap();
            g.check_invariants().unwrap();
            steps += 1;
        }
    }
}


// ================= Lockbox in the fight, spells cast twice =================

#[test]
fn a_lockbox_opens_before_the_fight_and_its_minion_can_be_summoned_from_hand() {
    use std::cell::Cell;
    // Diremuck Forager summons a Murloc from hand at the start of combat
    let db = db();
    let data = Arc::new(Data::bundled());
    let mut found = false;
    for seed in 0..60u64 {
        let mut g = Game::new_sandbox(data.clone(), seed, Some(bgcore::Tribes::of(Tribe::Murloc)), 0);
        put_on_board(&mut g, 0, "Diremuck Forager", false);
        g.give(0, "Lockbox").unwrap();
        // got on turn 1: 5, then 4 after turn 1, 3, 2, 1: opens at the end of turn 5
        assert_eq!(g.players[0].hand[0].improvements, 5);
        for turn in 1..=5u32 {
            assert_eq!(g.turn, turn);
            let left = g.players[0].hand[0].improvements;
            assert_eq!(left as u32, 6 - turn);
            g.step(Action::EndTurn).unwrap();
            if turn < 5 {
                assert_eq!(g.players[0].hand[0].improvements as u32, 5 - turn);
            }
        }
        let fight = g.sandbox.as_ref().unwrap().last_fight.as_ref().unwrap();
        assert_eq!(fight.turn, 5);
        // the minion was in hand when the fight began
        assert_eq!(fight.hand.len(), 1, "seed {seed}");
        let m = fight.hand[0];
        assert!(m.golden && !db.get(m.card).is_spell());
        if !db.get(m.card).tribes.has(Tribe::Murloc) {
            continue;
        }
        found = true;
        // the same board and hand in a fight: the Murloc is summoned from hand
        let summoned = Cell::new(0);
        struct W<'a>(&'a Cell<u32>);
        impl Watch for W<'_> {
            fn summon(&mut self, side: usize, _pos: usize, _m: &Minion) {
                if side == 0 {
                    self.0.set(self.0.get() + 1);
                }
            }
        }
        let mut w = W(&summoned);
        let hands: [&[Minion]; 2] = [&fight.hand, &fight.hand];
        let _ = bgcore::fight::fight_with_hands(
            &db,
            &fight.board,
            &[wall(&db)],
            hands,
            [Counters::default(); 2],
            3,
            &mut Rng::new(seed),
            &mut w,
        );
        assert!(summoned.get() >= 1, "seed {seed}: summoned from hand");
        // it can't be played yet: it is in hand after the fight, as normal
        assert_eq!(g.players[0].hand.len(), 1);
        break;
    }
    assert!(found, "a Murloc came out of some Lockbox");
}

#[test]
fn a_bounty_cast_twice_is_two_spells_for_every_counter_and_trigger() {
    let mut g = game(110);
    put_on_board(&mut g, 0, "Proud Privateer", false);
    put_on_board(&mut g, 0, "Felboar", false); // every 3 spells cast
    put_on_board(&mut g, 0, "Timecap'n Hooktail", false); // +1 Attack per spell
    let hooktail_attack = g.players[0].board[2].attack;
    cast(&mut g, "Wealthy Bounty");
    assert_eq!(g.players[0].board[1].improvements, 2, "two spells cast");
    // Hooktail gave its Attack to every minion twice
    assert_eq!(g.players[0].board[2].attack, hooktail_attack + 2);
    // a third cast (two more) fires the Felboar's every-3 once and keeps 1
    cast(&mut g, "Wealthy Bounty");
    assert_eq!(g.players[0].board[1].improvements, 1);
    // without a Privateer a Bounty is one spell
    let mut g = game(111);
    put_on_board(&mut g, 0, "Felboar", false);
    cast(&mut g, "Wealthy Bounty");
    assert_eq!(g.players[0].board[0].improvements, 1);
}

/// The Gold an Activate pays is counted on the click, but the cards that watch
/// Gold wait for the choice the Activate opened. The log's Sky Admiral Rogers
/// counts its ninth Gold on the Activate and hands its Bounty over at the
/// pick: its numbers go `[2, 0, 9, 0]` to `[9, 1, 9, 0]` on the Activate
/// (`Hearthstone_2026_10_26_00_48_15-g3` turn 7 step 29) and back to
/// `[9, 0, 9, 0]` with the Bounty in the hand at the pick (step 30).
#[test]
fn an_activates_gold_reaches_the_cards_that_watch_it_at_the_pick() {
    let mut g = game(23);
    put_on_board(&mut g, 0, "Sky Admiral Rogers", false);
    put_on_board(&mut g, 0, "Clever Castaway", false);
    g.players[0].gold = 10;
    // Two Gold short of its ninth: this Activate is the one that pays out.
    g.players[0].board[0].improvements = 7;
    let hand = g.players[0].hand.len();
    act(&mut g, 0, Action::Activate(1));
    assert_eq!(g.players[0].hand.len(), hand, "the Bounty waits for the pick");
    assert!(!g.players[0].discover.is_empty(), "the Activate opened a Discover");
    act(&mut g, 0, Action::Choose(0));
    assert_eq!(
        g.players[0].board[0].improvements,
        0,
        "the ninth Gold is counted once the Activate is done"
    );
    assert_eq!(
        g.players[0].hand.len(),
        hand + 2,
        "the discovered spell and the Bounty it waited for"
    );
    g.check_invariants().unwrap();
}
