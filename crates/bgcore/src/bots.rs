//! Simple players: the baselines a trained bot has to beat, and the random
//! player the fuzzer and benchmarks use.

use crate::action::{Action, N_ACTIONS, N_KINDS};
use crate::game::Game;
use crate::rng::Rng;

/// A random legal action: first a random kind among the legal kinds (buy,
/// play, roll, end turn...), then a random action of that kind. Picking
/// uniformly over all actions would almost never end a turn, since most
/// legal actions are plays and moves.
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

/// How much the rule bot likes a minion: tier first, then stats.
fn value(game: &Game, m: &crate::Minion) -> i32 {
    let tier = game.data.cards.get(m.card).tier as i32;
    let golden = if m.golden { 100 } else { 0 };
    golden + tier * 10 + m.attack + m.health
}

/// The tier the rule bot wants to be on by each turn.
const LEVEL_CURVE: [u8; 12] = [1, 2, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6];

/// The rule bot from DESIGN.md: buy the best minion it can, level on a
/// fixed curve, take triples, and never roll below 3 gold left. A baseline
/// to beat, not a good player.
pub fn rule_action(game: &Game) -> Action {
    let p = game.current_player().expect("game is over");
    let pl = &game.players[p];
    let r = &game.data.rules;
    let legal = |a: Action| game.is_legal(a);

    if pl.pending_target.is_some() {
        let best = (0..pl.board.len() as u8)
            .filter(|&s| legal(Action::Target(s)))
            .max_by_key(|&s| value(game, &pl.board[s as usize]));
        return Action::Target(best.unwrap());
    }
    if pl.pending_choose_one.is_some() {
        return Action::Choose(0);
    }
    if !pl.discover.is_empty() {
        let best = (0..pl.discover.len())
            .max_by_key(|&k| value(game, &game.data.cards.make(pl.discover[k], false)))
            .unwrap();
        return Action::Choose(best as u8);
    }
    let want = LEVEL_CURVE[(game.turn as usize - 1).min(LEVEL_CURVE.len() - 1)];
    if pl.tier < want && legal(Action::Level) {
        return Action::Level;
    }
    // play the best card in hand while there's room
    if !pl.hand.is_empty() && !pl.board.is_full() {
        let h = (0..pl.hand.len()).max_by_key(|&h| value(game, &pl.hand[h])).unwrap();
        return Action::Play { hand: h as u8, pos: pl.board.len() as u8 };
    }
    let held = |c: u8| pl.hand.iter().chain(&pl.board).filter(|m| m.card == c && !m.golden).count();
    let best_shop = (0..pl.shop.len()).max_by_key(|&s| {
        let m = &pl.shop[s];
        let triple = if held(m.card) == 2 { 1000 } else { 0 };
        triple + value(game, m)
    });
    let weakest = (0..pl.board.len()).min_by_key(|&s| value(game, &pl.board[s]));
    if let Some(s) = best_shop
        && pl.gold >= r.buy_cost
    {
        let m = &pl.shop[s];
        let room = !pl.board.is_full() || !pl.hand.is_empty();
        if room || held(m.card) == 2 {
            if legal(Action::Buy(s as u8)) {
                return Action::Buy(s as u8);
            }
        } else if let Some(w) = weakest
            && value(game, m) > value(game, &pl.board[w])
        {
            return Action::Sell(w as u8);
        }
    }
    if pl.gold >= r.roll_cost + r.buy_cost && legal(Action::Roll) {
        return Action::Roll;
    }
    Action::EndTurn
}
