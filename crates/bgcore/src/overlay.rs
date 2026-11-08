//! The comp overlay: standing rules and logit bonuses on top of a network.
//!
//! The overlay is not a bot. It returns a forced action or nothing, and a
//! per-action bonus for the network's logits. With no active comp it applies
//! the dominance layer and takes an offered enabler; otherwise it leaves the
//! network alone.
//!
//! The dominance layer is the general form of the Blue Shell play: at a shop
//! decision it tries short buy-play-sell lines on copies of the game and
//! forces the first line whose end strictly dominates the start
//! (`crate::search::Position`). The card names are never consulted.
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
use crate::search::Position;
use crate::{Action, BOARD, DISCOVER, HAND, N_ACTIONS, Data};

/// The overlay's per-decision flags, in `[i32; OVERLAY_FLAGS]` order.
pub const OVERLAY_FLAGS: usize = 6;
/// A new dominating line started at this decision; the action is its first.
pub const F_LINE_FOUND: usize = 0;
/// This action is part of a dominating line (its first action included).
pub const F_LINE_FORCED: usize = 1;
pub const F_ENABLER_OFFERED: usize = 2;
pub const F_ENABLER_TAKEN: usize = 3;
pub const F_ACTIVE: usize = 4;
/// No longer set; kept so the flag array stays six wide.
pub const F_UNUSED: usize = 5;

/// The longest forced line, in ordinary actions. A line buys or plays one
/// card, sells it, then plays and sells what that card generated. The game
/// caps a turn at 60 actions, so this is only a bound on one line.
pub const LINE_MAX: usize = 24;

/// The dominating line for one seat: one ordinary action per step. `turn` is
/// 0 when no line is running. `slot` is kept for the comp planner, which has
/// its own Shell rule; the overlay does not use it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShellLine {
    pub turn: u32,
    pub slot: Option<u8>,
    plan: [u8; LINE_MAX],
    len: u8,
    next: u8,
}

impl Default for ShellLine {
    fn default() -> ShellLine {
        ShellLine { turn: 0, slot: None, plan: [0; LINE_MAX], len: 0, next: 0 }
    }
}

impl ShellLine {
    fn clear(&mut self) {
        *self = ShellLine::default();
    }

    fn start(&mut self, turn: u32, plan: &Plan) {
        *self = ShellLine::default();
        self.turn = turn;
        self.len = plan.len as u8;
        for (slot, a) in self.plan.iter_mut().zip(plan.actions.iter()) {
            *slot = a.index() as u8;
        }
        self.next = 0;
    }

    fn next_action(&mut self) -> Option<Action> {
        if self.next >= self.len {
            return None;
        }
        let a = Action::from_index(self.plan[self.next as usize] as usize)?;
        self.next += 1;
        Some(a)
    }
}

/// One judged line: its ordinary actions in order.
#[derive(Clone, Copy)]
struct Plan {
    actions: [Action; LINE_MAX],
    len: usize,
}

impl Plan {
    fn new() -> Plan {
        Plan { actions: [Action::Roll; LINE_MAX], len: 0 }
    }

    fn push(&mut self, a: Action) -> Option<()> {
        if self.len >= LINE_MAX {
            return None;
        }
        self.actions[self.len] = a;
        self.len += 1;
        Some(())
    }
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
    /// Whether the dominance layer runs (on unless `set_dominance(false)`).
    pub dominance: bool,
}

impl Overlay {
    pub fn new(comps: Comps, bonus: f32, _data: &Data) -> Overlay {
        Overlay { comps, bonus, dominance: true }
    }

    /// Turn the dominance layer on or off without reloading the comp library.
    pub fn set_dominance(&mut self, on: bool) {
        self.dominance = on;
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
        // A line already judged plays its next action, one per decision.
        if line.turn != 0 && line.turn != game.turn {
            line.clear();
        }
        if line.turn == game.turn {
            if let Some(a) = line.next_action() {
                if game.is_legal(a) {
                    flags[F_LINE_FORCED] = 1;
                    if line.next >= line.len {
                        line.clear();
                    }
                    return Some(a);
                }
                line.clear();
            } else {
                line.clear();
            }
        }
        if let Some(a) = self.choice(game, p, active, flags) {
            return Some(a);
        }
        if self.dominance
            && let Some(plan) = self.dominating_line(game, p)
        {
            line.start(game.turn, &plan);
            let a = line.next_action()?;
            flags[F_LINE_FOUND] = 1;
            flags[F_LINE_FORCED] = 1;
            if line.next >= line.len {
                line.clear();
            }
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

    /// The first dominating line from this shop decision, if any. Lines are
    /// tried in the search's order: each Tavern minion, then each hand card.
    fn dominating_line(&self, game: &Game, p: usize) -> Option<Plan> {
        if !game.shopping(p) {
            return None;
        }
        let pl = &game.players[p];
        // A line plays the card it bought or took from hand, so it needs a
        // free board slot. On a full board the only way to make one is to
        // sell a minion first, and a sale on its own loses a minion: no line
        // from a full board can dominate.
        if pl.board.is_full() {
            return None;
        }
        let pos = pl.board.len() as u8;
        for s in 0..pl.shop.len() {
            let a = Action::Buy(s as u8);
            if game.is_legal(a)
                && let Some(plan) = self.line_from(game, p, a)
            {
                return Some(plan);
            }
        }
        for h in 0..pl.hand.len() {
            let a = Action::Play { hand: h as u8, pos };
            if game.is_legal(a)
                && let Some(plan) = self.line_from(game, p, a)
            {
                return Some(plan);
            }
        }
        None
    }

    /// Try one line: buy (for a Tavern card) and play the source card, sell
    /// it, then play and sell whatever the line put in the hand. A line that
    /// opens a Discover, a Choose One or another answer is dropped: the
    /// overlay would answer it, but answering on a copy would need the
    /// network's choice, and the short lines this layer judges have no
    /// business making one. A generated card that cannot be played is left in
    /// the hand, which the dominance judge counts as the gain it is.
    fn line_from(&self, game: &Game, p: usize, source: Action) -> Option<Plan> {
        let before = Position::of(game, p);
        let pl = &game.players[p];
        let hand_before: [CardId; HAND] =
            std::array::from_fn(|i| pl.hand.get(i).map_or(CardId::MAX, |m| m.card));
        let hand_len = pl.hand.len();
        let mut sim = game.clone();
        let mut plan = Plan::new();
        let (card, played_pos) = match source {
            Action::Buy(s) => {
                let card = pl.shop.get(s as usize)?.card;
                let pos = pl.board.len() as u8;
                plan.push(source)?;
                sim.step(source).ok()?;
                let hand = sim.players[p].hand.iter().rposition(|m| m.card == card)?;
                let play = Action::Play { hand: hand as u8, pos };
                plan.push(play)?;
                sim.step(play).ok()?;
                (card, pos)
            }
            Action::Play { hand, pos } => {
                let card = pl.hand.get(hand as usize)?.card;
                plan.push(source)?;
                sim.step(source).ok()?;
                (card, pos)
            }
            _ => return None,
        };
        // A source whose play waits on an answer is out: the layer leaves
        // Discover, Choose One, target and discard lines alone.
        if !sim.shopping(p) || sim.current_player() != Some(p) {
            return None;
        }
        if sim.players[p].board.get(played_pos as usize).map(|m| m.card) != Some(card) {
            return None;
        }
        // What the play put in the hand, as a multiset difference, before the
        // played card is sold (a sale may give cards too; those are gains).
        let mut generated = [0 as CardId; HAND];
        let mut n_generated = 0;
        let mut used = [false; HAND];
        for m in &sim.players[p].hand {
            if let Some(i) = (0..hand_len).find(|&i| !used[i] && hand_before[i] == m.card) {
                used[i] = true;
            } else if n_generated < HAND {
                generated[n_generated] = m.card;
                n_generated += 1;
            }
        }
        // Sell the played card first, which frees its slot for the cards the
        // play generated. The reverse order (generated first) cannot help on
        // a full board and gives up the slot a generated card needs.
        let sell = Action::Sell(played_pos);
        if sim.is_legal(sell) {
            plan.push(sell)?;
            sim.step(sell).ok()?;
        }
        for &card in &generated[..n_generated] {
            let Some(hand) = sim.players[p].hand.iter().position(|m| m.card == card) else {
                continue;
            };
            let pos = sim.players[p].board.len() as u8;
            if pos >= BOARD as u8 {
                // No slot for it: it stays in the hand, a gain.
                continue;
            }
            let play = Action::Play { hand: hand as u8, pos };
            if !sim.is_legal(play) {
                continue;
            }
            plan.push(play)?;
            sim.step(play).ok()?;
            if !sim.shopping(p) || sim.current_player() != Some(p) {
                return None;
            }
            // A spell leaves no minion at the slot: only sell what was put
            // there, never the minion that happened to stand in that slot.
            if sim.players[p].board.get(pos as usize).map(|m| m.card) != Some(card) {
                continue;
            }
            let sell = Action::Sell(pos);
            if sim.is_legal(sell) {
                plan.push(sell)?;
                sim.step(sell).ok()?;
            }
        }
        if !sim.shopping(p) || sim.current_player() != Some(p) {
            return None;
        }
        if before.judge(&sim).dominates { Some(plan) } else { None }
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
