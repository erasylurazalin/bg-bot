//! Trinkets in a game: the two offers, the pick, the slots they live in and
//! the hooks they fire. See docs/rules/design.md "Trinkets and Dark Gifts (2026-10-30)"
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
use crate::minion::{CardId, Minion};
use crate::trinkets::{
    Act,
    CardMod,
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

/// What a deal of options is for: one of the two standard offers (or the one
/// a hero power owes), or the Mystery Cube's free replacement of the trinket
/// its slot holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Offer {
    Standalone,
    /// `choices` options, all free and all Lesser, never the trinket the
    /// slot holds and never a one-time effect (docs/rules/design.md).
    Cube { exclude: u8, choices: u8 },
}

impl Offer {
    fn choices(self) -> usize {
        match self {
            Offer::Standalone => TRINKET_CHOICES,
            Offer::Cube { choices, .. } => (choices as usize).min(TRINKET_CHOICES),
        }
    }
}

/// What a Tavern minion costs this player right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MinionPrice {
    Gold(u8),
    /// Paid in Health: self damage, not Gold spent (docs/rules/design.md).
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
    OnAnySpell,
    OnCastSpellOnMinion,
    OnDiscard,
    OnGet,
    OnGoldSpent,
    OnCardPlay,
    OnBattlecry,
    OnGem,
    OnAttack,
    OnDestroy,
    OnMagnetize,
    OnCastSpellOnShop,
    OnConsume,
    OnHeroDamage,
    Rule,
}

impl Game {
    /// Hammer of Twilight: the minion that just landed gets the base plus
    /// one step for every card the player has discarded this game.
    pub(super) fn hammer_on_play(&mut self, p: usize, pos: usize) {
        let (ba, bh) = self.players[p].tp.hammer_base;
        let (pa, ph) = self.players[p].tp.hammer_per;
        if (ba, bh) == (0, 0) && (pa, ph) == (0, 0) {
            return;
        }
        let n = self.players[p].discarded as i32;
        if let Some(m) = self.players[p].board.get_mut(pos) {
            buff(m, ba + pa * n, bh + ph * n);
        }
    }

    /// The extra triggers War Drum gives one Battlecry, if its once-a-turn
    /// use is still there (the trinket's own `uses_per_turn`).
    pub(super) fn take_war_drum(&mut self, p: usize) -> u8 {
        let extra = self.players[p].tp.war_drum;
        if extra == 0 {
            return 0;
        }
        for slot in 0..TRINKET_SLOTS {
            let Some(i) = self.trinket_of(p, slot) else { continue };
            let has = self
                .data
                .trinkets
                .get(i)
                .on_pick
                .iter()
                .any(|a| matches!(a, Act::WarDrum(_)));
            if has && self.players[p].trinkets[slot].uses_left > 0 {
                self.players[p].trinkets[slot].uses_left -= 1;
                return extra;
            }
        }
        0
    }

    /// The rewrite a portrait gives this card, if the player holds one
    /// (docs/rules/trinkets.md "Portraits that rewrite a card").
    pub fn player_card_mod(&self, p: usize, card: CardId) -> Option<CardMod> {
        self.players[p]
            .tp
            .card_mods
            .iter()
            .flatten()
            .find(|(c, _)| *c == card)
            .map(|(_, k)| *k)
    }

    /// The trinket in this slot, as an index into `TrinketDb`, if any.
    pub fn trinket_of(&self, p: usize, slot: usize) -> Option<u8> {
        let of = self.players[p].trinkets[slot].of;
        // `then_some` would build `of - 1` even when `of` is 0.
        if of == 0 { None } else { Some(of - 1) }
    }

    /// The player's most common minion type now: the tribe a plurality of the
    /// board's minions have, a tie broken at random, and the all-type minions
    /// count for every tribe. `crate::darkgifts::most_common_type` is the one
    /// read of it (Dark Discovery's "your type" slot and the compasses' lock
    /// at the pick), so an empty or all-neutral board gives None.
    fn board_most_common_type(&mut self, p: usize) -> Option<Tribe> {
        let cards = &self.data.cards;
        let tribes: ArrayVec<Tribes, { crate::BOARD }> =
            self.players[p].board.iter().map(|m| cards.tribes_of(m)).collect();
        crate::darkgifts::most_common_type(&tribes, &mut self.rng)
    }

    /// The tribes the player has minions of, for the guarantee and the
    /// price: `[board counts, board plus hand counts]`. docs/rules/design.md counts the
    /// board only, except Murlocs, which count in hand too; the wider one is
    /// what the offer weights call "your tribes".
    fn tribe_counts(&self, p: usize) -> [[u32; Tribe::EVERY.len()]; 2] {
        let mut out = [[0u32; Tribe::EVERY.len()]; 2];
        let cards = &self.data.cards;
        for m in &self.players[p].board {
            let ts = cards.tribes_of(m);
            for (i, t) in Tribe::EVERY.into_iter().enumerate() {
                if ts.has(t) {
                    out[0][i] += 1;
                    out[1][i] += 1;
                }
            }
        }
        for m in &self.players[p].hand {
            let ts = cards.tribes_of(m);
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
            self.deal_trinket_offer(p, LESSER_SLOT, Slot::Greater, Offer::Standalone);
            return;
        }
        if turn == lesser_turn && self.players[p].trinkets[LESSER_SLOT].of == 0 {
            self.deal_trinket_offer(p, LESSER_SLOT, Slot::Lesser, Offer::Standalone);
            return;
        }
        if turn == greater_turn && self.players[p].trinkets[GREATER_SLOT].of == 0 {
            self.deal_trinket_offer(p, GREATER_SLOT, Slot::Greater, Offer::Standalone);
            return;
        }
        // A power that becomes a trinket: one extra offer, the same rules,
        // one turn early for the slot it names.
        let Some(st) = self.players[p].power else { return };
        let power = self.data.heroes.power(st.of);
        let PowerEffect::TrinketOffer(slot) = power.effect else { return };
        if turn < power.unlock_turn || self.players[p].trinkets[POWER_SLOT].of != 0 {
            return;
        }
        self.deal_trinket_offer(p, POWER_SLOT, slot, Offer::Standalone);
    }

    /// Deal the four options: one guaranteed tribe trinket when a tribe of
    /// the board reaches the threshold, then three drawn by weight. Each
    /// option carries the price it is sold at. A Mystery Cube offer is two
    /// free Lesser options instead, with neither the guarantee nor a price.
    fn deal_trinket_offer(&mut self, p: usize, slot: usize, mut kind: Slot, offer: Offer) {
        // "Your next Trinket is a Lesser Trinket" (Mysterious Orb).
        if self.players[p].tp.next_lesser {
            kind = Slot::Lesser;
        }
        if let Offer::Cube { .. } = offer {
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
            // The cube can't offer what the slot already holds (you can't
            // keep the current one) and never offers a one-time effect.
            if let Offer::Cube { exclude, .. } = offer
                && (i == exclude || !t.cube)
            {
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
        if !qualifying.is_empty() && offer == Offer::Standalone {
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
        while options.len() < offer.choices() {
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
            if offer != Offer::Standalone {
                price = 0;
            } else if !t.tribes.is_empty() {
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
            Hook::OnAnySpell => &t.on_any_spell,
            Hook::OnCastSpellOnMinion => &t.on_cast_spell_on_minion,
            Hook::OnDiscard => &t.on_discard,
            Hook::OnGet => &t.on_get,
            Hook::OnGoldSpent => &t.on_gold_spent,
            Hook::OnCardPlay => &t.on_card_play,
            Hook::OnBattlecry => &t.on_battlecry,
            Hook::OnGem => &t.on_gem,
            Hook::OnAttack => &t.on_attack,
            Hook::OnDestroy => &t.on_destroy,
            Hook::OnMagnetize => &t.on_magnetize,
            Hook::OnCastSpellOnShop => &t.on_cast_spell_on_shop,
            Hook::OnConsume => &t.on_consume,
            Hook::OnHeroDamage => &t.on_hero_damage,
            Hook::Rule => t.rule.as_ref().map_or(&[], |r| &r.acts),
        }
    }

    /// Put a trinket (`Some(index)` into `TrinketDb`, None to empty the slot)
    /// in one of the player's slots and bring its event mask up to date. The
    /// pick uses it; tests and the sandbox use it to hold a trinket without
    /// its pick effects.
    pub fn set_trinket(&mut self, p: usize, slot: usize, i: Option<u8>) {
        // A Mystery Cube's effect stays with the slot: putting another trinket
        // in it keeps the flag, emptying the slot drops it.
        let cube = i.is_some() && self.players[p].trinkets[slot].cube;
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
                    card: None,
                    tribe: None,
                    cube,
                    cube_choices: if cube {
                        self.players[p].trinkets[slot].cube_choices
                    } else {
                        0
                    },
                }
            }
        };
        self.players[p].trinkets[slot] = state;
        self.refresh_trinket_mask(p);
    }

    /// Recompute the player's trinket event mask from the slots it holds. A
    /// cube slot always wants the start of the turn, whatever trinket it
    /// holds (its own turn-start effect runs, then the new offer).
    fn refresh_trinket_mask(&mut self, p: usize) {
        let mut mask = 0;
        let mut cube = false;
        for slot in 0..TRINKET_SLOTS {
            let st = self.players[p].trinkets[slot];
            cube |= st.cube;
            if st.of != 0 {
                mask |= self.data.trinkets.get(st.of - 1).event_mask();
            }
        }
        if cube {
            mask |= crate::trinkets::STARTS_TURN | crate::trinkets::HAS_TRINKET;
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
        // Souvenir Stand: "When you buy a Greater Trinket, this transforms
        // into a copy of it."
        let souvenir = self.players[p].tp.souvenir;
        if souvenir != 0
            && self.data.trinkets.get(of).slot == Slot::Greater
            && souvenir as usize - 1 != slot
        {
            self.set_trinket(p, souvenir as usize - 1, Some(of));
        }
        self.run_trinket_hook(p, slot, of, Hook::OnPick);
        // A cube slot's turn-start offer had to wait for this one: deal it
        // now, so the cube never loses a turn to the turn 9 offer.
        let pending = self.players[p].tp.cube_pending;
        if pending != 0 && self.players[p].trinket_offer.is_empty() {
            self.players[p].tp.cube_pending = 0;
            self.deal_cube_offer(p, (pending as usize - 1).min(TRINKET_SLOTS - 1));
        }
    }

    /// The rules a pick leaves standing, for a trinket the loader seated
    /// instead of picked: the shop replay and a real board carry a trinket a
    /// real game had won, and its pick never ran (`Player::hold_trinket`).
    ///
    /// `hold_trinket` is a `Player` method with no `Game` in hand, so it sets
    /// only what a fight reads (the power's extra triggers, the Hammer's
    /// numbers). Everything else the pick leaves behind is set here, one act
    /// at a time through `run_trinket_act`, the code the pick itself runs, so
    /// the seat and the pick cannot drift apart.
    pub fn seat_trinket_pick_state(&mut self, p: usize) {
        for slot in 0..TRINKET_SLOTS {
            let Some(i) = self.trinket_of(p, slot) else { continue };
            for k in 0..self.trinket_list(i, Hook::OnPick).len() {
                let act = self.trinket_list(i, Hook::OnPick)[k];
                self.seat_trinket_act(p, slot, act);
            }
        }
    }

    /// One `on_pick` act of a seated trinket, and the split it sits on: the
    /// rules a logged state cannot carry run (the first group, through the
    /// pick's own code), and the things the state already holds do not (the
    /// last group: the board, hand, Gold, counters and Secret zone carry
    /// them, so running them again would double them). A few acts are both,
    /// and only their standing half is set.
    fn seat_trinket_act(&mut self, p: usize, slot: usize, act: Act) {
        match act {
            // ---- a rule that outlasts the step: the pick's own code ----
            // The Gold cap, a price rule ("every 4th buy costs Health"), a
            // payout or an offer still to come, and queued Fodders.
            Act::MaxGold(_)
            | Act::HealthPriceHighestTier
            | Act::EveryNthBuyHealth(_)
            | Act::GoldOnTier6(_)
            | Act::NextTrinketLesser
            | Act::GreaterOfferOnTurn(_)
            | Act::LockboxSooner(_)
            | Act::AddFodderRefreshes(_)
            // The Tavern's own rules (Glowing Gauntlet, Innkeeper's Stein,
            // Electrode Attractor, Bob-blehead).
            | Act::Gauntlet
            | Act::ExtraHighTier
            | Act::ExtraMagnetic
            | Act::MagneticCost(_)
            | Act::MinTier(_)
            // An event rule or a standing bonus, none of it in the counters.
            | Act::EndOfTurnTwice
            | Act::WarDrum(_)
            | Act::DragonBattlecry(_)
            | Act::Locket
            | Act::ConsumeClaw { .. }
            | Act::SpellBonusPerType { .. }
            | Act::SpellBonusThisTurn { .. }
            // A portrait's rewrite of every copy of a card (also written onto
            // the copies already on the board and in the hand).
            | Act::CardMod { .. }
            // A standing choice, the Eyepatch's triple rule, the Souvenir,
            // the Compass's next Golden.
            | Act::ChooseOneAll
            | Act::TripleNeedTwoPirate
            | Act::Souvenir
            | Act::GoldenBoughtType
            // The hero power's rule. `hold_trinket` sets the triggers too (a
            // fight reads them), to the same value.
            | Act::PowerTriggers(_)
            | Act::PowerGold(_) => self.run_trinket_act(p, slot, act),
            // ---- both, and only the standing half here ----
            // The flag stays; the Golden minion it pays out and the card in
            // hand are the file's.
            Act::GoldenWhenRich(tier) => self.players[p].tp.golden_when_rich = tier,
            Act::TriplePrize(card) => self.players[p].tp.triple_prize = Some(card),
            // The extra press stays; the presses left this turn are the
            // file's.
            Act::PowerExtraUses(n) => {
                let tp = &mut self.players[p].tp;
                tp.power_extra_uses = tp.power_extra_uses.saturating_add(n);
            }
            // The Whistle stays; the free Refresh it adds is the file's.
            Act::WarbandWhistle => self.players[p].tp.whistle = true,
            // `hold_trinket` set the numbers a fight reads, and the logged
            // board carries the buff the pick gives it.
            Act::Hammer { .. } => {}
            // ---- the logged state already carries these ----
            // A card, a Discover, or a trigger that hands one out.
            Act::Get { .. }
            | Act::GetLockbox(_)
            | Act::LinkedSpells(_)
            | Act::GetRemembered(_)
            | Act::GetGolden { .. }
            | Act::GetWithStats { .. }
            | Act::SatelliteOfBought(_)
            | Act::LastSpellCopies(_)
            | Act::PlainCopyRandomFriendly(_)
            | Act::CopyLastDiscarded
            | Act::RandomBounty(_)
            | Act::RandomBattlecry(_)
            | Act::RandomTierMinion { .. }
            | Act::RandomMinion { .. }
            | Act::RandomSpell(_)
            | Act::RandomChromadrake(_)
            | Act::RandomVolumizer(_)
            | Act::RandomChooseOne(_)
            | Act::RandomMostCommonType(_)
            | Act::RandomMagnetic(_)
            | Act::RandomPrize(_)
            | Act::MechaDemon(_)
            | Act::OneOf { .. }
            | Act::OneOf3 { .. }
            | Act::MakeGoldenRandom(_)
            | Act::DiscoverSpell(_)
            | Act::DiscoverDeathrattle(_)
            | Act::DiscoverTier(_)
            | Act::DiscoverRemember { .. }
            | Act::DiscoverTierStats { .. }
            | Act::DiscoverPrize(_)
            | Act::DarkGift { .. }
            | Act::DarkGiftCopies
            | Act::Cast { .. }
            | Act::CastOnLeftmost { .. }
            | Act::CastOnRandom { .. }
            | Act::TriggerBattlecriesEnds
            | Act::TriggerBattlecriesOf(_)
            | Act::TriggerAllRallies
            | Act::TriggerDeathrattlesLeftmost(_)
            | Act::TeachBoughtSpell(_)
            | Act::MagnetizeEnds { .. }
            | Act::GemsEachType(_)
            | Act::SecondPower
            // A die or a result the log rolled its own way: the wheel, the
            // Surprise's 5%, a random Tavern or a random spell.
            | Act::YoggWheel
            | Act::CurseOfFlesh
            | Act::DevouringHunger
            | Act::RodOfRoasting { .. }
            | Act::GoldenTavern
            | Act::CastRandomSpells(_)
            | Act::Surprise(_)
            // Gold, now.
            | Act::Gold(_)
            | Act::GoldPerType(_)
            | Act::GoldPerGolden(_)
            // Stats given once, now.
            | Act::Gems(_)
            | Act::GemBonus { .. }
            | Act::SpellBonus { .. }
            | Act::UndeadBonus { .. }
            | Act::BuffAll { .. }
            | Act::BuffLeftmostHand { .. }
            | Act::BuffTavernThisTurn { .. }
            | Act::BuffLeftmostBoard { .. }
            | Act::BuffAllGrowing { .. }
            | Act::BuffLeftmostGrowing { .. }
            | Act::BuffAllPerGolden { .. }
            | Act::BuffLeftmostTwoPerBattlecry { .. }
            | Act::BuffRandomOther { .. }
            | Act::BuffEnds { .. }
            | Act::BuffCardHandAndBoard { .. }
            | Act::BuffDeity { .. }
            | Act::HammerDiscarded
            | Act::GemsOnBoard(_)
            | Act::TavernBuff { .. }
            | Act::DemonsConsume(_)
            | Act::ConsumeShopHit
            | Act::DoubleHighestTavern
            | Act::StealHighestTierTavern
            // A count the file gives (`seat_trinket_counters`) or one that
            // starts over each turn, and the upgrade's discount (the file
            // shows the button's price).
            | Act::FreeBuys(_)
            | Act::FreeTribeBuys { .. }
            | Act::CopyBuys(_)
            | Act::HealthPriceMinions { .. }
            | Act::HealthPriceSpell(_)
            | Act::Tier6Refreshes(_)
            | Act::FirstPlayShield(_)
            | Act::FirstSpellTwice(_)
            | Act::DiscoverCopyBonus(_)
            | Act::FreeRefreshes(_)
            | Act::UpgradeDiscount(_)
            // The trinket's own number, which the file gives.
            | Act::GrowValue(_)
            | Act::DoubleValue
            | Act::FlipAlternate
            // The slots ahead: the file shows the trinket that replaced this
            // one, or the offer it opened.
            | Act::GreaterOfferIntoLesser(_)
            | Act::ReplaceWithRandom { .. }
            | Act::OfferReplace(_)
            // The Secret zone is the file's.
            | Act::IceBlock
            // The Deity's golden flag: the shop loader seats no Deity, and
            // the file's numbers carry the +1/+1 a golden one has.
            | Act::GoldenDeity
            // The type a Compass locks in is not in the file: the acts that
            // read it read the board when they run.
            | Act::LockType => {}
        }
    }

    /// The Mystery Cube's turn-start offer: two free Lesser trinkets to
    /// replace the one the slot holds (docs/rules/design.md). Nothing is dealt while
    /// another offer is open, and `cube_pending` remembers the slot until
    /// that offer is answered.
    fn deal_cube_offer(&mut self, p: usize, slot: usize) {
        let Some(held) = self.trinket_of(p, slot) else {
            // The slot lost its trinket (nothing empties a slot today), so
            // the cube has nothing to replace.
            return;
        };
        if !self.players[p].trinket_offer.is_empty() {
            self.players[p].tp.cube_pending = slot as u8 + 1;
            return;
        }
        let choices = self.players[p].trinkets[slot].cube_choices;
        self.deal_trinket_offer(p, slot, Slot::Lesser, Offer::Cube { exclude: held, choices });
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
        // Whether each slot's start-of-turn list runs this turn (a trinket
        // that fires only every N turns skips the ones between).
        let mut fires = [true; TRINKET_SLOTS];
        for slot in 0..TRINKET_SLOTS {
            let Some(i) = self.trinket_of(p, slot) else { continue };
            let (uses, ticks, start_every) = {
                let t = self.data.trinkets.get(i);
                (
                    t.uses_per_turn.unwrap_or(u8::MAX),
                    t.rule.as_ref().is_some_and(|r| r.event == Event::Turns),
                    t.start_every,
                )
            };
            let st = &mut self.players[p].trinkets[slot];
            st.uses_left = uses;
            if ticks && !st.done {
                st.count = st.count.saturating_add(1);
            }
            // "At the start of every two turns": the count runs the turns
            // since the pick (the pick turn's own start has passed).
            if let Some(n) = start_every {
                st.count = st.count.saturating_add(1);
                if st.count >= n as u16 {
                    st.count = 0;
                } else {
                    fires[slot] = false;
                }
            }
        }
        // "The first N times you Discover a card each turn" (Sinstone
        // Sticker): a fresh count every turn.
        let tp = &mut self.players[p].tp;
        tp.discover_copies = tp.discover_copies_base;
        tp.cube_pending = 0;
        for slot in 0..TRINKET_SLOTS {
            if !fires[slot] {
                continue;
            }
            let Some(i) = self.trinket_of(p, slot) else { continue };
            self.run_trinket_hook(p, slot, i, Hook::StartOfTurn);
        }
        // The Mystery Cube: the trinket the slot holds has had its own
        // start-of-turn effect, so now two new free Lesser trinkets replace
        // it (the one held is never offered back).
        for slot in 0..TRINKET_SLOTS {
            if self.players[p].trinkets[slot].cube {
                self.deal_cube_offer(p, slot);
            }
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
            // "Trigger all your Rallies" and "trigger your minions'
            // Battlecries" fire once more per Drakkari (docs/rules/design.md: Drakkari
            // doubles your end-of-turn effects).
            let runs = if self.data.trinkets.get(i).drakkari_end { self.drakkari_mult(p) } else { 1 };
            for _ in 0..runs {
                self.run_trinket_hook(p, slot, i, Hook::EndOfTurn);
            }
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

    /// An event a trinket listens for. `card` is the spell or card the event
    /// is about (the "a spell" filter, a death counter that names a card);
    /// `amount` counts for events that count a number (Gold spent). Anything
    /// about a minion goes through `trinket_event_minion`, so its tribes are
    /// the ones it has now.
    pub(super) fn trinket_event(&mut self, p: usize, event: Event, card: Option<CardId>, amount: u32) {
        let tribes = card.map(|c| self.data.cards.get(c).tribes);
        self.trinket_event_of(p, event, card, tribes, amount);
    }

    /// The same event about a minion (bought, played, sold): its tribes are
    /// the ones it has now, a Dark Gift's Amalgamation and a portrait's extra
    /// tribes included (`CardDb::tribes_of`, the one source), so "when you
    /// play a Dragon" sees an Amalgamation.
    pub(super) fn trinket_event_minion(&mut self, p: usize, event: Event, m: &Minion, amount: u32) {
        let tribes = self.data.cards.tribes_of(m);
        self.trinket_event_of(p, event, Some(m.card), Some(tribes), amount);
    }

    fn trinket_event_of(
        &mut self,
        p: usize,
        event: Event,
        card: Option<CardId>,
        card_tribes: Option<Tribes>,
        amount: u32,
    ) {
        if self.players[p].tp.golden_when_rich != 0 {
            self.check_golden_when_rich(p);
        }
        // Guiding Candle: this Refresh is over, so one of the turn's two
        // Tier 6 Refreshes is spent (the fill already read the count).
        if event == Event::Refresh && self.players[p].tp.refresh_tier6 > 0 {
            self.players[p].tp.refresh_tier6 -= 1;
        }
        // The Surprise Portrait's hidden part: its roll comes before the
        // lists, so an Elemental of Surprise can arrive with this action.
        if self.players[p].tp.surprise != 0
            && matches!(event, Event::Buy | Event::BuySpell | Event::Sell | Event::Refresh)
        {
            self.surprise_roll(p, event);
        }
        // The hot path: one `u32` on the player says whether any of its
        // trinkets listens for this event. Most of a game, and every game
        // before turn 6, has nothing here.
        if self.players[p].trinket_events & event.bit() == 0 {
            return;
        }
        let card_is_spell = card.is_some_and(|c| self.data.cards.get(c).is_spell());
        for slot in 0..TRINKET_SLOTS {
            let Some(i) = self.trinket_of(p, slot) else { continue };
            // Which list this event fires, and the rule it counts toward.
            let (hook, capped, uses_left, rule, death_card) = {
                let t = self.data.trinkets.get(i);
                let by_tribe = |want: Option<Tribe>| match want {
                    None => true,
                    Some(w) => card_tribes.is_some_and(|ts| ts.has(w)),
                };
                // An alternating trinket listens for one kind of buy at a
                // time: a minion, then a Tavern spell (Trickster's Sleeve).
                let step = self.players[p].trinkets[slot].value;
                let hook = match event {
                    Event::Buy if by_tribe(t.buy_tribe) && !(t.alternate && step != 0) => {
                        Some(Hook::OnBuy)
                    }
                    Event::BuySpell if !(t.alternate && step == 0) => Some(Hook::OnBuySpell),
                    Event::Play if by_tribe(t.play_tribe) => Some(Hook::OnPlay),
                    Event::Sell => Some(Hook::OnSell),
                    Event::Refresh => Some(Hook::OnRefresh),
                    Event::CastSpell => Some(Hook::OnCastSpell),
                    Event::SpellCast => Some(Hook::OnAnySpell),
                    Event::CastSpellOnMinion => Some(Hook::OnCastSpellOnMinion),
                    Event::Discard
                        if (!t.discard_spells_only || card_is_spell)
                            && (!t.discard_minions_only || !card_is_spell) =>
                    {
                        Some(Hook::OnDiscard)
                    }
                    Event::GetTribe(w) if t.get_tribe.is_none_or(|x| x == w) => Some(Hook::OnGet),
                    Event::GoldSpent => Some(Hook::OnGoldSpent),
                    Event::CardPlayed => Some(Hook::OnCardPlay),
                    Event::Battlecry => Some(Hook::OnBattlecry),
                    Event::Gem => Some(Hook::OnGem),
                    Event::Attack => Some(Hook::OnAttack),
                    Event::Destroy if t.destroy_card.is_none_or(|c| card == Some(c)) => {
                        Some(Hook::OnDestroy)
                    }
                    Event::Magnetize => Some(Hook::OnMagnetize),
                    Event::CastSpellOnShop => Some(Hook::OnCastSpellOnShop),
                    Event::Consume => Some(Hook::OnConsume),
                    Event::HeroDamage => Some(Hook::OnHeroDamage),
                    _ => None,
                };
                let rule = t.rule.as_ref().map(|r| (r.event, r.every, r.once));
                let uses_left = self.players[p].trinkets[slot].uses_left;
                (hook, t.uses_per_turn.is_some(), uses_left, rule, t.death_card)
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
                // A rule that counts one card's deaths only counts that
                // card's (Snarky Portrait's Fishbait); a plain death counter
                // takes the bulk count, which carries no card.
                Event::Death => match death_card {
                    Some(c) => event == Event::Death && card == Some(c),
                    None => event == Event::Death && card.is_none(),
                },
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
            // A bulk event (the 8 deaths of one shop destroy, the Beasts of
            // one fight) can cross the threshold more than once, and what is
            // left over counts toward the next one.
            let fires = (st.count / every as u16).min(8);
            st.count -= fires * every as u16;
            if once {
                st.done = true;
            }
            for _ in 0..fires {
                self.run_trinket_hook(p, slot, i, Hook::Rule);
            }
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
            Act::DarkGift { tier, yours, golden, use_table } => {
                self.open_dark_gift_trinket(p, tier, yours, golden, use_table);
            }
            Act::DarkGiftCopies => {
                let _ = self.players[p]
                    .discover_queue
                    .try_push(super::DiscoverRequest::DarkGiftCopies);
                self.maybe_start_discover(p);
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
            Act::DiscoverDeathrattle(n) => {
                for _ in 0..n {
                    let se = SpellEffect::Discover(DiscoverKind::DeathrattleMinion);
                    self.execute_spell_effect(p, se, false);
                }
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
            // A trinket's own cast (Easterly Winds): the player's, unlike the
            // same `cast` key on a minion (`Effect::CastSpell`).
            Act::Cast { card, times } => {
                self.cast_named_spell(p, card, times as u32, false)
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
                    let ts = cards.tribes_of(m);
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
                    .filter(|(_, m)| cards.tribes_of(m).is(tribe) && !cards.get(m.card).fixed_stats)
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
                // The Mystery Cube: this slot now offers `n` free Lesser
                // trinkets to replace whatever it holds, at the pick and at
                // the start of every turn after it (docs/rules/design.md).
                self.players[p].trinkets[slot].cube = true;
                self.players[p].trinkets[slot].cube_choices = n;
                self.refresh_trinket_mask(p);
                self.deal_cube_offer(p, slot);
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
                // "At the end of your turn, give your minions +1/+1.
                // (Improved by Golden minions you've played this game!)": the
                // base plus one more for each.
                let n = 1 + self.players[p].golden_played as i32;
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
            Act::DiscoverRemember { tier, typed } => {
                let req = if typed {
                    super::DiscoverRequest::TypedMinionTier(tier)
                } else {
                    super::DiscoverRequest::MinionTier(tier)
                };
                let _ = self.players[p].discover_queue.try_push(req);
                self.players[p].tp.remember_in = slot as u8 + 1;
                self.maybe_start_discover(p);
            }
            Act::DiscoverTier(tier) => {
                let _ = self.players[p].discover_queue.try_push(super::DiscoverRequest::MinionTier(tier));
                self.maybe_start_discover(p);
            }
            Act::CopyBuys(n) => {
                let tp = &mut self.players[p].tp;
                tp.copy_buys = tp.copy_buys.saturating_add(n);
            }
            Act::GetRemembered(n) => {
                let Some(card) = self.players[p].trinkets[slot].card else { return };
                for _ in 0..n {
                    self.give_generated(p, card);
                }
            }
            Act::RandomBattlecry(n) => {
                let tier = self.players[p].tier;
                for _ in 0..n {
                    match self.data.cards.random_battlecry_minion(tier, self.lobby, &mut self.rng) {
                        Some(c) => self.give_generated(p, c),
                        None => break,
                    }
                }
            }
            Act::PlainCopyRandomFriendly(n) => self.run_effect(
                p,
                &mut pos,
                Effect::PlainCopyRandomFriendly,
                false,
                n,
            ),
            Act::CopyLastDiscarded => {
                self.run_effect(p, &mut pos, Effect::CopyLastDiscarded, false, 1)
            }
            Act::GemsEachType(n) => {
                // One friendly minion of each type on the board, picked at
                // random when several share a type (Aggem Sticker).
                let cards = &self.data.cards;
                let mut picked: ArrayVec<usize, { Tribe::EVERY.len() }> = ArrayVec::new();
                for t in Tribe::EVERY {
                    let mut of: ArrayVec<usize, { crate::BOARD }> = self.players[p]
                        .board
                        .iter()
                        .enumerate()
                        .filter(|(_, m)| cards.tribes_of(m).has(t))
                        .map(|(i, _)| i)
                        .collect();
                    if of.is_empty() {
                        continue;
                    }
                    let k = self.rng.below(of.len() as u32) as usize;
                    let _ = picked.try_push(of.swap_remove(k));
                }
                for at in picked {
                    self.play_gems_on_board(p, at, n);
                }
            }
            Act::DiscoverCopyBonus(n) => {
                let tp = &mut self.players[p].tp;
                tp.discover_copies_base = n;
                tp.discover_copies = n;
            }
            Act::BuffLeftmostBoard { attack, health } => {
                self.execute_spell_effect(p, SpellEffect::BuffLeftmost { attack, health }, false)
            }
            Act::GetGolden { card } => {
                if self.players[p].hand.is_full() {
                    return;
                }
                let m = self.players[p].make_card(&self.data.cards, card, true, 0);
                self.add_to_hand(p, m);
            }
            Act::LockboxSooner(n) => {
                let tp = &mut self.players[p].tp;
                tp.lockbox_sooner = tp.lockbox_sooner.saturating_add(n);
            }
            Act::MagnetizeEnds { card, tribe } => {
                // A copy of the card is Magnetized to the left-most and the
                // right-most minion of the tribe (Accord-o-Tron Portrait). A
                // lone minion is both ends and takes it once, like the
                // "ends" picks of the Start of Combat trinkets. The right end
                // is found again after the first Magnetization: its triggers
                // can change the board (2026-10-31: a stale index panicked).
                let find_left = |g: &Self| {
                    g.players[p].board.iter().position(|m| g.data.cards.tribes_of(m).is(tribe))
                };
                if let Some(l) = find_left(self) {
                    let m = self.players[p].make_card(&self.data.cards, card, false, 0);
                    self.magnetize_to(p, l, m, false);
                    let r = self.players[p]
                        .board
                        .iter()
                        .rposition(|m| self.data.cards.tribes_of(m).is(tribe));
                    if let Some(r) = r
                        && r != l
                    {
                        let m = self.players[p].make_card(&self.data.cards, card, false, 0);
                        self.magnetize_to(p, r, m, false);
                    }
                }
            }
            Act::OneOf { a, b } => {
                let pick = if self.rng.below(2) == 0 { a } else { b };
                self.give_generated(p, pick);
            }
            Act::WarbandWhistle => {
                let tp = &mut self.players[p].tp;
                tp.whistle = true;
                self.players[p].free_rolls = self.players[p].free_rolls.saturating_add(1);
            }
            Act::TripleNeedTwoPirate => self.players[p].tp.pirate_triple = true,
            Act::PowerTriggers(n) => self.players[p].tp.power_triggers = n,
            Act::PowerExtraUses(n) => {
                let tp = &mut self.players[p].tp;
                tp.power_extra_uses = tp.power_extra_uses.saturating_add(n);
                // The extra use counts this turn too, not only from the next.
                if let Some(st) = self.players[p].power.as_mut() {
                    st.uses_left = st.uses_left.saturating_add(n);
                }
            }
            Act::PowerGold(n) => self.players[p].tp.power_gold = n,
            Act::SecondPower => {
                // The same exclusion list as Unmasked Identity (cards.toml
                // `exclude_heroes`), and a slot of its own: the main power
                // stays.
                let excludes =
                    self.data.heroes.hero_indices(&self.data.cards.hero_power_excludes);
                self.discover_hero_power(p, &excludes, true, 1);
            }
            Act::Gauntlet => self.players[p].tp.gauntlet = true,
            Act::ExtraHighTier => self.players[p].tp.extra_high_tier = true,
            Act::ExtraMagnetic => self.players[p].tp.extra_magnetic = true,
            Act::MagneticCost(n) => self.players[p].tp.magnetic_cost = n,
            Act::Tier6Refreshes(n) => {
                let tp = &mut self.players[p].tp;
                tp.refresh_tier6_base = n;
                tp.refresh_tier6 = n;
            }
            Act::MinTier(n) => self.players[p].tp.min_tier = n,
            Act::GoldenDeity => {
                if let Some(deity) = &mut self.players[p].deity
                    && !deity.golden
                {
                    // A Golden minion doubles its printed stats: the Deity's
                    // base is 1/1, and the buffs it already carries stay
                    // (`golden_up`'s rule).
                    deity.golden = true;
                    deity.attack += 1;
                    deity.health += 1;
                }
            }
            Act::RandomMagnetic(n) => {
                let tier = self.players[p].tier;
                for _ in 0..n {
                    match self.data.cards.random_magnetic_minion(tier, self.lobby, &mut self.rng) {
                        Some(c) => self.give_generated(p, c),
                        None => break,
                    }
                }
            }
            Act::FirstPlayShield(n) => self.players[p].tp.play_shields = n,
            Act::DoubleHighestTavern => {
                // Upstart Embers: the highest-Health minion in the Tavern
                // doubles its stats (ties at random; fixed-stat tokens such as
                // a Fodder are left alone).
                let cards = &self.data.cards;
                let mut best: ArrayVec<usize, { crate::SHOP }> = ArrayVec::new();
                for (i, m) in self.players[p].shop.iter().enumerate() {
                    if cards.get(m.card).is_spell() || cards.get(m.card).fixed_stats {
                        continue;
                    }
                    match best.first() {
                        Some(&b) if self.players[p].shop[b].health > m.health => {}
                        Some(&b) if self.players[p].shop[b].health == m.health => {
                            let _ = best.try_push(i);
                        }
                        _ => {
                            best.clear();
                            let _ = best.try_push(i);
                        }
                    }
                }
                if best.is_empty() {
                    return;
                }
                let k = self.rng.below(best.len() as u32) as usize;
                let at = best[k];
                let m = &mut self.players[p].shop[at];
                m.attack *= 2;
                m.health *= 2;
            }
            Act::StealHighestTierTavern => {
                // Rendle Sticker: the highest-Tier card of the Tavern (the
                // spell slot too), ties at random. A steal, so it leaves the
                // pool like a bought copy but fires no buy trigger.
                let cards = &self.data.cards;
                // SHOP + 1: the spell slot ties in too, and the Tavern can hold
                // one card more than its size (Twice as Nice's copy).
                let mut best: ArrayVec<usize, { crate::SHOP + 1 }> = ArrayVec::new();
                let mut best_tier = 0u8;
                for (i, m) in self.players[p].shop.iter().enumerate() {
                    let t = cards.get(m.card).tier;
                    if t > best_tier {
                        best_tier = t;
                        best.clear();
                        let _ = best.try_push(i);
                    } else if t == best_tier {
                        let _ = best.try_push(i);
                    }
                }
                let spells = self
                    .players[p]
                    .shop_spell
                    .map(|c| cards.get(c).tier)
                    .unwrap_or(0);
                if spells > best_tier {
                    best.clear();
                    let _ = best.try_push(self.players[p].shop.len());
                } else if spells == best_tier && spells > 0 {
                    let _ = best.try_push(self.players[p].shop.len());
                }
                if best.is_empty() {
                    return;
                }
                let k = self.rng.below(best.len() as u32) as usize;
                let slot = best[k];
                // A slot at the shop's length is the spell slot
                // (`steal_shop_card` reads it that way).
                self.steal_shop_card(p, slot);
            }
            Act::EndOfTurnTwice => {
                // Ghastly Sticker: the player's end-of-turn effects run one
                // more time in all, on top of any Drakkari (`drakkari_mult`
                // reads it).
                self.players[p].tp.end_of_turn_ghastly = true;
            }
            Act::FirstSpellTwice(n) => {
                let tp = &mut self.players[p].tp;
                tp.first_spell_double_base = n;
                tp.first_spell_double = n;
            }
            Act::DiscoverTierStats { tier, count, attack, health } => {
                self.players[p].tp.discover_stats = Some((attack, health, count));
                for _ in 0..count {
                    let _ = self.players[p]
                        .discover_queue
                        .try_push(super::DiscoverRequest::MinionTier(tier));
                }
                self.maybe_start_discover(p);
            }
            // ---- block 4 ----
            Act::WarDrum(n) => {
                let tp = &mut self.players[p].tp;
                tp.war_drum = tp.war_drum.max(n);
            }
            Act::DragonBattlecry(n) => self.players[p].tp.dragon_battlecry = n,
            Act::Locket => self.players[p].tp.locket = true,
            Act::TriggerBattlecriesEnds => {
                // The left-most minion, then the right-most; a lone minion is
                // both ends and triggers once. The board can change while a
                // Battlecry runs, so the right end is found again.
                if self.players[p].board.is_empty() {
                    return;
                }
                self.run_battlecry_at(p, 0, true);
                let last = self.players[p].board.len().saturating_sub(1);
                if last > 0 {
                    self.run_battlecry_at(p, last, true);
                }
            }
            Act::TriggerBattlecriesOf(card) => {
                let mut i = 0;
                while i < self.players[p].board.len() {
                    if self.players[p].board[i].parts().any(|(c, _)| c == card) {
                        self.run_battlecry_at(p, i, true);
                    }
                    i += 1;
                }
            }
            Act::TriggerAllRallies => {
                let mut i = 0;
                while i < self.players[p].board.len() {
                    let m = self.players[p].board[i];
                    for (card, golden) in m.parts() {
                        if let Some(e) = self.data.cards.get(card).rally {
                            // A Rally that needs the minion it attacks does
                            // nothing here (docs/rules/trinkets.md).
                            let mut pos = i;
                            self.run_effect(p, &mut pos, e, golden, 1);
                        }
                    }
                    i += 1;
                }
            }
            Act::TriggerDeathrattlesLeftmost(n) => self.trigger_deathrattles_in_shop(p, n),
            Act::BuffEnds { attack, health } => {
                // The left-most and the right-most minion; a lone minion is
                // both ends and takes it once.
                let n = self.players[p].board.len();
                if n == 0 {
                    return;
                }
                buff(&mut self.players[p].board[0], attack, health);
                if n > 1 {
                    buff(&mut self.players[p].board[n - 1], attack, health);
                }
            }
            Act::FlipAlternate => {
                let v = &mut self.players[p].trinkets[slot].value;
                *v = u8::from(*v == 0);
            }
            Act::CastOnLeftmost { card, tribe } => {
                let found = self.players[p]
                    .board
                    .iter()
                    .position(|m| self.data.cards.tribes_of(m).has(tribe));
                if let Some(i) = found {
                    self.cast_spell_on(p, card, i);
                }
            }
            Act::CastOnRandom { card, tribe } => {
                let picks: ArrayVec<usize, { crate::BOARD }> = self.players[p]
                    .board
                    .iter()
                    .enumerate()
                    .filter(|(_, m)| self.data.cards.tribes_of(m).has(tribe))
                    .map(|(i, _)| i)
                    .collect();
                if picks.is_empty() {
                    return;
                }
                let k = self.rng.below(picks.len() as u32) as usize;
                let at = picks[k];
                self.cast_spell_on(p, card, at);
            }
            Act::SatelliteOfBought(card) => {
                let (a, h) = self.players[p].tp.bought_stats;
                if self.players[p].hand.is_full() {
                    return;
                }
                let mut m = self.players[p].make_card(&self.data.cards, card, false, 0);
                m.attack = a;
                m.health = h;
                self.add_to_hand(p, m);
            }
            Act::GetWithStats { card, attack, health, per_spell } => {
                let n = self.players[p].spells_cast_turn as i32;
                let (a, h) = (attack + per_spell.0 * n, health + per_spell.1 * n);
                if self.players[p].hand.is_full() {
                    return;
                }
                let mut m = self.players[p].make_card(&self.data.cards, card, false, 0);
                m.attack = a;
                m.health = h;
                self.add_to_hand(p, m);
            }
            Act::TeachBoughtSpell(n) => {
                let Some(spell) = self.players[p].tp.bought_spell else { return };
                self.teach_murloc(p, spell, n);
            }
            Act::ConsumeShopHit => {
                let hit = self.players[p].tp.shop_hit;
                if hit == 0 || self.players[p].board.is_empty() {
                    return;
                }
                let s = hit as usize - 1;
                if s >= self.players[p].shop.len() {
                    return;
                }
                let k = self.rng.below(self.players[p].board.len() as u32) as usize;
                self.consume(p, k, s, false, 1);
            }
            Act::ConsumeClaw { attack, health } => {
                let tp = &mut self.players[p].tp;
                tp.consume_claw = (attack, health);
            }
            Act::DoubleValue => {
                let v = &mut self.players[p].trinkets[slot].value;
                *v = v.saturating_mul(2);
            }
            Act::SpellBonusPerType { attack, health } => {
                self.players[p].tp.spell_bonus_per_type = (attack, health);
            }
            Act::SpellBonusThisTurn { attack, health } => {
                let b = &mut self.players[p].tp.spell_bonus_turn;
                b.0 += attack;
                b.1 += health;
            }
            Act::BuffCardHandAndBoard { card, attack, health } => {
                let pl = &mut self.players[p];
                for m in pl.board.iter_mut().chain(pl.hand.iter_mut()) {
                    if m.parts().any(|(c, _)| c == card) {
                        buff(m, attack, health);
                    }
                }
            }
            Act::Hammer { base, per } => {
                let tp = &mut self.players[p].tp;
                tp.hammer_base = base;
                tp.hammer_per = per;
                let n = self.players[p].discarded as i32;
                let (da, dh) = (base.0 + per.0 * n, base.1 + per.1 * n);
                for m in &mut self.players[p].board {
                    buff(m, da, dh);
                }
            }
            Act::HammerDiscarded => {
                let (pa, ph) = self.players[p].tp.hammer_per;
                if (pa, ph) == (0, 0) {
                    return;
                }
                for m in &mut self.players[p].board {
                    buff(m, pa, ph);
                }
            }
            Act::CardMod { card, kind } => {
                // A tribe change reaches the copies the player already has:
                // "the change applies to every copy of that card you have or
                // will have" (docs/rules/trinkets.md), the ones already on the
                // board and in hand. A later copy gets it in `make_card`.
                if let CardMod::AlsoTribes(t) = kind {
                    for m in self.players[p].board.iter_mut().chain(self.players[p].hand.iter_mut()) {
                        if m.parts().any(|(c, _)| c == card) {
                            m.extra_tribes = m.extra_tribes.with_all(t);
                        }
                    }
                }
                let mods = &mut self.players[p].tp.card_mods;
                if mods.iter().all(|m| m.map(|(c, _)| c) != Some(card))
                    && let Some(empty) = mods.iter_mut().find(|m| m.is_none())
                {
                    *empty = Some((card, kind));
                }
            }
            Act::ChooseOneAll => self.players[p].tp.choose_one_all = true,
            Act::Souvenir => self.players[p].tp.souvenir = slot as u8 + 1,
            // ---- block 5: the Darkmoon Prizes and the last trinkets ----
            Act::TriplePrize(card) => {
                // Corrupted Tome: the card in hand now, and every later
                // Triple Reward becomes one (`give_triple_reward`).
                self.players[p].tp.triple_prize = Some(card);
                self.give_generated(p, card);
            }
            Act::DiscoverPrize(tier) => {
                let _ = self.players[p].discover_queue.try_push(super::DiscoverRequest::Prize(tier));
                self.maybe_start_discover(p);
            }
            Act::RandomPrize(n) => {
                for _ in 0..n {
                    let Some(card) = self.data.cards.random_prize_any(&mut self.rng) else {
                        break;
                    };
                    self.give_generated(p, card);
                }
            }
            Act::IceBlock => {
                self.players[p].secrets.add(crate::secrets::ICE_BLOCK);
            }
            Act::Surprise(percent) => self.players[p].tp.surprise = percent,
            Act::YoggWheel => {
                // The Wheel's six results are the trinket's own `wheel` list:
                // one of them, each equally likely.
                let of = self.players[p].trinkets[slot].of;
                if of == 0 {
                    return;
                }
                let wheel = &self.data.trinkets.get(of - 1).wheel;
                if wheel.is_empty() {
                    return;
                }
                let k = self.rng.below(wheel.len() as u32) as usize;
                let picked = wheel[k];
                self.run_trinket_act(p, slot, picked);
            }
            Act::CurseOfFlesh => {
                // A random friendly minion gains the stats of another one.
                let n = self.players[p].board.len();
                if n < 2 {
                    return;
                }
                let a = self.rng.below(n as u32) as usize;
                let mut b = self.rng.below((n - 1) as u32) as usize;
                if b >= a {
                    b += 1;
                }
                let (sa, sh) = (self.players[p].board[b].attack, self.players[p].board[b].health);
                buff(&mut self.players[p].board[a], sa, sh);
            }
            Act::DevouringHunger => {
                // Consume every minion in the Tavern (a real consume, so the
                // pool and every consume trigger are honours) and give its
                // stats to a random friendly minion, then Refresh.
                let mut guard = 0;
                while !self.players[p].shop.is_empty() && guard < crate::SHOP + 2 {
                    guard += 1;
                    if self.players[p].board.is_empty() {
                        break;
                    }
                    let k = self.rng.below(self.players[p].board.len() as u32) as usize;
                    self.consume(p, k, 0, false, 1);
                }
                self.refresh_tavern_now(p);
            }
            Act::RodOfRoasting { attack, health } => {
                // The most Pyrobuff casts one Rod of Roasting may throw; the
                // second target in three ends the run, so this is only a
                // guard.
                const ROD_OF_ROASTING_CASTS: u32 = 25;
                // Pyrobuff at random: a minion in play (your board or the
                // Tavern) takes it, or the bartender or your hero is hit and
                // the casting stops. Each target is equally likely, and the
                // loop is capped so a bad run cannot go on forever.
                for _ in 0..ROD_OF_ROASTING_CASTS {
                    let board = self.players[p].board.len();
                    let shop = self.players[p].shop.len();
                    if board + shop == 0 {
                        break;
                    }
                    let k = self.rng.below((board + shop + 2) as u32) as usize;
                    if k >= board + shop {
                        break;
                    }
                    if k < board {
                        buff(&mut self.players[p].board[k], attack, health);
                    } else {
                        buff(&mut self.players[p].shop[k - board], attack, health);
                    }
                }
            }
            Act::GoldenTavern => {
                self.execute_spell_effect(p, SpellEffect::GoldenTouch, false)
            }
            Act::CastRandomSpells(n) => {
                for _ in 0..n {
                    self.cast_random_spell(p);
                }
            }
            Act::MechaDemon(n) => {
                for _ in 0..n {
                    let Some(card) = self.data.cards.random_mecha_demon(&mut self.rng) else {
                        break;
                    };
                    self.give_generated(p, card);
                }
            }
            Act::OneOf3 { a, b, c } => {
                let pick = match self.rng.below(3) {
                    0 => a,
                    1 => b,
                    _ => c,
                };
                self.give_generated(p, pick);
            }
            Act::RandomMostCommonType(n) => {
                // A trinket that read its type at the pick keeps it: "the type
                // is fixed at the pick" (docs/rules/trinkets.md). One that never read
                // one (a copy a portrait made over it, a pick with no typed
                // minion on the board) reads the board now.
                let locked = self.players[p].trinkets[slot].tribe;
                let t = locked.or_else(|| self.board_most_common_type(p));
                let Some(t) = t else { return };
                for _ in 0..n {
                    self.run_effect(p, &mut pos, Effect::GetRandomMinion { tribe: t }, false, 1);
                }
            }
            Act::LockType => {
                let t = self.board_most_common_type(p);
                self.players[p].trinkets[slot].tribe = t;
            }
            Act::GoldenBoughtType => {
                // The type the pick locked in: the minion just bought turns
                // Golden if it is of it, and the type is spent.
                let Some(t) = self.players[p].trinkets[slot].tribe else { return };
                let Some(card) = self.players[p].tp.bought_card else { return };
                let cards = &self.data.cards;
                let Some(at) = self.players[p]
                    .hand
                    .iter()
                    .rposition(|m| m.card == card && !m.golden && cards.tribes_of(m).has(t))
                else {
                    return;
                };
                let mut m = self.players[p].hand[at];
                super::shop::golden_up(cards, &mut m);
                // "A minion made golden this way is not a triple and gives no
                // Triple Reward" (docs/rules/design.md): playing it gives none.
                m.no_reward = true;
                self.players[p].hand[at] = m;
                self.players[p].trinkets[slot].tribe = None;
            }
        }
    }

    /// The Surprise Portrait's hidden part (docs/rules/trinkets.md, Era's model):
    /// every buy, sell and Refresh has a 5% chance of a surprise, never twice
    /// in a row. A buy or a sell gets an Elemental of Surprise; a Refresh
    /// gets one in the Tavern or has every Tavern slot filled with them.
    /// Every Elemental made this way is generated, with no pool copy.
    fn surprise_roll(&mut self, p: usize, event: Event) {
        let chance = self.players[p].tp.surprise as u32;
        let last = self.players[p].tp.surprise_last;
        let roll = self.rng.below(100) < chance && !last;
        self.players[p].tp.surprise_last = roll;
        if !roll {
            return;
        }
        let Some(card) = self.data.cards.by_name("Elemental of Surprise") else { return };
        match event {
            Event::Refresh => {
                // Two ways a surprise shows up on a Refresh, picked evenly
                // (Era's rule names both and no odds).
                if self.rng.below(2) == 0 {
                    // One joins the Tavern, in a free slot or the spell slot.
                    // A Tavern can hold one card more than its size (Twice as
                    // Nice's copy), so the array's own bound is the guard.
                    let size = self.players[p].shop_size(&self.data.rules);
                    let room = self.players[p].shop.len() < size
                        || (self.players[p].shop.len() == size
                            && self.players[p].shop_spell.is_none());
                    if room && self.players[p].shop.len() < crate::SHOP {
                        let mut m = self.players[p].make_card(&self.data.cards, card, false, 0);
                        self.players[p].apply_tavern_buffs(&self.data.cards, &mut m);
                        self.players[p].shop.push(m);
                    }
                } else {
                    // Every Tavern slot fills with them, replacing what was
                    // there (the old cards' pool copies go back).
                    let Game { data, rng, players, pool, lobby, .. } = self;
                    let _ = (rng, lobby);
                    let size = players[p].shop_size(&data.rules);
                    let old: ArrayVec<Minion, { crate::SHOP }> = players[p].shop.drain(..).collect();
                    for m in old {
                        super::shop::give_back(pool, &data.cards, &m);
                    }
                    players[p].shop_spell = None;
                    for _ in 0..size.min(crate::SHOP) {
                        let mut m = players[p].make_card(&data.cards, card, false, 0);
                        players[p].apply_tavern_buffs(&data.cards, &mut m);
                        players[p].shop.push(m);
                    }
                }
            }
            // A buy or a sell: the Elemental arrives in hand if it fits, and
            // is lost to a full hand like any other generated card.
            _ => self.give_generated(p, card),
        }
    }

    /// What a minion in Tavern slot `s` costs this player right now: Gold
    /// (0 when a trinket makes it free) or Health (Pilgrimp Sticker, Demonic
    /// Tapestry, The Eye of Sargeras). docs/rules/design.md: a Health price is self
    /// damage, not Gold spent.
    pub(super) fn minion_price(&self, p: usize, slot: usize) -> MinionPrice {
        let pl = &self.players[p];
        let Some(m) = pl.shop.get(slot) else { return MinionPrice::Gold(self.data.rules.buy_cost) };
        let card = self.data.cards.get(m.card);
        // A Tavern minion can carry a Dark Gift (Amalgamation) or a
        // portrait's extra tribes, so its tribe tests are its own.
        let tribes = self.data.cards.tribes_of(m);
        let tp = pl.tp;
        // Warcry Totem's free buys are Battlecry minions only.
        if pl.free_minion_left > 0
            || (tp.free_buys > 0 && card.battlecry.is_some())
            || (tp.free_tribe_left > 0 && tp.free_tribe.is_some_and(|t| tribes.has(t)))
        {
            return MinionPrice::Gold(0);
        }
        let by_tribe = tp.health_price_left > 0
            && tp.health_price_tribe.is_none_or(|t| tribes.has(t));
        let highest = tp.highest_tier_health && self.is_highest_tier_slot(p, slot);
        let every_nth = tp.fourth_buy_every > 0
            && (tp.fourth_buy + 1).is_multiple_of(tp.fourth_buy_every);
        if by_tribe || highest || every_nth {
            MinionPrice::Health(self.data.rules.buy_cost)
        } else if let Some((price, _, _)) = self.power_prices(p) {
            // Manastorm: minions cost 2 (a trinket's own set price still
            // wins, it names the very cards it puts there).
            MinionPrice::Gold(tp.tavern_price.max(price))
        } else if tp.tavern_price > 0 {
            // Chillmere Mosaic: the Tavern cards its Refresh put there cost
            // this much (they keep it until the Tavern is filled again).
            MinionPrice::Gold(tp.tavern_price)
        } else if pl.prize.minion_price > 0 {
            // Friends and Family Discount: minions in the Tavern cost this
            // much for the rest of the game.
            MinionPrice::Gold(pl.prize.minion_price)
        } else if tp.magnetic_cost > 0
            && card.keywords.has(crate::minion::Keywords::MAGNETIC)
            && tribes.has(Tribe::Mech)
        {
            // "Magnetic Mechs cost (2)": a set price, not a discount
            // (Electrode Attractor).
            MinionPrice::Gold(tp.magnetic_cost)
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
        let pl = &mut self.players[p];
        let tp = &mut pl.tp;
        match price {
            MinionPrice::Gold(0) => {
                if pl.free_minion_left > 0 {
                    pl.free_minion_left -= 1;
                } else if tp.free_buys > 0 {
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
        // Two passes over the shared table and no `Arc` clone: this runs on a
        // slot swap, and a clone would bump the refcount of the table every
        // game shares.
        let fits = |t: &crate::trinkets::Trinket, lobby: Tribes| {
            t.slot == kind
                && (t.tribes.is_empty() || t.tribes.overlaps(lobby))
                && tribe.is_none_or(|w| t.tribes.has(w))
        };
        let lobby = self.lobby;
        let n = (0..self.data.trinkets.len() as u8)
            .filter(|&i| fits(self.data.trinkets.get(i), lobby))
            .count();
        if n == 0 {
            return;
        }
        let k = self.rng.below(n as u32) as usize;
        let Some(of) = (0..self.data.trinkets.len() as u8)
            .filter(|&i| fits(self.data.trinkets.get(i), lobby))
            .nth(k)
        else {
            return;
        };
        self.set_trinket(p, slot, Some(of));
    }
}
