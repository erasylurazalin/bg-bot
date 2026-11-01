//! Pending board and shop targets: offer, apply and resolve.

use arrayvec::ArrayVec;
use crate::trinkets::Event;
use crate::cards::{
    ConsumePick,
    DoubleStat,
    Effect,
    EventKind,
    SpellTarget,
    Tribe,
};
use crate::minion::{
    CardId,
    Keywords,
};
use crate::{
    BOARD,
    SHOP,
};
use super::{
    DiscoverRequest,
    Game,
    PendingTarget,
    ShopTargetKind,
    TargetKind,
    TavernBuff,
};
use super::shop::{give_back, golden_up};
use super::effects::buff;

impl Game {
    /// The Tavern buff "minions of its type" gives (Eonar's Favor): a tribe at
    /// random for a dual-type target (DESIGN.md's rule for a type picked at
    /// random), and every Tavern minion that has a type for an all-type one (an
    /// Amalgam), which the log shows. A tribeless target gives nothing.
    fn tavern_buff_type(&mut self, target: &crate::minion::Minion, attack: i32, health: i32) -> Option<TavernBuff> {
        let tribes = self.data.cards.tribes_of(target);
        if tribes.is_empty() {
            return None;
        }
        let tribe = if tribes == crate::cards::Tribes::ALL {
            None
        } else {
            Some(tribes.random_type(&mut self.rng)?)
        };
        Some(TavernBuff { tribe, attack, health, this_turn: false, typed_only: tribe.is_none() })
    }

    /// Set a Tavern minion's stats to `attack`/`health`, keeping the bonuses
    /// that apply wherever the card is (the Undead bonus, an Eternal Knight's
    /// deaths): the rule Pack Tactics' 3/3 copy follows (DESIGN.md "Exact
    /// copies").
    pub(super) fn set_shop_stats(&mut self, p: usize, slot: usize, attack: i32, health: i32) {
        if slot >= self.players[p].shop.len() {
            return;
        }
        let (a, h) = self.players[p].wherever_stats(&self.data.cards, &self.players[p].shop[slot]);
        let m = &mut self.players[p].shop[slot];
        m.attack = attack + a;
        m.health = health + h;
    }

    /// `set_shop_stats` on a board minion.
    pub(super) fn set_board_stats(&mut self, p: usize, pos: usize, attack: i32, health: i32) {
        if pos >= self.players[p].board.len() {
            return;
        }
        let (a, h) = self.players[p].wherever_stats(&self.data.cards, &self.players[p].board[pos]);
        let m = &mut self.players[p].board[pos];
        m.attack = attack + a;
        m.health = health + h;
    }

    pub(super) fn resolve_shop_target(&mut self, p: usize, slot: usize) {
        if let Some(pst) = self.players[p].pending_shop_target.take() {
            // The Tavern emptied while the pick was open (nothing does that
            // today): there is nothing to point at, so the pick fizzles.
            if slot >= self.players[p].shop.len() {
                self.maybe_start_discover(p);
                return;
            }
            match pst.kind {
                ShopTargetKind::Consume { keywords, remaining, stat_mul } => {
                    self.consume(p, pst.source as usize, slot, keywords, stat_mul);
                    if remaining > 1 {
                        self.start_consume(
                            p,
                            pst.source as usize,
                            ConsumePick::Chosen,
                            keywords,
                            remaining - 1,
                            stat_mul,
                        );
                    }
                }
                ShopTargetKind::Fishbait { card, golden } => {
                    self.put_fishbait(p, slot, card, golden)
                }
            }
            self.maybe_start_discover(p);
            return;
        }
        let Some(pt) = self.players[p].pending_target.take() else { return };
        self.apply_target_shop(p, &pt, slot);
        self.maybe_start_discover(p);
    }

    /// Resolve a pending target that the player picked in the Tavern.
    fn apply_target_shop(&mut self, p: usize, pt: &PendingTarget, slot: usize) {
        if let TargetKind::StealShopCard { damage } = pt.kind {
            if self.steal_shop_card(p, slot) {
                // The damage runs through the hero-damage path, so armor
                // takes it and a rewind on the board gives it back
                // (DESIGN.md "Heroes").
                self.take_shop_damage(p, damage, None);
            }
            return;
        }
        // Arcane Alteration and Imprison take any Tavern card, a spell in a
        // minion slot and the spell slot included, so they run before the
        // spell guard below.
        match pt.kind {
            TargetKind::ReplaceSameTier => {
                self.replace_tavern_same_tier(p, slot);
                return;
            }
            TargetKind::ImprisonShop { turns } => {
                self.imprison_tavern_card(p, slot, turns);
                return;
            }
            _ => {}
        }
        if slot >= self.players[p].shop.len()
            || self.data.cards.get(self.players[p].shop[slot].card).is_spell()
        {
            return;
        }
        match pt.kind {
            // Handled above, before the spell guard.
            TargetKind::ReplaceSameTier | TargetKind::ImprisonShop { .. } => {}
            TargetKind::AnyBuff { attack, health } => {
                for _ in 0..pt.runs {
                    buff(&mut self.players[p].shop[slot], attack, health);
                }
            }
            TargetKind::AnySetStats { attack, health } => {
                for _ in 0..pt.runs {
                    self.set_shop_stats(p, slot, attack, health);
                }
            }
            // A Battlecry that picks its target at random reaches the Tavern
            // too (Young Murk-Eye Sticker, Hackerfin Portrait): "a minion"
            // there is a Tavern minion. No played Battlecry lands here, only
            // `answer_target_at_random`.
            TargetKind::MinionBuff { tribe, attack, health, keyword, random_keyword, .. } => {
                let ok = tribe.is_none_or(|t| {
                    self.data.cards.tribes_of(&self.players[p].shop[slot]).has(t)
                });
                if !ok {
                    return;
                }
                for _ in 0..pt.runs {
                    buff(&mut self.players[p].shop[slot], attack, health);
                }
                if keyword != 0 {
                    self.players[p].shop[slot].keywords.set(keyword);
                }
                if random_keyword {
                    self.give_random_keyword_in_shop(p, slot);
                }
            }
            TargetKind::BecomeCopy => {
                let Some(src) = pt.source.map(|x| x as usize) else { return };
                let copy = self.players[p].shop[slot];
                self.become_copy_of(p, src, copy);
            }
            // Stir the Pot: a thrown Tavern minion is gone for good, so its
            // pool copy is not given back.
            TargetKind::ThrowInPot { count } => {
                let m = self.players[p].shop.remove(slot);
                self.stir_the_pot(p, m, count);
            }
            // Reborn Rites: a Tavern minion gains Reborn until the next turn,
            // so a bought one keeps it for this one.
            TargetKind::TempReborn => {
                let m = &mut self.players[p].shop[slot];
                m.keywords.set(Keywords::REBORN);
                m.temp_reborn = true;
            }
            // Galakrond's Greed: the old card goes back to the pool (the
            // Tavern's buffs are lost) and a Discover from exactly one Tier
            // higher takes its slot.
            TargetKind::ReplaceHigher => {
                let old = self.players[p].shop.remove(slot);
                let tier = self.data.cards.get(old.card).tier;
                give_back(&mut self.pool, &self.data.cards, &old);
                self.players[p].discover_into_shop =
                    (slot.min(crate::SHOP - 1) + 1) as u8;
                let _ = self.players[p]
                    .discover_queue
                    .try_push(DiscoverRequest::MinionTier((tier + 1).min(crate::MAX_TIER)));
                self.maybe_start_discover(p);
            }

            // See the Light: the minion goes to hand as a 2/2, as a steal.
            TargetKind::StealShopAsStats { attack, health } => {
                let mut m = self.players[p].shop.remove(slot);
                m.attack = attack;
                m.health = health;
                if !self.add_to_hand(p, m) {
                    self.players[p].shop.insert(slot, m);
                }
            }
            // Lock and Load: the minion leaves the Tavern to be summoned, and
            // attack at once, when there is space in the next combat. It fights
            // that combat only (nothing marks it for the board after), so its
            // pool copy goes back and the power holds a made-fresh copy.
            TargetKind::LockAndLoad => {
                let mut m = self.players[p].shop.remove(slot);
                give_back(&mut self.pool, &self.data.cards, &m);
                m.pool_copies = 0;
                self.players[p].locked_and_loaded = Some(m);
            }

            // Spirit Swap: the first of the two minions is remembered and the
            // second pick is offered.
            TargetKind::SpiritSwapFirst => {
                self.players[p].two_step = Some((slot as u8, true));
                let pt = PendingTarget {
                    source: None,
                    kind: TargetKind::SpiritSwapSecond,
                    runs: 1,
                    tavern: false,
                    gem: false,
                };
                self.offer_target(p, pt);
            }
            TargetKind::SpiritSwapSecond => {
                if let Some(first) = self.players[p].two_step.take() {
                    self.swap_attacks(p, first, (slot as u8, true));
                }
            }
            TargetKind::Spell(st) => {
                let target = slot;
                // A Tavern minion is not a friendly minion, so Balinda's
                // double cast does not apply to it.
                for _ in 0..1 {
                    if target >= self.players[p].shop.len()
                        || self.data.cards.get(self.players[p].shop[target].card).is_spell()
                    {
                        break;
                    }
                    let (da, dh) = if pt.tavern { self.total_spell_bonus(p) } else { (0, 0) };
                    let mut generated: Option<CardId> = None;
                    let mut stop_after = false;
                    match st {
                        SpellTarget::Buff {
                            attack,
                            health,
                            keyword,
                            times,
                            if_win_next_turn_buff,
                            random_keyword,
                            ..
                        } => {
                            if pt.gem {
                                // A Blood Gem on a Tavern minion: one cast,
                                // the player's Gem value, tracked as Blood
                                // Gem stats.
                                let (ga, gh) =
                                    (self.players[p].gem_attack, self.players[p].gem_health);
                                let m = &mut self.players[p].shop[target];
                                m.attack += ga;
                                m.health += gh;
                                m.gem_attack += ga;
                                m.gem_health += gh;
                            } else {
                                let m = &mut self.players[p].shop[target];
                                for _ in 0..times {
                                    // A keyword-only spell (Sacred Gift) gets no spell bonus.
                                    let (ba, bh) = if attack == 0 && health == 0 { (0, 0) } else { (attack + da, health + dh) };
                                    buff(m, ba, bh);
                                }
                                if keyword != 0 {
                                    m.keywords.set(keyword);
                                }
                                if let Some((wb_a, wb_h)) = if_win_next_turn_buff {
                                    m.winners_bread = m.winners_bread.saturating_add(1);
                                    self.players[p].winners_bread_buff = (wb_a, wb_h);
                                }
                            }
                            if random_keyword {
                                self.give_random_keyword_in_shop(p, target);
                            }
                        }
                        SpellTarget::BuffPerBonusKeyword { attack, health } => {
                            let k = self.players[p]
                                .distinct_bonus_keywords(self.data.rules.bonus_mask)
                                as i32;
                            let m = &mut self.players[p].shop[target];
                            for _ in 0..1 + k {
                                buff(m, attack + da, health + dh);
                            }
                        }
                        SpellTarget::BuffSameTribe { attack, health } => {
                            let t = self.data.cards.tribes_of(&self.players[p].shop[target]);
                            if let Some(tribe) = t.random_type(&mut self.rng) {
                                for o in &mut self.players[p].board {
                                    if self.data.cards.tribes_of(o).has(tribe) {
                                        buff(o, attack + da, health + dh);
                                    }
                                }
                                if self.data.cards.tribes_of(&self.players[p].shop[target]).has(tribe) {
                                    buff(&mut self.players[p].shop[target], attack + da, health + dh);
                                }
                            }
                        }
                        SpellTarget::SameTribeMinion => {
                            let c_id = self.players[p].shop[target].card;
                            let t = self.data.cards.get(c_id).tribes;
                            let tier = self.players[p].tier;
                            generated = self.data.cards.random_different_minion(t, c_id, tier, self.lobby, &mut self.rng);
                        }
                        SpellTarget::SetStats { attack, health } => {
                            self.set_shop_stats(p, target, attack, health);
                        }
                        SpellTarget::TrickyTrousers { attack, health } => {
                            let m = &mut self.players[p].shop[target];
                            buff(m, attack + da, health + dh);
                            if m.keywords.has(Keywords::TAUNT) {
                                m.keywords.clear(Keywords::TAUNT);
                            } else {
                                m.keywords.set(Keywords::TAUNT);
                            }
                        }
                        SpellTarget::RobustEvolution => {
                            self.robust_evolution_shop(p, target);
                            stop_after = true;
                        }
                        SpellTarget::TavernBuffType { attack, health } => {
                            let target_card = self.players[p].shop[target];
                            let b = self.tavern_buff_type(&target_card, attack + da, health + dh);
                            if let Some(b) = b {
                                self.players[p].add_tavern_buff(&self.data.cards, b);
                            }
                        }
                        SpellTarget::RefreshWithType => {
                            let t = self.data.cards.tribes_of(&self.players[p].shop[target]);
                            if let Some(tribe) = t.random_type(&mut self.rng) {
                                let Game { data, rng, players, pool, lobby, .. } = self;
                                Self::refresh_with_tribe(
                                    &mut players[p],
                                    pool,
                                    &data.cards,
                                    &data.rules,
                                    rng,
                                    *lobby,
                                    tribe,
                                );
                            }
                            stop_after = true;
                        }
                        SpellTarget::DestroyGetRandom { .. }
                        | SpellTarget::TransformHigher
                        | SpellTarget::WatchStats => {
                            // Friendly minions only: none of them reaches the
                            // Tavern (`SpellTarget::can_target_shop`).
                        }
                        SpellTarget::Confiscation { gems } => {
                            // 3 Gems on the Tavern minion, then its
                            // neighbours' Blood Gem stats move onto it.
                            let (ga, gh) =
                                (self.players[p].gem_attack, self.players[p].gem_health);
                            {
                                let m = &mut self.players[p].shop[target];
                                m.attack += ga * gems as i32;
                                m.health += gh * gems as i32;
                                m.gem_attack += ga * gems as i32;
                                m.gem_health += gh * gems as i32;
                            }
                            for n in [target.checked_sub(1), Some(target + 1)]
                                .into_iter()
                                .flatten()
                            {
                                let (sa, sh) = match self.players[p].shop.get(n) {
                                    Some(m) if !self.data.cards.get(m.card).is_spell() => {
                                        (m.gem_attack, m.gem_health)
                                    }
                                    _ => continue,
                                };
                                if sa == 0 && sh == 0 {
                                    continue;
                                }
                                let nm = &mut self.players[p].shop[n];
                                nm.attack -= sa;
                                nm.health -= sh;
                                nm.gem_attack = 0;
                                nm.gem_health = 0;
                                let m = &mut self.players[p].shop[target];
                                m.attack += sa;
                                m.health += sh;
                                m.gem_attack += sa;
                                m.gem_health += sh;
                            }
                        }
                        // The rest target a friendly board minion only.
                        SpellTarget::Consume { .. }
                        | SpellTarget::MakeGolden { .. }
                        | SpellTarget::HalfOfHighestTavern { .. }
                        | SpellTarget::Butchering { .. }
                        | SpellTarget::Devourer => {}
                        SpellTarget::BuffPerTribePlayed { attack, health, per, .. } => {
                            let n = self.players[p].elementals_played_turn as i32;
                            buff(
                                &mut self.players[p].shop[target],
                                attack + per.0 * n + da,
                                health + per.1 * n + dh,
                            );
                        }
                        SpellTarget::PlainCopy { .. } => {
                            let card = self.players[p].shop[target].card;
                            generated = Some(card);
                            stop_after = true;
                        }
                        SpellTarget::GemKeyword { keyword } => {
                            // A special Blood Gem cast on a Tavern minion.
                            let (ga, gh) =
                                (self.players[p].gem_attack, self.players[p].gem_health);
                            let m = &mut self.players[p].shop[target];
                            m.attack += ga;
                            m.health += gh;
                            m.gem_attack += ga;
                            m.gem_health += gh;
                            let quilboar =
                                self.data.cards.tribes_of(m).has(Tribe::Quilboar);
                            if quilboar {
                                m.keywords.set(keyword);
                            }
                        }
                        // A friendly board minion's Deathrattle: no Tavern
                        // minion has one to trigger.
                        SpellTarget::TriggerDeathrattle => {}
                        // "Give a minion +2 Attack, then double its Attack"
                        // ("I'm Still Just a Rat in a Cage") reaches a Tavern
                        // minion; it is a Prize, so it never counts as a
                        // Tavern spell (`pt.tavern` stays false).
                        SpellTarget::BuffThenDouble { attack, .. } => {
                            let m = &mut self.players[p].shop[target];
                            m.attack += attack + da;
                            m.health += dh;
                            m.attack = m.attack.saturating_mul(2);
                        }
                        // The Prizes' three "a friendly minion" effects never
                        // reach a Tavern minion (`can_target_shop` says so).
                        SpellTarget::ReturnToHand { .. }
                        | SpellTarget::MakeGoldenReturn
                        | SpellTarget::GrantEndOfTurn { .. } => {}
                    }
                    if pt.tavern {
                        self.trigger_on_cast_spell(p);
                    }
                    if !stop_after
                        && target < self.players[p].shop.len()
                        && self.data.cards.tribes_of(&self.players[p].shop[target]).has(Tribe::Mech)
                    {
                        self.glambot_on_shop_mech(p, target);
                    }
                    if let Some(gen_c) = generated {
                        let mut dummy = 0;
                        self.give_generated_tracking(p, gen_c, &mut dummy);
                    }
                    if stop_after {
                        break;
                    }
                }
                // A Blood Gem cast from the hand on a Tavern minion counts
                // for Ruby Tusk too.
                if pt.gem && pt.source.is_none() {
                    self.trinket_event(p, Event::Gem, None, 1);
                }
                // "After you cast a spell on a Tavern minion" (Felsteel
                // Cleaver): the slot is remembered so the trinket can
                // consume the minion the spell hit.
                if slot < self.players[p].shop.len() {
                    self.players[p].tp.shop_hit = slot as u8 + 1;
                    self.trinket_event(p, Event::CastSpellOnShop, None, 1);
                    // A Tavern minion is a minion (DESIGN.md "Targets in the
                    // shop"), so the spell was cast on one: "after you cast 3
                    // spells on minions" (Wand of Divination) counts it.
                    self.trinket_event(p, Event::CastSpellOnMinion, None, 1);
                }
            }
            _ => {}
        }
    }

    pub(super) fn offer_target(&mut self, p: usize, pt: PendingTarget) {
        let mut board: ArrayVec<usize, BOARD> = ArrayVec::new();
        for s in 0..self.players[p].board.len() {
            if self.valid_target(p, &pt, s) {
                let _ = board.try_push(s);
            }
        }
        let mut shop: ArrayVec<usize, SHOP> = ArrayVec::new();
        if pt.kind.takes_shop() {
            for s in 0..self.players[p].shop.len() {
                if self.valid_shop_target(p, &pt, s) {
                    let _ = shop.try_push(s);
                }
            }
            // A card steal, and the two powers that take any Tavern card
            // (Imprison, Arcane Alteration), may also take the spell slot,
            // one past the cards.
            if matches!(
                pt.kind,
                TargetKind::StealShopCard { .. }
                    | TargetKind::ImprisonShop { .. }
                    | TargetKind::ReplaceSameTier
            ) && self.valid_shop_target(p, &pt, self.players[p].shop.len())
            {
                let _ = shop.try_push(self.players[p].shop.len());
            }
        }
        if board.len() + shop.len() != 1 {
            if board.is_empty() && shop.is_empty() {
                return;
            }
            self.players[p].pending_target = Some(pt);
            return;
        }
        if let Some(&s) = board.first() {
            self.apply_target(p, &pt, s);
        } else if let Some(&s) = shop.first() {
            self.apply_target_shop(p, &pt, s);
        }
    }

    pub(super) fn apply_target(&mut self, p: usize, pt: &PendingTarget, s: usize) {
        match pt.kind {
            // Never reached: a card steal is a Tavern pick (`apply_target_shop`),
            // and so are Galakrond's Greed's replacement, See the Light and
            // Imprison.
            TargetKind::StealShopCard { .. }
            | TargetKind::ReplaceHigher
            | TargetKind::StealShopAsStats { .. }
            | TargetKind::ImprisonShop { .. }
            | TargetKind::LockAndLoad => {}
            TargetKind::MinionBuff { attack, health, keyword, random_keyword, .. } => {
                for _ in 0..pt.runs {
                    let m = &mut self.players[p].board[s];
                    buff(m, attack, health);
                    if keyword != 0 {
                        m.keywords.set(keyword);
                    }
                    if random_keyword {
                        self.give_random_keyword(p, s);
                    }
                }
            }
            TargetKind::MakeGolden { .. } => {
                // A minion made Golden on the board gives no Triple Reward
                // and doesn't count as a Golden minion played.
                golden_up(&self.data.cards, &mut self.players[p].board[s]);
                if pt.runs > 1 {
                    let again = PendingTarget { runs: pt.runs - 1, ..*pt };
                    self.offer_target(p, again);
                }
            }
            TargetKind::AnyBuff { attack, health } => {
                for _ in 0..pt.runs {
                    buff(&mut self.players[p].board[s], attack, health);
                }
            }
            TargetKind::DestroyDiscover { tribe, discover } => {
                self.destroy_in_shop(p, &[s], None);
                for _ in 0..discover {
                    let _ = self.players[p]
                        .discover_queue
                        .try_push(DiscoverRequest::TribeMinion(tribe));
                }
            }
            TargetKind::DestroyCopy { copies, .. } => {
                let card = self.players[p].board[s].card;
                self.destroy_in_shop(p, &[s], None);
                for _ in 0..copies {
                    self.give_generated(p, card);
                }
            }
            TargetKind::RebornDestroy { attack, health, .. } => {
                let host = pt.source.map(|x| x as usize);
                self.players[p].board[s].keywords.set(Keywords::REBORN);
                let tracked = self.destroy_in_shop(p, &[s], host);
                // The destroy's Deathrattles can hand over a card that
                // completes a triple, and a triple takes its copies off the
                // board: the tracked position can then be gone.
                if let Some(m) = tracked.and_then(|h| self.players[p].board.get_mut(h)) {
                    buff(m, attack, health);
                }
            }
            TargetKind::AnySetStats { attack, health } => {
                for _ in 0..pt.runs {
                    self.set_board_stats(p, s, attack, health);
                }
            }
            TargetKind::BecomeCopy => {
                if let Some(src) = pt.source.map(|x| x as usize) {
                    let copy = self.players[p].board[s];
                    self.become_copy_of(p, src, copy);
                }
            }
            // Trash for Treasure: the minion is removed (not sold: no Gold,
            // no sell triggers, its pool copy back) and the Discover is one
            // Tier below it, never below 1.
            TargetKind::RemoveTierBelow => {
                let tier = self.data.cards.get(self.players[p].board[s].card).tier;
                let m = self.players[p].board.remove(s);
                give_back(&mut self.pool, &self.data.cards, &m);
                let lower = tier.saturating_sub(1).max(1);
                let _ = self.players[p].discover_queue.try_push(DiscoverRequest::MinionTier(lower));
                self.maybe_start_discover(p);
            }
            // Swap, Lock, & Shop It: the picked minion goes into the Tavern
            // and a random Tavern minion takes its place on the board.
            TargetKind::SwapWithShop => self.swap_with_shop(p, s),
            // Stir the Pot: the minion is gone for good, so its pool copy is
            // not given back.
            TargetKind::ThrowInPot { count } => {
                let m = self.players[p].board.remove(s);
                self.stir_the_pot(p, m, count);
            }
            // Reborn Rites: Reborn until the next turn.
            TargetKind::TempReborn => {
                let m = &mut self.players[p].board[s];
                m.keywords.set(Keywords::REBORN);
                m.temp_reborn = true;
            }
            // Arcane Alteration on the board: a random card of the same Tier
            // takes the slot, keeping the stats the old one had gained.
            TargetKind::ReplaceSameTier => self.replace_board_same_tier(p, s),
            // Devour: the minion is sold (Gold and sell triggers, like
            // Channel the Devourer) and its stats go to another friendly one.
            TargetKind::DevourFirst => {
                if s < self.players[p].board.len() {
                    let m = self.players[p].board[s];
                    self.players[p].devour_stats = (m.attack, m.health);
                    self.sell_minion(p, s);
                    if !self.players[p].board.is_empty() {
                        let pt = PendingTarget {
                            source: None,
                            kind: TargetKind::DevourSecond,
                            runs: 1,
                            tavern: false,
                            gem: false,
                        };
                        self.offer_target(p, pt);
                    }
                }
            }
            TargetKind::DevourSecond => {
                let (a, h) = self.players[p].devour_stats;
                if let Some(m) = self.players[p].board.get_mut(s) {
                    buff(m, a, h);
                }
            }
            // Spirit Swap: the first minion is remembered, the second offered.
            TargetKind::SpiritSwapFirst => {
                self.players[p].two_step = Some((s as u8, false));
                let pt = PendingTarget {
                    source: None,
                    kind: TargetKind::SpiritSwapSecond,
                    runs: 1,
                    tavern: false,
                    gem: false,
                };
                self.offer_target(p, pt);
            }
            TargetKind::SpiritSwapSecond => {
                if let Some(first) = self.players[p].two_step.take() {
                    self.swap_attacks(p, first, (s as u8, false));
                }
            }
            // Rune of Damnation: a real death in the shop (Deathrattles fire)
            // for a random minion of that tribe.
            TargetKind::DestroyGetTribe { tribe } => {
                self.destroy_in_shop(p, &[s], None);
                let mut pos = 0usize;
                self.run_effect(p, &mut pos, Effect::GetRandomMinion { tribe }, false, 1);
            }
            // Cloning Gallery: an exact copy, summoned beside it, taking no
            // pool copy; it counts toward a triple.
            TargetKind::SummonExactCopy => {
                if s < self.players[p].board.len() && !self.players[p].board.is_full() {
                    let mut copy = self.players[p].board[s];
                    copy.pool_copies = 0;
                    copy.no_reward = true;
                    let card = copy.card;
                    self.players[p].board.insert(s + 1, copy);
                    self.check_triple(p, card);
                }
            }
            // Rapid Reanimation: mark the chosen minion for Start of Combat
            // destruction and resummoning, and arm the power slot.
            TargetKind::RapidReanimation => {
                if s < self.players[p].board.len() {
                    for m in &mut self.players[p].board {
                        m.reanimate = false;
                    }
                    self.players[p].board[s].reanimate = true;
                    let heroes = &self.data.heroes;
                    let pl = &mut self.players[p];
                    for slot in [&mut pl.power, &mut pl.power2] {
                        if let Some(st) = slot.as_mut()
                            && matches!(
                                heroes.power(st.of).effect,
                                crate::heroes::PowerEffect::RapidReanimation
                            )
                        {
                            st.armed = true;
                        }
                    }
                }
            }
            TargetKind::Consume { pick, keywords, count, stat_mul, .. } => {
                self.start_consume(p, s, pick, keywords, count, stat_mul);
            }
            TargetKind::TriggerRally => {
                for _ in 0..pt.runs {
                    let Some(m) = self.players[p].board.get(s).copied() else { break };
                    for (card, golden) in m.parts() {
                        let Some(e) = self.data.cards.get(card).rally else { continue };
                        let mut pos = s;
                        self.run_effect(p, &mut pos, e, golden, 1);
                    }
                }
            }
            TargetKind::TriggerBattlecry => {
                for _ in 0..pt.runs {
                    if s >= self.players[p].board.len() {
                        break;
                    }
                    self.battlecry_or_choose_one(p, s);
                    // A targeting Battlecry or a Choose One waits for its
                    // own choice; don't run it again.
                    if self.players[p].pending_target.is_some()
                        || self.players[p].pending_choose_one.is_some()
                    {
                        break;
                    }
                }
                self.maybe_start_discover(p);
            }
            TargetKind::Spell(st) => {
                let mut target_pos = s;
                for k in 0..pt.runs {
                    if target_pos >= self.players[p].board.len() {
                        break;
                    }
                    // Every extra cast (Balinda) is another spell cast: one
                    // more for the "every spell" counters always, and one
                    // more for the Tavern ones only (a Blood Gem does not
                    // count there).
                    if k > 0 {
                        self.fire_event(p, EventKind::AllSpellsCast, 1);
                        if pt.tavern {
                            self.fire_event(p, EventKind::SpellsCast, 1);
                        }
                        if target_pos >= self.players[p].board.len() {
                            break;
                        }
                    }
                    let (da, dh) = if pt.tavern { self.total_spell_bonus(p) } else { (0, 0) };
                    let mut generated: Option<CardId> = None;
                    // A spell that removes or transforms its target can't
                    // usefully be cast again on the same slot (Balinda).
                    let mut stop_after = false;
                    let mut target_removed = false;
                    match st {
                        SpellTarget::Buff {
                            attack,
                            health,
                            keyword,
                            times,
                            if_win_next_turn_buff,
                            random_keyword,
                            ..
                        } => {
                            if pt.gem {
                                // A Blood Gem: the player's Gem value, no
                                // spell power, tracked as Blood Gem stats.
                                let (ga, gh) =
                                    (self.players[p].gem_attack, self.players[p].gem_health);
                                let m = &mut self.players[p].board[target_pos];
                                m.attack += ga;
                                m.health += gh;
                                m.gem_attack += ga;
                                m.gem_health += gh;
                            } else {
                                let m = &mut self.players[p].board[target_pos];
                                for _ in 0..times {
                                    // A keyword-only spell (Sacred Gift) gets no spell bonus.
                                    let (ba, bh) = if attack == 0 && health == 0 { (0, 0) } else { (attack + da, health + dh) };
                                    buff(m, ba, bh);
                                }
                                if keyword != 0 {
                                    m.keywords.set(keyword);
                                }
                                if let Some((wb_a, wb_h)) = if_win_next_turn_buff {
                                    m.winners_bread = m.winners_bread.saturating_add(1);
                                    self.players[p].winners_bread_buff = (wb_a, wb_h);
                                }
                            }
                            if random_keyword {
                                self.give_random_keyword(p, target_pos);
                            }
                        }
                        SpellTarget::BuffPerBonusKeyword { attack, health } => {
                            let k = self.players[p]
                                .distinct_bonus_keywords(self.data.rules.bonus_mask)
                                as i32;
                            let m = &mut self.players[p].board[target_pos];
                            for _ in 0..1 + k {
                                buff(m, attack + da, health + dh);
                            }
                        }
                        SpellTarget::BuffSameTribe { attack, health } => {
                            let t = self.data.cards.tribes_of(&self.players[p].board[target_pos]);
                            // DESIGN.md "Types": a dual or all-type minion
                            // uses one of its types, picked at random.
                            if let Some(tribe) = t.random_type(&mut self.rng) {
                                for o in &mut self.players[p].board {
                                    if self.data.cards.tribes_of(o).has(tribe) {
                                        buff(o, attack + da, health + dh);
                                    }
                                }
                            }
                        }
                        SpellTarget::SameTribeMinion => {
                            let c_id = self.players[p].board[target_pos].card;
                            let t = self.data.cards.get(c_id).tribes;
                            let tier = self.players[p].tier;
                            generated = self.data.cards.random_different_minion(
                                t,
                                c_id,
                                tier,
                                self.lobby,
                                &mut self.rng,
                            );
                        }
                        SpellTarget::SetStats { attack, health } => {
                            self.set_board_stats(p, target_pos, attack, health);
                        }
                        SpellTarget::Consume { pick, keywords, count, .. } => {
                            self.start_consume(p, target_pos, pick, keywords, count, 1);
                        }
                        SpellTarget::MakeGolden { .. } => {
                            let m = &mut self.players[p].board[target_pos];
                            if !m.golden {
                                let base = self.data.cards.get(m.card);
                                let (ga, gh) = base
                                    .golden_stats
                                    .unwrap_or((base.attack * 2, base.health * 2));
                                m.golden = true;
                                m.attack += ga - base.attack;
                                m.health += gh - base.health;
                            }
                        }
                        SpellTarget::TrickyTrousers { attack, health } => {
                            let m = &mut self.players[p].board[target_pos];
                            buff(m, attack + da, health + dh);
                            if m.keywords.has(Keywords::TAUNT) {
                                m.keywords.clear(Keywords::TAUNT);
                            } else {
                                m.keywords.set(Keywords::TAUNT);
                            }
                        }
                        SpellTarget::RobustEvolution => {
                            // Balinda casts it again on the minion the first
                            // cast produced, in the same slot.
                            self.robust_evolution_board(p, target_pos);
                        }
                        SpellTarget::TavernBuffType { attack, health } => {
                            let target_card = self.players[p].board[target_pos];
                            let b = self.tavern_buff_type(&target_card, attack + da, health + dh);
                            if let Some(b) = b {
                                self.players[p].add_tavern_buff(&self.data.cards, b);
                            }
                        }
                        SpellTarget::RefreshWithType => {
                            let t = self.data.cards.tribes_of(&self.players[p].board[target_pos]);
                            if let Some(tribe) = t.random_type(&mut self.rng) {
                                let Game { data, rng, players, pool, lobby, .. } = self;
                                Self::refresh_with_tribe(
                                    &mut players[p],
                                    pool,
                                    &data.cards,
                                    &data.rules,
                                    rng,
                                    *lobby,
                                    tribe,
                                );
                            }
                            // Balinda casts it again: the Tavern refreshes
                            // twice and both casts' on-cast triggers fire.
                        }
                        SpellTarget::Devourer => {
                            self.channel_the_devourer(p, target_pos);
                            stop_after = true;
                            target_removed = true;
                        }
                        SpellTarget::HalfOfHighestTavern { .. } => {
                            // Half, rounded up, of the highest-Health Tavern
                            // minion. An empty Tavern gives nothing.
                            if let Some((a, h)) = self.highest_tavern_stats(p) {
                                buff(
                                    &mut self.players[p].board[target_pos],
                                    (a + 1) / 2 + da,
                                    (h + 1) / 2 + dh,
                                );
                            }
                        }
                        SpellTarget::BuffPerTribePlayed { attack, health, per, .. } => {
                            let n = self.players[p].elementals_played_turn as i32;
                            buff(
                                &mut self.players[p].board[target_pos],
                                attack + per.0 * n + da,
                                health + per.1 * n + dh,
                            );
                        }
                        SpellTarget::Butchering { attack } => {
                            // The bonus lands first, so a Reborn copy comes
                            // back with it. Like Channel the Devourer, a
                            // second cast (Balinda) does nothing: its
                            // target is gone.
                            self.add_undead_bonus(p, attack + da, dh, true);
                            self.destroy_in_shop(p, &[target_pos], None);
                            stop_after = true;
                            target_removed = true;
                        }
                        SpellTarget::DestroyGetRandom { tribe, count } => {
                            // Jailer Sticker: the Undead dies (a real death
                            // in the shop), then the player gets `count`
                            // random Undead.
                            self.destroy_in_shop(p, &[target_pos], None);
                            let mut pos = 0usize;
                            for _ in 0..count {
                                self.run_effect(
                                    p,
                                    &mut pos,
                                    Effect::GetRandomMinion { tribe },
                                    false,
                                    1,
                                );
                            }
                            stop_after = true;
                            target_removed = true;
                        }
                        SpellTarget::TransformHigher => {
                            // Token of the Old Gods: three cards of the tier
                            // above the chosen one; the pick transforms it
                            // (`resolve_choose`).
                            let tier = self.data.cards.get(self.players[p].board[target_pos].card).tier;
                            if tier < crate::MAX_TIER {
                                self.players[p].tp.transform_slot = target_pos as u8 + 1;
                                let _ = self.players[p]
                                    .discover_queue
                                    .try_push(DiscoverRequest::MinionTier(tier + 1));
                            }
                            stop_after = true;
                            target_removed = true;
                        }
                        SpellTarget::WatchStats => {
                            // Makeshift Master: from now on, whatever stats
                            // this minion gains this turn the Deity gains.
                            let m = self.players[p].board[target_pos];
                            let tp = &mut self.players[p].tp;
                            tp.deity_watch = target_pos as u8 + 1;
                            tp.deity_watch_card = Some(m.card);
                            tp.deity_watch_stats = (m.attack, m.health);
                        }
                        SpellTarget::Confiscation { gems } => {
                            // 3 Gems on the target, then the neighbours'
                            // Blood Gem stats move onto it: they lose them,
                            // it gains them.
                            let (ga, gh) =
                                (self.players[p].gem_attack, self.players[p].gem_health);
                            {
                                let m = &mut self.players[p].board[target_pos];
                                m.attack += ga * gems as i32;
                                m.health += gh * gems as i32;
                                m.gem_attack += ga * gems as i32;
                                m.gem_health += gh * gems as i32;
                            }
                            for n in [target_pos.checked_sub(1), Some(target_pos + 1)]
                                .into_iter()
                                .flatten()
                            {
                                let (sa, sh) = match self.players[p].board.get(n) {
                                    Some(m) => (m.gem_attack, m.gem_health),
                                    None => continue,
                                };
                                if sa == 0 && sh == 0 {
                                    continue;
                                }
                                let nm = &mut self.players[p].board[n];
                                nm.attack -= sa;
                                nm.health -= sh;
                                nm.gem_attack = 0;
                                nm.gem_health = 0;
                                let m = &mut self.players[p].board[target_pos];
                                m.attack += sa;
                                m.health += sh;
                                m.gem_attack += sa;
                                m.gem_health += sh;
                            }
                        }
                        SpellTarget::PlainCopy { .. } => {
                            // A generated copy: it takes no pool copy, like
                            // Disguised Graverobber's copies.
                            let card = self.players[p].board[target_pos].card;
                            let mut dummy = 0;
                            self.give_generated_tracking(p, card, &mut dummy);
                        }
                        SpellTarget::GemKeyword { keyword } => {
                            // Jewelry Box: the player's Gem value, and the
                            // keyword only when the target is a Quilboar
                            // (docs/trinkets.md).
                            let (ga, gh) =
                                (self.players[p].gem_attack, self.players[p].gem_health);
                            let m = &mut self.players[p].board[target_pos];
                            m.attack += ga;
                            m.health += gh;
                            m.gem_attack += ga;
                            m.gem_health += gh;
                            let quilboar = self
                                .data
                                .cards
                                .get(m.card)
                                .tribes
                                .has(Tribe::Quilboar);
                            if quilboar {
                                m.keywords.set(keyword);
                            }
                        }
                        SpellTarget::TriggerDeathrattle => {
                            // Floating Candle Set: the target's Deathrattle,
                            // fired outside combat (it does not die).
                            self.trigger_one_deathrattle(p, target_pos);
                            stop_after = true;
                        }
                        SpellTarget::BuffThenDouble { attack, health, keyword, stat } => {
                            // I'm Still Just a Rat in a Cage (+2 Attack, then
                            // double the Attack) and The Bouncer (Taunt, then
                            // double the Health). The spell bonus lands before
                            // the doubling, so it doubles too.
                            let m = &mut self.players[p].board[target_pos];
                            buff(m, attack + da, health + dh);
                            if keyword != 0 {
                                m.keywords.set(keyword);
                            }
                            match stat {
                                DoubleStat::Attack => m.attack = m.attack.saturating_mul(2),
                                DoubleStat::Health => m.health = m.health.saturating_mul(2),
                            }
                        }
                        SpellTarget::ReturnToHand { attack, health } => {
                            // Repeat Customer: the minion goes back to hand
                            // with the stats, its pool copy with it.
                            let mut m = self.players[p].board.remove(target_pos);
                            buff(&mut m, attack + da, health + dh);
                            self.add_to_hand(p, m);
                            stop_after = true;
                            target_removed = true;
                        }
                        SpellTarget::MakeGoldenReturn => {
                            // Raise the Stakes: Golden where it stands, then
                            // back to hand.
                            let mut m = self.players[p].board.remove(target_pos);
                            if !m.golden {
                                super::shop::golden_up(&self.data.cards, &mut m);
                            }
                            self.add_to_hand(p, m);
                            stop_after = true;
                            target_removed = true;
                        }
                        SpellTarget::GrantEndOfTurn { attack, health } => {
                            // Gruul Rules: "At the end of your turn, gain
                            // +4/+4", for good.
                            let m = &mut self.players[p].board[target_pos];
                            m.end_turn_gain.0 += attack + da;
                            m.end_turn_gain.1 += health + dh;
                        }
                    }
                    if pt.tavern {
                        self.trigger_on_cast_spell(p);
                    }
                    if !target_removed && target_pos < self.players[p].board.len() {
                        self.trigger_on_cast_spell_on_target(p, target_pos);
                        if pt.gem {
                            // A Gem was played from hand on the target.
                            self.roogug_on_gem(p, target_pos);
                        }
                        let is_mech = self
                            .data
                            .cards
                            .get(self.players[p].board[target_pos].card)
                            .tribes
                            .has(Tribe::Mech);
                        if is_mech {
                            self.trigger_on_cast_spell_on_mech(p, target_pos);
                        }
                    }
                    if let Some(gen_c) = generated {
                        self.give_generated_tracking(p, gen_c, &mut target_pos);
                    }
                    if stop_after {
                        break;
                    }
                }
                // A Blood Gem cast from the hand (Ruby Tusk counts them).
                if pt.gem && pt.source.is_none() {
                    self.trinket_event(p, Event::Gem, None, 1);
                }
                // Lovely Locket: the trinket casts the same spell on another
                // friendly minion, picked at random and ignoring the spell's
                // own target rules (docs/trinkets.md). The copy is a real
                // cast, and it never chains.
                if self.players[p].tp.locket && !self.players[p].tp.locket_copy {
                    let n = self.players[p].board.len();
                    let hit = s.min(n.saturating_sub(1));
                    if n > 1 {
                        let k = self.rng.below((n - 1) as u32) as usize;
                        let at = if k < hit { k } else { k + 1 };
                        self.players[p].tp.locket_copy = true;
                        let copy = PendingTarget {
                            source: None,
                            kind: TargetKind::Spell(st),
                            runs: 1,
                            tavern: pt.tavern,
                            gem: pt.gem,
                        };
                        self.apply_target(p, &copy, at);
                        self.players[p].tp.locket_copy = false;
                    }
                }
            }
        }
    }

    /// Arcane Alteration on a Tavern card: a random card of the same Tier
    /// takes the slot, and whatever buffs the old card carried are the new
    /// one's too (docs/heroes.md). A spell is replaced by a Tavern spell of
    /// its Tier, a minion by a pool minion; the old card's pool copy goes
    /// back. The slot may be the spell slot, one past the last card.
    pub(super) fn replace_tavern_same_tier(&mut self, p: usize, slot: usize) {
        let from_spell_slot = slot >= self.players[p].shop.len();
        let old = if from_spell_slot {
            match self.players[p].shop_spell.take() {
                Some(id) => self.players[p].make_card(&self.data.cards, id, false, 0),
                None => return,
            }
        } else {
            self.players[p].shop.remove(slot)
        };
        let card = self.data.cards.get(old.card);
        let tier = card.tier;
        let was_spell = card.is_spell();
        // The stats the old card had gained over its printed ones.
        let (da, dh) = if was_spell {
            (0, 0)
        } else {
            let base = self.players[p].make_card(&self.data.cards, old.card, false, 0);
            (old.attack - base.attack, old.health - base.health)
        };
        let golden = old.golden;
        give_back(&mut self.pool, &self.data.cards, &old);
        let Game { data, rng, players, pool, lobby, .. } = self;
        let pl = &mut players[p];
        if was_spell {
            let next = data.cards.random_tier_spell(tier, &[], *lobby, rng);
            let Some(id) = next else { return };
            if from_spell_slot {
                pl.shop_spell = Some(id);
            } else {
                let m = pl.make_card(&data.cards, id, false, 0);
                pl.shop.insert(slot.min(pl.shop.len()), m);
            }
            return;
        }
        let drawn = super::shop::draw_filtered(
            pool, &data.cards, rng, tier, tier, &[], *lobby, |_| true,
        );
        let Some(id) = drawn else { return };
        let mut m = pl.make_card(&data.cards, id, golden, 1);
        m.attack += da;
        m.health += dh;
        m.uses_left = m.turn_uses(&data.cards);
        pl.shop.insert(slot.min(pl.shop.len()), m);
    }

    /// Imprison: the card goes to hand, not bought, locked for its turns; its
    /// pool copy leaves with it, as a steal's does. `slot` may be the spell
    /// slot, one past the last card.
    fn imprison_tavern_card(&mut self, p: usize, slot: usize, turns: u8) {
        let from_spell_slot = slot >= self.players[p].shop.len();
        let mut m = if from_spell_slot {
            match self.players[p].shop_spell.take() {
                Some(id) => self.players[p].make_card(&self.data.cards, id, false, 0),
                None => return,
            }
        } else {
            self.players[p].shop.remove(slot)
        };
        m.locked_turns = turns;
        if !self.add_to_hand(p, m) {
            let pl = &mut self.players[p];
            if from_spell_slot {
                pl.shop_spell = Some(m.card);
            } else {
                pl.shop.insert(slot.min(pl.shop.len()), m);
            }
        }
    }

    /// Arcane Alteration on the board: a random pool minion of the same Tier
    /// takes the slot, keeping the stats the old one had gained and its
    /// Golden status.
    pub(super) fn replace_board_same_tier(&mut self, p: usize, s: usize) {
        if s >= self.players[p].board.len() {
            return;
        }
        let old = self.players[p].board[s];
        let tier = self.data.cards.get(old.card).tier;
        let base = self.players[p].make_card(&self.data.cards, old.card, false, 0);
        let (da, dh) = (old.attack - base.attack, old.health - base.health);
        let golden = old.golden;
        give_back(&mut self.pool, &self.data.cards, &old);
        let Game { data, rng, players, pool, lobby, .. } = self;
        let drawn = super::shop::draw_filtered(
            pool, &data.cards, rng, tier, tier, &[], *lobby, |_| true,
        );
        if let Some(id) = drawn {
            let mut m = players[p].make_card(&data.cards, id, golden, 1);
            m.attack += da;
            m.health += dh;
            m.uses_left = m.turn_uses(&data.cards);
            players[p].board[s] = m;
        }
    }

    /// Spirit Swap: the two minions gain each other's Attack until the next
    /// turn. The swap is read from the Attack both had at the same moment, and
    /// the change is kept as a `temp_attack` so a later buff still stands and
    /// the swap comes off cleanly at the start of the next turn.
    fn swap_attacks(&mut self, p: usize, first: (u8, bool), second: (u8, bool)) {
        let atk = |g: &Game, at: (u8, bool)| -> Option<i32> {
            if at.1 {
                g.players[p].shop.get(at.0 as usize).map(|m| m.attack)
            } else {
                g.players[p].board.get(at.0 as usize).map(|m| m.attack)
            }
        };
        let (Some(a), Some(b)) = (atk(self, first), atk(self, second)) else { return };
        let set = |g: &mut Game, at: (u8, bool), to: i32| {
            let m = if at.1 {
                g.players[p].shop.get_mut(at.0 as usize)
            } else {
                g.players[p].board.get_mut(at.0 as usize)
            };
            if let Some(m) = m {
                m.temp_attack += to - m.attack;
                m.attack = to;
            }
        };
        set(self, first, b);
        set(self, second, a);
    }

    pub(super) fn resolve_target(&mut self, p: usize, s: usize) {
        let pt = self.players[p].pending_target.take().unwrap();
        self.apply_target(p, &pt, s);
        self.maybe_start_discover(p);
    }
}
