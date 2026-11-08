//! The comp overlay: standing rules and logit bonuses on top of a network.
//!
//! The overlay is not a bot. It returns a forced action or nothing, and a
//! per-action bonus for the network's logits. With no active comp it only
//! applies the Shell rule and takes an offered enabler; otherwise it leaves
//! the network alone.
//!
//! A comp is ACTIVE for a player once one of its `enabler` cards is on that
//! player's board or in their hand. While active: an enabler or payoff cannot
//! be sold (the bonus is `-inf`), buying or playing a payoff is worth `B`, a
//! filler `B/2`, and on a full board selling a card that is not in the comp
//! is worth `B` when a comp card is waiting in the hand or Tavern.

use crate::cards::{ChooseOption, SpellEffect};
use crate::comps::{Comp, Comps};
use crate::game::Game;
use crate::minion::CardId;
use crate::{Action, BOARD, DISCOVER, N_ACTIONS, Data};

/// The overlay's per-decision flags, in `[i32; OVERLAY_FLAGS]` order.
pub const OVERLAY_FLAGS: usize = 6;
pub const F_SHELL_LEGAL: usize = 0;
pub const F_SHELL_TAKEN: usize = 1;
pub const F_ENABLER_OFFERED: usize = 2;
pub const F_ENABLER_TAKEN: usize = 3;
pub const F_ACTIVE: usize = 4;
pub const F_FORCED_SELL: usize = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pick {
    Enabler,
    Payoff,
    Filler,
}

/// The overlay rules, one per comp library and bonus value.
#[derive(Clone, Debug)]
pub struct Overlay {
    pub comps: Comps,
    pub bonus: f32,
    shell: Option<CardId>,
}

impl Overlay {
    pub fn new(comps: Comps, bonus: f32, data: &Data) -> Overlay {
        Overlay { comps, bonus, shell: data.cards.by_name("Tortollan Blue Shell") }
    }

    /// The comp that is active for `p`, if any.
    pub fn active_comp(&self, game: &Game, p: usize) -> Option<usize> {
        let pl = &game.players[p];
        self.comps.comps.iter().position(|c| {
            c.has_enabler()
                && c.enabler.iter().any(|id| {
                    pl.board.iter().chain(&pl.hand).any(|m| m.card == *id)
                })
        })
    }

    /// Fill `bonus` for one decision and return the forced action, if any.
    /// `bonus` is zero for no change and `-inf` for a masked action.
    pub fn decide(
        &self,
        game: &Game,
        p: usize,
        bonus: &mut [f32; N_ACTIONS],
        flags: &mut [i32; OVERLAY_FLAGS],
    ) -> Option<Action> {
        bonus.fill(0.0);
        flags.fill(0);
        let active = self.active_comp(game, p);
        if active.is_some() {
            flags[F_ACTIVE] = 1;
        }
        if let Some(a) = self.shell(game, p, active, flags) {
            return Some(a);
        }
        if let Some(a) = self.choice(game, p, active, flags) {
            return Some(a);
        }
        if let Some(ci) = active {
            self.bonuses(game, p, ci, bonus);
        }
        None
    }

    fn comp_in_lobby(&self, game: &Game, comp: &Comp) -> bool {
        comp.tribes.is_empty() || comp.tribes.iter().any(|t| game.lobby.has(*t))
    }

    fn shell(
        &self,
        game: &Game,
        p: usize,
        active: Option<usize>,
        flags: &mut [i32; OVERLAY_FLAGS],
    ) -> Option<Action> {
        let shell = self.shell?;
        let pl = &game.players[p];
        if !pl.lost_last_combat {
            return None;
        }
        let slot = pl.shop.iter().position(|m| m.card == shell)?;
        if pl.gold < game.data.cards.get(shell).cost {
            return None;
        }
        if game.is_legal(Action::CycleShop(slot as u8)) {
            flags[F_SHELL_LEGAL] = 1;
            flags[F_SHELL_TAKEN] = 1;
            return Some(Action::CycleShop(slot as u8));
        }
        // A full board needs a slot. Only an active comp says which card is
        // not part of it; with no comp the rule skips the sale.
        if pl.board.is_full()
            && let Some(ci) = active
        {
            let comp = &self.comps.comps[ci];
            let sell = (0..pl.board.len())
                .filter(|&s| !comp.in_comp(pl.board[s].card))
                .min_by_key(|&s| {
                    let m = &pl.board[s];
                    m.attack + m.health
                });
            if let Some(slot) = sell {
                let a = Action::Sell(slot as u8);
                if game.is_legal(a) {
                    flags[F_SHELL_LEGAL] = 1;
                    flags[F_FORCED_SELL] = 1;
                    return Some(a);
                }
            }
        }
        None
    }

    fn choice(
        &self,
        game: &Game,
        p: usize,
        active: Option<usize>,
        flags: &mut [i32; OVERLAY_FLAGS],
    ) -> Option<Action> {
        let pl = &game.players[p];
        if !pl.discover.is_empty() {
            let mut items = [(0u8, 0u16); DISCOVER];
            let mut n = 0;
            for k in 0..pl.discover.len().min(DISCOVER) {
                items[n] = (k as u8, pl.discover[k]);
                n += 1;
            }
            if let Some((action, pick)) = self.pick(game, active, &items[..n], flags) {
                let a = Action::Choose(action);
                if game.is_legal(a) {
                    flags[F_ENABLER_TAKEN] = (pick == Pick::Enabler) as i32;
                    return Some(a);
                }
            }
        }
        if let Some(pco) = pl.pending_choose_one {
            let mut items = [(0u8, 0u16); 2];
            let mut n = 0;
            for k in 0..2 {
                if let Some(id) = choose_card(pco.options[k]) {
                    items[n] = (k as u8, id);
                    n += 1;
                }
            }
            if let Some((action, pick)) = self.pick(game, active, &items[..n], flags) {
                let a = Action::Choose(action);
                if game.is_legal(a) {
                    flags[F_ENABLER_TAKEN] = (pick == Pick::Enabler) as i32;
                    return Some(a);
                }
            }
        }
        None
    }

    /// Pick an offered card: the active comp's payoff, then filler, then its
    /// enabler; with no active comp, an enabler of a comp whose tribe is in
    /// the lobby.
    fn pick(
        &self,
        game: &Game,
        active: Option<usize>,
        items: &[(u8, CardId)],
        flags: &mut [i32; OVERLAY_FLAGS],
    ) -> Option<(u8, Pick)> {
        if items.is_empty() {
            return None;
        }
        if items.iter().any(|&(_, id)| {
            self.comps.comps.iter().any(|c| {
                c.is_enabler(id) && self.comp_in_lobby(game, c)
            })
        }) {
            flags[F_ENABLER_OFFERED] = 1;
        }
        if let Some(ci) = active {
            let comp = &self.comps.comps[ci];
            for &(action, id) in items {
                if comp.is_payoff(id) {
                    return Some((action, Pick::Payoff));
                }
            }
            for &(action, id) in items {
                if comp.is_filler(id) {
                    return Some((action, Pick::Filler));
                }
            }
            for &(action, id) in items {
                if comp.is_enabler(id) {
                    return Some((action, Pick::Enabler));
                }
            }
        } else {
            for &(action, id) in items {
                let found = self.comps.comps.iter().any(|c| {
                    c.has_enabler() && c.is_enabler(id) && self.comp_in_lobby(game, c)
                });
                if found {
                    return Some((action, Pick::Enabler));
                }
            }
        }
        None
    }

    fn card_bonus(&self, comp: &Comp, id: CardId) -> f32 {
        if comp.is_payoff(id) || comp.is_enabler(id) {
            self.bonus
        } else if comp.is_filler(id) {
            self.bonus / 2.0
        } else {
            0.0
        }
    }

    fn bonuses(
        &self,
        game: &Game,
        p: usize,
        ci: usize,
        bonus: &mut [f32; N_ACTIONS],
    ) {
        let comp = &self.comps.comps[ci];
        let pl = &game.players[p];
        for s in 0..pl.shop.len() {
            let id = pl.shop[s].card;
            let b = self.card_bonus(comp, id);
            if b > 0.0 {
                let a = Action::Buy(s as u8);
                if game.is_legal(a) {
                    bonus[a.index()] = b;
                }
            }
            if comp.is_enabler_or_payoff(id) {
                bonus[Action::CycleShop(s as u8).index()] = f32::NEG_INFINITY;
            }
        }
        for h in 0..pl.hand.len() {
            let id = pl.hand[h].card;
            let b = self.card_bonus(comp, id);
            if b > 0.0 {
                for pos in 0..=BOARD as u8 {
                    let a = Action::Play { hand: h as u8, pos };
                    if game.is_legal(a) {
                        bonus[a.index()] = b;
                    }
                }
            }
            if comp.is_enabler_or_payoff(id) {
                bonus[Action::CycleHand(h as u8).index()] = f32::NEG_INFINITY;
            }
        }
        for s in 0..pl.board.len() {
            let id = pl.board[s].card;
            if comp.is_enabler_or_payoff(id) {
                bonus[Action::Sell(s as u8).index()] = f32::NEG_INFINITY;
            }
        }
        if pl.board.is_full() {
            let waits = pl.hand.iter().chain(&pl.shop).any(|m| comp.in_comp(m.card));
            if waits {
                for s in 0..pl.board.len() {
                    if !comp.in_comp(pl.board[s].card) {
                        let a = Action::Sell(s as u8);
                        if game.is_legal(a) {
                            bonus[a.index()] = self.bonus;
                        }
                    }
                }
            }
        }
    }
}

/// The card a Choose One option directly gives, if it names one.
fn choose_card(option: ChooseOption) -> Option<CardId> {
    match option {
        ChooseOption::SpellEffect(SpellEffect::GetCard { card, .. })
        | ChooseOption::SpellEffect(SpellEffect::FillHand { card })
        | ChooseOption::SpellEffect(SpellEffect::ReturnEachTurn { card }) => Some(card),
        _ => None,
    }
}
