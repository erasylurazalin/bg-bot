//! Triples and the Triple Reward.

use arrayvec::ArrayVec;
use crate::cards::{
    Effect,
    Tribe,
};
use crate::minion::{
    CardId,
    Minion,
};
use crate::MAX_TIER;
use super::{
    Game,
    Player,
};
use super::shop::give_back;

/// What turns Golden in a triple: the card, how many of its own copies go in,
/// and a stand-in card (Elemental of Surprise) with how many of those.
type TriplePlan = (CardId, usize, Option<(CardId, usize)>);

impl Game {
    /// A Golden minion played from hand puts a Triple Reward spell in hand,
    /// a 0-cost plain spell whose Discover tier is fixed now: one above the
    /// player's tier, tier 6 at most. Lost on a full hand.
    pub(super) fn give_triple_reward(&mut self, p: usize) {
        let id = self
            .data
            .cards
            .triple_reward
            .expect("the card data has no Triple Reward spell");
        if self.players[p].hand.is_full() {
            return;
        }
        let tier = (self.players[p].tier + 1).min(MAX_TIER);
        let mut m = self.players[p].make_card(&self.data.cards, id, false, 0);
        m.improvements = tier as u16;
        self.add_to_hand(p, m);
    }

    /// Three copies of a minion in hand and on board become a golden one in
    /// hand, with double printed stats plus the buffs and attachments of all
    /// three. Spells never triple.
    pub(super) fn check_triple(&mut self, p: usize, card: CardId) {
        let mut dummy = 0;
        self.check_triple_tracking(p, card, &mut dummy);
    }

    pub(super) fn check_triple_tracking(&mut self, p: usize, card: CardId, tracked_pos: &mut usize) {
        let cards = &self.data.cards;
        if cards.get(card).is_spell() || cards.get(card).no_triple {
            return;
        }
        let pl = &self.players[p];
        // Plain copies in hand and on the board; a Golden minion never counts.
        let copies = |id: CardId| {
            pl.hand.iter().chain(&pl.board).filter(|m| m.card == id && !m.golden).count()
        };
        // Which card turns Golden and which cards it is made of: `x` of its
        // own copies and `w` of one wildcard card (Elemental of Surprise
        // stands in for any Elemental). Three plain copies come first.
        // A stand-in counts even when it is Golden (the only card that
        // triples while already golden).
        let stand_ins = |id: CardId| {
            pl.hand.iter().chain(&pl.board).filter(|m| m.card == id).count()
        };
        let wild_of = |id: CardId| cards.get(id).triple_wildcard;
        let can_stand_in = |w: CardId, x: CardId| {
            w != x && wild_of(w).is_some_and(|t| cards.get(x).tribes.has(t))
        };
        // Designer Eyepatch: two copies of a Pirate make it Golden.
        let need = if pl.tp.pirate_triple && cards.get(card).tribes.has(Tribe::Pirate) {
            2
        } else {
            3
        };
        let mut plan: Option<TriplePlan> = None;
        if copies(card) >= need {
            plan = Some((card, need, None));
        } else if wild_of(card).is_none() {
            // One copy short of the need plus one stand-in, or two short plus
            // two stand-ins (the second needs a card of three).
            let n = copies(card);
            let ids = 0..cards.cards.len() as CardId;
            if n + 1 >= need
                && let Some(w) = ids.clone().find(|&w| can_stand_in(w, card) && stand_ins(w) >= 1)
            {
                plan = Some((card, need - 1, Some((w, 1))));
            } else if need >= 3
                && n + 2 >= need
                && let Some(w) = ids.clone().find(|&w| can_stand_in(w, card) && stand_ins(w) >= 2)
            {
                plan = Some((card, need - 2, Some((w, 2))));
            }
        } else {
            // The new card is a wildcard: find a card it completes. Two of
            // the card plus this wildcard first, then one plus two wildcards.
            let n_w = stand_ins(card);
            let pool_ids = 0..cards.cards.len() as CardId;
            let need = if pl.tp.pirate_triple && cards.get(card).tribes.has(Tribe::Pirate) {
                2usize
            } else {
                3usize
            };
            let x = pool_ids
                .clone()
                .find(|&x| can_stand_in(card, x) && copies(x) + 1 >= need && n_w >= 1)
                .map(|x| (x, need - 1, 1usize))
                .or_else(|| {
                    if need < 3 {
                        return None;
                    }
                    pool_ids
                        .clone()
                        .find(|&x| can_stand_in(card, x) && copies(x) + 2 >= need && n_w >= 2)
                        .map(|x| (x, need - 2, 2usize))
                });
            if let Some((x, nx, nw)) = x {
                plan = Some((x, nx, Some((card, nw))));
            }
        }
        let Some((x_card, x_count, wild)) = plan else { return };
        let pl = &mut self.players[p];
        let mut gold = pl.make_card(cards, x_card, true, 0);
        // A Living Prison's arms are the one "this turn" effect that adds up
        // in a triple instead of taking the highest count.
        let arms_add =
            matches!(cards.get(x_card).activate, Some(a) if a.effect == Effect::ArmNextBuy);
        // a Beast on the board carries the until-next-turn Beast bonus; the
        // golden one in hand gets it when it is played, not twice
        let beast_bonus = pl.beast_bonus;
        let mut returned: ArrayVec<Minion, 2> = ArrayVec::new();
        let mut take_from = |pl: &mut Player, id: CardId, n: usize, same: bool, gold: &mut Minion| {
            for _ in 0..n {
                let is_copy = |m: &Minion| m.card == id && (!m.golden || !same);
                let (m, from_board) = if let Some(i) = pl.hand.iter().position(is_copy) {
                    (pl.hand.remove(i), false)
                } else {
                    let i = pl.board.iter().position(is_copy).unwrap();
                    if i < *tracked_pos {
                        *tracked_pos -= 1;
                    }
                    (pl.board.remove(i), true)
                };
                let base = pl.make_card(cards, id, m.golden, 0);
                let bonus = if from_board && cards.get(id).tribes.has(Tribe::Beast) {
                    beast_bonus
                } else {
                    (0, 0)
                };
                gold.attack += m.attack - bonus.0 - base.attack;
                gold.health += m.health - bonus.1 - base.health;
                gold.winners_bread = gold.winners_bread.saturating_add(m.winners_bread);
                gold.magnetizations = gold.magnetizations.saturating_add(m.magnetizations);
                gold.volumizers =
                    gold.volumizers.saturating_add(m.volumizers.saturating_sub(base.volumizers));
                // Blood Gem stats are buffs like any other: they add up.
                gold.gem_attack = gold.gem_attack.saturating_add(m.gem_attack);
                gold.gem_health = gold.gem_health.saturating_add(m.gem_health);
                for &a in m.attached_slice() {
                    gold.push_attached(a);
                }
                if same {
                    gold.keywords.0 |= m.keywords.0;
                    gold.pool_copies = gold.pool_copies.saturating_add(m.pool_copies);
                    // Drone Duplicator's extra Magnetizations are a "this
                    // turn" effect a pressed copy was still holding: the
                    // golden keeps them, adding.
                    gold.extra_magnetize =
                        gold.extra_magnetize.saturating_add(m.extra_magnetize);
                    if arms_add {
                        // A Living Prison's arms add up the same way: each
                        // held arm is one share of the next bought minion's
                        // stats (the golden's own press adds its double on
                        // top).
                        gold.improvements = gold.improvements.saturating_add(m.improvements);
                    } else {
                        // `improvements` is one minion's own count (Leviathan
                        // summons, Snapper growth, Hopebringer shields,
                        // Wrathguard Activates, Patient Scout tier, Treasure
                        // Parrot damage, Scarlet Survivor's flag, event
                        // counters): the golden copy takes the highest of the
                        // three, never the sum.
                        gold.improvements = gold.improvements.max(m.improvements);
                    }
                } else {
                    // A stand-in brings its buffs and its Divine Shield (the
                    // point of the card); its pool copies go back to the pool.
                    gold.keywords.0 |= m.keywords.0;
                    let _ = returned.try_push(m);
                }
            }
        };
        take_from(pl, x_card, x_count, true, &mut gold);
        if let Some((w, n)) = wild {
            take_from(pl, w, n, false, &mut gold);
        }
        self.add_to_hand(p, gold);
        for m in returned {
            give_back(&mut self.pool, &self.data.cards, &m);
        }
    }
}
