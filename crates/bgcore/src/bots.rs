//! Simple players: the curve baselines a trained bot has to beat, and the
//! random player the fuzzer and benchmarks use.

use crate::action::{Action, N_ACTIONS, N_KINDS};
use crate::cards::Tribe;
use crate::game::Game;
use crate::minion::{Keywords, Minion};
use crate::rng::Rng;

/// A random legal action: first a random kind among the legal kinds (buy,
/// buy_spell, play, roll, end turn...), then a random action of that kind.
/// Picking uniformly over all actions would almost never end a turn, since
/// most legal actions are plays and moves.
pub fn random_action(game: &Game, rng: &mut Rng) -> Action {
    let mut legal = [Action::EndTurn; N_ACTIONS];
    let mut n = 0;
    let mut kinds = [0u8; N_KINDS];
    for i in 0..N_ACTIONS {
        let a = Action::from_index(i).unwrap();
        if game.is_legal(a) {
            legal[n] = a;
            n += 1;
            kinds[a.kind()] += 1;
        }
    }
    let n_kinds = kinds.iter().filter(|&&c| c > 0).count() as u32;
    let pick = rng.below(n_kinds) as usize;
    let kind = (0..N_KINDS).filter(|&k| kinds[k] > 0).nth(pick).unwrap();
    let k = rng.below(kinds[kind] as u32) as usize;
    *legal[..n].iter().filter(|a| a.kind() == kind).nth(k).unwrap()
}

/// Standard curve from docs/CONCEPT.md: tier 2 on turn 2, tier 3 on turn 5,
/// tier 4 on turn 7, tier 5 on turn 11, tier 6 when affordable after that.
pub const STANDARD_CURVE: [u8; 12] = [1, 2, 2, 2, 3, 3, 4, 4, 4, 4, 5, 6];

/// Rush curve from docs/CONCEPT.md: tier 2 on turn 2, tier 3 on turn 4,
/// tier 4 on turn 5, tier 5 on turn 7, tier 6 when affordable after that.
pub const RUSH_CURVE: [u8; 8] = [1, 2, 2, 3, 4, 4, 5, 6];

fn preferred_tribe(game: &Game, p: usize) -> Option<Tribe> {
    let pl = &game.players[p];
    let mut beasts = 0;
    let mut mechs = 0;
    for m in pl.board.iter().chain(&pl.hand) {
        let c = game.data.cards.get(m.card);
        if c.is_spell() {
            continue;
        }
        match c.tribe {
            Tribe::Beast => beasts += 1,
            Tribe::Mech => mechs += 1,
            Tribe::Neutral => {}
        }
    }
    if beasts > mechs {
        Some(Tribe::Beast)
    } else if mechs > beasts {
        Some(Tribe::Mech)
    } else {
        None
    }
}

fn card_value(game: &Game, pref: Option<Tribe>, m: &Minion) -> i32 {
    let c = game.data.cards.get(m.card);
    let golden = if m.golden { 100 } else { 0 };
    let tribe = if Some(c.tribe) == pref { 6 } else { 0 };
    golden + c.tier as i32 * 10 + m.attack + m.health + tribe
}

fn board_value(game: &Game, pref: Option<Tribe>, m: &Minion) -> i32 {
    let scaling = (m.magnetizations as i32 + m.improvements as i32) * 8;
    card_value(game, pref, m) + scaling
}

fn play_pos(game: &Game, p: usize, pref: Option<Tribe>, m: &Minion) -> Option<u8> {
    let pl = &game.players[p];
    if m.keywords.has(Keywords::MAGNETIC)
        && let Some(s) = (0..pl.board.len())
            .filter(|&s| game.data.cards.get(pl.board[s].card).tribe == Tribe::Mech)
            .max_by_key(|&s| board_value(game, pref, &pl.board[s]))
    {
        return Some(s as u8);
    }
    if !pl.board.is_full() {
        Some(pl.board.len() as u8)
    } else {
        None
    }
}

fn curve_action(game: &Game, curve: &[u8]) -> Action {
    let p = game.current_player().expect("game is over");
    let pl = &game.players[p];
    let r = &game.data.rules;
    let pref = preferred_tribe(game, p);
    let legal = |a: Action| game.is_legal(a);
    let held =
        |c: u8| pl.hand.iter().chain(&pl.board).filter(|m| m.card == c && !m.golden).count();

    if pl.pending_target.is_some() {
        let best = (0..pl.board.len() as u8)
            .filter(|&s| legal(Action::Target(s)))
            .max_by_key(|&s| board_value(game, pref, &pl.board[s as usize]));
        return Action::Target(best.unwrap());
    }
    if pl.pending_choose_one.is_some() {
        let choice = (0..2).find(|&k| legal(Action::Choose(k))).unwrap();
        return Action::Choose(choice);
    }
    if !pl.discover.is_empty() {
        let best = (0..pl.discover.len())
            .max_by_key(|&k| {
                let c = pl.discover[k];
                let card = game.data.cards.get(c);
                if card.is_spell() {
                    (card.gives_stats() as i32) * 20 + card.tier as i32 * 10
                } else {
                    let triple = if held(c) == 2 { 1000 } else { 0 };
                    let m = pl.make_card(&game.data.cards, c, false, 1);
                    triple + card_value(game, pref, &m)
                }
            })
            .unwrap();
        return Action::Choose(best as u8);
    }

    let want = curve[(game.turn as usize - 1).min(curve.len() - 1)];
    if pl.tier < want && legal(Action::Level) {
        return Action::Level;
    }

    // Play minions from hand, merging Magnetics left of our best Mech.
    if let Some((h, pos)) = (0..pl.hand.len())
        .filter(|&h| !game.data.cards.get(pl.hand[h].card).is_spell())
        .filter_map(|h| {
            let pos = play_pos(game, p, pref, &pl.hand[h])?;
            legal(Action::Play { hand: h as u8, pos }).then_some((h, pos))
        })
        .max_by_key(|&(h, _)| card_value(game, pref, &pl.hand[h]))
    {
        return Action::Play { hand: h as u8, pos };
    }

    // Cast spells in hand, preferring stat spells.
    if let Some(h) = (0..pl.hand.len())
        .filter(|&h| {
            game.data.cards.get(pl.hand[h].card).is_spell()
                && legal(Action::Play { hand: h as u8, pos: 0 })
        })
        .max_by_key(|&h| {
            let c = game.data.cards.get(pl.hand[h].card);
            (c.gives_stats() as u8, c.tier)
        })
    {
        return Action::Play { hand: h as u8, pos: 0 };
    }

    let has_mech = pl.board.iter().any(|m| game.data.cards.get(m.card).tribe == Tribe::Mech);
    let shop_score = |m: &Minion| {
        let triple = if held(m.card) == 2 {
            1000
        } else if held(m.card) == 1 {
            10
        } else {
            0
        };
        let mag = if m.keywords.has(Keywords::MAGNETIC) && has_mech { 8 } else { 0 };
        triple + mag + card_value(game, pref, m)
    };
    let best_shop = (0..pl.shop.len()).max_by_key(|&s| shop_score(&pl.shop[s]));
    let weakest = (0..pl.board.len()).min_by_key(|&s| board_value(game, pref, &pl.board[s]));

    if let Some(s) = best_shop
        && pl.gold >= r.buy_cost
    {
        let m = &pl.shop[s];
        let can_use = !pl.board.is_full()
            || held(m.card) == 2
            || (m.keywords.has(Keywords::MAGNETIC) && has_mech);
        if can_use {
            if legal(Action::Buy(s as u8)) {
                return Action::Buy(s as u8);
            }
        } else if let Some(w) = weakest
            && card_value(game, pref, m) >= board_value(game, pref, &pl.board[w]) + 10
            && legal(Action::Sell(w as u8))
        {
            return Action::Sell(w as u8);
        }
    }

    // Buy a stat spell after the minion plan when we have a board to buff.
    if let Some(sp) = pl.shop_spell
        && legal(Action::BuySpell)
        && game.can_cast_spell(p, sp)
        && game.data.cards.get(sp).gives_stats()
        && !pl.board.is_empty()
    {
        return Action::BuySpell;
    }

    if (pl.free_rolls > 0 || pl.gold >= r.roll_cost + r.buy_cost) && legal(Action::Roll) {
        return Action::Roll;
    }

    // Spend any leftover gold on a castable spell (such as a 1-cost spell on turn 4 rush).
    if let Some(sp) = pl.shop_spell
        && legal(Action::BuySpell)
        && game.can_cast_spell(p, sp)
    {
        return Action::BuySpell;
    }

    Action::EndTurn
}

/// Curve bot playing the standard leveling curve from docs/CONCEPT.md.
pub fn standard_action(game: &Game) -> Action {
    curve_action(game, &STANDARD_CURVE)
}

/// Curve bot playing the rush leveling curve from docs/CONCEPT.md.
pub fn rush_action(game: &Game) -> Action {
    curve_action(game, &RUSH_CURVE)
}

/// Alias for `standard_action` kept for compatibility with v0.1 tests.
pub fn rule_action(game: &Game) -> Action {
    standard_action(game)
}
