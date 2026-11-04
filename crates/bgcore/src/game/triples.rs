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
        // Corrupted Tome: "Whenever you would get a Triple Reward, get a
        // Triple Prize instead" (docs/rules/trinkets.md: nothing more than its own
        // Discover a Tier 3 Darkmoon Prize).
        if let Some(prize) = self.players[p].tp.triple_prize {
            self.give_generated(p, prize);
            return;
        }
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

    /// What playing a Golden minion gives: a Triple Reward, or with Double
    /// Time the Tavern Coin it names instead (docs/rules/heroes.md).
    pub(super) fn give_play_reward(&mut self, p: usize) {
        if let Some(coin) = self.players[p].double_time {
            self.give_generated(p, coin);
            return;
        }
        self.give_triple_reward(p);
    }

    /// Three copies of a minion in hand and on board become a golden one in
    /// hand, with double printed stats plus the buffs and attachments of all
    /// three. Spells never triple.
    pub(super) fn check_triple(&mut self, p: usize, card: CardId) {
        let mut dummy = 0;
        self.check_triple_tracking(p, card, &mut dummy);
    }

    /// A triple refused for want of a hand slot waits here instead of being
    /// lost, and the next action's own check merges it as soon as an action
    /// opens a slot (Era, 2026-11-04; `docs/rules/design.md` "A full hand").
    /// Only a card refused this way is remembered: the game's checks are
    /// queued by what happens (a summon, a gain), not run over the whole hand
    /// and board at every action, so re-arming one here would merge cards the
    /// game leaves alone (log 2026_10_24_15_25_26-g6 turn 8 step 15: three
    /// Skeletons stay on the board after the Discover is answered, and step
    /// 16, the second pick, is where the Golden one joins the hand).
    fn wait_for_room(&mut self, p: usize, card: CardId) {
        let wait = &mut self.players[p].triples_waiting;
        if !wait.contains(&card) {
            let _ = wait.try_push(card);
        }
    }

    /// The triples a full hand refused, asked again: a play, a spell cast,
    /// anything that takes a card out of the hand leaves the slot the Golden
    /// card needs, and this is the check that fills it. Cheap: only the cards
    /// that asked, each one through the ordinary check (`check_board_triples`
    /// and the cards an action gained run first, as they always did). A card
    /// that still has no room, or has lost a copy since, stays out: nothing
    /// re-adds it.
    pub(super) fn retry_waiting_triples(&mut self, p: usize) {
        if self.players[p].triples_waiting.is_empty() {
            return;
        }
        // The list comes off the player first: a card that still has no room
        // is put back by the check itself, and a list read in place would
        // hand it out again for ever. An `ArrayVec` on the stack, no heap.
        let mut waiting = std::mem::take(&mut self.players[p].triples_waiting);
        while let Some(card) = (!waiting.is_empty()).then(|| waiting.remove(0)) {
            self.check_triple(p, card);
        }
    }

    /// The triples a turn's start owes: a fight can leave three plain copies
    /// on the board, since nothing triples during the fights, and nothing asks
    /// for their check afterwards, so the game's own check at the turn's start
    /// is where they merge. With a full hand they wait for a slot instead
    /// (`triples_waiting`; Era 2026-11-04: "the same goes for a triple held
    /// from a fight", `docs/rules/design.md` "A full hand", night-note
    /// question 19). One pass over the hand and the board, no allocation, and
    /// it repeats while a merge still frees enough copies for a second one
    /// (`check_board_triples` loops the same way). A state a log holds mid-turn
    /// with three unmerged copies is not touched: this runs at the turn's
    /// start, not at every action (log 2026_10_24_15_25_26-g6 turn 8 step 15
    /// keeps its three Skeletons).
    pub(super) fn check_triples_at_turn_start(&mut self, p: usize) {
        loop {
            let before = self.players[p].hand.len() + self.players[p].board.len();
            // The copies a card needs: three, or two for Double Time and
            // Designer Eyepatch (`check_triple_tracking` works it out again
            // for the card it is given; this is only the loop's own filter).
            let need =
                if self.players[p].double_time.is_some() || self.players[p].tp.pirate_triple {
                    2
                } else {
                    3
                };
            let mut cards: ArrayVec<CardId, { crate::HAND + crate::BOARD }> = ArrayVec::new();
            // A stand-in (Elemental of Surprise) completes a card that is one
            // or two copies short, so the number around lowers the bar for
            // every other card.
            let mut stand_ins = 0usize;
            for m in self.players[p].hand.iter().chain(&self.players[p].board) {
                if self.data.cards.get(m.card).triple_wildcard.is_some() {
                    stand_ins = (stand_ins + 1).min(need);
                }
                if !cards.contains(&m.card) {
                    let _ = cards.try_push(m.card);
                }
            }
            let short = need.saturating_sub(stand_ins);
            for card in cards {
                let copies = self.players[p]
                    .hand
                    .iter()
                    .chain(&self.players[p].board)
                    .filter(|m| m.card == card && !m.golden)
                    .count();
                if copies >= short {
                    self.check_triple(p, card);
                }
            }
            if self.players[p].hand.len() + self.players[p].board.len() == before {
                break;
            }
        }
    }

    /// Nothing triples during the fights (`docs/rules/design.md` "Nothing
    /// triples during combat"): a minion card that arrives in the hand then
    /// (a Rally's Beast, a Deathrattle's card, a trinket's or a hero power's,
    /// Replication's copy at the end of the turn) is recorded here, and
    /// `dark_gift_start_of_turn` runs its triple check as the next turn
    /// starts, so the merge waits for the shop phase.
    pub(super) fn defer_triple(&mut self, p: usize, card: CardId) {
        let cards = &self.data.cards;
        if cards.get(card).is_spell() || cards.get(card).no_triple {
            return;
        }
        let wait = &mut self.players[p].triples_next_turn;
        if !wait.contains(&card) {
            let _ = wait.try_push(card);
        }
    }

    pub(super) fn check_triple_tracking(&mut self, p: usize, card: CardId, tracked_pos: &mut usize) {
        let cards = &self.data.cards;
        if cards.get(card).is_spell() || cards.get(card).no_triple {
            return;
        }
        // A third copy can only merge in the shop phase: a check asked for
        // while a fight's carry-over is applied waits for the start of the
        // next turn instead.
        if self.defer_triples {
            self.defer_triple(p, card);
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
        // The copies of `x` a triple would use: the hand-then-board order
        // `take_from` takes them in, and never a golden one. A wildcard
        // stands in for `x` when every one of them is of its tribe, which a
        // printed card always is and a Dark Gift can make one (the
        // Amalgamation on a minion with no type: Era, 2026-11-03,
        // docs/rules/design.md "Elemental of Surprise"). A triple that would
        // use a plain copy of `x` the gift never reached does not happen.
        let copies_of_tribe = |x: CardId, n: usize, t: Tribe| {
            let mut seen = 0;
            for m in pl.hand.iter().chain(&pl.board) {
                if seen == n {
                    break;
                }
                if m.card == x && !m.golden {
                    if !cards.tribes_of(m).has(t) {
                        return false;
                    }
                    seen += 1;
                }
            }
            seen == n
        };
        let can_stand_in = |w: CardId, x: CardId, n: usize| {
            w != x
                && wild_of(w)
                    .is_some_and(|t| cards.get(x).tribes.has(t) || copies_of_tribe(x, n, t))
        };
        // Designer Eyepatch: two copies of a Pirate make it Golden, so both
        // copies the pair would use have to be Pirates, by their printed card
        // or through a Dark Gift (the Amalgamation on a minion with no type:
        // Era, 2026-11-03, docs/notes/UNSURE.md). Double Time: two copies of
        // anything.
        let need = if pl.double_time.is_some()
            || (pl.tp.pirate_triple
                && (cards.get(card).tribes.has(Tribe::Pirate)
                    || copies_of_tribe(card, 2, Tribe::Pirate)))
        {
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
                && let Some(w) =
                    ids.clone().find(|&w| can_stand_in(w, card, need - 1) && stand_ins(w) >= 1)
            {
                plan = Some((card, need - 1, Some((w, 1))));
            } else if need >= 3
                && n + 2 >= need
                && let Some(w) =
                    ids.clone().find(|&w| can_stand_in(w, card, need - 2) && stand_ins(w) >= 2)
            {
                plan = Some((card, need - 2, Some((w, 2))));
            }
        } else {
            // The new card is a wildcard: find a card it completes. Two of
            // the card plus this wildcard first, then one plus two wildcards.
            let n_w = stand_ins(card);
            let pool_ids = 0..cards.cards.len() as CardId;
            let x = pool_ids
                .clone()
                .find(|&x| can_stand_in(card, x, need - 1) && copies(x) + 1 >= need && n_w >= 1)
                .map(|x| (x, need - 1, 1usize))
                .or_else(|| {
                    if need < 3 {
                        return None;
                    }
                    pool_ids
                        .clone()
                        .find(|&x| can_stand_in(card, x, need - 2) && copies(x) + 2 >= need && n_w >= 2)
                        .map(|x| (x, need - 2, 2usize))
                });
            if let Some((x, nx, nw)) = x {
                plan = Some((x, nx, Some((card, nw))));
            }
        }
        let Some((x_card, x_count, wild)) = plan else { return };
        // The Golden minion goes to hand: with no room for it the copies stay
        // where they are (a summon from a power can fill the hand first). A
        // slot the copies give up first counts, so the three plain copies of a
        // full hand still merge, losing three cards and taking one (log
        // 2026_10_24_15_25_26-g6 turn 8 step 16: a Discovered Cadaver
        // Caretaker into a hand of nine).
        //
        // The copies come out of the hand first (`take_from`) and the rest off
        // the board. A full hand still takes a board copy: the merge is one
        // action that takes all three copies and puts the Golden one in the
        // hand, so the slots it frees are free before the Golden card needs
        // one (log 2026_10_24_04_08_00-g1 turn 10 step 25: nine cards in hand,
        // one copy in hand, one on the board, and the bought third one makes
        // the hand ten; the log's Golden The Shadow of Doubt 21/169 is in the
        // hand and the board copy is gone). A copy off the board moves every
        // minion behind it, which may be the slot a Choose One or a step's
        // target still points at, so a choice left open holds the board's
        // copies where they stand: the game's own check runs as the action
        // ends and the choice is answered first.
        let (freed, from_board) = {
            let pl = &self.players[p];
            let in_hand = |id: CardId, n: usize, same: bool| {
                pl.hand
                    .iter()
                    .filter(|m| m.card == id && (!m.golden || !same))
                    .count()
                    .min(n)
            };
            let x = in_hand(x_card, x_count, true);
            let w = wild.map_or(0, |(card, n)| in_hand(card, n, false));
            let need = x_count + wild.map_or(0, |(_, n)| n);
            (x + w, need - x - w)
        };
        if from_board > 0 && Self::other_choice_open(&self.players[p]) {
            return;
        }
        if self.players[p].hand.len() + 1 > crate::HAND + freed {
            // The Golden card has no slot: the copies stay where they are and
            // the merge waits for one to open (`retry_waiting_triples`).
            self.wait_for_room(p, x_card);
            return;
        }
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
                let bonus = if from_board && cards.tribes_of(&m).has(Tribe::Beast) {
                    beast_bonus
                } else {
                    (0, 0)
                };
                gold.attack += m.attack - bonus.0 - base.attack;
                // A copy's damage never comes off the Golden one: the game
                // sums the copies into the Golden card and heals it, so what a
                // copy has left of its Health below the card's own is damage,
                // not a buff (log 2026_10_24_00_56_06-g1 turn 8 step 21: the
                // merged Plaguerunner 7051 takes ATK 25 and DAMAGE 1, then
                // DAMAGE 0, so a board copy at 21/1 gives a 25/4 Golden one,
                // not 25/3). The engine keeps no damage, so a buffed copy that
                // was also damaged loses the damage's worth of its buff.
                gold.health += (m.health - bonus.1 - base.health).max(0);
                gold.hats = gold.hats.saturating_add(m.hats);
                gold.winners_bread = gold.winners_bread.saturating_add(m.winners_bread);
                // A "gain +X/+X at the end of your turn" a Prize gave the
                // copies adds up, like any other buff.
                gold.end_turn_gain.0 = gold.end_turn_gain.0.saturating_add(m.end_turn_gain.0);
                gold.end_turn_gain.1 = gold.end_turn_gain.1.saturating_add(m.end_turn_gain.1);
                gold.magnetizations = gold.magnetizations.saturating_add(m.magnetizations);
                gold.volumizers =
                    gold.volumizers.saturating_add(m.volumizers.saturating_sub(base.volumizers));
                // Blood Gem stats are buffs like any other: they add up.
                gold.gem_attack = gold.gem_attack.saturating_add(m.gem_attack);
                gold.gem_health = gold.gem_health.saturating_add(m.gem_health);
                for &a in m.attached_slice() {
                    gold.push_attached(a);
                }
                // A triple's golden keeps the gifts of all three copies
                // (docs/rules/darkgifts.md), and gifts stack: two copies with the
                // same gift give two of it, except a gift that sets a state.
                // Countdown gifts stack too: the golden Cadaver Caretaker of
                // `Hearthstone_2026_10_26_00_48_15-g2` turn 7 carries two
                // Replication enchantments (4025, 4026) and pays out two
                // copies (entities 5585 and 5587).
                for &g in m.gift_slice() {
                    if cards.get(g.card()).gift_unique {
                        gold.push_gift_unique(g);
                    } else {
                        gold.push_gift(g);
                    }
                }
                // Toreth's Blessing's shield hits come along; the highest of
                // the copies is what the golden shield has.
                gold.shield_hits = gold.shield_hits.max(m.shield_hits);
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
