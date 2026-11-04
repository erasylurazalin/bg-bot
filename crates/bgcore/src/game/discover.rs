//! Discover, Choose One, and discards.

use arrayvec::ArrayVec;
use crate::cards::{
    ChooseOption,
    EventKind,
    OnDiscard,
    Tribe,
    TriggerWhen,
};
use crate::minion::{Keywords, Minion};
use crate::{BOARD, DISCOVER};
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
    /// Blood Gems on all friendly Quilboar. Each of those Gems is a Gem
    /// played on the minion it lands on, so a Geomagus Roogug there reacts
    /// (docs/rules/design.md "Geomagus Roogug").
    pub(super) fn after_choose_one_played(&mut self, p: usize) {
        let hogs: ArrayVec<(u8, bool), BOARD> = self.players[p]
            .board
            .iter()
            .filter_map(|m| {
                m.parts().find_map(|(c, g)| {
                    self.data.cards.get(c).after_choose_one_play.map(|n| (n, g))
                })
            })
            .collect();
        if hogs.is_empty() {
            return;
        }
        for (count, golden) in hogs {
            let times = count * (if golden { 2 } else { 1 });
            for i in 0..self.players[p].board.len() {
                if !self.data.cards.tribes_of(&self.players[p].board[i]).has(Tribe::Quilboar) {
                    continue;
                }
                self.play_gems_on_board(p, i, times);
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
        self.run_effect(p, &mut pos, pd.reward, pd.golden, pd.runs.max(1));
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
        // Dark Gifts Desecration grows on every discard.
        self.dark_gift_count(p, 1, |c| c.buff_per_discard);
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
    /// and the both-effects paths). `by_minion` says a minion's effect cast
    /// the spell: its target then fires no "you cast a spell" trigger.
    pub(super) fn apply_choose_option(
        &mut self,
        p: usize,
        source: Option<u8>,
        opt: ChooseOption,
        tavern: bool,
        by_minion: bool,
    ) {
        match opt {
            ChooseOption::MinionTarget { tribe, attack, health, keyword, in_shop } => {
                let pt = PendingTarget {
                    by_minion: false,
                    source,
                    kind: TargetKind::MinionBuff {
                        tribe,
                        attack,
                        health,
                        keyword,
                        random_keyword: false,
                        shop: in_shop,
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
                        by_minion,
                    },
                );
            }
            ChooseOption::SpellEffect(se) => {
                // A spell a minion casts for the player is not the player's
                // cast: no "whenever you cast a spell" card sees it.
                if tavern && !by_minion {
                    self.trigger_on_cast_spell(p, TriggerWhen::Now);
                }
                self.execute_spell_effect(p, se, tavern);
            }
        }
    }

    /// A Choose One with both effects combined: the first option runs now,
    /// the second right after it, or through the queue when the first leaves
    /// a target prompt open. `source.is_some()` means the Choose One is a
    /// minion's Battlecry, so "after you trigger a Battlecry" fires once.
    /// `by_minion` says a minion cast the spell (see `apply_choose_option`).
    pub(super) fn resolve_choose_options(
        &mut self,
        p: usize,
        source: Option<u8>,
        options: [ChooseOption; 2],
        tavern: bool,
        by_minion: bool,
        both: bool,
    ) {
        if !both {
            self.players[p].pending_choose_one = Some(PendingChooseOne {
                source,
                golden: false,
                options,
                tavern,
                repeats: 1,
            });
            return;
        }
        self.apply_choose_option(p, source, options[0], tavern, by_minion);
        let blocked = self.players[p].pending_target.is_some()
            || self.players[p].pending_shop_target.is_some()
            || self.players[p].pending_choose_one.is_some()
            || self.players[p].pending_discard.is_some()
            || !self.players[p].discover.is_empty();
        if blocked {
            let _ = self.players[p].choose_queue.try_push(QueuedOption {
                source,
                option: options[1],
                tavern,
                by_minion,
            });
        } else {
            self.apply_choose_option(p, source, options[1], tavern, by_minion);
        }
        if source.is_some() {
            self.fire_after_battlecry(p, 1);
        }
        self.maybe_start_discover(p);
    }

    pub(super) fn maybe_start_discover(&mut self, p: usize) {
        // A prompt `run_queued_repeats` answers re-enters here; the repeat
        // driver is already on the stack and carries on with what is left.
        if self.players[p].repeating {
            return;
        }
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
                self.apply_choose_option(p, q.source, q.option, q.tavern, q.by_minion);
                continue;
            }
            // The repeats a Battlecry trigger or a cast still owes, before its
            // "after" triggers (docs/rules/design.md "Set versus add"). They
            // wait while any other choice is open: what the first run opened
            // comes first.
            if !self.players[p].repeats.is_empty()
                && !Game::other_choice_open(&self.players[p])
            {
                self.run_queued_repeats(p);
                continue;
            }
            if self.players[p].discover_queue.is_empty() {
                // Nothing the cast opened is left: a Choose One spell's "after
                // you cast a spell" triggers fire now, the option and its
                // target behind them (docs/rules/design.md "Targets in the shop").
                if let Some(what) = self.players[p].cast_after_choice.take() {
                    self.fire_cast_after_choice(p, what);
                    continue;
                }
                // And a Battlecry that waited on a choice counts now that
                // nothing is left of it (docs/rules/design.md "Counting Battlecries": a
                // Discover Battlecry counts at the pick).
                if self.players[p].pending_battlecries > 0 {
                    self.pay_pending_battlecries(p);
                    continue;
                }
                return;
            }
            let req = self.players[p].discover_queue.remove(0);
            // A Discover a Battlecry an Activate triggered opened is the one
            // waiting at the front: the queue pushes its requests together
            // and drains before anything else can add one.
            let from_repeat = self.players[p].repeat_discover > 0;
            // A Dark Gift offer draws its own minions and gifts, one per
            // slot, so it fills the Discover itself and lets the loop go on.
            if let DiscoverRequest::DarkGift(plan) = req {
                let turn = plan.gift_turn;
                self.deal_dark_gift_offer(p, plan);
                let _ = turn;
                continue;
            }
            if matches!(req, DiscoverRequest::DarkGiftCopies) {
                let turn = self.turn;
                self.deal_dark_gift_copies(p, turn);
                continue;
            }
            // Offers whose options are not one filtered pool draw each.
            match req {
                DiscoverRequest::PotTypes(types) => {
                    self.deal_pot_offer(p, types);
                    continue;
                }
                DiscoverRequest::DeadWarband(i) => {
                    self.deal_dead_warband(p, i as usize);
                    continue;
                }
                DiscoverRequest::DeadLastFight => {
                    self.deal_dead_last_fight(p);
                    continue;
                }
                DiscoverRequest::Champion(gold) => {
                    self.deal_champion(p, gold);
                    continue;
                }
                DiscoverRequest::IspyNextOpponent => {
                    self.deal_ispy_offer(p);
                    continue;
                }
                DiscoverRequest::LastOpponentWarband => {
                    self.deal_last_opponent_offer(p);
                    continue;
                }
                _ => {}
            }
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
            pl.discover_silent = false;
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
                    // Three different Darkmoon Prizes of one tier: the list
                    // is the card data's, never the pool.
                    DiscoverRequest::Prize(t) => data.cards.random_prize(t, &pl.discover, rng),
                    // Tinker: a Magnetic Mech up to the player's Tier.
                    DiscoverRequest::MagneticMinion => draw_filtered(
                        pool,
                        &data.cards,
                        rng,
                        1,
                        pl.tier,
                        &pl.discover,
                        *lobby,
                        |c| c.keywords.has(Keywords::MAGNETIC) && c.tribes.has(Tribe::Mech),
                    ),
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
                    // Handled above, before the pool draws.
                    DiscoverRequest::DarkGift(_)
                    | DiscoverRequest::DarkGiftCopies
                    | DiscoverRequest::PotTypes(_)
                    | DiscoverRequest::DeadWarband(_)
                    | DiscoverRequest::DeadLastFight
                    | DiscoverRequest::Champion(_)
                    | DiscoverRequest::IspyNextOpponent
                    | DiscoverRequest::LastOpponentWarband => None,
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
                // A Discover a repeat opened is still spent, even when there
                // is nothing to offer.
                if from_repeat {
                    pl.repeat_discover = pl.repeat_discover.saturating_sub(1);
                }
            } else if from_repeat {
                // A Discover a Battlecry an Activate triggered opened gives a
                // random card of its pool instead of being offered, and that
                // card is not a Discover for "after you Discover" cards
                // (docs/rules/design.md "Shop replay answers", Era 2026-11-03
                // and 2026-11-04: a Kelp Keeper's Activate).
                self.players[p].repeat_discover -= 1;
                let k = self.rng.below(self.players[p].discover.len() as u32) as usize;
                self.players[p].discover_silent = true;
                self.resolve_choose(p, k);
                self.players[p].discover_silent = false;
            }
        }
    }

    pub(super) fn resolve_choose(&mut self, p: usize, k: usize) {
        if let Some(pco) = self.players[p].pending_choose_one.take() {
            // The option the player picked: every repeat a queue entry still
            // owes takes this same one (docs/rules/design.md "Set versus
            // add", Era 2026-11-04).
            if let Some(q) = self.players[p].repeats.first_mut()
                && q.started
                && q.option.is_none()
                && q.source() == pco.source
            {
                q.option = Some(k as u8);
            }
            self.apply_choose_option(p, pco.source, pco.options[k], pco.tavern, false);
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
        // Prestidigitation's offer: the chosen Secret joins the Secret zone.
        if !self.players[p].secret_offer.is_empty() {
            self.pick_secret(p, k);
            self.maybe_start_discover(p);
            return;
        }
        // Friendly Wager: the chosen player is first in the pair, the other
        // second, settled after combat.
        if let Some((a, b)) = self.players[p].wager_offer.take() {
            let (picked, other) = if k == 0 { (a, b) } else { (b, a) };
            let _ = self.players[p].wagers.try_push((picked, other));
            self.maybe_start_discover(p);
            return;
        }
        // Detective for Hire: a correct guess gives a Tavern Coin.
        if let Some((_, correct)) = self.players[p].holmes_offer.take() {
            if k as u8 == correct
                && let Some(coin) = self.tavern_coin(p)
            {
                self.give_generated(p, coin);
            }
            self.maybe_start_discover(p);
            return;
        }
        // A Discover whose options are ready-made minions (Kel'Thuzad's
        // Kitty): the pick lands as it stands, and no pool copy moves.
        if !self.players[p].discover_minions.is_empty() {
            let mut m = self.players[p].discover_minions[k];
            self.players[p].discover_minions.clear();
            self.players[p].discover.clear();
            if !self.players[p].hand.is_full() {
                m.pool_copies = 0;
                m.no_reward = false;
                let card = m.card;
                self.add_to_hand(p, m);
                // The check waits for the end of the action, as for any
                // Discover pick.
                self.check_triple_later(card);
            }
            self.fire_after_discover(p);
            self.maybe_start_discover(p);
            return;
        }
        let Game { data, players, pool, .. } = self;
        let pl = &mut players[p];
        let locked = pl.discover_locked;
        pl.discover_locked = false;
        let locked_tier = std::mem::take(&mut pl.discover_locked_tier);
        let locked_gold = std::mem::take(&mut pl.discover_locked_gold);
        let doomed = std::mem::take(&mut pl.discover_doomed);
        let both = std::mem::take(&mut pl.discover_both);
        let c = pl.discover[k];
        // A Dark Gift offer: the gift on the pick, the pool copies each offer
        // held (a Golden offer holds three, a Dagger copy none).
        let dark = pl.dark_offer;
        let gift = pl.discover_gifts.get(k).copied();
        let gift_turn = pl.dark_offer_turn;
        let golden_offer = pl.dark_offer_golden
            || gift.is_some_and(|g| data.cards.get(g.card()).makes_golden);
        let locked_turns = pl.dark_offer_locked;
        let mut copies = [1u8; DISCOVER];
        for (i, &o) in pl.discover.iter().enumerate() {
            if dark {
                copies[i] = pl.discover_copies.get(i).copied().unwrap_or(0);
            } else if data.cards.get(o).is_spell() || data.cards.get(o).prize.is_some() {
                copies[i] = 0;
            }
            if i != k && copies[i] > 0 && !data.cards.get(o).is_spell() {
                let slot = data.cards.get(o).pool_slot as usize;
                pool[slot] += copies[i];
            }
        }
        pl.discover.clear();
        pl.clear_dark_offer();
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
            } else if !is_spell && copies[k] > 0 {
                // Nothing to transform (the board moved, or the Discover was
                // a spell's): the copy goes back.
                let slot = data.cards.get(c).pool_slot as usize;
                self.pool[slot] += copies[k];
            }
            self.maybe_start_discover(p);
            return;
        }
        // Galakrond's Greed: this pick takes the Tavern slot the old card
        // held, from the pool. It lands in the Tavern, so it gets the Tavern's
        // standing "this game" buffs like any card dealt there (docs/rules/heroes.md;
        // Era 2026-11-02 saw a 6/8 offer land 46/48). The old card stayed on
        // show while the Discover was open; it goes back to the pool now, its
        // own buffs with it.
        let into_shop = std::mem::take(&mut pl.discover_into_shop);
        if into_shop != 0 {
            if !is_spell {
                let slot = (into_shop as usize - 1).min(crate::SHOP - 1);
                let at = slot.min(pl.shop.len().saturating_sub(1));
                if at < pl.shop.len() {
                    let old = pl.shop.remove(at);
                    give_back(pool, &data.cards, &old);
                }
                let mut m = pl.make_card(&data.cards, c, false, copies[k].max(1));
                pl.apply_tavern_buffs(&data.cards, &mut m);
                m.uses_left = m.turn_uses(&data.cards);
                pl.shop.insert(at, m);
            } else if copies[k] > 0 {
                let slot = data.cards.get(c).pool_slot as usize;
                self.pool[slot] += copies[k];
            }
            self.fire_after_discover(p);
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
            // A normal minion offer holds one pool copy; a Golden Dark Gift
            // offer, a token-free offer (a Prize's minion) and a spell hold
            // what `copies` says.
            let pool_copies = if is_spell { 0 } else { copies[k] };
            let mut m = pl.make_card(&data.cards, c, golden_offer, pool_copies);
            if golden_offer {
                // Gilding and Sinister Invitation: no Triple Reward.
                m.no_reward = true;
            }
            if let Some(gift) = gift
                && !is_spell
                && let Some(index) = data.gifts.gift_of_card(gift.card())
            {
                super::darkgifts::land_gift(data, pl, &mut m, index, gift_turn);
            }
            if locked_turns > 0 && !is_spell {
                m.locked_turns = locked_turns;
            }
            if let Some((a, h)) = set_stats {
                // A set of the pick's stats keeps the bonuses that apply
                // wherever the card is (the Undead bonus, an Eternal Knight's
                // deaths), as every other stat set does.
                let (wa, wh) = pl.wherever_stats(&data.cards, &m);
                m.attack = a + wa;
                m.health = h + wh;
            }
            if locked && !is_spell {
                m.locked_turns = 1;
            }
            if locked_tier > 0 && !is_spell {
                m.locked_tier = locked_tier;
            }
            if locked_gold > 0 && !is_spell {
                m.locked_gold = locked_gold;
            }
            if doomed && !is_spell {
                m.doomed = true;
            }
            if both && !is_spell {
                m.both_effects = true;
            }
            self.add_to_hand(p, m);
            if !is_spell {
                // The game's own triple check runs as the action ends, after
                // the copies a summon in the same action left on the board
                // (log 2026_10_24_15_25_26-g6 turn 8 step 16: the Skeletons
                // become a Golden card before the Discovered Cadaver Caretaker
                // makes one).
                self.check_triple_later(c);
            }
        } else if !is_spell && copies[k] > 0 {
            let slot = self.data.cards.get(c).pool_slot as usize;
            // A full hand loses the pick, and every pool copy it held goes
            // back (three for a Golden Dark Gift offer, none for a token or a
            // Dagger copy).
            self.pool[slot] += copies[k];
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

    /// Stir the Pot: one minion per thrown type, each drawn from the pool up
    /// to the player's Tier and all different, so a Demon, a Beast and a Mech
    /// give one of each and three Mechs give three Mechs (docs/rules/heroes.md).
    /// The slots hold the tribe plus one, 0 for an empty slot.
    fn deal_pot_offer(&mut self, p: usize, types: [u8; 3]) {
        let Game { data, rng, players, pool, lobby, .. } = self;
        let pl = &mut players[p];
        let tier = pl.tier;
        for &slot in types.iter() {
            if slot == 0 || pl.discover.is_full() {
                continue;
            }
            let Some(tribe) = Tribe::EVERY.get(slot as usize - 1).copied() else { continue };
            if let Some(c) = draw_filtered(pool, &data.cards, rng, 1, tier, &pl.discover, *lobby, |c| {
                c.tribes.has(tribe)
            }) {
                pl.discover.push(c);
            }
        }
    }

    /// Kel'Thuzad's Kitty: up to three of the dead hero's minions, exactly as
    /// they stood (buffs, keywords, Dark Gifts, Golden included), none of them
    /// taking a pool copy (docs/rules/heroes.md). The warband is the `i`th hero
    /// that died in the round.
    fn deal_dead_warband(&mut self, p: usize, i: usize) {
        let Game { kitty_warbands, rng, players, .. } = self;
        let Some((_, board)) = kitty_warbands.get(i) else { return };
        let pl = &mut players[p];
        pl.discover_minions.clear();
        // A copy, so the warband stays for the other holders.
        let mut left: ArrayVec<Minion, BOARD> = board.clone();
        while !pl.discover.is_full() && !left.is_empty() {
            let k = rng.below(left.len() as u32) as usize;
            let m = left.remove(k);
            if pl.discover.contains(&m.card) {
                continue;
            }
            pl.discover.push(m.card);
            pl.discover_minions.push(m);
        }
    }

    /// Reclaimed Souls: up to three different minions that died in the last
    /// fight, either side, tokens and summoned ones included, each offered as
    /// a plain copy (printed stats, no buffs, no pool copy: a Beetle can be
    /// found).
    fn deal_dead_last_fight(&mut self, p: usize) {
        let Game { data, rng, players, .. } = self;
        let pl = &mut players[p];
        let mut tried = 0;
        while !pl.discover.is_full() && tried < pl.dead_last_fight.len() * 2 + 1 {
            tried += 1;
            if pl.dead_last_fight.is_empty() {
                break;
            }
            let k = rng.below(pl.dead_last_fight.len() as u32) as usize;
            let c = pl.dead_last_fight[k];
            if pl.discover.contains(&c) {
                continue;
            }
            pl.discover.push(c);
            let m = pl.make_card(&data.cards, c, false, 0);
            pl.discover_minions.push(m);
        }
    }

    /// I Spy: up to three different minions from the warband of the opponent
    /// this player is about to fight, as it stood at the start of the round,
    /// each offered as a plain copy (printed stats, no buffs, no pool copy).
    fn deal_ispy_offer(&mut self, p: usize) {
        let Game { data, rng, players, .. } = self;
        let pl = &mut players[p];
        let mut left = pl.next_board.clone();
        pl.discover_minions.clear();
        let mut tried = 0;
        while !pl.discover.is_full() && !left.is_empty() && tried < left.len() * 4 + 1 {
            tried += 1;
            let k = rng.below(left.len() as u32) as usize;
            let m = left.remove(k);
            if pl.discover.contains(&m.card) {
                continue;
            }
            pl.discover.push(m.card);
            let copy = pl.make_card(&data.cards, m.card, false, 0);
            pl.discover_minions.push(copy);
        }
    }

    /// Time Thief: a plain copy of a minion from the last opponent's warband,
    /// as this player last saw it (a generated copy, like I Spy's).
    fn deal_last_opponent_offer(&mut self, p: usize) {
        let Game { data, rng, players, .. } = self;
        let pl = &mut players[p];
        let mut left = match pl.last_opponent {
            Some(super::Opponent::Player(q)) => pl.seen[q as usize].board.clone(),
            Some(super::Opponent::Ghost(g)) => pl.seen[g as usize].board.clone(),
            None => ArrayVec::new(),
        };
        pl.discover_minions.clear();
        let mut tried = 0;
        while !pl.discover.is_full() && !left.is_empty() && tried < left.len() * 4 + 1 {
            tried += 1;
            let k = rng.below(left.len() as u32) as usize;
            let m = left.remove(k);
            if pl.discover.contains(&m.card) {
                continue;
            }
            pl.discover.push(m.card);
            let copy = pl.make_card(&data.cards, m.card, false, 0);
            pl.discover_minions.push(copy);
        }
    }

    /// Choose Your Champion: three Tier 7 minions from the pool, the pick
    /// landing in hand locked until `gold` Gold has been spent this game
    /// (docs/rules/heroes.md, docs/rules/design.md "Cards locked in hand").
    fn deal_champion(&mut self, p: usize, gold: u16) {
        let Game { data, rng, players, pool, lobby, .. } = self;
        let pl = &mut players[p];
        while !pl.discover.is_full() {
            let tier = crate::CARD_MAX_TIER;
            match draw_filtered(pool, &data.cards, rng, tier, tier, &pl.discover, *lobby, |_| true) {
                Some(c) => pl.discover.push(c),
                None => break,
            }
        }
        pl.discover_locked_gold = gold;
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
        // The random card a repeat's Discover gave is not a Discover for
        // "after you Discover" cards (Hooktusk, Master Marauder): docs/rules/
        // design.md "Set versus add" and "Shop replay answers" (Era
        // 2026-11-03, 2026-11-04: a Kelp Keeper's Activate).
        if std::mem::take(&mut self.players[p].discover_silent) {
            return;
        }
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
