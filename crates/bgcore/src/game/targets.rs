//! Pending board and shop targets: offer, apply and resolve.

use arrayvec::ArrayVec;
use crate::trinkets::Event;
use crate::cards::{
    ChooseOption,
    ConsumePick,
    DoubleStat,
    Effect,
    EventKind,
    SpellTarget,
    Tribe,
    TriggerWhen,
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
    CastAfterChoice,
    DiscoverRequest,
    Game,
    PendingTarget,
    QueuedRepeat,
    RepeatWhat,
    ShopTargetKind,
    TargetKind,
    TavernBuff,
};
use super::shop::{give_back, golden_up};
use super::effects::buff;

impl Game {
    /// The Tavern buff "minions of its type" gives (Eonar's Favor): a tribe at
    /// random for a dual-type target (docs/rules/design.md's rule for a type picked at
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
    /// deaths): the rule Pack Tactics' 3/3 copy follows (docs/rules/design.md "Exact
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
            // today): there is nothing to point at, so the pick fizzles. A
            // pick that takes any Tavern card may name the spell slot.
            let spell_slot = slot == self.players[p].shop.len()
                && pst.kind.takes_spell_slot()
                && self.players[p].shop_spell.is_some();
            if slot > self.players[p].shop.len() || (slot == self.players[p].shop.len() && !spell_slot) {
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
        // The Tavern card a repeated Choose One or Battlecry picked: every
        // repeat lands on it (docs/rules/design.md "Set versus add").
        if let Some(q) = self.players[p].repeats.first_mut()
            && q.started
            && q.at.is_none()
            && q.source() == pt.source
        {
            q.at = Some(slot as u8);
        }
        self.apply_target_shop(p, &pt, slot);
        self.maybe_start_discover(p);
    }

    /// Resolve a pending target that the player picked in the Tavern.
    fn apply_target_shop(&mut self, p: usize, pt: &PendingTarget, slot: usize) {
        if let TargetKind::StealShopCard { damage } = pt.kind {
            if self.steal_shop_card(p, slot) {
                // The damage runs through the hero-damage path, so armor
                // takes it and a rewind on the board gives it back
                // (docs/rules/design.md "Heroes").
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
            // Galakrond's Greed: a Discover from exactly one Tier higher
            // takes this slot, and the old card goes back to the pool (the
            // Tavern's buffs are lost) when the pick lands: the game keeps the
            // card on show while the Discover is open, so the state the client
            // shows still has it.
            TargetKind::ReplaceHigher => {
                let tier = self.data.cards.get(self.players[p].shop[slot].card).tier;
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
                    by_minion: false,
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
                    // How many Blood Gems this cast played on the Tavern
                    // minion: a Roogug there reacts once for each of them.
                    let mut gems_played: u8 = 0;
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
                                // Gem stats. A Roogug there reacts below.
                                let (ga, gh) =
                                    (self.players[p].gem_attack, self.players[p].gem_health);
                                let m = &mut self.players[p].shop[target];
                                m.attack += ga;
                                m.health += gh;
                                m.gem_attack += ga;
                                m.gem_health += gh;
                                gems_played = gems_played.saturating_add(1);
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
                            // "all minions that share a type with it", the
                            // Tavern's included: a Tavern minion of the type
                            // gains it too (Era, 2026-11-02; log 15_25_26-g2
                            // turn 7 step 27).
                            let t = self.data.cards.tribes_of(&self.players[p].shop[target]);
                            if let Some(tribe) = t.random_type(&mut self.rng) {
                                let cards = &self.data.cards;
                                let pl = &mut self.players[p];
                                for o in &mut pl.board {
                                    if cards.tribes_of(o).has(tribe) {
                                        buff(o, attack + da, health + dh);
                                    }
                                }
                                for o in &mut pl.shop {
                                    if cards.tribes_of(o).has(tribe) {
                                        buff(o, attack + da, health + dh);
                                    }
                                }
                            }
                        }
                        SpellTarget::SameTribeMinion => {
                            let c_id = self.players[p].shop[target].card;
                            let t = self.data.cards.tribes_of(&self.players[p].shop[target]);
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
                        | SpellTarget::TransformHigher => {
                            // Friendly minions only: none of them reaches the
                            // Tavern (`SpellTarget::can_target_shop`).
                        }
                        // Makeshift Master: "Choose a minion" takes a Tavern
                        // minion too (docs/rules/design.md "Targets in the shop"), and
                        // the watch stays on the card when it is bought.
                        SpellTarget::WatchStats => {
                            self.set_deity_watch(p, target, true);
                        }
                        SpellTarget::Confiscation { gems } => {
                            // 3 Gems on the Tavern minion, then its
                            // neighbours' Blood Gem stats move onto it.
                            self.add_gem_stats_shop(p, target, gems);
                            gems_played = gems_played.saturating_add(gems);
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
                            self.add_gem_stats_shop(p, target, 1);
                            gems_played = gems_played.saturating_add(1);
                            let quilboar = self
                                .data
                                .cards
                                .tribes_of(&self.players[p].shop[target])
                                .has(Tribe::Quilboar);
                            if quilboar {
                                self.players[p].shop[target].keywords.set(keyword);
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
                    // A spell a minion casts for the player is not the
                    // player's cast (Era 2026-11-02): no Tavern spell trigger
                    // and no "cast on a Tavern Mech" watcher sees it.
                    if pt.tavern && !pt.by_minion {
                        self.trigger_on_cast_spell(p);
                    }
                    // A Roogug in the Tavern doesn't react to a Gem cast on
                    // it: whether a Tavern Roogug reacts at all is open
                    // (docs/notes/UNSURE.md).
                    let _ = gems_played;
                    if !pt.by_minion
                        && !stop_after
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
                if pt.gem && pt.source.is_none() && !pt.by_minion {
                    self.trinket_event(p, Event::Gem, None, 1);
                }
                // "After you cast a spell on a Tavern minion" (Felsteel
                // Cleaver): the slot is remembered so the trinket can
                // consume the minion the spell hit. A minion's cast for the
                // player is not one of those.
                if !pt.by_minion && slot < self.players[p].shop.len() {
                    self.players[p].tp.shop_hit = slot as u8 + 1;
                    self.trinket_event(p, Event::CastSpellOnShop, None, 1);
                    // A Tavern minion is a minion (docs/rules/design.md "Targets in the
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

    /// The cast triggers of a spell that is about to take its target off the
    /// board (Channel the Devourer eats it, Butchering destroys it): the
    /// "whenever you cast a spell" minions and the target's own "whenever you
    /// cast a spell on this" watcher run while it is still there, so
    /// whatever they give it goes on to the eater. docs/rules/design.md "Eating a
    /// minion with a spell".
    fn fire_cast_triggers_early(&mut self, p: usize, target_pos: usize, tavern: bool, by_minion: bool) {
        if target_pos >= self.players[p].board.len() {
            return;
        }
        if tavern && !by_minion {
            self.trigger_on_cast_spell(p);
        }
        if !by_minion {
            self.trigger_on_cast_spell_on_target(p, target_pos);
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
                // and doesn't count as a Golden minion played. A repeat of the
                // Battlecry or cast made the pick again only for the card's own
                // count (a golden Elite Navigator picks two Pirates): the
                // golden itself happens once (docs/rules/design.md "Set versus
                // add").
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
                            by_minion: false,
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
                    by_minion: false,
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
                // Every trigger is owed, even when one asks for a target: the
                // queue runs them in turn, the repeats landing on the target
                // the first one picked (docs/rules/design.md "Set versus add").
                if pt.runs > 0 {
                    if !self.players[p].repeats.is_full() {
                        self.players[p].repeats.try_push(QueuedRepeat {
                            what: RepeatWhat::Battlecry { slot: s as u8 },
                            runs: pt.runs,
                            started: false,
                            at: None,
                            option: None,
                            // A Kelp Keeper's Activate: a Discover its
                            // Battlecry opens gives a random card instead
                            // (docs/rules/design.md "Shop replay answers").
                            random_discover: true,
                        })
                        .ok();
                    }
                    self.run_queued_repeats(p);
                }
                self.maybe_start_discover(p);
            }
            TargetKind::Spell(st) => {
                let mut target_pos = s;
                // A cast's own "after you cast a spell" triggers fire as that
                // cast resolves, before the next one starts (docs/rules/design.md
                // "Eating a minion with a spell"): Felboar's "After you cast 3
                // spells" eats the Tavern minion with that one cast's Tavern
                // buff on it, not with the whole play's (log
                // 2026_11_02_00_10_22-g4 turn 9 step 39, where its counter
                // reaches 3 on the first of the play's three casts and the
                // Distractor's buff of that cast is the only one it has seen).
                // This cast's "whenever" half already fired at the play, so
                // each of the play's casts is owed one (`cast_after_choice`,
                // at_play). A Locket's copy is inside its own cast and fires
                // none.
                let owe_after = !pt.by_minion
                    && !self.players[p].tp.locket_copy
                    && self.players[p].cast_after_choice.is_some_and(|w| w.at_play);
                for k in 0..pt.runs {
                    if target_pos >= self.players[p].board.len() {
                        break;
                    }
                    // Every extra cast is another spell cast: the "whenever
                    // you cast a spell" triggers fire before it, one cast's
                    // worth each (the first cast fired them at the play). A
                    // minion's cast for the player fires none (Era 2026-11-02).
                    // The "after you cast a spell" triggers follow the cast
                    // below (`fire_cast_after_choice`).
                    if k > 0 && !pt.by_minion {
                        self.run_event_triggers(p, EventKind::AllSpellsCast, 1, None, TriggerWhen::Now);
                        if pt.tavern {
                            self.run_event_triggers(p, EventKind::SpellsCast, 1, None, TriggerWhen::Now);
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
                    // A spell that eats or destroys its target runs the cast
                    // triggers before that (docs/rules/design.md "Eating a minion with a
                    // spell"), so the generic block below must not run them
                    // again.
                    let mut cast_triggers_done = false;
                    // How many Blood Gems this cast played on the target: a
                    // Roogug there reacts once for each of them.
                    let mut gems_played: u8 = 0;
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
                                // spell power, tracked as Blood Gem stats. A
                                // Gem the player played (or the Lovely Locket
                                // copied for them) gets Surveyor Portrait's
                                // from-hand bonus; one a minion's own effect
                                // casts for the player does not. A Roogug's
                                // reaction to it waits for the cast's other
                                // "played on this" triggers below.
                                self.players[p].add_gem_stats(target_pos, 1, pt.source.is_none());
                                gems_played = gems_played.saturating_add(1);
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
                            // docs/rules/design.md "Types": a dual or all-type minion
                            // uses one of its types, picked at random. The
                            // Tavern's minions of that type gain it too ("all
                            // minions that share a type with it", Era
                            // 2026-11-02).
                            if let Some(tribe) = t.random_type(&mut self.rng) {
                                let cards = &self.data.cards;
                                let pl = &mut self.players[p];
                                for o in &mut pl.board {
                                    if cards.tribes_of(o).has(tribe) {
                                        buff(o, attack + da, health + dh);
                                    }
                                }
                                for o in &mut pl.shop {
                                    if cards.tribes_of(o).has(tribe) {
                                        buff(o, attack + da, health + dh);
                                    }
                                }
                            }
                        }
                        SpellTarget::SameTribeMinion => {
                            let c_id = self.players[p].board[target_pos].card;
                            let t =
                                self.data.cards.tribes_of(&self.players[p].board[target_pos]);
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
                            // Everything that fires on the cast fires first:
                            // "whenever you cast a spell" minions and the
                            // target's own watcher buff it while it is still
                            // on the board, so the eater gains those stats
                            // (docs/rules/design.md "Eating a minion with a spell").
                            self.fire_cast_triggers_early(p, target_pos, pt.tavern, pt.by_minion);
                            cast_triggers_done = true;
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
                            // The Undead dies first: what its death gives the
                            // Tavern spells (Friendly Geist's "your Tavern
                            // spells give +1 Attack") is a Tavern spell
                            // number, and the +Attack this game is one too, so
                            // this cast reads the spell power after the death
                            // (Era, 2026-11-02; log 00_56_06-g1 turn 12).
                            // Like Channel the Devourer, a second cast
                            // (Balinda) does nothing: its target is gone.
                            //
                            // Everything that fires on the cast fires first,
                            // so a Forsaken Weaver's "after you cast a Tavern
                            // spell" +3 lands though the Weaver is the one
                            // destroyed (docs/rules/design.md "Eating a minion with a
                            // spell", docs/notes/UNSURE.md).
                            self.fire_cast_triggers_early(p, target_pos, pt.tavern, pt.by_minion);
                            cast_triggers_done = true;
                            self.destroy_in_shop(p, &[target_pos], None);
                            let (da, dh) =
                                if pt.tavern { self.total_spell_bonus(p) } else { (0, 0) };
                            self.add_undead_bonus(p, attack + da, dh, true);
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
                            self.set_deity_watch(p, target_pos, false);
                        }
                        SpellTarget::Confiscation { gems } => {
                            // 3 Gems on the target, then the neighbours'
                            // Blood Gem stats move onto it: they lose them,
                            // it gains them.
                            self.players[p].add_gem_stats(target_pos, gems, false);
                            gems_played = gems_played.saturating_add(gems);
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
                            // (docs/rules/trinkets.md). The Gem is a Gem
                            // played on it, so a Roogug reacts below.
                            self.players[p].add_gem_stats(target_pos, 1, false);
                            gems_played = gems_played.saturating_add(1);
                            let quilboar = self
                                .data
                                .cards
                                .tribes_of(&self.players[p].board[target_pos])
                                .has(Tribe::Quilboar);
                            if quilboar {
                                self.players[p].board[target_pos].keywords.set(keyword);
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
                    if pt.tavern && !pt.by_minion && !cast_triggers_done {
                        self.trigger_on_cast_spell(p);
                    }
                    if !target_removed && target_pos < self.players[p].board.len() {
                        // "Whenever you cast a spell on" cards and the
                        // target's own watcher are reactions to a spell the
                        // player cast: a minion's cast for the player fires
                        // none of them.
                        if !pt.by_minion {
                            self.trigger_on_cast_spell_on_target(p, target_pos);
                        }
                        if gems_played > 0 {
                            // Every Gem played on the target, from however
                            // the cast got them there: a Geomagus Roogug
                            // reacts to each one (docs/rules/design.md
                            // "Geomagus Roogug").
                            self.roogug_reacts(p, target_pos, gems_played);
                        }
                        let is_mech = self
                            .data
                            .cards
                            .tribes_of(&self.players[p].board[target_pos])
                            .has(Tribe::Mech);
                        if is_mech && !pt.by_minion {
                            self.trigger_on_cast_spell_on_mech(p, target_pos);
                        }
                    }
                    if let Some(gen_c) = generated {
                        self.give_generated_tracking(p, gen_c, &mut target_pos);
                    }
                    // Lovely Locket: after each cast on a friendly minion the
                    // trinket casts the same spell on another one, picked at
                    // random and ignoring the spell's own target rules
                    // (docs/rules/trinkets.md). One copy per cast, so a Gem
                    // that casts three times gets three copies. The copy is a
                    // real cast, and it never chains.
                    if self.players[p].tp.locket && !self.players[p].tp.locket_copy {
                        let n = self.players[p].board.len();
                        if n > 1 {
                            let hit = target_pos.min(n - 1);
                            let k = self.rng.below((n - 1) as u32) as usize;
                            let at = if k < hit { k } else { k + 1 };
                            self.players[p].tp.locket_copy = true;
                            let copy = PendingTarget {
                                source: None,
                                kind: TargetKind::Spell(st),
                                runs: 1,
                                tavern: pt.tavern,
                                gem: pt.gem,
                                by_minion: pt.by_minion,
                            };
                            self.apply_target(p, &copy, at);
                            self.players[p].tp.locket_copy = false;
                        }
                    }
                    // This cast has resolved: its "after you cast a spell"
                    // triggers fire now, before the next cast of the play. A
                    // cast that still has something open (a Discover it
                    // opened, a choice, a discard) keeps them owed:
                    // `fire_cast_after_choice` runs whatever is left once the
                    // player has answered.
                    if owe_after
                        && self.players[p].cast_after_choice.is_some_and(|w| w.casts > 0)
                        && !Game::other_choice_open(&self.players[p])
                        && let Some(mut what) = self.players[p].cast_after_choice
                    {
                        self.fire_cast_after_choice(p, CastAfterChoice { casts: 1, ..what });
                        what.casts -= 1;
                        self.players[p].cast_after_choice = (what.casts > 0).then_some(what);
                    }
                    if stop_after {
                        break;
                    }
                }
                // A Blood Gem cast from the hand (Ruby Tusk counts them).
                if pt.gem && pt.source.is_none() {
                    self.trinket_event(p, Event::Gem, None, 1);
                }
            }
        }
    }

    /// Arcane Alteration on a Tavern card: a random card of the same Tier
    /// takes the slot, and whatever buffs the old card carried are the new
    /// one's too (docs/rules/heroes.md). A spell is replaced by a Tavern spell of
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

    /// Makeshift Master: watch the minion at `at`, in the Tavern (`tavern`)
    /// or on the board, and remember the stats it has now. One minion is
    /// watched at a time, so any earlier mark comes off.
    fn set_deity_watch(&mut self, p: usize, at: usize, tavern: bool) {
        let pl = &mut self.players[p];
        for m in pl.board.iter_mut().chain(pl.hand.iter_mut()).chain(pl.shop.iter_mut()) {
            m.watched = false;
        }
        let m = if tavern { &mut pl.shop[at] } else { &mut pl.board[at] };
        m.watched = true;
        pl.tp.deity_watching = true;
        pl.tp.deity_watch_stats = (m.attack, m.health);
    }

    pub(super) fn resolve_target(&mut self, p: usize, s: usize) {
        let pt = self.players[p].pending_target.take().unwrap();
        // The target a repeated Battlecry trigger or cast picked: every repeat
        // lands on it (docs/rules/design.md "Set versus add").
        if let Some(q) = self.players[p].repeats.first_mut()
            && q.started
            && q.at.is_none()
            && q.source() == pt.source
        {
            q.at = Some(s as u8);
        }
        self.apply_target(p, &pt, s);
        self.maybe_start_discover(p);
    }

    /// Run the repeats a Battlecry trigger or a spell cast still owes, the
    /// ones in `Player.repeats`, until a prompt is left open. The first run of
    /// an entry is the one that asks, so the player answers it; every repeat
    /// lands on the target that run picked and takes the Choose One option the
    /// player picked there (docs/rules/design.md "Set versus add", Era
    /// 2026-11-04). An entry whose first run had no pick of its own (a spell a
    /// minion casts) picks a new option at random for each run. Called by
    /// `maybe_start_discover` and by whoever queues the first run.
    pub(super) fn run_queued_repeats(&mut self, p: usize) {
        // Nothing runs while the player still has a prompt to answer: the
        // driver picks up again when the answer lands.
        if self.players[p].repeating || Game::other_choice_open(&self.players[p]) {
            return;
        }
        self.players[p].repeating = true;
        while !self.players[p].repeats.is_empty() {
            let mut q = self.players[p].repeats[0];
            if q.runs == 0 {
                self.players[p].repeats.remove(0);
                continue;
            }
            q.runs -= 1;
            let first = !q.started;
            q.started = true;
            self.players[p].repeats[0] = q;
            let discovers_before = self.players[p].discover_queue.len();
            // One run of the entry.
            match q.what {
                RepeatWhat::Battlecry { slot } => {
                    if slot as usize >= self.players[p].board.len() {
                        self.players[p].repeats.remove(0);
                        continue;
                    }
                    self.battlecry_or_choose_one(p, slot as usize);
                }
                RepeatWhat::ChooseOne { source, options, both, tavern, by_minion } => {
                    if both {
                        self.resolve_choose_options(p, source, options, tavern, by_minion, true);
                    } else {
                        // The option the player picked for the first run, or
                        // a fresh random one for a cast that had no pick.
                        let Some(k) = self.repeat_choose_option(p, q.option, options) else {
                            self.players[p].repeats.remove(0);
                            continue;
                        };
                        self.apply_choose_option(p, source, options[k], tavern, by_minion);
                    }
                }
            }
            // A Discover this run opened is answered with a random card of
            // its pool, with no Discover offered, when the entry is a
            // Battlecry an Activate triggered (a Kelp Keeper), and that card
            // is not a Discover for "after you Discover" cards (docs/rules/
            // design.md "Shop replay answers", Era 2026-11-03 and 2026-11-04).
            if q.random_discover {
                let opened =
                    self.players[p].discover_queue.len().saturating_sub(discovers_before);
                self.players[p].repeat_discover =
                    self.players[p].repeat_discover.saturating_add(opened as u8);
            }
            // A Choose One a repeat opened takes the option the first run's
            // player picked, on the same target. Only a cast with no pick of
            // its own chooses at random.
            if !first
                && let Some(pco) = self.players[p].pending_choose_one
            {
                match self.repeat_choose_option(p, q.option, pco.options) {
                    Some(k) => self.resolve_choose(p, k),
                    None => self.players[p].pending_choose_one = None,
                }
            }
            // The first run's own target goes to the player; a repeat's is
            // answered here.
            if self.players[p].pending_target.is_some() {
                if first {
                    break;
                }
                self.answer_repeat_target(p, q.at);
            }
            if Game::other_choice_open(&self.players[p]) {
                break;
            }
        }
        self.players[p].repeating = false;
    }

    /// Answer the target a repeat asks for with the board or Tavern slot its
    /// entry's first run picked, or a random legal one when it picked none or
    /// that one is gone.
    fn answer_repeat_target(&mut self, p: usize, at: Option<u8>) {
        let Some(pt) = self.players[p].pending_target else { return };
        if let Some(s) = at {
            let s = s as usize;
            if pt.kind.takes_shop() {
                if s < self.players[p].shop.len() && self.valid_shop_target(p, &pt, s) {
                    self.players[p].pending_target = None;
                    self.apply_target_shop(p, &pt, s);
                    return;
                }
            } else if self.valid_target(p, &pt, s) {
                self.players[p].pending_target = None;
                self.apply_target(p, &pt, s);
                return;
            }
        }
        self.answer_target_at_random(p);
    }

    /// The option a repeat takes: the one the player picked for the entry's
    /// first run when there is one, and a random pickable one otherwise (a
    /// cast with no choices of its own, or a pick the board can no longer
    /// answer).
    fn repeat_choose_option(
        &mut self,
        p: usize,
        picked: Option<u8>,
        options: [ChooseOption; 2],
    ) -> Option<usize> {
        let k = picked.map(|k| k as usize).filter(|&k| k < 2);
        if let Some(k) = k
            && self.can_choose_option(p, options[k])
        {
            return Some(k);
        }
        self.random_choose_option(p, options)
    }

    /// One of a Choose One's options at random, out of those the board can
    /// answer: what a spell a minion casts picks (Era 2026-11-04: its casts
    /// get no choice, so the option is random).
    pub(super) fn random_choose_option(
        &mut self,
        p: usize,
        options: [ChooseOption; 2],
    ) -> Option<usize> {
        let mut ok: ArrayVec<usize, 2> = ArrayVec::new();
        for k in 0..2 {
            if self.can_choose_option(p, options[k]) {
                let _ = ok.try_push(k);
            }
        }
        if ok.is_empty() {
            return None;
        }
        Some(ok[self.rng.below(ok.len() as u32) as usize])
    }
}
