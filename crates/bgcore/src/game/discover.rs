//! Discover, Choose One, and discards.

use arrayvec::ArrayVec;
use crate::cards::{
    ChooseOption,
    EventKind,
    OnDiscard,
    Tribe,
};
use crate::minion::Minion;
use crate::BOARD;
use super::{
    DiscoverRequest,
    Game,
    PendingChooseOne,
    PendingTarget,
    QueuedOption,
    TargetKind,
};
use super::shop::draw_filtered;
use super::shop::draw_golden;
use super::shop::give_back;

impl Game {
    /// A Choose One card was just played: every Turbo Hogrider plays its
    /// Blood Gems on all friendly Quilboar.
    pub(super) fn after_choose_one_played(&mut self, p: usize) {
        let cards = &self.data.cards;
        let hogs: ArrayVec<(u8, bool), BOARD> = self.players[p]
            .board
            .iter()
            .filter_map(|m| {
                m.parts().find_map(|(c, g)| {
                    cards.get(c).after_choose_one_play.map(|n| (n, g))
                })
            })
            .collect();
        if hogs.is_empty() {
            return;
        }
        for (count, golden) in hogs {
            let times = count * (if golden { 2 } else { 1 });
            let (ga, gh) = (self.players[p].gem_attack, self.players[p].gem_health);
            for m in &mut self.players[p].board {
                if cards.get(m.card).tribes.has(Tribe::Quilboar) {
                    m.attack += ga * times as i32;
                    m.health += gh * times as i32;
                    m.gem_attack += ga * times as i32;
                    m.gem_health += gh * times as i32;
                }
            }
        }
    }

    /// The player picked the card in hand slot `s` to discard (Mangled
    /// Bandit's Activate): it is gone, a pool copy goes back to the pool, and
    /// the reward runs.
    pub(super) fn resolve_discard(&mut self, p: usize, s: usize) {
        let Some(pd) = self.players[p].pending_discard.take() else { return };
        if !self.valid_discard(p, s) {
            return;
        }
        self.discard_card(p, s);
        // A hero power has no board slot: the reward runs with the board's
        // first slot as its `pos`, and only effects that do not read it are
        // ever given that way (Incubate).
        let mut pos = pd.source.unwrap_or(0) as usize;
        self.run_effect(p, &mut pos, pd.reward, pd.golden, 1);
        self.maybe_start_discover(p);
    }

    pub(super) fn discard_card(&mut self, p: usize, s: usize) -> Option<Minion> {
        if s >= self.players[p].hand.len() {
            return None;
        }
        let m = self.players[p].hand.remove(s);
        give_back(&mut self.pool, &self.data.cards, &m);
        self.players[p].discarded = self.players[p].discarded.saturating_add(1);
        self.players[p].last_discarded = Some(m);
        self.fire_event(p, EventKind::Discarded, 1);
        self.trinket_event(p, crate::trinkets::Event::Discard, Some(m.card), 1);
        match self.data.cards.get(m.card).on_discard {
            Some(OnDiscard::CastSpell(times)) => {
                for _ in 0..times {
                    self.cast_spell(p, m);
                }
            }
            Some(OnDiscard::MaxGold(n)) => {
                self.players[p].extra_max_gold = self.players[p].extra_max_gold.saturating_add(n);
            }
            None => {}
        }
        Some(m)
    }

    /// Run one option of a Choose One (the shared body of `resolve_choose`
    /// and the both-effects paths).
    fn apply_choose_option(&mut self, p: usize, source: Option<u8>, opt: ChooseOption, tavern: bool) {
        match opt {
            ChooseOption::MinionTarget { tribe, attack, health, keyword } => {
                let pt = PendingTarget {
                    source,
                    kind: TargetKind::MinionBuff {
                        tribe,
                        attack,
                        health,
                        keyword,
                        random_keyword: false,
                    },
                    runs: 1,
                    tavern: true,
                    gem: false,
                };
                self.offer_target(p, pt);
            }
            ChooseOption::SpellBonus { attack, health } => {
                self.players[p].spell_bonus.0 += attack;
                self.players[p].spell_bonus.1 += health;
            }
            ChooseOption::SpellTarget(st) => {
                let runs = self.balinda_mult(p);
                // offer_target, not a raw set: the shop may have changed since
                // the choice was offered (Saloon's Finest), and a prompt with
                // no legal target could never be answered.
                self.offer_target(
                    p,
                    PendingTarget {
                        source: None,
                        kind: TargetKind::Spell(st),
                        runs,
                        tavern,
                        gem: false,
                    },
                );
            }
            ChooseOption::SpellEffect(se) => {
                if tavern {
                    self.trigger_on_cast_spell(p);
                }
                self.execute_spell_effect(p, se, tavern);
            }
        }
    }

    /// A Choose One with both effects combined: the first option runs now,
    /// the second right after it, or through the queue when the first leaves
    /// a target prompt open. `source.is_some()` means the Choose One is a
    /// minion's Battlecry, so "after you trigger a Battlecry" fires once.
    pub(super) fn resolve_choose_options(
        &mut self,
        p: usize,
        source: Option<u8>,
        options: [ChooseOption; 2],
        tavern: bool,
        both: bool,
    ) {
        if !both {
            self.players[p].pending_choose_one = Some(PendingChooseOne {
                source,
                golden: false,
                options,
                tavern,
            });
            return;
        }
        self.apply_choose_option(p, source, options[0], tavern);
        let blocked = self.players[p].pending_target.is_some()
            || self.players[p].pending_shop_target.is_some()
            || self.players[p].pending_choose_one.is_some()
            || self.players[p].pending_discard.is_some()
            || !self.players[p].discover.is_empty();
        if blocked {
            let _ = self.players[p]
                .choose_queue
                .try_push(QueuedOption { source, option: options[1], tavern });
        } else {
            self.apply_choose_option(p, source, options[1], tavern);
        }
        if source.is_some() {
            self.fire_after_battlecry(p, 1);
        }
        self.maybe_start_discover(p);
    }

    pub(super) fn maybe_start_discover(&mut self, p: usize) {
        loop {
            {
                let pl = &self.players[p];
                if pl.pending_target.is_some()
                    || pl.pending_shop_target.is_some()
                    || pl.pending_choose_one.is_some()
                    || pl.pending_discard.is_some()
                    || !pl.discover.is_empty()
                {
                    return;
                }
            }
            // A both-effects Choose One option waiting for the one before it.
            if let Some(q) = self.players[p].choose_queue.first().copied() {
                self.players[p].choose_queue.remove(0);
                self.apply_choose_option(p, q.source, q.option, q.tavern);
                continue;
            }
            if self.players[p].discover_queue.is_empty() {
                return;
            }
            let req = self.players[p].discover_queue.remove(0);
            let damage_from = match req {
                DiscoverRequest::DemonMinion { source, .. } => Some(source),
                _ => None,
            };
            let locked = matches!(req, DiscoverRequest::SearchMinionTier(_));
            let locked_tier = match req {
                DiscoverRequest::TierLockedMinion(t) => t,
                _ => 0,
            };
            let doomed = matches!(req, DiscoverRequest::DoomedTribeMinion(_));
            let both = matches!(req, DiscoverRequest::ChooseOneMinion);
            let Game { data, rng, players, pool, lobby, .. } = self;
            let pl = &mut players[p];
            pl.discover_damage_from = damage_from;
            pl.discover_locked = locked;
            pl.discover_locked_tier = locked_tier;
            pl.discover_doomed = doomed;
            pl.discover_both = both;
            while !pl.discover.is_full() {
                let next = match req {
                    DiscoverRequest::MinionTier(t)
                    | DiscoverRequest::SearchMinionTier(t)
                    | DiscoverRequest::TierLockedMinion(t) => {
                        draw_filtered(pool, &data.cards, rng, t, t, &pl.discover, *lobby, |_| true)
                    }
                    DiscoverRequest::SpellTier(t) => {
                        data.cards.random_tier_spell(t, &pl.discover, *lobby, rng)
                    }
                    DiscoverRequest::SpellMaxTier(t) => {
                        data.cards.random_spell_up_to(t, &pl.discover, *lobby, rng)
                    }
                    DiscoverRequest::DeathrattleMinion(max_t) => draw_filtered(
                        pool,
                        &data.cards,
                        rng,
                        1,
                        max_t,
                        &pl.discover,
                        *lobby,
                        |c| c.deathrattle.is_some(),
                    ),
                    DiscoverRequest::BattlecryMinion(max_t) => draw_filtered(
                        pool,
                        &data.cards,
                        rng,
                        1,
                        max_t,
                        &pl.discover,
                        *lobby,
                        |c| c.battlecry.is_some(),
                    ),
                    DiscoverRequest::ChooseOneMinion => draw_filtered(
                        pool,
                        &data.cards,
                        rng,
                        1,
                        pl.tier,
                        &pl.discover,
                        *lobby,
                        |c| c.choose_one.is_some(),
                    ),
                    DiscoverRequest::TribeMinion(t) | DiscoverRequest::DoomedTribeMinion(t) => {
                        // a "Discover a Murloc" only offers up to the tier
                        draw_filtered(pool, &data.cards, rng, 1, pl.tier, &pl.discover, *lobby, |c| {
                            c.tribes.has(t)
                        })
                    }
                    DiscoverRequest::TypedMinionTier(t) => draw_filtered(
                        pool,
                        &data.cards,
                        rng,
                        t,
                        t,
                        &pl.discover,
                        *lobby,
                        |c| !c.tribes.is_neutral(),
                    ),
                    DiscoverRequest::DemonMinion { max_tier: max_t, .. } => draw_filtered(
                        pool,
                        &data.cards,
                        rng,
                        1,
                        max_t,
                        &pl.discover,
                        *lobby,
                        |c| c.tribes.has(Tribe::Demon),
                    ),
                };
                match next {
                    Some(c) => pl.discover.push(c),
                    None => break,
                }
            }
            if pl.discover.is_empty() {
                // Nothing was left to offer (the pool or the filters ran
                // out), so this Discover is lost. Its side flags go with it:
                // they must not reach the next Discover.
                pl.discover_damage_from = None;
                pl.discover_locked = false;
                pl.discover_locked_tier = 0;
                pl.discover_doomed = false;
                pl.discover_both = false;
                pl.tp.remember_in = 0;
            }
        }
    }

    pub(super) fn resolve_choose(&mut self, p: usize, k: usize) {
        if let Some(pco) = self.players[p].pending_choose_one.take() {
            self.apply_choose_option(p, pco.source, pco.options[k], pco.tavern);
            // A minion's Choose One is its Battlecry.
            if pco.source.is_some() {
                self.fire_after_battlecry(p, 1);
            }
            self.maybe_start_discover(p);
            return;
        }
        // A Hero Power pick (Unmasked Identity, Finley's Discover, King of
        // Duality, a chooser): the chosen power replaces the one in the slot
        // the offer named with a fresh state; the offers left standing are
        // simply dropped (powers are not pool cards).
        if !self.players[p].power_offer.is_empty() {
            self.pick_power(p, k);
            self.maybe_start_discover(p);
            return;
        }
        let Game { data, players, pool, .. } = self;
        let pl = &mut players[p];
        let locked = pl.discover_locked;
        pl.discover_locked = false;
        let locked_tier = std::mem::take(&mut pl.discover_locked_tier);
        let doomed = std::mem::take(&mut pl.discover_doomed);
        let both = std::mem::take(&mut pl.discover_both);
        let c = pl.discover[k];
        for (i, &o) in pl.discover.iter().enumerate() {
            if i != k && !data.cards.get(o).is_spell() {
                let slot = data.cards.get(o).pool_slot as usize;
                pool[slot] += 1;
            }
        }
        pl.discover.clear();
        // Imposing Percussionist: the hero takes damage equal to the Tier of
        // the Demon chosen.
        let damage = pl
            .discover_damage_from
            .take()
            .map(|source| (source, data.cards.get(c).tier as i32));
        let is_spell = data.cards.get(c).is_spell();
        // Token of the Old Gods: this Discover does not put the card in hand,
        // it turns the minion the spell chose into it (printed stats, no
        // buffs, and the new minion holds the pool copy).
        let transform = pl.tp.transform_slot;
        if transform != 0 {
            pl.tp.transform_slot = 0;
            let slot = (transform as usize - 1).min(crate::BOARD.saturating_sub(1));
            if !is_spell && slot < pl.board.len() {
                let old = pl.board[slot];
                let mut m = pl.make_card(&data.cards, c, false, 1);
                m.uses_left = m.turn_uses(&data.cards);
                pl.board[slot] = m;
                give_back(pool, &data.cards, &old);
                self.check_triple(p, c);
            } else {
                // Nothing to transform (the board moved): the copy goes back.
                let slot = data.cards.get(c).pool_slot as usize;
                self.pool[slot] += 1;
            }
            self.maybe_start_discover(p);
            return;
        }
        // Innkeeper's Hearth: this pick's stats are set, buffs included, for
        // each of the picks the trinket dealt.
        let set_stats = match pl.tp.discover_stats {
            Some((a, h, left)) if left > 0 => {
                pl.tp.discover_stats = if left > 1 { Some((a, h, left - 1)) } else { None };
                Some((a, h))
            }
            _ => None,
        };
        if !pl.hand.is_full() {
            let pool_copies = if is_spell { 0 } else { 1 };
            let mut m = pl.make_card(&data.cards, c, false, pool_copies);
            if let Some((a, h)) = set_stats {
                m.attack = a;
                m.health = h;
            }
            if locked && !is_spell {
                m.locked_turns = 1;
            }
            if locked_tier > 0 && !is_spell {
                m.locked_tier = locked_tier;
            }
            if doomed && !is_spell {
                m.doomed = true;
            }
            if both && !is_spell {
                m.both_effects = true;
            }
            self.add_to_hand(p, m);
            if !is_spell {
                self.check_triple(p, c);
            }
        } else if !is_spell {
            let slot = self.data.cards.get(c).pool_slot as usize;
            self.pool[slot] += 1;
        }
        if let Some((source, d)) = damage {
            self.take_shop_damage(p, d, Some(source as usize));
        }
        // Portable Factory remembers the card it Discovered, so it can hand
        // out another copy each turn.
        let remember = std::mem::take(&mut self.players[p].tp.remember_in);
        if remember != 0 {
            let slot = (remember as usize - 1).min(crate::TRINKET_SLOTS - 1);
            self.players[p].trinkets[slot].card = Some(c);
        }
        // Sinstone Sticker: the first N Discovers each turn also give a copy
        // of the pick (a generated copy, no pool copy).
        if self.players[p].tp.discover_copies > 0 {
            self.players[p].tp.discover_copies -= 1;
            self.give_generated(p, c);
        }
        self.fire_after_discover(p);
        self.maybe_start_discover(p);
    }

    /// Draw a Golden minion for the player (from the pool, three copies) and
    /// put it in hand. `no_reward` marks it as giving no Triple Reward.
    /// False when the hand is full or nothing is left.
    pub(super) fn give_golden_minion(&mut self, p: usize, lo: u8, hi: u8, typed: bool, no_reward: bool) -> bool {
        if self.players[p].hand.is_full() {
            return false;
        }
        let Some(m) = self.draw_golden_minion(p, lo, hi, typed, no_reward) else { return false };
        self.add_to_hand(p, m);
        true
    }

    fn draw_golden_minion(&mut self, p: usize, lo: u8, hi: u8, typed: bool, no_reward: bool) -> Option<Minion> {
        let Game { data, rng, players, pool, lobby, .. } = self;
        let pl = &mut players[p];
        let (id, copies) = draw_golden(pool, &data.cards, rng, lo, hi, typed, *lobby)?;
        let mut m = pl.make_card(&data.cards, id, true, copies);
        m.no_reward |= no_reward;
        Some(m)
    }

    /// Get a Lockbox. With one already in hand it opens `sooner` turns
    /// sooner instead (and opens now if that takes it to 0). With a full
    /// hand and no Lockbox the new one is lost.
    pub(super) fn give_lockbox(&mut self, p: usize, sooner: u8) {
        let cards = &self.data.cards;
        let Some(id) = cards.lockbox else { return };
        let pl = &mut self.players[p];
        if let Some(i) = pl.hand.iter().position(|m| m.card == id) {
            let left = pl.hand[i].improvements.saturating_sub(sooner as u16);
            pl.hand[i].improvements = left;
            if left == 0 {
                self.open_lockbox(p, i);
            }
            return;
        }
        if pl.hand.is_full() {
            return;
        }
        let mut m = pl.make_card(cards, id, false, 0);
        // Escapee Portrait: every Lockbox opens one turn sooner.
        let sooner = self.players[p].tp.lockbox_sooner;
        if sooner > 0 {
            m.improvements = m.improvements.saturating_sub(sooner as u16);
            if m.improvements == 0 {
                // Already due: it opens at once, like a card given with no
                // turns left on it.
                let i = self.players[p].hand.len();
                self.add_to_hand(p, m);
                self.open_lockbox(p, i);
                return;
            }
        }
        self.add_to_hand(p, m);
    }

    /// The Lockbox in hand slot `i` breaks open: it becomes a random Golden
    /// minion with a type, up to the player's tier, taken from the pool.
    fn open_lockbox(&mut self, p: usize, i: usize) {
        let tier = self.players[p].tier;
        match self.draw_golden_minion(p, 1, tier, true, false) {
            Some(m) => self.players[p].hand[i] = m,
            None => {
                self.players[p].hand.remove(i);
            }
        }
    }

    /// End of the owner's turn: every Lockbox in hand counts down `times`
    /// (Drakkari doubles it) and opens at 0.
    pub(super) fn tick_lockboxes(&mut self, p: usize, times: u8) {
        let Some(id) = self.data.cards.lockbox else { return };
        for _ in 0..times {
            let mut i = 0;
            while i < self.players[p].hand.len() {
                if self.players[p].hand[i].card == id {
                    let m = &mut self.players[p].hand[i];
                    m.improvements = m.improvements.saturating_sub(1);
                    if m.improvements == 0 {
                        let before = self.players[p].hand.len();
                        self.open_lockbox(p, i);
                        if self.players[p].hand.len() < before {
                            continue;
                        }
                    }
                }
                i += 1;
            }
        }
    }

    /// After the player Discovers a card: Hooktusk and its like run.
    fn fire_after_discover(&mut self, p: usize) {
        let mut pos = 0;
        while pos < self.players[p].board.len() {
            let m = self.players[p].board[pos];
            for (card, golden) in m.parts() {
                if let Some(e) = self.data.cards.get(card).after_discover
                    && pos < self.players[p].board.len()
                {
                    self.run_effect(p, &mut pos, e, golden, 1);
                }
            }
            pos += 1;
        }
    }

    /// Get a random tier 7 minion, as a generated card (no pool copy), for
    /// "Get a Tier 7 minion". False when the hand is full or none exists.
    /// Nothing else in the lobby makes a tier 7 minion; the Pirate run's
    /// Greedy Conniver is the first card that will use this.
    pub fn give_tier7_minion(&mut self, p: usize) -> bool {
        let Some(c) = self.data.cards.random_tier7_minion(self.lobby, &mut self.rng) else {
            return false;
        };
        if self.players[p].hand.is_full() {
            return false;
        }
        self.give_generated(p, c);
        true
    }

    /// Queue "Discover a Tier 7 minion": three different tier 7 minions from
    /// the pool, the pick takes its copy (the same as every Discover) and
    /// the rest go back. Opens at once unless a choice is already open.
    pub fn discover_tier7_minion(&mut self, p: usize) {
        let _ = self.players[p]
            .discover_queue
            .try_push(DiscoverRequest::MinionTier(crate::CARD_MAX_TIER));
        self.maybe_start_discover(p);
    }
}
