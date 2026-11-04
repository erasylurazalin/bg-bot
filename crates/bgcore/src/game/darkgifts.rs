//! Dark Gifts in a game: the landing (a gift onto a minion), the Dark
//! Discovery button and its offers, the gifts' start-of-turn work, and the
//! two hooks the gift tokens read (the "whenever you play a card" buffs and
//! the "for each ... this game" counters).
//!
//! The gift table, its windows and its conditions live in
//! `crate::darkgifts`; what a gift does is a token in cards.toml.

use arrayvec::ArrayVec;

use crate::cards::{Card, CountdownEffect, EndOfTurn, EventKind};
use crate::darkgifts::{DarkGiftPlan, OfferSlot};
use crate::minion::{CardId, GiftOn, Minion};
use crate::{BOARD, DISCOVER};

use super::shop::golden_up;
use super::{DiscoverRequest, Game, Player};

/// A countdown gift whose count ran out at the end of the last turn and whose
/// effect waits for the start of this one, in the gift's own counter
/// (`GiftOn::state`). No countdown gift's count ever reaches it (they hold
/// two turns at most), so it marks a spent gift that has not paid out yet,
/// and a gift the log shows at 0 keeps meaning one that already paid.
const PAYOUT_PENDING: u8 = crate::minion::GIFT_STATE_MAX;

/// A gift lands on `m`: its stats, its keywords, its counter and whatever
/// the landing itself applies (Gilding makes the minion Golden; the "for
/// each ... this game" gifts take their count now). `turn` is the turn the
/// gift was offered, which fixes the numbers of the turn-dependent ones. A
/// gift never doubles: a golden host gets what a plain one does.
pub fn land_gift(data: &crate::Data, pl: &Player, m: &mut Minion, gift: usize, turn: u32) {
    {
        let Some(g) = data.gifts.gifts.get(gift) else { return };
        let card_id = g.card;
        let c = data.cards.get(card_id);
        // A gift that sets a state instead of firing again never stacks; a
        // second one changes nothing (docs/rules/darkgifts.md "Gifts stack").
        if c.gift_unique && m.gift_slice().iter().any(|o| o.card() == card_id) {
            return;
        }
        let (attack, health) = (c.attack, c.health);
        m.attack += attack;
        m.health += health;
        m.keywords.0 |= c.keywords.0;
        if c.shield_hits > 1 {
            m.shield_hits = m.shield_hits.max(c.shield_hits);
        }
        let makes_golden = c.makes_golden;
        let countdown = c.countdown;
        let growth = matches!(c.end_of_turn, Some(EndOfTurn::GiftBuffSelf));
        let counted: ArrayVec<Option<(i32, i32)>, 4> = [
            c.buff_per_battlecry,
            c.buff_per_tavern_spell,
            c.buff_per_discard,
            c.buff_per_deathrattle,
        ]
        .into_iter()
        .collect();
        // The gift's own counter: a countdown in turns, or the turn Steady
        // Growth landed, which fixes its numbers (they are read back at the
        // end of every turn from darkgifts.toml).
        let mut state = 0u8;
        if let Some(cd) = countdown {
            state = cd.turns;
        }
        if growth {
            state = turn.min(crate::minion::GIFT_STATE_MAX as u32) as u8;
        }
        let pushed = if c.gift_unique {
            m.push_gift_unique(GiftOn::new(card_id, state))
        } else {
            m.push_gift(GiftOn::new(card_id, state))
        };
        if !pushed {
            // No room for one more gift: the stats still came.
            if makes_golden {
                golden_up(pl, &data.cards, m);
            }
            return;
        }
        if makes_golden {
            golden_up(pl, &data.cards, m);
        }
        let counts = [
            pl.battlecries_triggered as i32,
            pl.spells_cast as i32,
            pl.discarded as i32,
            pl.deathrattles_triggered as i32,
        ];
        let (mut da, mut dh) = (0, 0);
        for (k, c) in counted.iter().enumerate() {
            if let Some((a, h)) = c {
                da += a * counts[k];
                dh += h * counts[k];
            }
        }
        m.attack += da;
        m.health += dh;
    }
}

impl Game {
    /// A gift lands on the player's minion `m` (the wrapper the shop-side
    /// paths use).
    pub(super) fn apply_gift(&mut self, p: usize, m: &mut Minion, gift: usize, turn: u32) {
        land_gift(&self.data, &self.players[p], m, gift, turn);
    }

    /// Take a minion into the hand the way a card path does (the "when you
    /// get" hooks, the in-hand triggers and Double Vision's copy). The buy,
    /// the Discover and the generated-card paths all go through
    /// `add_to_hand`; tests use this one.
    pub fn take_into_hand(&mut self, p: usize, m: Minion) -> bool {
        self.add_to_hand(p, m)
    }

    /// Magnetize the minion `m` into the board minion in slot `pos`, with
    /// every hook a real Magnetization runs. The play path and tests use it.
    pub fn magnetize_onto(&mut self, p: usize, pos: usize, m: Minion) {
        self.magnetize_to(p, pos, m, false);
    }

    /// Give the board minion in slot `pos` the named Dark Gift, as if it
    /// landed now. The sandbox's `gift` command and the tests use it; false
    /// for an unknown name or an empty slot.
    pub fn give_gift(&mut self, p: usize, pos: usize, name: &str, turn: u32) -> bool {
        let Some(i) = self.data.gifts.gifts.iter().position(|g| g.name == name) else {
            return false;
        };
        let mut m = match self.players[p].board.get(pos) {
            Some(m) => *m,
            None => return false,
        };
        self.apply_gift(p, &mut m, i, turn);
        self.players[p].board[pos] = m;
        true
    }

    /// The Dark Discovery button: every player has it, it costs
    /// `dark_gift_cost` Gold, works once a turn, has `dark_gift_uses` uses a
    /// game and is locked until `dark_gift_turn` (docs/rules/darkgifts.md).
    pub(super) fn dark_gift_usable(&self, p: usize) -> bool {
        if self.data.gifts.is_empty() || self.turn < self.data.rules.dark_gift_turn as u32 {
            return false;
        }
        let pl = &self.players[p];
        pl.dark_gift_uses > 0
            && !pl.dark_gift_pressed
            && pl.gold >= self.data.rules.dark_gift_cost
            && pl.discover.is_empty()
            && pl.discover_queue.is_empty()
            && pl.pending_target.is_none()
            && pl.pending_choose_one.is_none()
            && pl.pending_shop_target.is_none()
            && pl.pending_discard.is_none()
            && pl.power_offer.is_empty()
            && pl.secret_offer.is_empty()
            && pl.trinket_offer.is_empty()
            && !pl.skipped
            && !pl.done
    }

    /// Press the button: pay, spend a use and open the offer.
    pub(super) fn press_dark_gift(&mut self, p: usize) {
        if !self.dark_gift_usable(p) {
            return;
        }
        let cost = self.data.rules.dark_gift_cost;
        self.players[p].gold -= cost;
        self.players[p].dark_gift_uses -= 1;
        self.players[p].dark_gift_pressed = true;
        self.fire_event(p, EventKind::GoldSpent, cost as u32);
        self.open_dark_gift_offer(p);
    }

    /// The button's offer: this turn's three tiers, one gift each.
    pub(super) fn open_dark_gift_offer(&mut self, p: usize) {
        let Some(step) = self.data.gifts.offer_step(self.turn).copied() else { return };
        let plan = DarkGiftPlan {
            slots: step.slots,
            gift_turn: self.turn,
            golden: false,
            locked_turns: 0,
        };
        let _ = self.players[p].discover_queue.try_push(DiscoverRequest::DarkGift(plan));
        self.maybe_start_discover(p);
    }

    /// A gift trinket's offer (Ominous Stone, Wax Lance, Sinister
    /// Invitation). `tier` is fixed, or the button's tiers for this turn when
    /// `use_table`; `yours` asks for minions of the player's most common
    /// type, `golden` makes every offer Golden.
    pub fn open_dark_gift_trinket(
        &mut self,
        p: usize,
        tier: Option<u8>,
        yours: bool,
        golden: bool,
        use_table: bool,
    ) {
        let slots = match tier {
            Some(t) => [OfferSlot { tiers: 1 << t, yours }; DISCOVER],
            None => {
                let mut slots = self.current_offer_slots();
                if yours {
                    for s in &mut slots {
                        s.yours = true;
                    }
                }
                slots
            }
        };
        let _ = use_table;
        let plan = DarkGiftPlan { slots, gift_turn: self.turn, golden, locked_turns: 0 };
        let _ = self.players[p].discover_queue.try_push(DiscoverRequest::DarkGift(plan));
        self.maybe_start_discover(p);
    }

    /// The button's tiers for this turn, for the gift sources whose text
    /// gives no tier (Sinister Invitation, Xavius).
    pub(super) fn current_offer_slots(&self) -> [OfferSlot; DISCOVER] {
        self.data
            .gifts
            .offer_step(self.turn.max(1))
            .map(|s| s.slots)
            .unwrap_or([OfferSlot { tiers: 1 << 4, yours: false }; DISCOVER])
    }

    /// The player's most common minion type on the board (the hand is not
    /// counted), a tie broken at random. None when no minion on it has a
    /// type.
    pub fn most_common_board_type(&mut self, p: usize) -> Option<crate::cards::Tribe> {
        let tribes: ArrayVec<crate::cards::Tribes, BOARD> = self.players[p]
            .board
            .iter()
            .map(|m| self.data.cards.tribes_of(m))
            .collect();
        crate::darkgifts::most_common_type(&tribes, &mut self.rng)
    }

    /// The board's most common type read off the cards alone, the way the
    /// Dark Discovery's "your type" slot reads it: a tribe a portrait gave a
    /// minion (Bronzebeard Portrait's Branns counting as Murlocs and Dragons)
    /// does not count there (docs/rules/trinkets.md, Era's rule).
    fn most_common_card_type(&mut self, p: usize) -> Option<crate::cards::Tribe> {
        let tribes: ArrayVec<crate::cards::Tribes, BOARD> = self.players[p]
            .board
            .iter()
            .map(|m| self.data.cards.get(m.card).tribes)
            .collect();
        crate::darkgifts::most_common_type(&tribes, &mut self.rng)
    }

    /// Deal a Dark Gift offer into the player's Discover: one pool minion per
    /// slot, each with a gift, three different minions and three different
    /// gifts (docs/rules/darkgifts.md "Era's rules"). Each offer holds a pool copy.
    pub(super) fn deal_dark_gift_offer(&mut self, p: usize, plan: DarkGiftPlan) {
        let mut used: ArrayVec<u8, DISCOVER> = ArrayVec::new();
        let mut taken: ArrayVec<CardId, DISCOVER> = ArrayVec::new();
        for k in 0..DISCOVER {
            let slot = plan.slots[k];
            let mine = if slot.yours { self.most_common_card_type(p) } else { None };
            let Some(card) = self.draw_gift_minion(plan.gift_turn, slot, mine, &taken) else {
                continue;
            };
            let gift = {
                let c = self.data.cards.get(card);
                let deity = self.players[p].deity.is_some();
                // A Golden offer's minion is already golden, so the gifts that
                // make it golden (Gilding) are not offered on it (Era
                // 2026-11-03, docs/rules/darkgifts.md).
                self.data
                    .gifts
                    .pick(plan.gift_turn, c, c.tribes, deity, &used, plan.golden, &mut self.rng)
            };
            let Some(gift) = gift else {
                // Nothing fits it (a pool this thin): the copy goes back.
                let slot = self.data.cards.get(card).pool_slot as usize;
                self.pool[slot] += 1;
                continue;
            };
            let _ = used.try_push(gift as u8);
            let _ = taken.try_push(card);
            let gift_card = self.data.gifts.gifts[gift].card;
            let pl = &mut self.players[p];
            pl.discover.push(card);
            pl.discover_gifts.push(GiftOn::new(gift_card, 0));
            pl.discover_copies.push(1);
            pl.dark_offer = true;
            pl.dark_offer_turn = plan.gift_turn;
            pl.dark_offer_golden = plan.golden;
            pl.dark_offer_locked = plan.locked_turns;
        }
        // A Golden offer stands for the three pool copies a Golden minion
        // takes (Gilding's own offer, and every offer of Sinister
        // Invitation), as many as the pool still has.
        let golden_offers: ArrayVec<usize, DISCOVER> = (0..self.players[p].discover.len())
            .filter(|&k| {
                self.players[p].dark_offer_golden
                    || self
                        .data
                        .cards
                        .get(self.players[p].discover_gifts[k].card())
                        .makes_golden
            })
            .collect();
        for k in golden_offers {
            let card = self.players[p].discover[k];
            let slot = self.data.cards.get(card).pool_slot as usize;
            let extra = self.pool[slot].min(2);
            self.pool[slot] -= extra;
            self.players[p].discover_copies[k] += extra;
        }
        if self.players[p].discover.is_empty() {
            self.players[p].clear_dark_offer();
        }
    }

    /// The Maldraxxus Dagger's offer: plain copies of three of the player's
    /// own minions, gifted or not, each with a NEW Dark Gift picked for that
    /// minion by the normal rules (its window and host condition for this
    /// turn, never a gift twice within the offer). A copy is generated, so it
    /// takes no pool copy. This is the one gift offer whose sources may be
    /// Magnetic minions: they are the player's, not the pool's, so the pool's
    /// exclusions do not apply (Era: the one way a Magnetic minion gets a
    /// gift).
    pub fn deal_dark_gift_copies(&mut self, p: usize, turn: u32) {
        // Three different minions, the way a Discover always offers three
        // different cards.
        let mut pool: ArrayVec<CardId, BOARD> = ArrayVec::new();
        for m in &self.players[p].board {
            if !pool.contains(&m.card) {
                let _ = pool.try_push(m.card);
            }
        }
        if pool.is_empty() {
            return;
        }
        let mut used: ArrayVec<u8, DISCOVER> = ArrayVec::new();
        for _ in 0..DISCOVER {
            if pool.is_empty() {
                break;
            }
            let k = self.rng.below(pool.len() as u32) as usize;
            let card = pool.remove(k);
            // The host is one of your minions of that card: its types read
            // through `tribes_of`, so an Amalgamation host counts for every
            // type its gift may want.
            let Some(host) = self.players[p].board.iter().find(|m| m.card == card).copied() else {
                continue;
            };
            let tribes = self.data.cards.tribes_of(&host);
            let deity = self.players[p].deity.is_some();
            let gift = self
                .data
                .gifts
                .pick(turn, self.data.cards.get(card), tribes, deity, &used, false, &mut self.rng);
            // No gift fits it (only on a turn with fewer than three gifts
            // open): the offer has fewer options.
            let Some(gift) = gift else { continue };
            let _ = used.try_push(gift as u8);
            let gift_card = self.data.gifts.gifts[gift].card;
            let pl = &mut self.players[p];
            pl.discover.push(card);
            pl.discover_gifts.push(GiftOn::new(gift_card, 0));
            pl.discover_copies.push(0);
            pl.dark_offer = true;
            pl.dark_offer_turn = turn;
            pl.dark_offer_golden = false;
            pl.dark_offer_locked = 0;
        }
    }

    /// One pool minion for a gift offer: its tier picked among the slot's
    /// tiers, of the player's type when the slot asks for it, never one
    /// already offered this press and never one the gift pool leaves out.
    /// The fallbacks, in order: drop the "your type" rule, then try the
    /// slot's other tiers. None when the pool has nothing at all.
    fn draw_gift_minion(
        &mut self,
        turn: u32,
        slot: OfferSlot,
        mine: Option<crate::cards::Tribe>,
        taken: &[CardId],
    ) -> Option<CardId> {
        let mut tiers: ArrayVec<u8, 8> = ArrayVec::new();
        for t in 1..=crate::CARD_MAX_TIER {
            if slot.tiers & (1 << t) != 0 {
                let _ = tiers.try_push(t);
            }
        }
        let require_type: [bool; 2] = if mine.is_some() && slot.yours {
            [true, false]
        } else {
            [false, false]
        };
        for want in require_type {
            // One tier at random among the slot's, then the others in turn,
            // so a tier the pool is out of does not cost the slot its offer.
            let start = if tiers.is_empty() {
                0
            } else {
                self.rng.below(tiers.len() as u32) as usize
            };
            for k in 0..tiers.len().max(1) {
                let tier = if tiers.is_empty() { 0 } else { tiers[(start + k) % tiers.len()] };
                let found = self.pick_gift_pool(turn, tier, mine.filter(|_| want), taken);
                if let Some(card) = found {
                    let slot = self.data.cards.get(card).pool_slot as usize;
                    self.pool[slot] -= 1;
                    return Some(card);
                }
                if tiers.is_empty() {
                    break;
                }
            }
            if want && !slot.yours {
                break;
            }
        }
        None
    }

    /// A random pool card with a copy left that a gift offer may show: a
    /// minion of `tier` (any when 0), of `tribe` when given, inside the
    /// lobby, not taken yet.
    fn pick_gift_pool(
        &mut self,
        turn: u32,
        tier: u8,
        tribe: Option<crate::cards::Tribe>,
        taken: &[CardId],
    ) -> Option<CardId> {
        let cards = &self.data.cards;
        let gifts = &self.data.gifts;
        let pool = &self.pool;
        let lobby = self.lobby;
        let ok = |id: CardId| {
            let c = cards.get(id);
            (tier == 0 || c.tier == tier)
                && c.lobby_ok(lobby)
                && gifts.pool_allows(c, id, turn)
                && tribe.is_none_or(|t| c.tribes.has(t))
                && !taken.contains(&id)
                && pool[c.pool_slot as usize] > 0
        };
        let candidates: ArrayVec<CardId, 256> = (0..cards.pool_cards as CardId)
            .filter(|&c| cards.get(c).pool_slot == c)
            .filter(|&c| ok(c))
            .collect();
        if candidates.is_empty() {
            return None;
        }
        Some(candidates[self.rng.below(candidates.len() as u32) as usize])
    }

    /// The start-of-turn gift work for one player: the countdown payouts that
    /// wait for the start of the turn (see `PAYOUT_PENDING`), a copy a
    /// Replication payout handed over at the end of the last turn gets the
    /// triple check it could not have then, and a minion's Time Turning runs
    /// its end of turn effects again. The countdown gifts (Incubation,
    /// Replication and Affinity) do *not* tick here: they count down at the
    /// END of the turn (docs/rules/darkgifts.md, Era 2026-11-02).
    pub fn dark_gift_start_of_turn(&mut self, p: usize) {
        // The payouts a gift's count ran out on at the end of the last turn,
        // before anything can move the minions they sit on: Incubation's
        // doubling, which the game lands on the next turn's board (see
        // `dark_gift_end_of_turn`). It runs once: Incubation's text says "in 2
        // turns", not "at the end of", so it is a start-of-turn effect and
        // Drakkari Enchanter doesn't repeat it (Era, 2026-11-04).
        if !self.data.gifts.is_empty() {
            let runs = 1;
            let mut pos = 0;
            while pos < self.players[p].board.len() {
                let m = self.players[p].board[pos];
                let mut fire: ArrayVec<CountdownEffect, { crate::MAX_GIFTS }> = ArrayVec::new();
                let mut spent: ArrayVec<usize, { crate::MAX_GIFTS }> = ArrayVec::new();
                if m.n_gifts > 0 {
                    for (k, g) in m.gift_slice().iter().enumerate() {
                        if g.state() != PAYOUT_PENDING {
                            continue;
                        }
                        let Some(cd) = self.data.cards.get(g.card()).countdown else { continue };
                        if self.gift_repeats(g.card()) {
                            self.players[p].board[pos].set_gift_state_at(k, cd.turns);
                        } else {
                            let _ = spent.try_push(k);
                        }
                        let _ = fire.try_push(cd.effect);
                    }
                }
                for &k in spent.iter().rev() {
                    self.players[p].board[pos].remove_gift_at(k);
                }
                for &effect in &fire {
                    for _ in 0..runs {
                        self.run_countdown(p, pos, effect);
                    }
                }
                pos += 1;
            }
        }
        // Nothing triples during the fights, so a payout's copy that makes a
        // third copy merges now: three copies become the golden one in the
        // hand, gifts and all (Era 2026-11-03, `check_triple`).
        while let Some(card) = self.players[p].triples_next_turn.pop() {
            self.check_triple(p, card);
        }
        if self.data.gifts.is_empty() {
            return;
        }
        let mut pos = 0;
        while pos < self.players[p].board.len() {
            let m = self.players[p].board[pos];
            let mut turning = false;
            if m.n_gifts > 0 {
                for g in m.gift_slice() {
                    if self.data.cards.get(g.card()).end_of_turn_at_start {
                        turning = true;
                    }
                }
            }
            if turning {
                let mut at = pos;
                self.run_minion_end_of_turn(p, &mut at, 1);
            }
            pos += 1;
        }
    }

    /// The end-of-turn gift work for one player: every countdown gift ticks
    /// down a turn, and the one that reaches 0 fires. A gift marked
    /// `repeats` in darkgifts.toml (Replication, Affinity) stays on the
    /// minion and starts its count again; the rest (Incubation) comes off.
    /// The payout runs `drakkari_mult` times (Drakkari Enchanter sets 2,
    /// golden 3; Ghastly Sticker adds one), never the count: a gift fires on
    /// one end of turn only (docs/rules/darkgifts.md).
    ///
    /// Incubation is the exception: its count runs out here but its doubling
    /// lands with the start of the next turn (`dark_gift_start_of_turn`), so
    /// the gift is marked `PAYOUT_PENDING` and stays on the minion. Log
    /// `Hearthstone_2026_10_28_19_47_59-g1` turn 7 (my seat is `player=2`,
    /// `Diverge#21215`): the end of the turn sets the gift's
    /// `TAG_SCRIPT_DATA_NUM_1` to 0 at `19:59:50.8835530` and the Hot-Air
    /// Surveyor is still 12/16; the next turn's board is 24/32, the double
    /// landing in its own block at `19:59:53.5441960`, after the fight.
    pub fn dark_gift_end_of_turn(&mut self, p: usize) {
        if self.data.gifts.is_empty() {
            return;
        }
        let runs = self.drakkari_mult(p);
        let mut pos = 0;
        while pos < self.players[p].board.len() {
            let m = self.players[p].board[pos];
            let mut fire: ArrayVec<(usize, CountdownEffect), { crate::MAX_GIFTS }> =
                ArrayVec::new();
            let mut spent: ArrayVec<usize, { crate::MAX_GIFTS }> = ArrayVec::new();
            if m.n_gifts > 0 {
                for (k, g) in m.gift_slice().iter().enumerate() {
                    let Some(cd) = self.data.cards.get(g.card()).countdown else { continue };
                    // A gift already waiting for its payout is not counted
                    // down again.
                    if g.state() == PAYOUT_PENDING {
                        continue;
                    }
                    let left = g.state().saturating_sub(1);
                    self.players[p].board[pos].set_gift_state_at(k, left);
                    if left > 0 {
                        continue;
                    }
                    if matches!(cd.effect, CountdownEffect::DoubleStats) {
                        // Incubation: the count ran out, the double waits for
                        // the next turn to start.
                        self.players[p].board[pos].set_gift_state_at(k, PAYOUT_PENDING);
                        continue;
                    }
                    if self.gift_repeats(g.card()) {
                        self.players[p].board[pos].set_gift_state_at(k, cd.turns);
                    } else {
                        let _ = spent.try_push(k);
                    }
                    let _ = fire.try_push((k, cd.effect));
                }
            }
            // A spent countdown comes off (highest index first, so the ones
            // below it stay where they are), then the payouts run.
            for &k in spent.iter().rev() {
                self.players[p].board[pos].remove_gift_at(k);
            }
            for &(_, effect) in &fire {
                for _ in 0..runs {
                    self.run_countdown(p, pos, effect);
                }
            }
            pos += 1;
        }
    }

    /// Whether the countdown gift whose effect card is `card` starts its
    /// count again after paying out instead of coming off the minion
    /// (darkgifts.toml's `repeats`: Replication and Affinity, while
    /// Incubation fires once and is gone).
    fn gift_repeats(&self, card: CardId) -> bool {
        self.data
            .gifts
            .gift_of_card(card)
            .is_some_and(|i| self.data.gifts.gifts[i].repeats)
    }

    /// A countdown gift fired: its effect runs (the caller has already taken
    /// a spent gift off the minion, and left a repeating one in place with
    /// its count reset).
    fn run_countdown(&mut self, p: usize, pos: usize, effect: CountdownEffect) {
        match effect {
            CountdownEffect::DoubleStats => {
                if let Some(m) = self.players[p].board.get_mut(pos) {
                    let (a, h) = (m.attack, m.health);
                    m.attack += a;
                    m.health += h;
                }
            }
            CountdownEffect::PlainCopy => {
                // "get a plain copy of this": the base card, never the host
                // as it stands (docs/rules/design.md "Lichling Hoarder": no golden, no
                // buffs, no added keywords, no gift). The same shape a
                // spell's `plain_copy` target and Funeral Wreath use
                // (`make_card`, no pool copy). The copy does not go through
                // `check_triple_tracking` now: the turn is over and nothing
                // triples during the fights, so a triple it completes merges
                // at the start of the next turn (Era 2026-11-03).
                let Some(m) = self.players[p].board.get(pos).copied() else { return };
                let copy = self.players[p].make_card(&self.data.cards, m.card, false, 0);
                let card = copy.card;
                let joined = self.add_to_hand(p, copy);
                if joined {
                    self.defer_triple(p, card);
                }
            }
            CountdownEffect::RandomMinionOfType => {
                let Some(m) = self.players[p].board.get(pos).copied() else { return };
                let tribes = self.data.cards.tribes_of(&m);
                let Some(t) = tribes.random_type(&mut self.rng) else { return };
                let tier = self.players[p].tier;
                if let Some(c) =
                    self.data.cards.random_pool_minion(t, tier, self.lobby, &mut self.rng)
                {
                    let mut at = pos;
                    self.give_generated_tracking(p, c, &mut at);
                }
            }
        }
    }

    /// "Whenever you play a card": every board minion whose gift says it
    /// gains stats on a play does (Dark Gifts Dexterity, Sharpened Sword,
    /// Toughened Shield). A Blood Gem, a Banana and a Tavern spell all count.
    pub fn dark_gift_card_played(&mut self, p: usize) {
        if self.data.gifts.is_empty() {
            return;
        }
        let cards = &self.data.cards;
        for i in 0..self.players[p].board.len() {
            let m = self.players[p].board[i];
            // A minion whose own card fires on a play (Dark Paradox t3)
            // counts as much as one with a gift that does.
            if m.n_gifts == 0 && cards.get(m.card).on_card_play.is_none() {
                continue;
            }
            let (mut da, mut dh) = (0, 0);
            // Every part: the minion's own card (Dark Paradox t3, whose
            // golden gives +6) and any gift that fires on a play. `parts`
            // yields a gift as `golden: false`, so a gift never doubles.
            for (card, golden) in m.parts() {
                if let Some((a, h)) = cards.get(card).on_card_play {
                    let mul = if golden { 2 } else { 1 };
                    da += a * mul;
                    dh += h * mul;
                }
            }
            if da != 0 || dh != 0 {
                let b = &mut self.players[p].board[i];
                b.attack += da;
                b.health += dh;
            }
        }
    }

    /// The "for each ... this game" gifts grow on their event (Battle Scars,
    /// Death's Embrace, Spell Siphon, Desecration), wherever the minion is:
    /// the board, the hand and Bob's Tavern (a Tavern copy of a Dark Paradox
    /// t6 grows too, docs/rules/design.md "Counting Battlecries").
    pub fn dark_gift_count(
        &mut self,
        p: usize,
        n: i32,
        pick: impl Fn(&Card) -> Option<(i32, i32)>,
    ) {
        if self.data.gifts.is_empty() || n == 0 {
            return;
        }
        let cards = &self.data.cards;
        let pl = &mut self.players[p];
        for m in pl
            .board
            .iter_mut()
            .chain(pl.hand.iter_mut())
            .chain(pl.shop.iter_mut())
        {
            // The minion's own card counts too (Dark Paradox t6, whose golden
            // gets +6/+6 per Battlecry); a gift part is never golden, so a
            // gift never doubles.
            if m.n_gifts == 0 && pick(cards.get(m.card)).is_none() {
                continue;
            }
            let (mut da, mut dh) = (0, 0);
            for (card, golden) in m.parts() {
                if let Some((a, h)) = pick(cards.get(card)) {
                    let mul = if golden { 2 } else { 1 };
                    da += a * n * mul;
                    dh += h * n * mul;
                }
            }
            if da != 0 || dh != 0 {
                m.attack += da;
                m.health += dh;
            }
        }
    }

    /// "Get an extra copy of this" (Dark Gifts Double Vision): the plain
    /// copies a minion brings with it when it enters the hand. The copies
    /// carry no gifts of their own, so nothing chains.
    pub(super) fn dark_gift_on_acquire(&mut self, p: usize, m: &Minion) {
        if self.data.gifts.is_empty() || m.n_gifts == 0 {
            return;
        }
        let mut copies = 0u8;
        for g in m.gift_slice() {
            copies = copies.saturating_add(self.data.cards.get(g.card()).acquire_copies);
        }
        for _ in 0..copies {
            let mut copy = *m;
            copy.pool_copies = 0;
            copy.gifts = [GiftOn::default(); crate::MAX_GIFTS];
            copy.n_gifts = 0;
            copy.shield_hits = 0;
            self.add_to_hand(p, copy);
        }
    }
}
