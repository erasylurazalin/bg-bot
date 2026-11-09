//! The comp overlay: standing rules and logit bonuses on top of a network.
//!
//! The overlay is not a bot. It returns a forced action or nothing, and a
//! per-action bonus for the network's logits. With no active comp it only
//! applies the Shell rule and takes an offered enabler; otherwise it leaves
//! the network alone.
//!
//! A comp is ACTIVE for a player once one of its enabler entries is complete:
//! every group has one of its cards on that player's board or in their hand,
//! the comp's lobby tribes are all present, and its abort rule has not fired.
//! While active: an enabler or payoff cannot be sold (the bonus is `-inf`),
//! buying or playing a payoff is worth `B`, a filler `B/2`, and on a full
//! board selling a card that is not in the comp is worth `B` when a comp card
//! is waiting in the hand or Tavern.

use crate::cards::{ChooseOption, SpellEffect, Tribe};
use crate::comps::{Comp, Comps};
use crate::game::{Game, TargetKind};
use crate::minion::CardId;
use crate::{Action, BOARD, DISCOVER, N_ACTIONS, Data};

/// The overlay's per-decision flags, in `[i32; OVERLAY_FLAGS]` order.
pub const OVERLAY_FLAGS: usize = 7;
pub const F_SHELL_LEGAL: usize = 0;
pub const F_SHELL_TAKEN: usize = 1;
pub const F_ENABLER_OFFERED: usize = 2;
pub const F_ENABLER_TAKEN: usize = 3;
pub const F_ACTIVE: usize = 4;
pub const F_FORCED_SELL: usize = 5;
/// The active comp's index plus one, or zero when no comp is active.
pub const F_COMP: usize = 6;

/// The Tortollan Blue Shell line for one seat: buy, play, sell as three
/// ordinary actions. `turn` is 0 when no line is running; `slot` is the board
/// slot the line put the Shell in, so a Shell that was already on the board
/// for another reason is not sold.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShellLine {
    pub turn: u32,
    pub slot: Option<u8>,
}

/// The overlay's per-seat, per-game state: the Shell line and one bit per comp
/// whose abort rule has fired. A bit stays set for the rest of the game, so an
/// aborted comp's enabler never activates it later.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OverlayState {
    pub shell: ShellLine,
    pub aborted: u64,
}

impl OverlayState {
    pub fn is_aborted(&self, comp: usize) -> bool {
        comp < crate::comps::MAX_COMPS && self.aborted & (1 << comp) != 0
    }

    fn abort(&mut self, comp: usize) {
        if comp < crate::comps::MAX_COMPS {
            self.aborted |= 1 << comp;
        }
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
    shell: Option<CardId>,
    wrath_weaver: Option<CardId>,
}

impl Overlay {
    pub fn new(comps: Comps, bonus: f32, data: &Data) -> Overlay {
        Overlay {
            comps,
            bonus,
            shell: data.cards.by_name("Tortollan Blue Shell"),
            wrath_weaver: data.cards.by_name("Wrath Weaver"),
        }
    }

    /// The comp that is active for `p`, if any. Several comps can be active;
    /// the first in file order wins (the order the file lists them in).
    pub fn active_comp(&self, game: &Game, p: usize, state: &OverlayState) -> Option<usize> {
        let pl = &game.players[p];
        for (i, c) in self.comps.comps.iter().enumerate() {
            if !state.is_aborted(i)
                && c.has_enabler()
                && c.lobby_ok(&game.lobby)
                && c.enabler_held(&pl.board, &pl.hand)
                && !c.abort_condition(game.turn, pl.tier, &pl.board, &pl.hand)
            {
                return Some(i);
            }
        }
        None
    }

    /// Fill `bonus` for one decision and return the forced action, if any.
    /// `bonus` is zero for no change and `-inf` for a masked action.
    pub fn decide(
        &self,
        game: &Game,
        p: usize,
        state: &mut OverlayState,
        bonus: &mut [f32; N_ACTIONS],
        flags: &mut [i32; OVERLAY_FLAGS],
    ) -> Option<Action> {
        bonus.fill(0.0);
        flags.fill(0);
        // Latch every comp whose abort condition is true now; the state keeps
        // it latched for the rest of the game.
        let pl = &game.players[p];
        for i in 0..self.comps.comps.len() {
            if !state.is_aborted(i)
                && self.comps.comps[i].abort_condition(game.turn, pl.tier, &pl.board, &pl.hand)
            {
                state.abort(i);
            }
        }
        let active = self.active_comp(game, p, state);
        if let Some(ci) = active {
            flags[F_ACTIVE] = 1;
            flags[F_COMP] = ci as i32 + 1;
        }
        // Era's Health-loss rule: a Health cost on the player's own turn is
        // heavily punished while no rewind (Soul Rewinder or Ashen Corruptor)
        // is on the board. It is a rule for every comp and every card, so it
        // is applied before and regardless of the active comp.
        self.health_penalties(game, p, bonus);
        if let Some(a) = self.shell(game, p, &mut state.shell, active, flags) {
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

    /// `decide` on a copy of `state`: the forced action and flags the overlay
    /// would give, without starting a Shell line or latching an abort. Used
    /// to measure the network with the wheels off.
    pub fn decide_probe(
        &self,
        game: &Game,
        p: usize,
        state: &OverlayState,
        bonus: &mut [f32; N_ACTIONS],
        flags: &mut [i32; OVERLAY_FLAGS],
    ) -> Option<Action> {
        let mut state = *state;
        self.decide(game, p, &mut state, bonus, flags)
    }

    fn comp_in_lobby(&self, game: &Game, comp: &Comp) -> bool {
        comp.lobby_ok(&game.lobby)
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
            if let Some((action, pick)) = self.pick(game, p, active, &items[..n], flags) {
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
            if let Some((action, pick)) = self.pick(game, p, active, &items[..n], flags) {
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
    /// enabler; with no active comp, a card that completes an enabler of a
    /// comp whose tribe is in the lobby. Half of a two-card enabler is not forced.
    fn pick(
        &self,
        game: &Game,
        p: usize,
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
            let pl = &game.players[p];
            for &(action, id) in items {
                let found = self.comps.comps.iter().any(|c| {
                    c.has_enabler()
                        && c.completes_enabler(id, &pl.board, &pl.hand)
                        && self.comp_in_lobby(game, c)
                });
                if found {
                    return Some((action, Pick::Enabler));
                }
            }
        }
        None
    }

    /// Era's Health-loss rule (`docs/notes/era-strategy.md` "Taking damage on
    /// your own turn"): any action that costs the hero Health while no Soul
    /// Rewinder or Ashen Corruptor is on the board gets a large negative
    /// bonus. Both cards are the engine's `can_rewind`; the rule applies with
    /// no active comp and to every comp and card.
    fn health_penalties(&self, game: &Game, p: usize, bonus: &mut [f32; N_ACTIONS]) {
        let pen = self.comps.defaults.health_loss_penalty;
        if pen >= 0.0 {
            return;
        }
        let pl = &game.players[p];
        // A rewind on the board gives the Health back, so there is no loss to
        // punish. The engine's check is exactly Soul Rewinder or Ashen
        // Corruptor (the only two cards with a rewind).
        if pl.can_rewind(&game.data.cards) {
            return;
        }
        let mut punish = |a: Action| {
            if game.is_legal(a) {
                bonus[a.index()] += pen;
            }
        };
        for s in 0..pl.shop.len() {
            let m = &pl.shop[s];
            let c = game.data.cards.get(m.card);
            let costs_health = if c.is_spell() {
                c.pay_health || pl.tp.spell_health_left > 0
            } else {
                game.minion_costs_health(p, s)
            };
            if costs_health {
                punish(Action::Buy(s as u8));
            }
        }
        if let Some(id) = pl.shop_spell
            && (game.data.cards.get(id).pay_health || pl.tp.spell_health_left > 0)
        {
            punish(Action::BuySpell);
        }
        // A Refresh paid in Health (Malchezaar, Prince of Dance, or a
        // trinket's Health Refreshes).
        if pl.free_rolls == 0 && pl.can_refresh_with_health(&game.data.cards, &game.data.rules) {
            punish(Action::Roll);
        }
        // Playing a Demon while Wrath Weaver is on the board hurts the hero.
        // Wrath Weaver's own play is excluded by the engine (`just_played`),
        // and a Golden one triggers twice.
        if let Some(ww) = self.wrath_weaver {
            for h in 0..pl.hand.len() {
                let m = &pl.hand[h];
                if game.data.cards.get(m.card).is_spell()
                    || !game.data.cards.tribes_of(m).has(Tribe::Demon)
                {
                    continue;
                }
                let hits: i32 = pl
                    .board
                    .iter()
                    .filter(|b| b.card == ww)
                    .map(|b| if b.golden { 2 } else { 1 })
                    .sum();
                if hits == 0 {
                    continue;
                }
                if let Some(pos) = (0..=BOARD as u8)
                    .find(|&pos| game.is_legal(Action::Play { hand: h as u8, pos }))
                {
                    punish(Action::Play { hand: h as u8, pos });
                }
            }
        }
        // Imposing Percussionist's Discover: the pick costs Health equal to
        // the chosen Demon's Tier.
        if pl.discover_damage_from.is_some() {
            for k in 0..pl.discover.len().min(DISCOVER) {
                punish(Action::Choose(k as u8));
            }
        }
        // Graveyard Shift's card steal takes Health in the Tavern.
        if let Some(pt) = pl.pending_target
            && matches!(pt.kind, TargetKind::StealShopCard { damage } if damage > 0)
        {
            for s in 0..=pl.shop.len() {
                punish(Action::TargetShop(s as u8));
            }
        }
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
