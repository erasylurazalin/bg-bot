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

/// The Tortollan Blue Shell line for one seat: buy, play, sell as three
/// ordinary actions. `turn` is 0 when no line is running; `slot` is the board
/// slot the line put the Shell in, so a Shell that was already on the board
/// for another reason is not sold.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShellLine {
    pub turn: u32,
    pub slot: Option<u8>,
}

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
        line: &mut ShellLine,
        bonus: &mut [f32; N_ACTIONS],
        flags: &mut [i32; OVERLAY_FLAGS],
    ) -> Option<Action> {
        bonus.fill(0.0);
        flags.fill(0);
        let active = self.active_comp(game, p);
        if active.is_some() {
            flags[F_ACTIVE] = 1;
        }
        if let Some(a) = self.shell(game, p, line, active, flags) {
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
        line: &mut ShellLine,
        active: Option<usize>,
        flags: &mut [i32; OVERLAY_FLAGS],
    ) -> Option<Action> {
        let shell = self.shell?;
        let pl = &game.players[p];
        if line.turn != 0 && line.turn != game.turn {
            *line = ShellLine::default();
        }
        if !pl.lost_last_combat {
            *line = ShellLine::default();
            return None;
        }
        // A line already started: play the Shell in hand, then sell the one
        // this line played. The stored slot tells it from a Shell that was
        // already on the board for another reason.
        if line.turn == game.turn {
            if let Some(h) = pl.hand.iter().position(|m| m.card == shell) {
                let pos = pl.board.len() as u8;
                let a = Action::Play { hand: h as u8, pos };
                if game.is_legal(a) {
                    flags[F_SHELL_LEGAL] = 1;
                    flags[F_SHELL_TAKEN] = 1;
                    line.slot = Some(pos);
                    return Some(a);
                }
                return self.make_room(game, p, active, flags);
            }
            if let Some(slot) = line
                .slot
                .filter(|&slot| {
                    (slot as usize) < pl.board.len() && pl.board[slot as usize].card == shell
                })
                .or_else(|| pl.board.iter().position(|m| m.card == shell).map(|s| s as u8))
            {
                let a = Action::Sell(slot);
                if game.is_legal(a) {
                    flags[F_SHELL_LEGAL] = 1;
                    flags[F_SHELL_TAKEN] = 1;
                    *line = ShellLine::default();
                    return Some(a);
                }
            }
            *line = ShellLine::default();
            return None;
        }
        // Start a line: a Shell in the Tavern is bought first, a Shell in
        // hand is played first. A Shell already on the board is not sold
        // unless this line put it there.
        let shop = pl.shop.iter().position(|m| m.card == shell);
        let hand = pl.hand.iter().position(|m| m.card == shell);
        let shop_ok = shop.is_some_and(|_| {
            pl.gold >= game.data.cards.get(shell).cost && !pl.hand.is_full()
        });
        if shop_ok && !pl.board.is_full() {
            let a = Action::Buy(shop.unwrap() as u8);
            if game.is_legal(a) {
                flags[F_SHELL_LEGAL] = 1;
                flags[F_SHELL_TAKEN] = 1;
                line.turn = game.turn;
                line.slot = None;
                return Some(a);
            }
        }
        if pl.board.is_full() {
            // Only sell for a Shell the line can actually use.
            if !shop_ok && hand.is_none() {
                return None;
            }
            return self.make_room(game, p, active, flags);
        }
        if let Some(h) = hand {
            let pos = pl.board.len() as u8;
            let a = Action::Play { hand: h as u8, pos };
            if game.is_legal(a) {
                flags[F_SHELL_LEGAL] = 1;
                flags[F_SHELL_TAKEN] = 1;
                line.turn = game.turn;
                line.slot = Some(pos);
                return Some(a);
            }
        }
        None
    }

    /// On a full board, sell the weakest card that is not in the active comp.
    /// With no active comp the rule does not know what to sell, so it skips.
    fn make_room(
        &self,
        game: &Game,
        p: usize,
        active: Option<usize>,
        flags: &mut [i32; OVERLAY_FLAGS],
    ) -> Option<Action> {
        let pl = &game.players[p];
        if !pl.board.is_full() {
            return None;
        }
        let ci = active?;
        let comp = &self.comps.comps[ci];
        let sell = (0..pl.board.len())
            .filter(|&s| !comp.in_comp(pl.board[s].card))
            .min_by_key(|&s| {
                let m = &pl.board[s];
                m.attack + m.health
            })?;
        let a = Action::Sell(sell as u8);
        if game.is_legal(a) {
            flags[F_SHELL_LEGAL] = 1;
            flags[F_FORCED_SELL] = 1;
            return Some(a);
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
