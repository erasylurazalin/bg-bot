//! The hero pick, hero powers, armor and hero damage.

use arrayvec::ArrayVec;
use crate::cards::{
    Effect,
    EventKind,
    SpellEffect,
    Tribe,
};
use crate::heroes::PowerEffect;
use crate::trinkets::{CardMod, Event};
use crate::minion::{
    CardId,
    Keywords,
};
use crate::{
    BOARD,
    CHOICES,
    DISCOVER,
    HAND,
    PLAYERS,
    SHOP,
};
use super::{
    DiscoverRequest,
    Game,
    PendingDiscard,
    PendingTarget,
    PowerState,
    TargetKind,
};
use super::effects::buff;

impl Game {
    /// Deal the hero offers and start the pick phase (turn 0): the living
    /// players, in a shuffled order, each answer their offer with
    /// `Action::Choose`; a player who picked is done, and once everyone has,
    /// turn 1 starts (`advance`). Offers are drawn without replacement from
    /// the heroes the lobby allows, round by round so a short hero table
    /// still gives everyone a hero.
    pub(super) fn open_hero_picks(&mut self) {
        let mut deck: Vec<u8> = self.data.heroes.lobby_allowed(self.lobby).collect();
        self.rng.shuffle(&mut deck);
        let n = self.data.rules.hero_choices as usize;
        let mut at = 0;
        for round in 0..n {
            for p in 0..PLAYERS {
                if !self.players[p].alive || self.players[p].real || at >= deck.len() {
                    continue;
                }
                self.players[p].hero_offer[round] = deck[at];
                at += 1;
            }
        }
        for p in 0..PLAYERS {
            // A player the hero table could not feed picks nothing and waits
            // the pick phase out (unreachable with the heroes built so far).
            let empty = self.players[p].hero_offer.iter().all(|&h| h == u8::MAX);
            self.players[p].done = !self.players[p].alive || empty;
        }
        self.order = (0..PLAYERS as u8).filter(|&p| self.players[p as usize].alive).collect();
        self.rng.shuffle(&mut self.order);
        self.cursor = 0;
    }

    /// Every living player takes a random hero the lobby allows, no two the
    /// same. A real seat gets none and uses up no hero from the deck. Turn 0
    /// then only waits for Finley's power Discover; with none open, turn 1
    /// starts right away.
    pub(super) fn deal_random_heroes(&mut self) {
        let mut deck: Vec<u8> = self.data.heroes.lobby_allowed(self.lobby).collect();
        self.rng.shuffle(&mut deck);
        let mut at = 0;
        for p in 0..PLAYERS {
            if self.players[p].alive && !self.players[p].real && at < deck.len() {
                self.take_hero(p, deck[at]);
                at += 1;
            }
        }
        for p in 0..PLAYERS {
            self.players[p].done = !self.players[p].alive || !Self::choice_open(&self.players[p]);
        }
        self.order = (0..PLAYERS as u8).filter(|&p| self.players[p as usize].alive).collect();
        self.rng.shuffle(&mut self.order);
        match self.order.iter().position(|&p| !self.players[p as usize].done) {
            Some(c) => self.cursor = c,
            None => self.start_turn(),
        }
    }

    /// Give player `p` this hero outright, no offer: the pick phase's
    /// effects (armor, extra Health, the power, Finley's Discover when
    /// `discover`, which the pick phase runs by itself). For tests (the
    /// sandbox's --hero goes through the pick at turn 0).
    ///
    /// A hero given this way while a later turn is already being played is
    /// the hero the game started with, so that turn's power work runs now:
    /// Faelin's Discovers open on turn 1, Clairvoyance's free Refresh lands.
    pub fn assign_hero(&mut self, p: usize, hero: u8) {
        self.take_hero(p, hero);
        if self.turn > 0 {
            self.power_start_of_turn(p);
        }
    }

    /// The player takes the hero: health, armor, power state, and Finley's
    /// power Discover if the hero's power is one.
    fn take_hero(&mut self, p: usize, hero: u8) {
        let h = self.data.heroes.get(hero);
        let (extra_health, armor, starting, excludes) =
            (h.extra_health, h.armor, h.power, h.power_excludes.clone());
        let effect = self.data.heroes.power(starting).effect;
        self.players[p].health += extra_health;
        self.players[p].armor = armor as i32;
        self.players[p].hero = Some(hero);
        self.players[p].power2 = None;
        let power = self.data.heroes.power(starting).clone();
        self.players[p].power = Some(PowerState::fresh(starting, &power));
        self.refresh_power_unlock(p);
        self.refresh_skip(p);
        if effect == PowerEffect::DiscoverPower {
            let excludes = self.data.heroes.hero_indices(&excludes);
            self.discover_hero_power(p, &excludes, false, 1);
        } else {
            // A power that is a rotating family (A Tale of Kings) starts on a
            // random tribe of the lobby, before turn 1.
            self.rotate_family_power(p, false);
        }
    }

    /// Set each held power's unlock flag from the player's tier and the turn
    /// ("Unlocks at Tier N", "Unlocks on Turn N").
    pub(super) fn refresh_power_unlock(&mut self, p: usize) {
        let (tier, turn) = (self.players[p].tier, self.turn);
        for second in [false, true] {
            let Some(st) = self.players[p].power_at(second) else { continue };
            let power = self.data.heroes.power(st.of);
            let unlocked = tier >= power.unlock_tier && turn >= power.unlock_turn;
            if let Some(st) = self.players[p].power_at_mut(second) {
                st.unlocked = unlocked;
            }
        }
    }

    /// Put this power in the given slot, its state fresh. `None` clears it.
    pub(super) fn set_power(&mut self, p: usize, second: bool, of: Option<u8>) {
        let state = of.map(|of| {
            let power = self.data.heroes.power(of).clone();
            PowerState::fresh(of, &power)
        });
        if second {
            self.players[p].power2 = state;
        } else {
            self.players[p].power = state;
        }
        self.refresh_power_unlock(p);
        self.refresh_skip(p);
    }

    /// Whether this power is one of the player's own (the Discover of a Hero
    /// Power never offers one the player already holds).
    fn holds_power(&self, p: usize, of: u8) -> bool {
        self.players[p].power.is_some_and(|st| st.of == of)
            || self.players[p].power2.is_some_and(|st| st.of == of)
    }

    /// A rotating power (A Tale of Kings) becomes a random other power of its
    /// family whose tribe is in the lobby, never the same tribe twice
    /// (docs/heroes.md "King of Beasts, Rat King").
    pub(super) fn rotate_family_power(&mut self, p: usize, second: bool) {
        let Some(st) = self.players[p].power_at(second) else { return };
        let power = self.data.heroes.power(st.of);
        if power.family == 0 {
            return;
        }
        let options =
            self.data.heroes.family_options(power.family, self.lobby, power.tribe_of());
        if options.is_empty() {
            return;
        }
        let k = self.rng.below(options.len() as u32) as usize;
        self.set_power(p, second, Some(options[k]));
    }

    /// Whether the player's MAIN power button may be pressed right now.
    pub fn power_usable(&self, p: usize) -> bool {
        self.power_usable_at(p, false)
    }

    /// Whether the player's SECOND power button may be pressed right now.
    pub fn power_usable2(&self, p: usize) -> bool {
        self.power_usable_at(p, true)
    }

    /// Whether one of the player's two power buttons may be pressed right
    /// now: a button, unlocked, uses left, not past its once-per-game, the
    /// Gold for it, and whatever its effect needs to land on something.
    pub fn power_usable_at(&self, p: usize, second: bool) -> bool {
        let Some(st) = self.players[p].power_at(second) else { return false };
        let power = self.data.heroes.power(st.of);
        let pl = &self.players[p];
        if power.passive
            // A power that became a trinket (Marin, Buttons) has no button.
            || pl.trinkets[super::trinkets::POWER_SLOT].of != 0
            || !st.unlocked
            || st.uses_left == 0
            || (power.once_per_game && st.used_game > 0)
            || pl.gold < power.cost
        {
            return false;
        }
        let hand_room = !pl.hand.is_full();
        let shop_minion = self.shop_has_minion(p);
        match power.effect {
            PowerEffect::GiveDivineShield | PowerEffect::MakeGolden => {
                !pl.board.is_empty() && pl.board.iter().any(|m| !m.golden)
            }
            PowerEffect::StealCardDamage { .. } => {
                hand_room && (shop_minion || pl.shop_spell.is_some())
            }
            PowerEffect::StealRandomDoubleHealth => hand_room && shop_minion,
            PowerEffect::GetRandomSpell => hand_room,
            // A Battlecry to trigger (Snicker-snack); the minion itself may
            // be the target, so no slot is excluded.
            PowerEffect::TriggerBattlecry => self.has_battlecry_target(p, None),
            // A card to throw away (Incubate): with none the press would do
            // nothing.
            PowerEffect::DiscardGetRandomMinion { .. } => self.has_discardable(p),
            // Both minions must fit in hand to stay a linked pair (Dark
            // Ritual).
            PowerEffect::GetLinkedMinions(n) => pl.hand.len() + n as usize <= HAND,
            // A press that only completes a chooser's pick needs nothing.
            _ => true,
        }
    }

    /// One of the hero power buttons is pressed: pay, spend a use, run the
    /// effect. "Your Hero Power triggers twice" (Ancient Wishbone) runs the
    /// effect twice for the one press and the one payment.
    pub(super) fn press_power(&mut self, p: usize, second: bool) {
        let (of, cost) = {
            let st = self.players[p].power_at(second).unwrap();
            (st.of, self.data.heroes.power(st.of).cost)
        };
        let effect = self.data.heroes.power(of).effect;
        let st = self.players[p].power_at_mut(second).unwrap();
        st.uses_left = st.uses_left.saturating_sub(1);
        st.used_game += 1;
        self.players[p].gold -= cost;
        let runs = self.players[p].tp.power_triggers.max(1);
        match effect {
            PowerEffect::ExtraMaxGold(n) => {
                let n = n.saturating_mul(runs);
                self.players[p].extra_max_gold = self.players[p].extra_max_gold.saturating_add(n);
            }
            PowerEffect::GiveDivineShield => {
                let pt = PendingTarget {
                    source: None,
                    kind: TargetKind::MinionBuff {
                        tribe: None,
                        attack: 0,
                        health: 0,
                        keyword: Keywords::DIVINE_SHIELD,
                        random_keyword: false,
                    },
                    runs,
                    tavern: false,
                    gem: false,
                };
                self.offer_target(p, pt);
            }
            PowerEffect::StealCardDamage(damage) => {
                let pt = PendingTarget {
                    source: None,
                    kind: TargetKind::StealShopCard { damage },
                    runs,
                    tavern: false,
                    gem: false,
                };
                self.offer_target(p, pt);
            }
            PowerEffect::StealRandomDoubleHealth => {
                for _ in 0..runs {
                    if !self.steal_random_shop_double_health(p) {
                        break;
                    }
                }
            }
            PowerEffect::MakeGolden => {
                let pt = PendingTarget {
                    source: None,
                    kind: TargetKind::MakeGolden { tribe: None, max_tier: crate::CARD_MAX_TIER },
                    runs,
                    tavern: false,
                    gem: false,
                };
                self.offer_target(p, pt);
            }
            PowerEffect::GetGems(n) => self.give_gems(p, n.saturating_mul(runs)),
            PowerEffect::DiscoverTribe(t) => {
                for _ in 0..runs {
                    let _ = self.players[p].discover_queue.try_push(DiscoverRequest::TribeMinion(t));
                }
                self.maybe_start_discover(p);
            }
            PowerEffect::SpellDiscount(n) => {
                let n = n.saturating_mul(runs);
                let pl = &mut self.players[p];
                pl.spell_discount = pl.spell_discount.saturating_add(n);
            }
            PowerEffect::GetRandomSpell => {
                let tier = self.players[p].tier;
                for _ in 0..runs {
                    if let Some(card) = self.data.cards.random_spell(tier, self.lobby, &mut self.rng) {
                        self.give_generated(p, card);
                    }
                }
            }
            PowerEffect::TriggerBattlecry => {
                let pt = PendingTarget {
                    source: None,
                    kind: TargetKind::TriggerBattlecry,
                    runs,
                    tavern: true,
                    gem: false,
                };
                self.offer_target(p, pt);
            }
            PowerEffect::DiscardGetRandomMinion { tribe: t } => {
                // The reward runs from `resolve_discard` once a card is gone,
                // the same path an Activate with `discard = true` takes.
                self.players[p].pending_discard = Some(PendingDiscard {
                    source: None,
                    reward: Effect::GetRandomMinion { tribe: t },
                    golden: false,
                });
            }
            PowerEffect::GetLinkedMinions(n) => {
                let tier = self.players[p].tier;
                let n = n.saturating_mul(runs);
                let mut got: ArrayVec<CardId, 4> = ArrayVec::new();
                for _ in 0..n {
                    let drawn = self.data.cards.random_any_minion(tier, self.lobby, &mut self.rng);
                    if let Some(card) = drawn {
                        let _ = got.try_push(card);
                    }
                }
                self.give_linked(p, &got, false);
            }
            PowerEffect::RefreshWithSpells => {
                for _ in 0..runs {
                    let Game { data, rng, players, pool, lobby, .. } = self;
                    Self::refresh_with_spells(
                        &mut players[p], &data.cards, &data.rules, rng, pool, *lobby,
                    );
                }
            }
            // King of Duality: two Hero Power Discovers, the first into the
            // second slot and the second into the main one.
            PowerEffect::DiscoverPowers(n) => {
                self.players[p].power_offer_left = n;
                // The first pick is the second power (docs/heroes.md).
                self.players[p].power_offer_second = !second;
                self.deal_power_offer(p);
            }
            // Embrace the Elements, Dungar's Gryphon: a fixed choice of
            // powers, never the one that just completed.
            PowerEffect::ChooseSubPowers => {
                let last = self.players[p].power_at(second).map_or(0, |st| st.last_pick);
                let mut picks: ArrayVec<u8, CHOICES> = ArrayVec::new();
                for &o in &self.data.heroes.power(of).options {
                    if o as u16 + 1 != last as u16 {
                        let _ = picks.try_push(o);
                    }
                }
                let pl = &mut self.players[p];
                pl.power_offer = picks;
                pl.power_offer_second = second;
            }
            _ => {}
        }
        // "After you use your Hero Power, gain (1) Gold" (Sous Chef Sticker):
        // once per press, after the press's own cost.
        let back = self.players[p].tp.power_gold;
        if back > 0 {
            self.players[p].gold = self.players[p].gold.saturating_add(back);
        }
        self.fire_event(p, EventKind::GoldSpent, cost as u32);
    }

    /// Everything a player's two powers do at the start of their turn: the
    /// uses come back, an unlock condition may be met, a start-of-turn passive
    /// runs, a rotating power turns to another tribe, a chosen power counts
    /// down (or completes), and the powers that hand out cards do it.
    pub(super) fn power_start_of_turn(&mut self, p: usize) {
        let turn = self.turn;
        for second in [false, true] {
            let Some(st) = self.players[p].power_at(second) else { continue };
            // Only the Copy numbers of the power are read here, so nothing is
            // cloned or allocated on the start of every turn.
            let power = self.data.heroes.power(st.of);
            let (uses_per_turn, complete_turns) = (power.uses_per_turn, power.complete_turns);
            let counts_down = power.parent.is_some() && complete_turns > 0;
            let runs = self.players[p].tp.power_triggers.max(1);
            let uses = uses_per_turn.saturating_add(self.players[p].tp.power_extra_uses);
            if let Some(sl) = self.players[p].power_at_mut(second) {
                sl.uses_left = uses;
            }
            // A power chosen from a chooser counts down; when it completes,
            // its bonus runs and the chooser comes back.
            if counts_down {
                let done = {
                    let sl = self.players[p].power_at_mut(second).unwrap();
                    sl.progress += 1;
                    sl.progress >= complete_turns as u32
                };
                if done {
                    self.complete_power(p, second);
                    continue;
                }
            }
            self.refresh_power_unlock(p);
            // A power that is part of a family (A Tale of Kings) swaps its
            // tribe at the start of every turn.
            self.rotate_family_power(p, second);
            let Some(st) = self.players[p].power_at(second) else { continue };
            let effect = self.data.heroes.power(st.of).effect;
            match effect {
                PowerEffect::FreeRefreshPerTurn(n) => {
                    let n = n.saturating_mul(runs);
                    let pl = &mut self.players[p];
                    pl.free_rolls = pl.free_rolls.saturating_add(n);
                }
                PowerEffect::Bananarama { mine, others, big_percent, banana, big } => {
                    let n = mine.saturating_mul(runs);
                    self.give_bananas(p, n, big_percent, banana, big);
                    for q in 0..PLAYERS {
                        // A real seat is a logged board, not a shopper: its
                        // hand is the file's, so it takes no Banana.
                        if q == p || !self.players[q].alive || self.players[q].real {
                            continue;
                        }
                        self.give_bananas(q, others, big_percent, banana, big);
                    }
                }
                PowerEffect::TurnDiscovers { turn: at, tiers } => {
                    if turn == at {
                        for &t in tiers.iter().filter(|&&t| t != 0) {
                            let _ = self.players[p]
                                .discover_queue
                                .try_push(DiscoverRequest::TierLockedMinion(t));
                        }
                        self.maybe_start_discover(p);
                    }
                }
                _ => {}
            }
        }
        // Procrastinate and Expedition Plans: is this turn spent?
        self.refresh_skip(p);
    }

    /// Hand `n` Bananas to one player, each the plain one or, `percent` of the
    /// time, the big one (Bananarama). A hand with no room simply misses it.
    fn give_bananas(&mut self, p: usize, n: u8, percent: u8, banana: CardId, big: CardId) {
        for _ in 0..n {
            let card = if percent > 0 && self.rng.below(100) < percent as u32 { big } else { banana };
            let m = self.players[p].make_card(&self.data.cards, card, false, 0);
            self.add_to_hand(p, m);
        }
    }

    /// A chosen power completes: its effect runs once, then the chooser it
    /// came from is back, remembering what just completed so it is not
    /// offered straight away (Galewing).
    fn complete_power(&mut self, p: usize, second: bool) {
        let Some(st) = self.players[p].power_at(second) else { return };
        let power = self.data.heroes.power(st.of);
        let Some(parent) = power.parent else { return };
        let effect = power.effect;
        let mut pos = 0usize;
        match effect {
            PowerEffect::Gold(n) => {
                self.execute_spell_effect(p, SpellEffect::GainGold(n), false)
            }
            PowerEffect::GetRandomSpellOfCost(c) => self.run_effect(
                p,
                &mut pos,
                Effect::GetRandomSpellUpTo { cost: Some(c), count: 1 },
                false,
                1,
            ),
            PowerEffect::DiscoverMyTier => {
                let tier = self.players[p].tier;
                let _ = self.players[p].discover_queue.try_push(DiscoverRequest::MinionTier(tier));
                self.maybe_start_discover(p);
            }
            _ => {}
        }
        self.set_power(p, second, Some(parent));
        if let Some(sl) = self.players[p].power_at_mut(second) {
            sl.last_pick = st.of + 1;
        }
    }

    /// The end-of-turn effects of the player's two powers (Sulfuras).
    pub(super) fn power_end_of_turn(&mut self, p: usize) {
        for second in [false, true] {
            let Some(st) = self.players[p].power_at(second) else { continue };
            let effect = self.data.heroes.power(st.of).effect;
            let runs = self.players[p].tp.power_triggers.max(1);
            if let PowerEffect::EndOfTurnEnds { attack, health } = effect {
                for _ in 0..runs {
                    let n = self.players[p].board.len();
                    if n == 0 {
                        continue;
                    }
                    buff(&mut self.players[p].board[0], attack, health);
                    // With one minion it is both ends, so it gets both buffs.
                    buff(&mut self.players[p].board[n - 1], attack, health);
                }
            }
        }
    }

    /// A card was bought (BUY, INSECT! counts them): the power may become
    /// another one. Free buys and Tavern spells count; a steal does not, as it
    /// never comes through here.
    pub(super) fn power_bought_card(&mut self, p: usize) {
        for second in [false, true] {
            let Some(st) = self.players[p].power_at(second) else { continue };
            let effect = self.data.heroes.power(st.of).effect;
            if let PowerEffect::BuyMilestone { count, becomes } = effect {
                let sl = self.players[p].power_at_mut(second).unwrap();
                sl.progress = sl.progress.saturating_add(1);
                if sl.progress >= count as u32 {
                    self.set_power(p, second, Some(becomes));
                }
            }
        }
    }

    /// A fight is over: a power chosen for the next Start of Combat has fired,
    /// and the chooser it came from is back (Bru'kan's Elements).
    pub(super) fn power_after_fight(&mut self, p: usize) {
        for second in [false, true] {
            let Some(st) = self.players[p].power_at(second) else { continue };
            let power = self.data.heroes.power(st.of);
            if power.parent.is_some() && power.complete_turns == 0 {
                self.complete_power(p, second);
            }
        }
    }

    /// Whether this player's turn is skipped (Procrastinate, Expedition
    /// Plans): only Freeze and End Turn are legal, and the fight still
    /// happens. `refresh_skip` keeps the flag, so this is one read on the
    /// legality path.
    pub fn turn_skipped(&self, p: usize) -> bool {
        self.players[p].skipped
    }

    /// Work out whether this player's turn is skipped now and keep it on the
    /// player, so the legality check stays one read. A hero's first turns are
    /// counted from turn 1, so the pick phase (turn 0) is never skipped.
    fn refresh_skip(&mut self, p: usize) {
        let turn = self.turn;
        let skipped = turn > 0
            && [false, true].iter().any(|&second| {
                self.players[p].power_at(second).is_some_and(|st| {
                    let skip = self.data.heroes.power(st.of).skip_turns;
                    skip > 0 && turn <= skip as u32
                })
            });
        self.players[p].skipped = skipped;
    }

    /// Steal a random minion from the Tavern into hand and double its Health
    /// (Brick by Brick). The pool copy leaves with the card, like a buy, and
    /// goes back where it stood when the hand is full (a press that triggers
    /// twice can fill it). False when nothing was stolen.
    fn steal_random_shop_double_health(&mut self, p: usize) -> bool {
        let slots: ArrayVec<usize, SHOP> = (0..self.players[p].shop.len())
            .filter(|&s| !self.data.cards.get(self.players[p].shop[s].card).is_spell())
            .collect();
        if slots.is_empty() {
            return false;
        }
        let k = self.rng.below(slots.len() as u32) as usize;
        let slot = slots[k];
        let mut m = self.players[p].shop.remove(slot);
        m.health *= 2;
        if self.add_to_hand(p, m) {
            return true;
        }
        self.players[p].shop.insert(slot, m);
        false
    }

    /// Steal the Tavern card the player picked into hand: a shop slot (any
    /// card in it) or, one past the last, the spell slot. Graveyard Shift's
    /// damage lands after the card is in hand. The hand can fill between the
    /// press and the pick (a "gold spent" trigger hands out a card), and then
    /// the steal fizzles: the card goes back where it stood (a pool copy
    /// never vanishes). Returns whether the card was taken.
    pub(super) fn steal_shop_card(&mut self, p: usize, slot: usize) -> bool {
        let from_spell = slot >= self.players[p].shop.len();
        let pl = &mut self.players[p];
        let m = if !from_spell {
            pl.shop.remove(slot)
        } else if let Some(id) = pl.shop_spell.take() {
            pl.make_card(&self.data.cards, id, false, 0)
        } else {
            return false;
        };
        if self.add_to_hand(p, m) {
            return true;
        }
        // The hand filled up: put the card back where it stood.
        let pl = &mut self.players[p];
        if from_spell {
            pl.shop_spell = Some(m.card);
        } else {
            pl.shop.insert(slot, m);
        }
        false
    }

    /// Open one Hero Power Discover for the player: `count` picks in all, the
    /// first of them into the slot `to_second` says (the first of King of
    /// Duality's two is the second power, the second pick the main one).
    pub(super) fn discover_hero_power(
        &mut self,
        p: usize,
        excludes: &[u8],
        to_second: bool,
        count: u8,
    ) {
        self.players[p].power_offer_left = count;
        self.players[p].power_offer_second = to_second;
        let list = &mut self.players[p].power_excludes;
        list.clear();
        for &h in excludes {
            let _ = list.try_push(h);
        }
        self.deal_power_offer(p);
    }

    /// Deal the open Hero Power Discover: three starting powers of built
    /// heroes, never one the player already holds, never a hero on the
    /// exclusion list, never a hero tied only to tribes outside the lobby
    /// (DESIGN.md "Discovering a hero power"). The slot it answers into is
    /// the player's `power_offer_second`.
    fn deal_power_offer(&mut self, p: usize) {
        let options: Vec<u8> = self
            .data
            .heroes
            .lobby_allowed(self.lobby)
            .filter(|&h| !self.players[p].power_excludes.contains(&h))
            .map(|h| self.data.heroes.get(h).power)
            .filter(|&of| !self.holds_power(p, of))
            .collect();
        if options.is_empty() {
            self.players[p].power_offer_left = 0;
            return;
        }
        // Draw three distinct ones (a reservoir sample over the candidates).
        let mut picks: ArrayVec<u8, CHOICES> = ArrayVec::new();
        let mut left = options.len() as u32;
        for &h in &options {
            let want = (DISCOVER - picks.len()) as u32;
            if picks.len() < DISCOVER && self.rng.below(left) < want {
                picks.push(h);
            }
            left -= 1;
        }
        self.players[p].power_offer = picks;
    }

    /// The player answered an open Hero Power offer: the power lands in the
    /// slot the offer named, and King of Duality's second pick is dealt next.
    pub(super) fn pick_power(&mut self, p: usize, k: usize) {
        let Some(&of) = self.players[p].power_offer.get(k) else { return };
        let second = self.players[p].power_offer_second;
        self.players[p].power_offer.clear();
        self.set_power(p, second, Some(of));
        // A start-of-game power picked before turn 1 (Finley's Discover
        // finding All Patched Up) still does its part: the extra Health.
        if self.turn == 0 {
            self.players[p].health += self.power_extra_health(of);
        }
        let left = self.players[p].power_offer_left.saturating_sub(1);
        self.players[p].power_offer_left = left;
        if left > 0 {
            // The next pick goes to the main power (Genn's second pick).
            self.players[p].power_offer_second = false;
            self.deal_power_offer(p);
        } else {
            self.players[p].power_offer_second = false;
        }
    }

    /// The Health a power gives the hero for good when it is picked (All
    /// Patched Up), 0 for every other power.
    fn power_extra_health(&self, of: u8) -> i32 {
        self.data.heroes.heroes.iter().find(|h| h.power == of).map_or(0, |h| h.extra_health)
    }

    /// The player picked a hero from the offer (turn 0): the offer closes,
    /// the hero lands (health, armor, power, Finley's Discover).
    pub(super) fn pick_hero(&mut self, p: usize, k: usize) {
        let hero = self.players[p].hero_offer[k];
        self.players[p].hero_offer = [u8::MAX; DISCOVER];
        self.take_hero(p, hero);
    }

    /// The hero takes `amount` damage, then the "after your hero takes
    /// damage" effects and the damage event triggers run. They fire even
    /// when a rewind among them gives the Health back. Combat damage doesn't
    /// come through here (no minion reacts to it) and is settled by
    /// `knockouts` after the fights; Health costs are checked by
    /// `can_pay_health` before they're paid, so they can't kill.
    ///
    /// `source` is the board slot of the friendly Demon that dealt it, if
    /// any: a Health cost (Malchezaar) or the combat damage to the hero is
    /// not a Demon dealing damage, so they pass `None`.
    pub(super) fn take_hero_damage(&mut self, p: usize, amount: i32, source: Option<usize>) {
        let pl = &mut self.players[p];
        if amount <= 0 {
            return;
        }
        // Armor takes it first (DESIGN.md "Health and armor").
        let to_armor = pl.armor.min(amount);
        pl.armor -= to_armor;
        pl.health -= amount - to_armor;
        pl.last_damage = amount;
        pl.last_damage_armor = to_armor;
        self.note_shop_demon_damage(p, source);
        let mut pos = 0;
        while pos < self.players[p].board.len() {
            let m = self.players[p].board[pos];
            for (card, golden) in m.parts() {
                for k in 0..self.data.cards.get(card).after_hero_damage.len() {
                    if pos >= self.players[p].board.len() {
                        break;
                    }
                    let e = self.data.cards.get(card).after_hero_damage[k];
                    self.run_effect(p, &mut pos, e, golden, 1);
                }
            }
            pos += 1;
        }
        self.fire_event(p, EventKind::HeroDamage, amount as u32);
        // "Every 3 times your hero takes damage outside combat" (Nether
        // Pendant): one count per damage, whatever the amount.
        self.trinket_event(p, Event::HeroDamage, None, 1);
        // The CardMods a portrait gives the wounded minions are read here.
        self.after_hero_damage_mods(p);
    }

    /// A portrait that gives the card's "after your hero takes damage"
    /// reward more (Rewinder Portrait: your Soul Rewinders also gain Attack).
    /// The reward itself has just run, so this adds the portrait's share.
    fn after_hero_damage_mods(&mut self, p: usize) {
        let cards = &self.data.cards;
        let mods = self.players[p].tp.card_mods;
        for m in &mut self.players[p].board {
            let mut extra = 0;
            for (c, _) in m.parts() {
                if cards.get(c).after_hero_damage.is_empty() {
                    continue;
                }
                if let Some(CardMod::AlsoAttack(n)) =
                    mods.iter().flatten().find(|(card, _)| *card == c).map(|(_, k)| *k)
                {
                    extra += n;
                }
            }
            m.attack += extra;
        }
    }

    /// Damage to the hero in the shop phase (Wrath Weaver, Imposing
    /// Percussionist). Once its reactions have run, a hero at 0 Health or
    /// below is knocked out on the spot: a rewind on the board has already
    /// had its chance to give the Health back.
    pub(super) fn take_shop_damage(&mut self, p: usize, amount: i32, source: Option<usize>) {
        self.take_hero_damage(p, amount, source);
        if self.players[p].alive && self.players[p].health <= 0 {
            self.knock_out_mid_turn(p);
        }
    }

    /// A friendly Demon dealt damage in the shop phase: every other friendly
    /// minion with "After another friendly Demon deals damage" gains its
    /// stats for good right away (Devout Hellcaller). A damage source that
    /// is no longer a friendly Demon does not count.
    fn note_shop_demon_damage(&mut self, p: usize, source: Option<usize>) {
        let Some(src) = source else { return };
        let cards = &self.data.cards;
        let Some(src_card) = self.players[p].board.get(src).map(|m| m.card) else { return };
        if !cards.get(src_card).tribes.has(Tribe::Demon) {
            return;
        }
        let mut hits: ArrayVec<(usize, i32, i32), BOARD> = ArrayVec::new();
        for (i, m) in self.players[p].board.iter().enumerate() {
            if i == src {
                continue;
            }
            for (card, golden) in m.parts() {
                if let Some((a, h)) = cards.get(card).after_demon_damage {
                    let mul = if golden { 2 } else { 1 };
                    let _ = hits.try_push((i, a * mul, h * mul));
                }
            }
        }
        for (i, da, dh) in hits {
            buff(&mut self.players[p].board[i], da, dh);
        }
    }
}
