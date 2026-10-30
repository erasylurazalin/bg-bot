//! Trinkets in a game: the two offers, the pick, the slots they live in and
//! the hooks they fire. See DESIGN.md "Trinkets and Dark Gifts (2026-10-30)"
//! and `crates/bgcore/src/trinkets.rs` for the data.

use arrayvec::ArrayVec;

use crate::cards::{
    DiscoverKind,
    Effect,
    EventKind,
    SpellEffect,
    Tribe,
    Tribes,
};
use crate::game::effects::buff;
use crate::heroes::PowerEffect;
use crate::minion::CardId;
use crate::trinkets::{
    Act,
    Event,
    Mechanic,
    Slot,
    TRINKET_CHOICES,
    TRINKET_SLOTS,
};
use super::{
    Game,
    TrinketState,
};

/// Slot 0 is the lesser one, 1 the greater one, 2 the trinket a hero power
/// became (Marin the Manager, Buttons).
pub const LESSER_SLOT: usize = 0;
pub const GREATER_SLOT: usize = 1;
pub const POWER_SLOT: usize = 2;

/// What a Tavern minion costs this player right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MinionPrice {
    Gold(u8),
    /// Paid in Health: self damage, not Gold spent (DESIGN.md).
    Health(u8),
}

/// Which of a trinket's effect lists a hook runs. `Copy`, like `Act`, so the
/// frame can fetch one act out of the shared `Data`, drop the borrow and run
/// it: no `Arc<Data>` clone and no `Vec` clone in a per-event path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hook {
    OnPick,
    StartOfTurn,
    EndOfTurn,
    OnBuy,
    OnBuySpell,
    OnPlay,
    OnSell,
    OnRefresh,
    OnCastSpell,
    OnCastSpellOnMinion,
    OnDiscard,
    OnGoldSpent,
    Rule,
}

impl Game {
    /// The trinket in this slot, as an index into `TrinketDb`, if any.
    pub fn trinket_of(&self, p: usize, slot: usize) -> Option<u8> {
        let of = self.players[p].trinkets[slot].of;
        // `then_some` would build `of - 1` even when `of` is 0.
        if of == 0 { None } else { Some(of - 1) }
    }

    /// The tribes the player has minions of, for the guarantee and the
    /// price: `[board counts, board plus hand counts]`. DESIGN.md counts the
    /// board only, except Murlocs, which count in hand too; the wider one is
    /// what the offer weights call "your tribes".
    fn tribe_counts(&self, p: usize) -> [[u32; Tribe::EVERY.len()]; 2] {
        let mut out = [[0u32; Tribe::EVERY.len()]; 2];
        let cards = &self.data.cards;
        for m in &self.players[p].board {
            let ts = cards.get(m.card).tribes;
            for (i, t) in Tribe::EVERY.into_iter().enumerate() {
                if ts.has(t) {
                    out[0][i] += 1;
                    out[1][i] += 1;
                }
            }
        }
        for m in &self.players[p].hand {
            let ts = cards.get(m.card).tribes;
            for (i, t) in Tribe::EVERY.into_iter().enumerate() {
                if ts.has(t) {
                    out[1][i] += 1;
                }
            }
        }
        // Murlocs in hand count for the guarantee and the price too.
        let murloc = Tribe::Murloc as usize;
        out[0][murloc] = out[1][murloc];
        out
    }

    /// What the two slots owe this player this turn: the standard offers at
    /// their turns, and a hero power that becomes a trinket (Marin the
    /// Manager's lesser one on turn 5, Buttons' greater one on turn 8).
    pub(super) fn open_trinket_offers(&mut self, p: usize) {
        if !self.players[p].trinket_offer.is_empty() {
            return;
        }
        let rules = &self.data.rules;
        let turn = self.turn;
        let (lesser_turn, greater_turn) = (rules.trinket_lesser_turn, rules.trinket_greater_turn);
        // A trinket can move or add an offer (Ornate Clock, Trip Vouchers).
        let tp = self.players[p].tp;
        let greater_turn = if tp.greater_offer_turn != 0 { tp.greater_offer_turn } else { greater_turn };
        if turn == tp.greater_into_lesser_turn && self.players[p].trinkets[LESSER_SLOT].of != 0 {
            // Trip Vouchers: a greater offer into the lesser slot, replacing
            // the trinket that named this turn.
            self.deal_trinket_offer(p, LESSER_SLOT, Slot::Greater);
            return;
        }
        if turn == lesser_turn && self.players[p].trinkets[LESSER_SLOT].of == 0 {
            self.deal_trinket_offer(p, LESSER_SLOT, Slot::Lesser);
            return;
        }
        if turn == greater_turn && self.players[p].trinkets[GREATER_SLOT].of == 0 {
            self.deal_trinket_offer(p, GREATER_SLOT, Slot::Greater);
            return;
        }
        // A power that becomes a trinket: one extra offer, the same rules,
        // one turn early for the slot it names.
        let Some(st) = self.players[p].power else { return };
        let power = &self.data.heroes.get(st.of).power;
        let PowerEffect::TrinketOffer(slot) = power.effect else { return };
        if turn < power.unlock_turn || self.players[p].trinkets[POWER_SLOT].of != 0 {
            return;
        }
        self.deal_trinket_offer(p, POWER_SLOT, slot);
    }

    /// Deal the four options: one guaranteed tribe trinket when a tribe of
    /// the board reaches the threshold, then three drawn by weight. Each
    /// option carries the price it is sold at.
    fn deal_trinket_offer(&mut self, p: usize, slot: usize, mut kind: Slot) {
        // "Your next Trinket is a Lesser Trinket" (Mysterious Orb).
        if self.players[p].tp.next_lesser {
            kind = Slot::Lesser;
        }
        // A field borrow, not an `Arc` clone: this runs twice per player per
        // game, but the shared `Data` is never refcounted in a game path.
        let data = &self.data;
        let rules = &data.rules;
        let need = match kind {
            Slot::Lesser => rules.trinket_lesser_need,
            Slot::Greater => rules.trinket_greater_need,
        } as u32;
        // A trinket is offered only when one of its tribes is in the lobby.
        let mut eligible: ArrayVec<u8, 256> = ArrayVec::new();
        for i in 0..data.trinkets.len() as u8 {
            let t = data.trinkets.get(i);
            if t.slot != kind || (!t.tribes.is_empty() && !t.tribes.overlaps(self.lobby)) {
                continue;
            }
            let _ = eligible.try_push(i);
        }
        if eligible.is_empty() {
            return;
        }
        let [counts, mine] = self.tribe_counts(p);
        // "A tribe you have" as a mask, once, instead of a scan per candidate.
        let mut mine_mask = Tribes::NONE;
        for (i, t) in Tribe::EVERY.into_iter().enumerate() {
            if mine[i] > 0 {
                mine_mask = mine_mask.with(t);
            }
        }
        let mut options: ArrayVec<u8, TRINKET_CHOICES> = ArrayVec::new();
        // The guarantee: one of the tribes that reaches the threshold, at
        // random when several do.
        let mut qualifying: ArrayVec<Tribe, { Tribe::EVERY.len() }> = ArrayVec::new();
        for (i, t) in Tribe::EVERY.into_iter().enumerate() {
            if counts[i] >= need {
                let _ = qualifying.try_push(t);
            }
        }
        if !qualifying.is_empty() {
            let t = qualifying[self.rng.below(qualifying.len() as u32) as usize];
            let mut of_tribe: ArrayVec<u8, 256> = ArrayVec::new();
            for &i in &eligible {
                if data.trinkets.get(i).tribes.has(t) {
                    let _ = of_tribe.try_push(i);
                }
            }
            if !of_tribe.is_empty() {
                let k = self.rng.below(of_tribe.len() as u32) as usize;
                let _ = options.try_push(of_tribe[k]);
            }
        }
        // The rest by weight. The draw order is the offer's own RNG path, so
        // it stays exactly as it was: candidate order and one draw per slot.
        let leans = self.mechanic_leans(p);
        while options.len() < TRINKET_CHOICES {
            let mut total = 0u32;
            for &i in &eligible {
                if options.contains(&i) {
                    continue;
                }
                total = total.saturating_add(Self::trinket_weight(
                    data.trinkets.get(i), mine_mask, leans, rules,
                ));
            }
            if total == 0 {
                break;
            }
            let mut k = self.rng.below(total) as i32;
            let mut chosen = None;
            for &i in &eligible {
                if options.contains(&i) {
                    continue;
                }
                let w = Self::trinket_weight(data.trinkets.get(i), mine_mask, leans, rules) as i32;
                if k < w {
                    chosen = Some(i);
                    break;
                }
                k -= w;
            }
            match chosen {
                Some(i) => {
                    let _ = options.try_push(i);
                }
                None => break,
            }
        }
        // Prices: a tribe trinket whose tribe falls short costs less; a
        // neutral one and one of a qualifying tribe keep their price.
        let pl = &mut self.players[p];
        pl.trinket_offer_slot = slot as u8 + 1;
        for i in options {
            let t = data.trinkets.get(i);
            let mut price = t.cost;
            if !t.tribes.is_empty() {
                let qualified = Tribe::EVERY
                    .into_iter()
                    .enumerate()
                    .any(|(k, tr)| t.tribes.has(tr) && counts[k] >= need);
                if !qualified {
                    price = price.saturating_sub(rules.trinket_discount);
                }
            }
            let _ = pl.trinket_offer.try_push((i, price));
        }
    }

    /// The draw weight of one option for the other three slots.
    fn trinket_weight(
        t: &crate::trinkets::Trinket,
        mine: Tribes,
        leans: [bool; 3],
        rules: &crate::rules::Rules,
    ) -> u32 {
        let own = t.tribes.overlaps(mine);
        let mut w = if t.tribes.is_empty() {
            rules.trinket_offer_neutral
        } else if own {
            rules.trinket_offer_own
        } else {
            rules.trinket_offer_other
        };
        let lean = match t.mechanic {
            Some(Mechanic::Battlecry) => leans[0],
            Some(Mechanic::Deathrattle) => leans[1],
            Some(Mechanic::Spells) => leans[2],
            None => false,
        };
        if lean {
            w = w.saturating_add(rules.trinket_offer_mechanic);
        }
        w
    }

    /// Which mechanics the board leans toward: a Battlecry or Choose One
    /// minion, a Deathrattle minion, or a spell cast this game.
    fn mechanic_leans(&self, p: usize) -> [bool; 3] {
        let cards = &self.data.cards;
        let mut leans = [false; 3];
        for m in &self.players[p].board {
            for (c, _) in m.parts() {
                let card = cards.get(c);
                leans[0] |= card.battlecry.is_some() || card.choose_one.is_some();
                leans[1] |= card.deathrattle.is_some();
            }
        }
        leans[2] = self.players[p].spells_cast > 0 || self.players[p].last_spell.is_some();
        leans
    }

    /// One of a trinket's effect lists, borrowed from the shared table.
    fn trinket_list(&self, i: u8, hook: Hook) -> &[Act] {
        let t = self.data.trinkets.get(i);
        match hook {
            Hook::OnPick => &t.on_pick,
            Hook::StartOfTurn => &t.start_of_turn,
            Hook::EndOfTurn => &t.end_of_turn,
            Hook::OnBuy => &t.on_buy,
            Hook::OnBuySpell => &t.on_buy_spell,
            Hook::OnPlay => &t.on_play,
            Hook::OnSell => &t.on_sell,
            Hook::OnRefresh => &t.on_refresh,
            Hook::OnCastSpell => &t.on_cast_spell,
            Hook::OnCastSpellOnMinion => &t.on_cast_spell_on_minion,
            Hook::OnDiscard => &t.on_discard,
            Hook::OnGoldSpent => &t.on_gold_spent,
            Hook::Rule => t.rule.as_ref().map_or(&[], |r| &r.acts),
        }
    }

    /// Put a trinket (`Some(index)` into `TrinketDb`, None to empty the slot)
    /// in one of the player's slots and bring its event mask up to date. The
    /// pick uses it; tests and the sandbox use it to hold a trinket without
    /// its pick effects.
    pub fn set_trinket(&mut self, p: usize, slot: usize, i: Option<u8>) {
        let state = match i {
            None => TrinketState::default(),
            Some(i) => {
                let t = self.data.trinkets.get(i);
                TrinketState {
                    of: i + 1,
                    count: if t.rule.as_ref().is_some_and(|r| r.event == Event::Turns) {
                        1
                    } else {
                        0
                    },
                    value: 0,
                    uses_left: t.uses_per_turn.unwrap_or(u8::MAX),
                    done: false,
                }
            }
        };
        self.players[p].trinkets[slot] = state;
        self.refresh_trinket_mask(p);
    }

    /// Recompute the player's trinket event mask from the slots it holds.
    fn refresh_trinket_mask(&mut self, p: usize) {
        let mut mask = 0;
        for slot in 0..TRINKET_SLOTS {
            let of = self.players[p].trinkets[slot].of;
            if of != 0 {
                mask |= self.data.trinkets.get(of - 1).event_mask();
            }
        }
        self.players[p].trinket_events = mask;
    }

    /// Run every act of one list: each `Act` is copied out of the shared
    /// table first, so no borrow of `Data` is held across the effect and
    /// nothing is allocated.
    fn run_trinket_hook(&mut self, p: usize, slot: usize, i: u8, hook: Hook) {
        for k in 0..self.trinket_list(i, hook).len() {
            let act = self.trinket_list(i, hook)[k];
            self.run_trinket_act(p, slot, act);
        }
    }

    /// The player answered the offer: pay the price, take the trinket (the
    /// pick turn counts as turn 1 for its "every N turns"), and run what it
    /// does on the pick. A power slot's pick consumes the hero power.
    pub(super) fn pick_trinket(&mut self, p: usize, k: usize) {
        let Some(&(of, price)) = self.players[p].trinket_offer.get(k) else { return };
        let slot = (self.players[p].trinket_offer_slot.saturating_sub(1) as usize)
            .min(TRINKET_SLOTS - 1);
        self.players[p].trinket_offer.clear();
        self.players[p].trinket_offer_slot = 0;
        self.set_trinket(p, slot, Some(of));
        self.players[p].gold = self.players[p].gold.saturating_sub(price);
        self.players[p].tp.next_lesser = false;
        self.fire_event(p, EventKind::GoldSpent, price as u32);
        self.run_trinket_hook(p, slot, of, Hook::OnPick);
    }

    /// The trinket hooks that fire at the start of the player's own turn: the
    /// new offers, the "every N turns" counters, the uses back, and the
    /// start-of-turn effects (which a trinket picked this turn never gets,
    /// because the offer is dealt here and picked later).
    pub(super) fn trinket_start_of_turn(&mut self, p: usize) {
        self.open_trinket_offers(p);
        if self.players[p].trinket_events & crate::trinkets::STARTS_TURN == 0 {
            return;
        }
        for slot in 0..TRINKET_SLOTS {
            let Some(i) = self.trinket_of(p, slot) else { continue };
            let (uses, ticks) = {
                let t = self.data.trinkets.get(i);
                (t.uses_per_turn.unwrap_or(u8::MAX), t.rule.as_ref().is_some_and(|r| r.event == Event::Turns))
            };
            let st = &mut self.players[p].trinkets[slot];
            st.uses_left = uses;
            if ticks && !st.done {
                st.count = st.count.saturating_add(1);
            }
        }
        for slot in 0..TRINKET_SLOTS {
            let Some(i) = self.trinket_of(p, slot) else { continue };
            self.run_trinket_hook(p, slot, i, Hook::StartOfTurn);
        }
    }

    /// The trinket hooks that fire at the end of the player's turn: its
    /// end-of-turn effects, then an "every N turns" rule that has counted
    /// up to N (the turn it reaches N is the turn it fires on).
    pub(super) fn trinket_end_of_turn(&mut self, p: usize) {
        if self.players[p].trinket_events & crate::trinkets::ENDS_TURN == 0 {
            return;
        }
        for slot in 0..TRINKET_SLOTS {
            let Some(i) = self.trinket_of(p, slot) else { continue };
            self.run_trinket_hook(p, slot, i, Hook::EndOfTurn);
        }
        for slot in 0..TRINKET_SLOTS {
            let Some(i) = self.trinket_of(p, slot) else { continue };
            let every = {
                let t = self.data.trinkets.get(i);
                match &t.rule {
                    Some(r) if r.event == Event::Turns => Some(r),
                    _ => None,
                }
                .map(|r| (r.every, r.once))
            };
            let Some((every, once)) = every else { continue };
            let st = &mut self.players[p].trinkets[slot];
            if st.done || st.count < every as u16 {
                continue;
            }
            st.count = 0;
            if once {
                st.done = true;
            }
            self.run_trinket_hook(p, slot, i, Hook::Rule);
        }
    }

    /// An event a trinket listens for. `card` is the minion, spell or card
    /// the event is about (for the tribe and "a spell" filters); `amount`
    /// counts for events that count a number (Gold spent).
    pub(super) fn trinket_event(&mut self, p: usize, event: Event, card: Option<CardId>, amount: u32) {
        if self.players[p].tp.golden_when_rich != 0 {
            self.check_golden_when_rich(p);
        }
        // The hot path: one `u16` on the player says whether any of its
        // trinkets listens for this event. Most of a game, and every game
        // before turn 6, has nothing here.
        if self.players[p].trinket_events & event.bit() == 0 {
            return;
        }
        let card_tribes = card.map(|c| self.data.cards.get(c).tribes);
        let card_is_spell = card.is_some_and(|c| self.data.cards.get(c).is_spell());
        for slot in 0..TRINKET_SLOTS {
            let Some(i) = self.trinket_of(p, slot) else { continue };
            // Which list this event fires, and the rule it counts toward.
            let (hook, capped, uses_left, rule) = {
                let t = self.data.trinkets.get(i);
                let by_tribe = |want: Option<Tribe>| match want {
                    None => true,
                    Some(w) => card_tribes.is_some_and(|ts| ts.has(w)),
                };
                let hook = match event {
                    Event::Buy if by_tribe(t.buy_tribe) => Some(Hook::OnBuy),
                    Event::BuySpell => Some(Hook::OnBuySpell),
                    Event::Play if by_tribe(t.play_tribe) => Some(Hook::OnPlay),
                    Event::Sell => Some(Hook::OnSell),
                    Event::Refresh => Some(Hook::OnRefresh),
                    Event::CastSpell => Some(Hook::OnCastSpell),
                    Event::CastSpellOnMinion => Some(Hook::OnCastSpellOnMinion),
                    Event::Discard if !t.discard_spells_only || card_is_spell => Some(Hook::OnDiscard),
                    Event::GoldSpent => Some(Hook::OnGoldSpent),
                    _ => None,
                };
                let rule = t.rule.as_ref().map(|r| (r.event, r.every, r.once));
                let uses_left = self.players[p].trinkets[slot].uses_left;
                (hook, t.uses_per_turn.is_some(), uses_left, rule)
            };
            if let Some(hook) = hook {
                // An empty list is no hook: it must not spend a capped use.
                let len = self.trinket_list(i, hook).len();
                if len > 0 && (!capped || uses_left > 0) {
                    if capped {
                        self.players[p].trinkets[slot].uses_left = uses_left - 1;
                    }
                    self.run_trinket_hook(p, slot, i, hook);
                }
            }
            // The counter: "every N turns" counts turns only, so any other
            // event that lands on it is a rule of its own kind.
            let Some((rule_event, every, once)) = rule else { continue };
            if rule_event == Event::Turns {
                continue;
            }
            let hit = match rule_event {
                Event::PlayTribe(w) => event == Event::Play && card_tribes.is_some_and(|ts| ts.has(w)),
                other => other == event,
            };
            if !hit {
                continue;
            }
            let st = &mut self.players[p].trinkets[slot];
            if st.done {
                continue;
            }
            st.count = st.count.saturating_add(amount.min(u16::MAX as u32) as u16);
            if st.count < every as u16 {
                continue;
            }
            st.count = 0;
            if once {
                st.done = true;
            }
            self.run_trinket_hook(p, slot, i, Hook::Rule);
        }
    }

    /// Run one trinket effect. Every act with a card behind it goes through
    /// the same `Effect`/`SpellEffect` arm the card path runs, with no host
    /// slot (`pos` 0) and no golden multiplier; the four acts with no card
    /// behind them are done here.
    fn run_trinket_act(&mut self, p: usize, slot: usize, act: Act) {
        let mut pos = 0usize;
        match act {
            Act::Get { card, count } => {
                self.run_effect(p, &mut pos, Effect::GetCard { card, count }, false, 1)
            }
            Act::Gold(n) => self.execute_spell_effect(p, SpellEffect::GainGold(n), false),
            Act::MaxGold(n) => self.execute_spell_effect(p, SpellEffect::MaxGold(n), false),
            Act::UpgradeDiscount(n) => {
                let pl = &mut self.players[p];
                pl.upgrade_cost = pl.upgrade_cost.saturating_sub(n);
            }
            Act::Gems(n) => self.run_effect(p, &mut pos, Effect::GetGems(n), false, 1),
            Act::GemBonus { attack, health } => {
                self.run_effect(p, &mut pos, Effect::GemBonus { attack, health }, false, 1)
            }
            Act::SpellBonus { attack, health } => {
                self.run_effect(p, &mut pos, Effect::SpellBonus { attack, health }, false, 1)
            }
            Act::DiscoverSpell(n) => {
                for _ in 0..n {
                    let se = SpellEffect::Discover(DiscoverKind::MyTierSpell);
                    self.execute_spell_effect(p, se, false);
                }
                // The queue is started by the cast path; a trinket is not a
                // cast, so start it here.
                self.maybe_start_discover(p);
            }
            Act::RandomChromadrake(n) => {
                for _ in 0..n {
                    self.run_effect(p, &mut pos, Effect::RandomChromadrake, false, 1);
                }
            }
            Act::RandomVolumizer(n) => {
                for _ in 0..n {
                    self.run_effect(p, &mut pos, Effect::RandomVolumizer, false, 1);
                }
            }
            Act::RandomBounty(n) => {
                for _ in 0..n {
                    self.run_effect(p, &mut pos, Effect::GetRandomBounty, false, 1);
                }
            }
            Act::RandomChooseOne(n) => {
                self.run_effect(p, &mut pos, Effect::GetRandomChooseOne(n), false, 1)
            }
            Act::RandomTierMinion { tier, count } => {
                for _ in 0..count {
                    self.run_effect(p, &mut pos, Effect::GetRandomTierMinion { tier }, false, 1);
                }
            }
            Act::RandomMinion { tribe, count } => {
                self.run_effect(p, &mut pos, Effect::GetRandomMinion { tribe }, false, count)
            }
            Act::RandomSpell(n) => self.run_effect(
                p,
                &mut pos,
                Effect::GetRandomSpellUpTo { cost: None, count: n },
                false,
                1,
            ),
            Act::Cast { card, times } => {
                self.run_effect(p, &mut pos, Effect::CastSpell { card, times }, false, 1)
            }
            Act::TavernBuff { tribe, attack, health, this_turn } => self.run_effect(
                p,
                &mut pos,
                Effect::TavernBuff { tribe, attack, health, this_turn },
                false,
                1,
            ),
            // The one act with no card behind it: see `Act::BuffTavernThisTurn`.
            Act::BuffTavernThisTurn { attack, health } => {
                let cards = &self.data.cards;
                for m in &mut self.players[p].shop {
                    if !cards.get(m.card).is_spell() && !cards.get(m.card).fixed_stats {
                        buff(m, attack, health);
                    }
                }
            }
            Act::BuffLeftmostHand { attack, health } => self.run_effect(
                p,
                &mut pos,
                Effect::BuffLeftmostHand { attack, health },
                false,
                1,
            ),
            Act::BuffAll { tribe, attack, health } => self.run_effect(
                p,
                &mut pos,
                Effect::BuffAll { tribe, attack, health },
                false,
                1,
            ),
            Act::UndeadBonus { attack, health } => self.run_effect(
                p,
                &mut pos,
                Effect::UndeadBonus { attack, health, outside_attack: 0 },
                false,
                1,
            ),
            Act::BuffDeity { attack, health } => self.run_effect(
                p,
                &mut pos,
                Effect::BuffDeity { attack, health, per_spell: (0, 0) },
                false,
                1,
            ),
            Act::FreeRefreshes(n) => {
                self.run_effect(p, &mut pos, Effect::FreeRefreshes(n), false, 1)
            }
            Act::DemonsConsume(n) => {
                self.run_effect(p, &mut pos, Effect::DemonsConsume { count: n }, false, 1)
            }
            Act::AddFodderRefreshes(n) => {
                self.run_effect(p, &mut pos, Effect::AddFodderToNextRefresh(n), false, 1)
            }
            Act::GetLockbox(sooner) => {
                self.run_effect(p, &mut pos, Effect::GetLockbox { sooner }, false, 1)
            }
            Act::LinkedSpells(n) => {
                self.run_effect(p, &mut pos, Effect::GetLinkedSpells { count: n }, false, 1)
            }
            Act::MakeGoldenRandom(max_tier) => {
                let cards = &self.data.cards;
                let ok: ArrayVec<usize, { crate::BOARD }> = self.players[p]
                    .board
                    .iter()
                    .enumerate()
                    .filter(|(_, m)| !m.golden && cards.get(m.card).tier <= max_tier)
                    .map(|(i, _)| i)
                    .collect();
                if ok.is_empty() {
                    return;
                }
                let k = self.rng.below(ok.len() as u32) as usize;
                let at = ok[k];
                let mut m = self.players[p].board[at];
                super::shop::golden_up(&self.data.cards, &mut m);
                self.players[p].board[at] = m;
            }
            Act::LastSpellCopies(n) => {
                let Some(spell) = self.players[p].last_spell else { return };
                for _ in 0..n {
                    self.give_generated(p, spell);
                }
            }
            Act::GemsOnBoard(n) => {
                self.execute_spell_effect(p, SpellEffect::GemsOnBoard(n), false)
            }
            Act::HealthPriceMinions { tribe, count } => {
                let tp = &mut self.players[p].tp;
                tp.health_price_tribe = tribe;
                tp.health_price_left = count;
            }
            Act::HealthPriceHighestTier => self.players[p].tp.highest_tier_health = true,
            Act::EveryNthBuyHealth(n) => self.players[p].tp.fourth_buy_every = n,
            Act::HealthPriceSpell(n) => self.players[p].tp.spell_health_left = n,
            Act::FreeBuys(n) => {
                let tp = &mut self.players[p].tp;
                tp.free_buys = tp.free_buys.saturating_add(n);
            }
            Act::FreeTribeBuys { tribe, count } => {
                let tp = &mut self.players[p].tp;
                tp.free_tribe = Some(tribe);
                tp.free_tribe_left = count;
            }
            Act::GoldOnTier6(n) => {
                let tp = &mut self.players[p].tp;
                tp.gold_on_tier6 = tp.gold_on_tier6.saturating_add(n);
            }
            Act::GoldPerType(n) => {
                let mut seen = crate::cards::Tribes::NONE;
                let cards = &self.data.cards;
                let mut types = 0u8;
                for m in &self.players[p].board {
                    let ts = cards.get(m.card).tribes;
                    if ts.is_empty() || seen.overlaps(ts) {
                        continue;
                    }
                    seen = seen.with_all(ts);
                    types = types.saturating_add(1);
                }
                self.players[p].gold = self.players[p].gold.saturating_add(n.saturating_mul(types));
            }
            Act::GoldPerGolden(n) => {
                let goldens = self.players[p].board.iter().filter(|m| m.golden).count() as u8;
                self.players[p].gold = self.players[p].gold.saturating_add(n.saturating_mul(goldens));
            }
            Act::GoldenWhenRich(tier) => {
                // "When you have 15 Gold": a standing flag, checked wherever
                // the Gold can reach it, and paid once.
                self.players[p].tp.golden_when_rich = tier;
                if self.players[p].trinkets[slot].done || self.players[p].gold < 15 {
                    return;
                }
                self.players[p].trinkets[slot].done = true;
                self.run_effect(p, &mut pos, Effect::GetGoldenMinion { tier, count: 1 }, false, 1);
                self.players[p].tp.golden_when_rich = 0;
            }
            Act::NextTrinketLesser => self.players[p].tp.next_lesser = true,
            Act::GreaterOfferOnTurn(turn) => self.players[p].tp.greater_offer_turn = turn,
            Act::GreaterOfferIntoLesser(n) => {
                self.players[p].tp.greater_into_lesser_turn = self.turn + n as u32;
            }
            Act::ReplaceWithRandom { slot: kind, tribe } => {
                self.replace_trinket_at(p, slot, kind, tribe);
            }
            Act::BuffRandomOther { tribe, attack, health, count } => {
                // `Effect::BuffRandomOther` is a fight effect; in the shop it
                // is this: up to `count` different friendly minions of the
                // tribe, at random (Booty Bay Brew).
                let cards = &self.data.cards;
                let mut picks: ArrayVec<usize, { crate::BOARD }> = self.players[p]
                    .board
                    .iter()
                    .enumerate()
                    .filter(|(_, m)| cards.get(m.card).is(tribe) && !cards.get(m.card).fixed_stats)
                    .map(|(i, _)| i)
                    .collect();
                for _ in 0..count {
                    if picks.is_empty() {
                        break;
                    }
                    let k = self.rng.below(picks.len() as u32) as usize;
                    let at = picks.swap_remove(k);
                    buff(&mut self.players[p].board[at], attack, health);
                }
            }
            Act::OfferReplace(n) => {
                // A free offer of this slot, to replace this trinket with
                // (Mystery Cube). The pick overwrites the slot.
                if !self.players[p].trinket_offer.is_empty() {
                    return;
                }
                let kind = self.data.trinkets.get(self.players[p].trinkets[slot].of - 1).slot;
                self.deal_trinket_offer(p, slot, kind);
                // Only the first `n` options, all free.
                while self.players[p].trinket_offer.len() > n as usize {
                    self.players[p].trinket_offer.pop();
                }
                for o in self.players[p].trinket_offer.iter_mut() {
                    o.1 = 0;
                }
            }
            Act::GrowValue(n) => {
                let v = &mut self.players[p].trinkets[slot].value;
                *v = v.saturating_add(n);
            }
            Act::BuffAllGrowing { tribe, attack, health } => {
                let v = self.players[p].trinkets[slot].value as i32;
                let (a, h) = (attack + v, health + v);
                self.run_effect(p, &mut pos, Effect::BuffAll { tribe, attack: a, health: h }, false, 1);
            }
            Act::BuffLeftmostGrowing { attack, health } => {
                let v = self.players[p].trinkets[slot].value as i32;
                let e = SpellEffect::BuffLeftmost { attack: attack + v, health: health + v };
                self.execute_spell_effect(p, e, false);
            }
            Act::BuffAllPerGolden { attack, health } => {
                let n = self.players[p].golden_played as i32;
                let e = Effect::BuffAll { tribe: None, attack: attack * n, health: health * n };
                self.run_effect(p, &mut pos, e, false, 1);
            }
            Act::BuffLeftmostTwoPerBattlecry { attack, health } => {
                let n = self.players[p].tp.battlecries as i32;
                let e = SpellEffect::BuffLeftmost { attack: attack * n, health: health * n };
                for _ in 0..2 {
                    self.execute_spell_effect(p, e, false);
                }
            }
            Act::RandomMostCommonType(n) => {
                let mut counts = [0u8; Tribe::EVERY.len()];
                let cards = &self.data.cards;
                for m in &self.players[p].board {
                    let ts = cards.get(m.card).tribes;
                    for (i, t) in Tribe::EVERY.into_iter().enumerate() {
                        if ts.has(t) {
                            counts[i] = counts[i].saturating_add(1);
                        }
                    }
                }
                let Some(best) = counts.iter().max().copied().filter(|&c| c > 0) else { return };
                let tied: ArrayVec<Tribe, { Tribe::EVERY.len() }> = Tribe::EVERY
                    .into_iter()
                    .enumerate()
                    .filter(|&(i, _)| counts[i] == best)
                    .map(|(_, t)| t)
                    .collect();
                let t = tied[self.rng.below(tied.len() as u32) as usize];
                for _ in 0..n {
                    self.run_effect(p, &mut pos, Effect::GetRandomMinion { tribe: t }, false, 1);
                }
            }
        }
    }

    /// What a minion in Tavern slot `s` costs this player right now: Gold
    /// (0 when a trinket makes it free) or Health (Pilgrimp Sticker, Demonic
    /// Tapestry, The Eye of Sargeras). DESIGN.md: a Health price is self
    /// damage, not Gold spent.
    pub(super) fn minion_price(&self, p: usize, slot: usize) -> MinionPrice {
        let pl = &self.players[p];
        let Some(m) = pl.shop.get(slot) else { return MinionPrice::Gold(self.data.rules.buy_cost) };
        let card = self.data.cards.get(m.card);
        let tp = pl.tp;
        if tp.free_buys > 0 || (tp.free_tribe_left > 0 && tp.free_tribe.is_some_and(|t| card.tribes.has(t)))
        {
            return MinionPrice::Gold(0);
        }
        let by_tribe = tp.health_price_left > 0
            && tp.health_price_tribe.is_none_or(|t| card.tribes.has(t));
        let highest = tp.highest_tier_health && self.is_highest_tier_slot(p, slot);
        let every_nth = tp.fourth_buy_every > 0
            && (tp.fourth_buy + 1).is_multiple_of(tp.fourth_buy_every);
        if by_tribe || highest || every_nth {
            MinionPrice::Health(self.data.rules.buy_cost)
        } else {
            MinionPrice::Gold(self.data.rules.buy_cost)
        }
    }

    /// Whether this Tavern slot holds one of the highest-Tier minions on
    /// offer (Demonic Tapestry).
    fn is_highest_tier_slot(&self, p: usize, slot: usize) -> bool {
        let cards = &self.data.cards;
        let tier = self.players[p]
            .shop
            .get(slot)
            .map(|m| cards.get(m.card).tier)
            .unwrap_or(0);
        tier > 0
            && tier
                == self.players[p]
                    .shop
                    .iter()
                    .map(|m| cards.get(m.card).tier)
                    .max()
                    .unwrap_or(0)
    }

    /// Spend the trinket prices a minion buy used up: a free buy's use, a
    /// Health price's use, and the every-Nth-buy counter.
    pub(super) fn note_minion_bought(&mut self, p: usize, price: MinionPrice) {
        let tp = &mut self.players[p].tp;
        match price {
            MinionPrice::Gold(0) => {
                if tp.free_buys > 0 {
                    tp.free_buys -= 1;
                } else if tp.free_tribe_left > 0 {
                    tp.free_tribe_left -= 1;
                }
            }
            MinionPrice::Health(_) => {
                if tp.health_price_left > 0 {
                    tp.health_price_left -= 1;
                }
            }
            MinionPrice::Gold(_) => {}
        }
        if tp.fourth_buy_every > 0 {
            let n = tp.fourth_buy.saturating_add(1);
            tp.fourth_buy = if n >= tp.fourth_buy_every { 0 } else { n };
        }
    }

    /// "When you have 15 Gold, get a random Golden Tier 5 minion"
    /// (Splinter of Aurum): checked wherever the player's Gold can reach it.
    pub fn check_golden_when_rich(&mut self, p: usize) {
        let tier = self.players[p].tp.golden_when_rich;
        if tier == 0 || self.players[p].gold < 15 {
            return;
        }
        for slot in 0..TRINKET_SLOTS {
            let of = self.players[p].trinkets[slot].of;
            if of == 0 || self.players[p].trinkets[slot].done {
                continue;
            }
            if self.data.trinkets.get(of - 1).golden_when_rich() == Some(tier) {
                self.players[p].trinkets[slot].done = true;
                let mut pos = 0;
                self.run_effect(p, &mut pos, Effect::GetGoldenMinion { tier, count: 1 }, false, 1);
            }
        }
    }

    /// Replace the trinket in `slot` with a random one of `kind`, its tribes
    /// in the lobby (Orb of the Unknown, Quartermaster's Hook): the new one's
    /// own on-pick does not run, it is placed, not picked.
    fn replace_trinket_at(&mut self, p: usize, slot: usize, kind: Slot, tribe: Option<Tribe>) {
        let data = self.data.clone();
        let mut pool: ArrayVec<u8, 256> = ArrayVec::new();
        for i in 0..data.trinkets.len() as u8 {
            let t = data.trinkets.get(i);
            if t.slot != kind || (!t.tribes.is_empty() && !t.tribes.overlaps(self.lobby)) {
                continue;
            }
            if let Some(w) = tribe
                && !t.tribes.has(w)
            {
                continue;
            }
            let _ = pool.try_push(i);
        }
        if pool.is_empty() {
            return;
        }
        let k = self.rng.below(pool.len() as u32) as usize;
        let of = pool[k];
        self.set_trinket(p, slot, Some(of));
    }
}
