//! The hero pick, hero powers, armor and hero damage.

use arrayvec::ArrayVec;
use crate::cards::{
    EventKind,
    Tribe,
};
use crate::heroes::PowerEffect;
use crate::minion::Keywords;
use crate::{
    BOARD,
    DISCOVER,
    PLAYERS,
    SHOP,
};
use super::{
    DiscoverRequest,
    Game,
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
    /// `discover`, which the pick phase runs by itself). For tests and the
    /// sandbox's --hero.
    pub fn assign_hero(&mut self, p: usize, hero: u8) {
        self.take_hero(p, hero);
    }

    /// The player takes the hero: health, armor, power state, and Finley's
    /// power Discover if the hero's power is one.
    fn take_hero(&mut self, p: usize, hero: u8) {
        let h = self.data.heroes.get(hero);
        let (extra_health, armor, discovers, excludes) =
            (h.extra_health, h.armor, h.power.effect == PowerEffect::DiscoverPower, h.power_excludes.clone());
        self.players[p].health += extra_health;
        self.players[p].armor = armor as i32;
        self.players[p].hero = Some(hero);
        self.players[p].power = Some(PowerState::fresh(hero, &self.data.heroes.get(hero).power));
        self.refresh_power_unlock(p);
        if discovers {
            self.discover_hero_power(p, &excludes);
        }
    }

    /// Set the power's unlock flag from the player's tier and the turn
    /// ("Unlocks at Tier N", "Unlocks on Turn N").
    pub(super) fn refresh_power_unlock(&mut self, p: usize) {
        let Some(st) = self.players[p].power else { return };
        let power = &self.data.heroes.get(st.of).power;
        let pl = &self.players[p];
        let unlocked = pl.tier >= power.unlock_tier && self.turn >= power.unlock_turn;
        self.players[p].power.as_mut().unwrap().unlocked = unlocked;
    }

    /// Whether the player's power button may be pressed right now: a button,
    /// unlocked, uses left, not past its once-per-game, the Gold for it, and
    /// whatever its effect needs to land on something.
    pub fn power_usable(&self, p: usize) -> bool {
        let Some(st) = self.players[p].power else { return false };
        let power = &self.data.heroes.get(st.of).power;
        let pl = &self.players[p];
        if power.passive
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
            _ => true,
        }
    }

    /// The hero power button is pressed: pay, spend a use, run the effect.
    pub(super) fn press_power(&mut self, p: usize) {
        let (of, cost) = {
            let st = self.players[p].power.unwrap();
            (st.of, self.data.heroes.get(st.of).power.cost)
        };
        let effect = self.data.heroes.get(of).power.effect;
        let st = self.players[p].power.as_mut().unwrap();
        st.uses_left = st.uses_left.saturating_sub(1);
        st.used_game += 1;
        self.players[p].gold -= cost;
        match effect {
            PowerEffect::ExtraMaxGold(n) => {
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
                    runs: 1,
                    tavern: false,
                    gem: false,
                };
                self.offer_target(p, pt);
            }
            PowerEffect::StealCardDamage(damage) => {
                let pt = PendingTarget {
                    source: None,
                    kind: TargetKind::StealShopCard { damage },
                    runs: 1,
                    tavern: false,
                    gem: false,
                };
                self.offer_target(p, pt);
            }
            PowerEffect::StealRandomDoubleHealth => {
                self.steal_random_shop_double_health(p);
            }
            PowerEffect::MakeGolden => {
                let pt = PendingTarget {
                    source: None,
                    kind: TargetKind::MakeGolden { tribe: None, max_tier: crate::CARD_MAX_TIER },
                    runs: 1,
                    tavern: false,
                    gem: false,
                };
                self.offer_target(p, pt);
            }
            PowerEffect::GetGems(n) => self.give_gems(p, n),
            PowerEffect::DiscoverTribe(t) => {
                let _ = self.players[p].discover_queue.try_push(DiscoverRequest::TribeMinion(t));
                self.maybe_start_discover(p);
            }
            _ => {}
        }
        self.fire_event(p, EventKind::GoldSpent, cost as u32);
    }

    /// Steal a random minion from the Tavern into hand and double its Health
    /// (Brick by Brick). The pool copy leaves with the card, like a buy.
    fn steal_random_shop_double_health(&mut self, p: usize) {
        let slots: ArrayVec<usize, SHOP> = (0..self.players[p].shop.len())
            .filter(|&s| !self.data.cards.get(self.players[p].shop[s].card).is_spell())
            .collect();
        if slots.is_empty() {
            return;
        }
        let k = self.rng.below(slots.len() as u32) as usize;
        let mut m = self.players[p].shop.remove(slots[k]);
        m.health *= 2;
        self.add_to_hand(p, m);
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

    /// Offer the player a choice of hero powers, the one engine function
    /// Unmasked Identity and Finley's Adventure! share (DESIGN.md
    /// "Discovering a hero power"): three starting powers of built heroes,
    /// never the player's current power, never a hero on `excludes` (hero
    /// card_ids), never a hero tied only to tribes outside the lobby.
    pub(super) fn discover_hero_power(&mut self, p: usize, excludes: &[String]) {
        let current = self.players[p].power.map(|st| st.of);
        let options: Vec<u8> = self
            .data
            .heroes
            .lobby_allowed(self.lobby)
            .filter(|&h| Some(h) != current && !excludes.contains(&self.data.heroes.get(h).card_id))
            .collect();
        if options.is_empty() {
            return;
        }
        // Draw three distinct ones (a reservoir sample over the candidates).
        let mut picks: ArrayVec<u8, DISCOVER> = ArrayVec::new();
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
