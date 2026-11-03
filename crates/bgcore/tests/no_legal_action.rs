//! Regression tests for the "no legal action" audit (docs/notes/KNOWN-GAPS.md):
//! a pending choice nothing can answer fizzles, a Choose One with no
//! choosable option is skipped, the action limit picks a choosable option, a
//! Tavern pick fizzles when the Tavern empties, and a Discover that finds
//! nothing clears its side flags.
//!
//! No current card can reach any of these states, so each test builds the
//! state by hand. The Choose One and Discover tests need cards no real card
//! set provides (a Choose One minion whose options can't be chosen, a
//! Discover with nothing left to offer), so they play a small card set.

use std::sync::Arc;

use bgcore::cards::{ChooseOption, SpellTarget};
use bgcore::game::{DiscoverRequest, PendingChooseOne, PendingTarget, ShopTargetKind, TargetKind};
use bgcore::{Action, Data, Game, Minion, N_ACTIONS, Tribe};

fn lobby(ts: &[Tribe]) -> bgcore::Tribes {
    ts.iter().copied().fold(bgcore::Tribes::NONE, |m, t| m.with(t))
}

/// The real card set, with a lobby without Demons.
fn game(seed: u64) -> Game {
    let ts = [Tribe::Beast, Tribe::Mech, Tribe::Murloc, Tribe::Pirate, Tribe::Undead];
    Game::new_with_lobby(Arc::new(Data::bundled()), seed, lobby(&ts))
}

/// A game on a hand-written card set, for the states no real card provides.
fn custom(cards: &str, seed: u64) -> Game {
    let data = Data::from_toml(cards, include_str!("../../../rules.toml"))
        .expect("the test card set parses");
    Game::new_with_lobby(Arc::new(data), seed, lobby(&[Tribe::Beast]))
}

/// The smallest card set that plays: one minion, with `fields` as its extra
/// fields, a token to put in hand or on the board for free, and the Triple
/// Reward spell every card set needs.
fn card_set(name: &str, fields: &str) -> String {
    format!(
        r#"
[[minion]]
name = "{name}"
tier = 1
tribe = "neutral"
stats = [1, 1]
{fields}

[[token]]
name = "Test Token"
tier = 1
tribe = "neutral"
stats = [1, 1]

[[spell]]
name = "Triple Reward"
tier = 0
cost = 0
token = true
tavern_spell = false
effect = {{ triple_reward = true }}
"#
    )
}

/// Let the other players toggle freeze until it's `p`'s action.
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

fn legal(g: &Game) -> Vec<Action> {
    let mut mask = [false; N_ACTIONS];
    g.legal_mask(&mut mask);
    (0..N_ACTIONS).filter(|&i| mask[i]).map(|i| Action::from_index(i).unwrap()).collect()
}

fn put_on_board(g: &mut Game, p: usize, name: &str) {
    let c = g.data.cards.by_name(name).unwrap();
    let slot = g.data.cards.get(c).pool_slot as usize;
    g.pool[slot] -= 1;
    let m = g.players[p].make_card(&g.data.cards, c, false, 1);
    g.players[p].board.push(m);
}

fn put_in_hand(g: &mut Game, p: usize, name: &str) {
    let c = g.data.cards.by_name(name).unwrap();
    let slot = g.data.cards.get(c).pool_slot as usize;
    g.pool[slot] -= 1;
    let m = g.players[p].make_card(&g.data.cards, c, false, 1);
    g.players[p].hand.push(m);
}

/// Put a token on the board or in hand: tokens hold no pool copy, so eight
/// players' shops can never run the pool dry first.
fn put_token(g: &mut Game, p: usize, name: &str, on_board: bool) {
    let c = g.data.cards.by_name(name).unwrap();
    let m = g.players[p].make_card(&g.data.cards, c, false, 0);
    if on_board {
        g.players[p].board.push(m);
    } else {
        g.players[p].hand.push(m);
    }
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
        let c = g.data.cards.by_name(n).unwrap();
        let slot = g.data.cards.get(c).pool_slot as usize;
        g.pool[slot] -= 1;
        let m = g.players[p].make_card(&g.data.cards, c, false, 1);
        g.players[p].shop.push(m);
    }
}

fn stats(m: &Minion) -> (i32, i32) {
    (m.attack, m.health)
}

/// A Choose One option that needs a friendly minion (on the board or in the
/// Tavern), so it can't be chosen with both empty.
fn needs_a_minion(attack: i32, health: i32) -> ChooseOption {
    ChooseOption::SpellTarget(SpellTarget::Buff {
        attack,
        health,
        keyword: 0,
        times: 1,
        if_win_next_turn_buff: None,
        tribe: None,
        random_keyword: false,
    })
}

// ---- 1. The safety net: a choice no answer can satisfy fizzles ----

#[test]
fn a_choose_one_nothing_can_answer_fizzles() {
    let mut g = game(1);
    turn_of(&mut g, 0);
    set_shop(&mut g, 0, &[]);
    // No minion anywhere, so neither option of this Choose One can be taken.
    g.players[0].pending_choose_one = Some(PendingChooseOne {
        source: None,
        golden: false,
        options: [needs_a_minion(1, 1), needs_a_minion(2, 2)],
        tavern: true,
        by_minion: false,
        repeats: 1,
    });
    // Answering the choice is illegal (that is what used to strand the bots)...
    assert!(!g.is_legal(Action::Choose(0)));
    assert!(!g.is_legal(Action::Choose(1)));
    // ...and the normal actions are legal, in `is_legal` and in `legal_mask`.
    assert!(g.is_legal(Action::Roll));
    assert!(g.is_legal(Action::EndTurn));
    let a = legal(&g);
    assert!(!a.contains(&Action::Choose(0)) && !a.contains(&Action::Choose(1)), "{a:?}");
    assert!(a.contains(&Action::EndTurn) && a.contains(&Action::Roll), "{a:?}");
    // The first action fizzles the choice: the effect is lost.
    act(&mut g, 0, Action::Roll);
    assert!(g.players[0].pending_choose_one.is_none());
    g.check_invariants().unwrap();
}

#[test]
fn a_target_prompt_that_lost_its_last_target_fizzles() {
    let mut g = game(2);
    turn_of(&mut g, 0);
    // A "give a friendly Demon" buff whose Demon has been sold since it was
    // offered: a neutral minion is all that is left to point at.
    put_on_board(&mut g, 0, "Deadly Spore");
    g.players[0].pending_target = Some(PendingTarget {
        by_minion: false,
        source: Some(0),
        kind: TargetKind::MinionBuff {
            tribe: Some(Tribe::Demon),
            attack: 2,
            health: 2,
            keyword: 0,
            random_keyword: false,
            shop: false,
        },
        runs: 1,
        tavern: true,
        gem: false,
    });
    assert!(!g.is_legal(Action::Target(0)));
    assert!(!g.is_legal(Action::TargetShop(0)));
    assert!(g.is_legal(Action::EndTurn));
    assert!(legal(&g).contains(&Action::EndTurn));
    // Ending the turn fizzles it instead of panicking, and the minion keeps
    // its stats: the buff is lost.
    act(&mut g, 0, Action::EndTurn);
    assert!(g.players[0].pending_target.is_none());
    assert_eq!(stats(&g.players[0].board[0]), (1, 1));
    g.check_invariants().unwrap();
}

// ---- 2. A Choose One minion with no choosable option skips the choice ----

/// Neither option can be chosen: both want a friendly Undead and the set has
/// no Undead card at all.
fn both_options_need_an_undead() -> String {
    card_set(
        "Test Two-Undead Options",
        "choose_one = [{ target = \"chosen\", butchering = 3 }, { target = \"chosen\", butchering = 5 }]",
    )
}

#[test]
fn a_choose_one_minion_with_no_choosable_option_is_played_without_its_choice() {
    let mut g = custom(&both_options_need_an_undead(), 3);
    turn_of(&mut g, 0);
    set_shop(&mut g, 0, &[]);
    put_in_hand(&mut g, 0, "Test Two-Undead Options");
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    // The minion is played...
    assert_eq!(g.players[0].board.len(), 1);
    assert_eq!(g.data.cards.get(g.players[0].board[0].card).name, "Test Two-Undead Options");
    assert_eq!(stats(&g.players[0].board[0]), (1, 1));
    // ...and the choice is skipped rather than opened with no answer.
    assert!(g.players[0].pending_choose_one.is_none());
    assert!(g.players[0].choose_queue.is_empty());
    assert!(g.is_legal(Action::EndTurn));
    // Nothing waits on a choice any more, so Turbo Hogrider's trigger for the
    // Choose One card played has already run.
    assert_eq!(g.players[0].hogrider_pending, 0);
    g.check_invariants().unwrap();
}

// ---- 3. The action limit picks an option that can be chosen ----

/// The first option wants a friendly Undead, which the set has none of; the
/// second one can always be taken.
fn one_option_needs_an_undead() -> String {
    card_set(
        "Test Half-Undead Choice",
        "choose_one = [{ target = \"chosen\", butchering = 2 }, { spell_buff = [1, 3] }]",
    )
}

#[test]
fn the_action_limit_takes_the_option_that_can_be_chosen() {
    let mut g = custom(&one_option_needs_an_undead(), 4);
    turn_of(&mut g, 0);
    set_shop(&mut g, 0, &[]);
    put_in_hand(&mut g, 0, "Test Half-Undead Choice");
    let max = g.data.rules.max_actions_per_turn;
    // One action short of the limit: playing the minion ends the turn, and
    // `finish_shop` has to answer the choice it just opened. Its first option
    // needs a friendly Undead, so only the second can be taken.
    g.players[0].actions = max - 1;
    act(&mut g, 0, Action::Play { hand: 0, pos: 0 });
    assert!(g.players[0].pending_choose_one.is_none(), "the choice was answered");
    assert_eq!(g.players[0].spell_bonus, (1, 3), "the option that could be chosen ran");
    assert!(g.players[0].done, "the turn ended");
    g.check_invariants().unwrap();
}

// ---- 4. A Tavern pick fizzles when the Tavern empties ----

#[test]
fn a_tavern_pick_fizzles_when_the_tavern_empties() {
    let mut g = game(5);
    turn_of(&mut g, 0);
    put_on_board(&mut g, 0, "Lurking Lionfish");
    g.players[0].gold = 5;
    act(&mut g, 0, Action::Activate(0));
    assert!(g.players[0].pending_shop_target.is_some());
    turn_of(&mut g, 0);
    // Nothing can empty the Tavern while the pick is open today, so empty it
    // by hand (the state `check_invariants` already knew about): the pick has
    // nothing left to point at. The spell slot goes too: a Fishbait may take
    // a Tavern spell, so the empty row alone is not empty.
    set_shop(&mut g, 0, &[]);
    g.players[0].shop_spell = None;
    assert!(matches!(
        g.players[0].pending_shop_target.map(|pst| pst.kind),
        Some(ShopTargetKind::Fishbait { .. })
    ));
    assert!(!g.is_legal(Action::TargetShop(0)));
    assert!(g.is_legal(Action::EndTurn));
    assert!(legal(&g).contains(&Action::EndTurn));
    // Ending the turn fizzles the pick instead of panicking, and the Fishbait
    // is lost: nothing replaced a Tavern card and the Lionfish kept its stats.
    act(&mut g, 0, Action::EndTurn);
    assert!(g.players[0].pending_shop_target.is_none());
    assert_eq!(stats(&g.players[0].board[0]), (3, 4));
    assert!(g.players[0].shop.is_empty());
    g.check_invariants().unwrap();
}

// ---- 5. A Discover that finds nothing clears its side flags ----

#[test]
fn a_discover_that_finds_nothing_clears_its_side_flags() {
    // One neutral minion, so every request below comes back empty: there is
    // no Demon, no tier 6 minion, no Beast and no Choose One card.
    let mut g = custom(&card_set("Test Plain", ""), 6);
    turn_of(&mut g, 0);
    // Board slot 0 is the source of the damage flag (the way Imposing
    // Percussionist sets it); nothing is ever chosen, so no damage is dealt.
    put_token(&mut g, 0, "Test Token", true);
    let health = g.players[0].health;
    let requests = [
        DiscoverRequest::DemonMinion { max_tier: 6, source: 0 },
        DiscoverRequest::SearchMinionTier(6),
        DiscoverRequest::DoomedTribeMinion(Tribe::Beast),
        DiscoverRequest::ChooseOneMinion,
    ];
    for req in requests {
        g.players[0].discover_queue.push(req);
        let pos = g.players[0].board.len() as u8;
        put_token(&mut g, 0, "Test Token", false);
        // Playing a minion starts the queued Discover.
        act(&mut g, 0, Action::Play { hand: 0, pos });
        assert!(g.players[0].discover.is_empty(), "{req:?} found nothing");
        assert!(g.players[0].discover_queue.is_empty(), "{req:?} left nothing queued");
        // The flags of the lost Discover went with it.
        assert_eq!(g.players[0].discover_damage_from, None, "{req:?}");
        assert!(!g.players[0].discover_locked, "{req:?}");
        assert!(!g.players[0].discover_doomed, "{req:?}");
        assert!(!g.players[0].discover_both, "{req:?}");
    }
    assert_eq!(g.players[0].health, health, "no damage from a lost Discover");
    g.check_invariants().unwrap();
}
