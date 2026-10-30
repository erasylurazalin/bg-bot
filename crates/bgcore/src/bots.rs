//! Simple players: the curve baselines a trained bot has to beat, and the
//! random player the fuzzer and benchmarks use.
//!
//! The curve bots are deterministic (no RNG) and only ever pick actions the
//! engine says are legal; if a heuristic finds nothing, they fall back to the
//! first legal action, so they never get stuck.

use crate::action::{ACTION_KINDS, Action, N_ACTIONS, N_KINDS};
use crate::cards::{ChooseOption, Effect, SpellEffect, SpellTarget, Tribe};
use crate::heroes::PowerEffect;
use crate::game::{Game, ShopTargetKind, TargetKind};
use crate::minion::{CardId, Keywords, Minion};
use crate::rng::Rng;

/// A random legal action: first a random kind among the legal kinds (buy,
/// buy_spell, play, roll, end turn...), then a random action of that kind.
/// Picking uniformly over all actions would almost never end a turn, since
/// most legal actions are plays and moves.
pub fn random_action(game: &Game, rng: &mut Rng) -> Action {
    let mut mask = [false; N_ACTIONS];
    game.legal_mask(&mut mask);
    // The kinds are laid out in index order (a test in action.rs checks it),
    // so each kind is one slice of the mask.
    let mut kinds = [0u8; N_KINDS];
    let mut starts = [0usize; N_KINDS];
    let mut at = 0;
    for (k, &(_, size)) in ACTION_KINDS.iter().enumerate() {
        starts[k] = at;
        kinds[k] = mask[at..at + size].iter().filter(|&&m| m).count() as u8;
        at += size;
    }
    let n_kinds = kinds.iter().filter(|&&c| c > 0).count() as u32;
    let pick = rng.below(n_kinds) as usize;
    let kind = (0..N_KINDS).filter(|&k| kinds[k] > 0).nth(pick).unwrap();
    let k = rng.below(kinds[kind] as u32) as usize;
    let size = ACTION_KINDS[kind].1;
    let slice = &mask[starts[kind]..starts[kind] + size];
    let i = slice.iter().enumerate().filter(|&(_, &m)| m).nth(k).unwrap().0;
    Action::from_index(starts[kind] + i).unwrap()
}

/// Standard curve: tier 2 on turn 2, tier 3 on turn 5,
/// tier 4 on turn 7, tier 5 on turn 11, tier 6 when affordable after that.
pub const STANDARD_CURVE: [u8; 12] = [1, 2, 2, 2, 3, 3, 4, 4, 4, 4, 5, 6];

/// Rush curve: tier 2 on turn 2, tier 3 on turn 4,
/// tier 4 on turn 5, tier 5 on turn 7, tier 6 when affordable after that.
pub const RUSH_CURVE: [u8; 8] = [1, 2, 2, 3, 4, 4, 5, 6];

/// Health the curve bots keep when paying with Health (Malchezaar's
/// Refreshes, Hasty Excavation).
const HEALTH_FLOOR: i32 = 10;

fn preferred_tribe(game: &Game, p: usize) -> Option<Tribe> {
    let pl = &game.players[p];
    let mut counts = [0u32; Tribe::EVERY.len()];
    for m in pl.board.iter().chain(&pl.hand) {
        let c = game.data.cards.get(m.card);
        if c.is_spell() {
            continue;
        }
        for (n, t) in counts.iter_mut().zip(Tribe::EVERY) {
            // an out-of-lobby tribe can never be preferred: an all-type
            // minion counts for it, but no card of it can ever be bought
            if game.lobby.has(t) {
                *n += c.tribes.has(t) as u32;
            }
        }
    }
    // the tribe with the most cards, if one leads alone
    let best = *counts.iter().max().unwrap();
    let mut leaders = Tribe::EVERY.into_iter().zip(counts).filter(|&(_, n)| n == best);
    match (leaders.next(), leaders.next()) {
        (Some((t, _)), None) if best > 0 => Some(t),
        _ => None,
    }
}

fn card_value(game: &Game, pref: Option<Tribe>, m: &Minion) -> i32 {
    let c = game.data.cards.get(m.card);
    if c.is_spell() {
        return c.tier as i32 * 10;
    }
    let golden = if m.golden { 100 } else { 0 };
    let tribe = if pref.is_some_and(|t| c.tribes.has(t)) { 6 } else { 0 };
    golden + c.tier as i32 * 10 + m.attack + m.health + tribe
}

fn board_value(game: &Game, pref: Option<Tribe>, m: &Minion) -> i32 {
    let scaling = (m.magnetizations as i32 + m.improvements as i32) * 8;
    card_value(game, pref, m) + scaling
}

fn is_mech(game: &Game, m: &Minion) -> bool {
    game.data.cards.get(m.card).tribes.has(Tribe::Mech)
}

fn play_pos(game: &Game, p: usize, pref: Option<Tribe>, m: &Minion) -> Option<u8> {
    let pl = &game.players[p];
    if m.keywords.has(Keywords::MAGNETIC)
        && let Some(s) = (0..pl.board.len())
            .filter(|&s| is_mech(game, &pl.board[s]))
            // a Drone Duplicator charge doubles the merge
            .max_by_key(|&s| {
                board_value(game, pref, &pl.board[s]) + pl.board[s].extra_magnetize as i32 * 100
            })
    {
        return Some(s as u8);
    }
    if !pl.board.is_full() { Some(pl.board.len() as u8) } else { None }
}

/// The first legal action, the fallback when no heuristic applies.
fn first_legal(game: &Game) -> Action {
    let mut mask = [false; N_ACTIONS];
    game.legal_mask(&mut mask);
    let Some(i) = mask.iter().position(|&m| m) else {
        // Training hit this once (2026-10-25) and random play never does:
        // print the stuck player's state so the cause can be found.
        let p = game.current_player();
        if let Some(pl) = p.map(|p| &game.players[p]) {
            let names = |ms: &[crate::Minion]| -> Vec<String> {
                ms.iter().map(|m| game.data.cards.get(m.card).name.clone()).collect()
            };
            eprintln!(
                "no legal action: turn {} player {:?} hero {:?} power {:?} gold {} tier {}\n  \
                 pending_target {:?}\n  pending_shop_target {:?}\n  pending_choose_one {:?}\n  \
                 pending_discard {:?}\n  discover {:?} queue {}\n  choose_queue {}\n  power_offer {:?}\n  \
                 board {:?}\n  hand {:?}\n  shop {:?} spell {:?}",
                game.turn, p, pl.hero.map(|h| game.data.heroes.get(h).name.clone()), pl.power,
                pl.gold, pl.tier, pl.pending_target, pl.pending_shop_target, pl.pending_choose_one,
                pl.pending_discard,
                pl.discover.iter().map(|&c| game.data.cards.get(c).name.clone()).collect::<Vec<_>>(),
                pl.discover_queue.len(), pl.choose_queue.len(), pl.power_offer,
                names(&pl.board), names(&pl.hand), names(&pl.shop),
                pl.shop_spell.map(|c| game.data.cards.get(c).name.clone()),
            );
        }
        panic!("no legal action");
    };
    Action::from_index(i).unwrap()
}

/// How much the bot likes `m` (on the board, or in the Tavern when
/// `on_board` is false) as the target of a pending effect.
fn target_score(game: &Game, pref: Option<Tribe>, kind: TargetKind, m: &Minion, on_board: bool) -> i32 {
    let value = board_value(game, pref, m);
    // Board minions are kept; a Tavern minion only matters if bought later.
    let place = if on_board { 10_000 } else { 0 };
    let c = game.data.cards.get(m.card);
    let set_gain = |a: i32, h: i32| (a - m.attack) + (h - m.health);
    place
        + match kind {
            // Boon of Light: the highest-Attack minion without a shield.
            TargetKind::MinionBuff { keyword, attack: 0, health: 0, .. }
                if keyword == crate::minion::Keywords::DIVINE_SHIELD =>
            {
                if m.keywords.has(Keywords::DIVINE_SHIELD) { -1000 } else { m.attack }
            }
            TargetKind::AnySetStats { attack, health } => set_gain(attack, health),
            TargetKind::Spell(st) => match st {
                SpellTarget::SetStats { attack, health } => set_gain(attack, health),
                // these use up or replace the minion: pick the weakest
                SpellTarget::Devourer | SpellTarget::RobustEvolution => -value,
                SpellTarget::TrickyTrousers { .. } if m.keywords.has(Keywords::TAUNT) => -1000,
                SpellTarget::TavernBuffType { .. }
                | SpellTarget::RefreshWithType
                | SpellTarget::SameTribeMinion
                | SpellTarget::BuffSameTribe { .. } => {
                    let tribe = if pref.is_some_and(|t| c.tribes.has(t)) { 1000 } else { 0 };
                    tribe + value
                }
                _ => value,
            },
            _ => value,
        }
}

/// A rough value of a Choose One option, for picking between two.
fn choose_one_score(game: &Game, p: usize, opt: ChooseOption) -> i32 {
    let pl = &game.players[p];
    let n = pl.board.len() as i32;
    match opt {
        ChooseOption::MinionTarget { tribe, attack, health, .. } => {
            let any = pl.board.iter().any(|m| game.data.cards.get(m.card).is(tribe));
            if any { attack + health } else { -100 }
        }
        // a permanent bonus on every later spell
        ChooseOption::SpellBonus { attack, health } => (attack + health) * 2,
        ChooseOption::SpellTarget(st) => match st {
            SpellTarget::Buff { attack, health, .. }
            | SpellTarget::BuffSameTribe { attack, health }
            | SpellTarget::BuffPerBonusKeyword { attack, health }
            | SpellTarget::TrickyTrousers { attack, health } => attack + health,
            _ => 5,
        },
        ChooseOption::SpellEffect(se) => match se {
            SpellEffect::BuffBoard { attack, health, .. }
            | SpellEffect::NextTurnBoardBuff { attack, health, .. }
            | SpellEffect::MightyDragonbreath { attack, health } => (attack + health) * n,
            SpellEffect::BuffRandomBoard { count, attack, health } => {
                (attack + health) * n.min(count as i32)
            }
            SpellEffect::GainGold(g) => g as i32 * 4,
            _ => 5,
        },
    }
}

/// How useful the Activate of the board minion in slot `s` is right now:
/// 2 worth paying for over rolling, 1 only with gold that can't buy
/// anything else, 0 not useful.
fn activate_worth(game: &Game, p: usize, s: usize) -> u8 {
    let pl = &game.players[p];
    let cards = &game.data.cards;
    let Some(act) = pl.board[s].parts().find_map(|(c, _)| cards.get(c).activate) else {
        return 0;
    };
    let others = || pl.board.iter().enumerate().filter(move |&(i, _)| i != s).map(|(_, m)| m);
    let has_tribe = |t: Tribe| pl.board.iter().any(|m| cards.get(m.card).tribes.has(t));
    let shop_minion = pl.shop.iter().any(|m| !cards.get(m.card).is_spell());
    match act.effect {
        Effect::BuffAnyMinion { .. } => 2,
        Effect::SetStatsAnyMinion { attack, health } => {
            let gain = pl.board.iter().any(|m| m.attack + m.health < attack + health);
            if gain { 2 } else { 0 }
        }
        Effect::BuffChosen { tribe, .. } => {
            let target = others().any(|m| cards.get(m.card).is(tribe));
            if target { 2 } else { 0 }
        }
        Effect::DemonsConsume { .. } => {
            if has_tribe(Tribe::Demon) && shop_minion { 2 } else { 0 }
        }
        Effect::TriggerRally | Effect::TriggerBattlecry | Effect::RandomChromadrake => 2,
        Effect::StealTavern { .. } | Effect::GetCard { .. } => {
            if pl.hand.is_full() { 0 } else { 2 }
        }
        Effect::ExtraMagnetization(_) => {
            let magnetic = pl.hand.iter().any(|m| m.keywords.has(Keywords::MAGNETIC));
            if magnetic && pl.board[s].extra_magnetize == 0 { 2 } else { 0 }
        }
        // a Fishbait only pays off when a Beast attacks it
        Effect::ReplaceWithFishbait { .. } => {
            if has_tribe(Tribe::Beast) { 1 } else { 0 }
        }
        _ => 1,
    }
}

/// Whether paying `cost` Health leaves the hero comfortably alive.
fn health_ok(game: &Game, p: usize, cost: i32) -> bool {
    game.players[p].health - cost > HEALTH_FLOOR
}

/// Answer whatever choice is open, or None if nothing is.
pub fn answer_pending(game: &Game, p: usize, pref: Option<Tribe>) -> Option<Action> {
    let pl = &game.players[p];
    let legal = |a: Action| game.is_legal(a);
    // The hero pick before turn 1: take the first offered hero (the offers
    // differ per player, so games still vary).
    if game.turn == 0 && pl.hero_offer.iter().any(|&h| h != u8::MAX) {
        let k = pl.hero_offer.iter().position(|&h| h != u8::MAX).unwrap();
        return Some(Action::Choose(k as u8));
    }
    // A Hero Power Discover: take the first offered power.
    if !pl.power_offer.is_empty() {
        return Some(Action::Choose(0));
    }
    if let Some(pt) = pl.pending_target {
        let board = (0..pl.board.len() as u8)
            .map(Action::Target)
            .filter(|&a| legal(a))
            .map(|a| (a, target_score(game, pref, pt.kind, &pl.board[a_slot(a)], true)));
        let shop = (0..pl.shop.len() as u8)
            .map(Action::TargetShop)
            .filter(|&a| legal(a))
            .map(|a| (a, target_score(game, pref, pt.kind, &pl.shop[a_slot(a)], false)));
        return Some(best_of(board.chain(shop)).unwrap_or_else(|| first_legal(game)));
    }
    if let Some(pst) = pl.pending_shop_target {
        let has_mech = pl.board.iter().any(|m| is_mech(game, m));
        let scored = (0..pl.shop.len() as u8)
            .map(Action::TargetShop)
            .filter(|&a| legal(a))
            .map(|a| {
                let m = &pl.shop[a_slot(a)];
                let spell = game.data.cards.get(m.card).is_spell();
                let score = match pst.kind {
                    // eat the biggest minion
                    ShopTargetKind::Consume { .. } => {
                        if spell { -1000 } else { m.attack + m.health }
                    }
                    // replace the card we want least
                    ShopTargetKind::Fishbait { .. } => -shop_score(game, p, pref, has_mech, m),
                };
                (a, score)
            });
        return Some(best_of(scored).unwrap_or_else(|| first_legal(game)));
    }
    if pl.pending_discard.is_some() {
        // Throw away the least valuable discardable card.
        let scored = (0..pl.hand.len() as u8)
            .map(Action::Discard)
            .filter(|&a| legal(a))
            .map(|a| (a, -card_value(game, pref, &pl.hand[a_slot(a)])));
        return Some(best_of(scored).unwrap_or_else(|| first_legal(game)));
    }
    if let Some(pco) = pl.pending_choose_one {
        let scored = (0..2u8)
            .map(Action::Choose)
            .filter(|&a| legal(a))
            .map(|a| (a, choose_one_score(game, p, pco.options[a_slot(a)])));
        return Some(best_of(scored).unwrap_or_else(|| first_legal(game)));
    }
    if !pl.discover.is_empty() {
        let held = |c: CardId| {
            pl.hand.iter().chain(&pl.board).filter(|m| m.card == c && !m.golden).count()
        };
        let damage = pl.discover_damage_from.is_some();
        let scored = (0..pl.discover.len() as u8).map(Action::Choose).filter(|&a| legal(a)).map(|a| {
            let c = pl.discover[a_slot(a)];
            let card = game.data.cards.get(c);
            let mut score = if card.is_spell() {
                (card.gives_stats() as i32) * 20 + card.tier as i32 * 10
            } else {
                let triple = if held(c) == 2 { 1000 } else { 0 };
                let m = pl.make_card(&game.data.cards, c, false, 1);
                triple + card_value(game, pref, &m)
            };
            // Imposing Percussionist: the pick costs Health equal to its Tier
            if damage {
                let tier = card.tier as i32;
                if pl.health - tier <= 0 && !pl.can_rewind(&game.data.cards) {
                    score -= 1_000_000;
                } else if !health_ok(game, p, tier) {
                    score -= tier * 100;
                }
            }
            (a, score)
        });
        return Some(best_of(scored).unwrap_or_else(|| first_legal(game)));
    }
    // A trinket offer, once nothing else is open (`is_legal` gives the other
    // choices priority): the cheapest one of a tribe the board has, else the
    // cheapest. A simple rule on purpose.
    if !pl.trinket_offer.is_empty() {
        let own = |of: usize| {
            let t = game.data.trinkets.get(of as u8);
            t.tribes.is_empty()
                || pl.board.iter().any(|m| game.data.cards.get(m.card).tribes.overlaps(t.tribes))
        };
        let pick = (0..pl.trinket_offer.len())
            .map(|k| Action::ChooseTrinket(k as u8))
            .filter(|&a| legal(a))
            .min_by_key(|&a| {
                let k = match a {
                    Action::ChooseTrinket(k) => k as usize,
                    _ => 0,
                };
                let (of, price) = pl.trinket_offer[k];
                (!own(of as usize), price, k)
            });
        if let Some(a) = pick {
            return Some(a);
        }
    }
    None
}

/// The slot or option index an answering action points at.
fn a_slot(a: Action) -> usize {
    match a {
        Action::Target(s) | Action::TargetShop(s) | Action::Choose(s) | Action::Discard(s) => {
            s as usize
        }
        _ => unreachable!(),
    }
}

/// The highest scored action; ties go to the first.
fn best_of(it: impl Iterator<Item = (Action, i32)>) -> Option<Action> {
    it.fold(None, |best: Option<(Action, i32)>, (a, s)| match best {
        Some((_, b)) if b >= s => best,
        _ => Some((a, s)),
    })
    .map(|(a, _)| a)
}

fn held_copies(game: &Game, p: usize, c: CardId) -> usize {
    let pl = &game.players[p];
    pl.hand.iter().chain(&pl.board).filter(|m| m.card == c && !m.golden).count()
}

/// How much the bot wants a Tavern minion.
fn shop_score(game: &Game, p: usize, pref: Option<Tribe>, has_mech: bool, m: &Minion) -> i32 {
    let held = held_copies(game, p, m.card);
    let triple = match held {
        2 => 1000,
        1 => 10,
        _ => 0,
    };
    let mag = if m.keywords.has(Keywords::MAGNETIC) && has_mech { 8 } else { 0 };
    triple + mag + card_value(game, pref, m)
}

/// Whether buying the spell `sp` now is sensible: castable on our board,
/// and if it costs Health, affordable.
fn spell_buy_ok(game: &Game, p: usize, sp: CardId, stat_only: bool) -> bool {
    let c = game.data.cards.get(sp);
    game.can_cast_spell(p, sp)
        && (!stat_only || (c.gives_stats() && !game.players[p].board.is_empty()))
        && (!c.pay_health || health_ok(game, p, c.cost as i32))
}

fn curve_action(game: &Game, curve: &[u8]) -> Action {
    let p = game.current_player().expect("game is over");
    let pl = &game.players[p];
    let r = &game.data.rules;
    let cards = &game.data.cards;
    let pref = preferred_tribe(game, p);
    let legal = |a: Action| game.is_legal(a);

    if let Some(a) = answer_pending(game, p, pref) {
        return a;
    }

    let want = curve[(game.turn as usize - 1).min(curve.len() - 1)];
    if pl.tier < want && legal(Action::Level) {
        return Action::Level;
    }

    // Play minions from hand, merging Magnetics left of our best Mech.
    if let Some((h, pos)) = (0..pl.hand.len())
        .filter(|&h| !cards.get(pl.hand[h].card).is_spell())
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
        .filter(|&h| cards.get(pl.hand[h].card).is_spell() && legal(Action::Play { hand: h as u8, pos: 0 }))
        .max_by_key(|&h| {
            let c = cards.get(pl.hand[h].card);
            (c.gives_stats() as u8, c.tier)
        })
    {
        return Action::Play { hand: h as u8, pos: 0 };
    }

    let has_mech = pl.board.iter().any(|m| is_mech(game, m));
    let best_shop = (0..pl.shop.len())
        .filter(|&s| !cards.get(pl.shop[s].card).is_spell())
        .max_by_key(|&s| shop_score(game, p, pref, has_mech, &pl.shop[s]));
    let weakest = (0..pl.board.len()).min_by_key(|&s| board_value(game, pref, &pl.board[s]));

    if let Some(s) = best_shop
        && pl.gold >= r.buy_cost
    {
        let m = &pl.shop[s];
        let can_use = !pl.board.is_full()
            || held_copies(game, p, m.card) == 2
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

    // Activates worth their gold (Suspicious Prisonguard, Tyrael, Sewer
    // Escapee, Soulkeeping Jailer, ...), the best value first.
    let activate = |level: u8| {
        (0..pl.board.len())
            .filter(|&s| legal(Action::Activate(s as u8)) && activate_worth(game, p, s) >= level)
            .max_by_key(|&s| board_value(game, pref, &pl.board[s]))
            .map(|s| Action::Activate(s as u8))
    };
    if let Some(a) = activate(2) {
        return a;
    }

    // Buy a stat spell (the spell slot, or one in a minion slot after
    // Saloon's Finest) when we have a board to buff.
    if let Some(sp) = pl.shop_spell
        && legal(Action::BuySpell)
        && spell_buy_ok(game, p, sp, true)
    {
        return Action::BuySpell;
    }
    if let Some(s) = (0..pl.shop.len()).find(|&s| {
        let c = pl.shop[s].card;
        cards.get(c).is_spell() && legal(Action::Buy(s as u8)) && spell_buy_ok(game, p, c, true)
    }) {
        return Action::Buy(s as u8);
    }

    // Refresh: with spare gold for a buy, for free, with Health while it is
    // comfortably high, or for queued Fodders a Demon on the board will eat.
    if legal(Action::Roll) {
        let pays_health = pl.free_rolls == 0 && pl.can_refresh_with_health(cards, r);
        let fodder = pl.fodder_refreshes.first().is_some_and(|&n| n > 0)
            && pl.board.iter().any(|m| cards.get(m.card).tribes.has(Tribe::Demon));
        let roll = if pl.free_rolls > 0 {
            true
        } else if pays_health {
            health_ok(game, p, r.health_refresh_cost) && (pl.gold >= r.buy_cost || fodder)
        } else {
            pl.gold >= r.roll_cost + r.buy_cost || (fodder && pl.gold >= r.roll_cost)
        };
        if roll {
            return Action::Roll;
        }
    }

    // Leftover gold: smaller Activates, then any castable spell.
    if let Some(a) = activate(1) {
        return a;
    }
    if let Some(sp) = pl.shop_spell
        && legal(Action::BuySpell)
        && spell_buy_ok(game, p, sp, false)
    {
        return Action::BuySpell;
    }

    // The hero power button: any gold left here would be left over, so a
    // legal press is worth it. Gonna Be Rich! waits for a strong minion
    // (turn 8 on): it is once per game.
    if legal(Action::HeroPower) {
        let waits = pl.power.is_some_and(|st| {
            matches!(game.data.heroes.get(st.of).power.effect, PowerEffect::MakeGolden)
                && game.turn < 8
        });
        if !waits {
            return Action::HeroPower;
        }
    }

    Action::EndTurn
}

/// Curve bot playing the standard leveling curve.
pub fn standard_action(game: &Game) -> Action {
    curve_action(game, &STANDARD_CURVE)
}

/// Curve bot playing the rush leveling curve.
pub fn rush_action(game: &Game) -> Action {
    curve_action(game, &RUSH_CURVE)
}
