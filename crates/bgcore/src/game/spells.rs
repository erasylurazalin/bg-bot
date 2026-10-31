//! Casting spells, spell triggers and Blood Gems.

use arrayvec::ArrayVec;
use crate::trinkets::{CardMod, Event};
use crate::cards::{
    Aura,
    DiscoverKind,
    EventKind,
    OnCastSpell,
    OnCastSpellOnMech,
    SpellEffect,
    TimesRole,
    Tribe,
    Tribes,
    how_many_times,
    pick_one_per_type,
};
use crate::minion::{
    CardId,
    Keywords,
    Minion,
};
use crate::{
    BOARD,
    MAX_TIER,
};
use super::{
    DiscoverRequest,
    Game,
    PendingChooseOne,
    PendingTarget,
    TargetKind,
    TavernBuff,
};
use super::effects::buff;

impl Game {
    /// Total spell stat bonus for player `p`, combining Intrepid Botanist's
    /// permanent bonus with board auras (Enchanted Sentinel, including when
    /// Magnetized into a host, and Humon'gozz).
    pub fn total_spell_bonus(&self, p: usize) -> (i32, i32) {
        self.players[p].total_spell_bonus(&self.data.cards)
    }

    pub(super) fn trigger_on_cast_spell(&mut self, p: usize) {
        let len = self.players[p].board.len();
        for i in 0..len {
            let m = self.players[p].board[i];
            for (card, golden) in m.parts() {
                match self.data.cards.get(card).on_cast_spell {
                    Some(OnCastSpell::BuffDivineShield { attack, health }) => {
                        let mul = if golden { 2 } else { 1 };
                        // Czarina Portrait: the Charging Czarinas also give
                        // Health, as much as their Attack.
                        let also_health = matches!(
                            self.player_card_mod(p, card),
                            Some(CardMod::AlsoHealth)
                        );
                        for o in &mut self.players[p].board {
                            if o.keywords.has(Keywords::DIVINE_SHIELD) {
                                buff(o, attack * mul, health * mul);
                                if also_health {
                                    buff(o, 0, attack * mul);
                                }
                            }
                        }
                    }
                    Some(OnCastSpell::UndeadBonus { attack, health }) => {
                        let mul = if golden { 2 } else { 1 };
                        self.add_undead_bonus(p, attack * mul, health * mul, true);
                    }
                    Some(OnCastSpell::BuffSelfAndDeity { attack, health }) => {
                        let mul = if golden { 2 } else { 1 };
                        buff(&mut self.players[p].board[i], attack * mul, health * mul);
                        // Mindslasher Portrait: the Vicious Mindslashers also
                        // give their stats to the minions beside them.
                        if matches!(self.player_card_mod(p, card), Some(CardMod::AlsoNeighbors)) {
                            for j in [i.checked_sub(1), Some(i + 1)].into_iter().flatten() {
                                if j < self.players[p].board.len() {
                                    buff(&mut self.players[p].board[j], attack * mul, health * mul);
                                }
                            }
                        }
                        if let Some(deity) = &mut self.players[p].deity {
                            deity.attack += attack * mul;
                            deity.health += health * mul;
                        }
                    }
                    Some(OnCastSpell::BuffAllAndDeity { attack, health }) => {
                        let mul = if golden { 2 } else { 1 };
                        for o in &mut self.players[p].board {
                            buff(o, attack * mul, health * mul);
                        }
                        if let Some(deity) = &mut self.players[p].deity {
                            deity.attack += attack * mul;
                            deity.health += health * mul;
                        }
                    }
                    Some(OnCastSpell::BuffAll { attack, health }) => {
                        // A golden one gives the same buff twice.
                        let times = if golden { 2 } else { 1 };
                        for _ in 0..times {
                            for o in &mut self.players[p].board {
                                buff(o, attack, health);
                            }
                        }
                    }
                    None => {}
                }
            }
        }
    }

    pub(super) fn trigger_on_cast_spell_on_mech(&mut self, p: usize, target_pos: usize) {
        let len = self.players[p].board.len();
        for i in 0..len {
            let m = self.players[p].board[i];
            for (card, golden) in m.parts() {
                if let Some(OnCastSpellOnMech::MagnetizeSatellite {
                    card: sat_id,
                    attack,
                    health,
                }) = self.data.cards.get(card).on_cast_spell_on_mech
                {
                    let count = if golden { 2 } else { 1 };
                    for _ in 0..count {
                        let mut sat = self.data.cards.make(sat_id, false);
                        sat.attack = attack;
                        sat.health = health;
                        self.magnetize_to(p, target_pos, sat, false);
                    }
                }
            }
        }
    }

    /// A spell was cast on the board minion in `target_pos`: run the
    /// "whenever you cast a spell on a Murloc" watchers (Shamanic
    /// Tidecaller), then the target's own watcher (Twilight Tidehunter).
    pub(super) fn trigger_on_cast_spell_on_target(&mut self, p: usize, target_pos: usize) {
        if target_pos >= self.players[p].board.len() {
            return;
        }
        // A trinket that watches spells cast on a friendly minion.
        self.trinket_event(p, Event::CastSpellOnMinion, None, 1);
        let target_tribes = self.data.cards.get(self.players[p].board[target_pos].card).tribes;
        let len = self.players[p].board.len();
        for i in 0..len {
            if i >= self.players[p].board.len() {
                break;
            }
            let m = self.players[p].board[i];
            for (card, golden) in m.parts() {
                let (on_tribe, on_self) = {
                    let c = self.data.cards.get(card);
                    (c.on_cast_spell_on_tribe, c.on_cast_spell_on_self)
                };
                if let Some((tribe, e)) = on_tribe
                    && target_tribes.has(tribe)
                {
                    let mut pos = i;
                    self.run_effect(p, &mut pos, e, golden, 1);
                }
                if i == target_pos
                    && let Some(e) = on_self
                {
                    let mut pos = i;
                    self.run_effect(p, &mut pos, e, golden, 1);
                }
            }
        }
    }

    /// Cast a named spell on the friendly minion in board slot `target`,
    /// for a trinket that casts one itself (Emergency Gearblade,
    /// Spell-powered Wrench). It is a real cast: it counts for the cast
    /// triggers, the spell counters and Balinda's extra cast. A card with no
    /// minion target is cast the ordinary way, so it may open a prompt.
    pub(super) fn cast_spell_on(&mut self, p: usize, card: CardId, target: usize) {
        if target >= self.players[p].board.len() {
            return;
        }
        let c = self.data.cards.get(card);
        let Some(st) = c.spell_target else {
            let m = self.data.cards.make(card, false);
            self.cast_spell(p, m);
            return;
        };
        let tavern = c.tavern_spell;
        self.players[p].last_spell = Some(card);
        let pt = PendingTarget {
            source: None,
            kind: TargetKind::Spell(st),
            runs: self.balinda_mult(p),
            tavern,
            gem: false,
        };
        self.apply_target(p, &pt, target);
        if self.players[p].tp.first_spell_double > 0 {
            self.players[p].tp.first_spell_double -= 1;
        }
        self.fire_event(p, EventKind::AllSpellsCast, 1);
        self.trinket_event(p, Event::SpellCast, Some(card), 1);
        if tavern {
            self.fire_event(p, EventKind::SpellsCast, 1);
            self.players[p].spells_cast_turn = self.players[p].spells_cast_turn.saturating_add(1);
            self.trinket_event(p, Event::CastSpell, Some(card), 1);
        }
        self.maybe_start_discover(p);
    }

    /// Get `count` 1/1 Murloc tokens taught `spell` (Magicfin Sticker's
    /// "get a 1/1 Murloc and teach it that spell").
    pub(super) fn teach_murloc(&mut self, p: usize, spell: CardId, count: u8) {
        let Some(token) = self.data.cards.taught_token else { return };
        for _ in 0..count {
            if self.players[p].hand.is_full() {
                return;
            }
            let mut m = self.players[p].make_card(&self.data.cards, token, false, 0);
            m.taught_spell = Some(spell);
            self.add_to_hand(p, m);
        }
    }

    /// After buying a Tavern spell, a Magicfin Mycologist on the board
    /// teaches a 1/1 Murloc token that spell, once per turn each: the first
    /// one left to right that still has its use for this turn.
    pub(super) fn magicfin_teach(&mut self, p: usize, spell: CardId) {
        let Some(token) = self.data.cards.taught_token else { return };
        let cards = &self.data.cards;
        let Some(i) = (0..self.players[p].board.len()).find(|&i| {
            self.players[p].board[i].uses_left > 0
                && self.players[p].board[i].parts().any(|(c, _)| cards.get(c).teaches_spell)
        }) else { return };
        if self.players[p].hand.is_full() {
            return;
        }
        self.players[p].board[i].uses_left -= 1;
        let mut m = self.players[p].make_card(cards, token, false, 0);
        m.taught_spell = Some(spell);
        self.add_to_hand(p, m);
    }

    /// "Set versus add, as one mechanic" (DESIGN.md): every "how many times"
    /// count is the highest value a setter on the board sets (1 if none),
    /// plus what each adder adds. See `cards::how_many_times`.
    fn times_count(&self, p: usize, role: impl Fn(Aura) -> Option<TimesRole> + Copy) -> u8 {
        let cards = &self.data.cards;
        let parts = self.players[p].board.iter().flat_map(|m| m.parts());
        how_many_times(cards, parts, role)
    }

    pub(super) fn brann_mult(&self, p: usize) -> u8 {
        self.times_count(p, |a| (a == Aura::Brann).then_some(TimesRole::Setter))
    }

    pub(super) fn drakkari_mult(&self, p: usize) -> u8 {
        let n = self.times_count(p, |a| (a == Aura::Drakkari).then_some(TimesRole::Setter));
        // Ghastly Sticker: one more time for the player's end-of-turn
        // effects, on top of a Drakkari (they add, unlike two setters).
        if self.players[p].tp.end_of_turn_ghastly { n.saturating_add(1) } else { n }
    }

    /// How many times the player's Bounties cast: once, twice with a Proud
    /// Privateer, three times with a Golden one. "Twice" is a fixed count, so
    /// two Privateers don't stack (the highest wins).
    fn bounty_mult(&self, p: usize) -> u8 {
        self.times_count(p, |a| (a == Aura::ProudPrivateer).then_some(TimesRole::Setter))
    }

    pub(super) fn balinda_mult(&self, p: usize) -> u8 {
        let n = self.times_count(p, |a| (a == Aura::Balinda).then_some(TimesRole::Setter));
        // Replica Cathedral: the first Tavern spell of the turn casts one
        // more time, on top of a Balinda (as its own extra cast, not a
        // setter: two Balindas still cast twice).
        if self.players[p].tp.first_spell_double > 0 { n.saturating_add(1) } else { n }
    }

    /// How many times a Blood Gem cast from hand on a friendly minion plays:
    /// Balinda sets the count (golden: 3), each Hot-Air Surveyor adds one
    /// (golden: two). Balinda plus one Surveyor: 3 casts.
    fn gem_casts(&self, p: usize) -> u8 {
        let n = self.times_count(p, |a| match a {
            Aura::Balinda => Some(TimesRole::Setter),
            Aura::HotAirSurveyor => Some(TimesRole::Adder),
            _ => None,
        });
        // Replica Cathedral: a Gem is a spell, so the first one of the turn
        // plays once more.
        if self.players[p].tp.first_spell_double > 0 { n.saturating_add(1) } else { n }
    }

    /// Spend one Thorned Trailblazer combine: the first one left to right
    /// that still has a use this turn, so none is wasted.
    pub(super) fn take_trailblazer_cover(&mut self, p: usize) -> bool {
        let cards = &self.data.cards;
        let Some(i) = (0..self.players[p].board.len()).find(|&i| {
            self.players[p].board[i].uses_left > 0
                && cards.get(self.players[p].board[i].card).choose_one_cover > 0
        }) else { return false };
        self.players[p].board[i].uses_left -= 1;
        true
    }

    /// Get `n` Blood Gems in hand; the ones past a full hand are lost.
    pub(super) fn give_gems(&mut self, p: usize, n: u8) {
        let Some(gem) = self.data.cards.blood_gem else { return };
        for _ in 0..n {
            self.give_generated(p, gem);
        }
    }

    /// Play `times` Blood Gems on a friendly minion (the player's Gem value,
    /// tracked as Blood Gem stats). Shop only: a fight plays its Gems through
    /// the fight's own counters.
    pub(super) fn play_gems_on_board(&mut self, p: usize, pos: usize, times: u8) {
        let (ga, gh) = (self.players[p].gem_attack, self.players[p].gem_health);
        if let Some(m) = self.players[p].board.get_mut(pos) {
            m.attack += ga * times as i32;
            m.health += gh * times as i32;
            m.gem_attack += ga * times as i32;
            m.gem_health += gh * times as i32;
        }
    }

    /// A Blood Gem was just cast from hand onto the friendly minion in
    /// `target`: every Geomagus Roogug there plays its Blood Gems on a random
    /// different friendly minion. Those Gems are not from hand, so two
    /// Roogugs never loop.
    pub(super) fn roogug_on_gem(&mut self, p: usize, target: usize) {
        let Some((count, golden)) = self.players[p].board.get(target).and_then(|m| {
            m.parts().find_map(|(c, g)| self.data.cards.get(c).on_gem_played_on_self.map(|n| (n, g)))
        }) else {
            return;
        };
        let times = count * (if golden { 2 } else { 1 });
        let n = self.players[p].board.len();
        if n <= 1 {
            return;
        }
        for _ in 0..times {
            // A different friendly minion, uniformly at random.
            let k = self.rng.below((n - 1) as u32) as usize;
            let at = if k < target { k } else { k + 1 };
            self.play_gems_on_board(p, at, 1);
        }
    }

    /// Pay for a spell bought from the Tavern. A spell bought with Health
    /// (Hasty Excavation) is hero damage, not Gold; a Gold purchase uses up
    /// the next-spell discount.
    pub(super) fn pay_for_spell(&mut self, p: usize, pay_health: bool, cost: u8) {
        if pay_health {
            self.take_shop_damage(p, cost as i32, None);
        } else {
            let paid = cost.saturating_sub(self.players[p].spell_discount);
            self.players[p].gold = self.players[p].gold.saturating_sub(paid);
            self.players[p].spell_discount = 0;
            self.fire_event(p, EventKind::GoldSpent, paid as u32);
        }
    }

    pub(super) fn cast_spell(&mut self, p: usize, m: Minion) {
        // A Bounty cast again (Proud Privateer) is another spell cast for
        // every trigger and every "spells cast" counter.
        let c = self.data.cards.get(m.card);
        let casts = if c.is_bounty && c.spell_effect.is_some() { self.bounty_mult(p) } else { 1 };
        let tavern = c.tavern_spell;
        // A Triple Reward is a reward you open, not a spell you cast
        // (DESIGN.md "Read the card's exact words, every time").
        let reward = matches!(c.spell_effect, Some(SpellEffect::TripleReward));
        self.cast_spell_inner(p, m);
        if reward {
            return;
        }
        // Replica Cathedral: its doubling is spent by the first spell of the
        // turn, Tavern spell or not: the cast itself read it already.
        if self.players[p].tp.first_spell_double > 0 {
            self.players[p].tp.first_spell_double -= 1;
        }
        // Every spell is a spell cast: a Tavern spell, a Blood Gem, a Gem
        // Day, a Banana, a Spellcraft spell (DESIGN.md). "Tavern spell" is
        // the narrower one, and only those touch the Tavern spell counters.
        for _ in 0..if tavern { casts } else { 1 } {
            self.fire_event(p, EventKind::AllSpellsCast, 1);
        }
        self.trinket_event(p, Event::SpellCast, Some(m.card), 1);
        if tavern {
            for _ in 0..casts {
                self.fire_event(p, EventKind::SpellsCast, 1);
            }
            self.players[p].spells_cast_turn = self.players[p].spells_cast_turn.saturating_add(1);
            self.trinket_event(p, Event::CastSpell, Some(m.card), 1);
        }
    }

    fn cast_spell_inner(&mut self, p: usize, m: Minion) {
        self.players[p].last_spell = Some(m.card);
        let c = self.data.cards.get(m.card);
        let tavern = c.tavern_spell;
        if let Some(st) = c.spell_target {
            // A spell cast outside `Action::Play` (a taught spell) can have no
            // legal target; it fizzles instead of waiting on a choice that
            // could never be made.
            let castable =
                (0..self.players[p].board.len()).any(|s| self.valid_spell_target(p, st, s))
                    || (st.can_target_shop()
                        && (0..self.players[p].shop.len())
                            .any(|s| self.spell_target_ok(st, &self.players[p].shop[s])));
            if !castable {
                return;
            }
            // A Blood Gem plays once per cast: Balinda sets the count and
            // each Hot-Air Surveyor adds one ("Set versus add").
            let (runs, gem) = if c.blood_gem {
                (self.gem_casts(p), true)
            } else {
                (self.balinda_mult(p), false)
            };
            self.players[p].pending_target = Some(PendingTarget {
                source: None,
                kind: TargetKind::Spell(st),
                runs,
                tavern,
                gem,
            });
        } else if let Some(options) = c.choose_one {
            if !options.iter().any(|&opt| self.can_choose_option(p, opt)) {
                return;
            }
            // A Trailblazer's coverage runs both effects instead of asking,
            // and Trailblazer Sticker covers every Choose One card.
            if self.players[p].tp.choose_one_all || self.take_trailblazer_cover(p) {
                self.resolve_choose_options(p, None, options, tavern, true);
            } else {
                self.players[p].pending_choose_one = Some(PendingChooseOne {
                    source: None,
                    golden: false,
                    options,
                    tavern,
                });
            }
        } else if let Some(se) = c.spell_effect {
            if matches!(se, SpellEffect::TripleReward) {
                // The tier was fixed when the reward was given.
                let tier = (m.improvements as u8).clamp(1, MAX_TIER);
                let _ = self.players[p]
                    .discover_queue
                    .try_push(DiscoverRequest::MinionTier(tier));
                self.maybe_start_discover(p);
                return;
            }
            if matches!(se, SpellEffect::DiscoverHeroPower) {
                // The exclusion list lives on the card, so the intercept runs
                // here where the card is known.
                let excludes = c.exclude_heroes.clone();
                if tavern {
                    self.trigger_on_cast_spell(p);
                }
                self.discover_hero_power(p, &excludes);
                return;
            }
            // Proud Privateer: a Bounty casts again, each cast a spell cast.
            // Replica Cathedral's extra cast lands here for a spell with no
            // target; a targeting or Choose One spell gets it from
            // `balinda_mult` instead.
            let mut casts = if c.is_bounty { self.bounty_mult(p) } else { 1 };
            if self.players[p].tp.first_spell_double > 0 {
                casts = casts.saturating_add(1);
            }
            for _ in 0..casts {
                if tavern {
                    self.trigger_on_cast_spell(p);
                }
                self.execute_spell_effect(p, se, tavern);
            }
            self.maybe_start_discover(p);
        }
    }

    /// Run a spell effect (or a Choose One option). `tavern` says whether the
    /// spell cast is a Tavern spell: only those get spell power.
    pub(super) fn execute_spell_effect(&mut self, p: usize, se: SpellEffect, tavern: bool) {
        let (da, dh) = if tavern { self.total_spell_bonus(p) } else { (0, 0) };
        match se {
            SpellEffect::BuffDeity { attack, health } => {
                if let Some(deity) = &mut self.players[p].deity {
                    deity.attack += attack + da;
                    deity.health += health + dh;
                }
            }
            SpellEffect::GainGold(g) => {
                self.players[p].gold = self.players[p].gold.saturating_add(g);
            }
            SpellEffect::GoldNextTurn(g) => {
                self.players[p].gold_next_turn = self.players[p].gold_next_turn.saturating_add(g);
            }
            SpellEffect::MaxGold(g) => {
                self.players[p].extra_max_gold = self.players[p].extra_max_gold.saturating_add(g);
            }
            SpellEffect::FreeRolls(n) => {
                self.players[p].free_rolls = self.players[p].free_rolls.saturating_add(n);
            }
            SpellEffect::Overconfidence { win, tie } => {
                self.players[p].overconfidence_win =
                    self.players[p].overconfidence_win.saturating_add(win);
                self.players[p].overconfidence_tie =
                    self.players[p].overconfidence_tie.saturating_add(tie);
            }
            SpellEffect::BuffShop { attack, health } => {
                let cards = &self.data.cards;
                for m in &mut self.players[p].shop {
                    if !cards.get(m.card).is_spell() {
                        buff(m, attack + da, health + dh);
                    }
                }
            }
            SpellEffect::BuffBoard { attack, health, times, golden_extra } => {
                for _ in 0..times {
                    for m in &mut self.players[p].board {
                        buff(m, attack + da, health + dh);
                        if golden_extra && m.golden {
                            buff(m, attack + da, health + dh);
                        }
                    }
                }
            }
            SpellEffect::BuffRandomBoard { count, attack, health } => {
                let mut indices: ArrayVec<usize, BOARD> =
                    (0..self.players[p].board.len()).collect();
                self.rng.shuffle(&mut indices);
                for &i in indices.iter().take(count as usize) {
                    buff(&mut self.players[p].board[i], attack + da, health + dh);
                }
            }
            SpellEffect::NextTurnBoardBuff { attack, health, times } => {
                let _ = self.players[p]
                    .next_turn_board_buffs
                    .try_push((attack, health, times));
            }
            SpellEffect::RandomTierMinion(tier) => {
                if let Some(c) = self.data.cards.random_tier_minion(tier, self.lobby, &mut self.rng) {
                    self.give_generated(p, c);
                }
            }
            SpellEffect::Discover(kind) => {
                let tier = self.players[p].tier;
                let req = match kind {
                    DiscoverKind::Tier1Minion => DiscoverRequest::MinionTier(1),
                    DiscoverKind::MyTierMinion => DiscoverRequest::MinionTier(tier),
                    DiscoverKind::MyTierSpell => DiscoverRequest::SpellTier(tier),
                    DiscoverKind::DeathrattleMinion => DiscoverRequest::DeathrattleMinion(tier),
                    DiscoverKind::BattlecryMinion => DiscoverRequest::BattlecryMinion(tier),
                    DiscoverKind::Tier7Minion => DiscoverRequest::MinionTier(crate::CARD_MAX_TIER),
                    DiscoverKind::ChooseOneMinion => DiscoverRequest::ChooseOneMinion,
                    DiscoverKind::TierMinion(t) => DiscoverRequest::MinionTier(t),
                    DiscoverKind::TierMinionWithType(t) => DiscoverRequest::TypedMinionTier(t),
                };
                let _ = self.players[p].discover_queue.try_push(req);
            }
            SpellEffect::TombTurning => {
                let _ = self.players[p]
                    .discover_queue
                    .try_push(DiscoverRequest::DoomedTribeMinion(Tribe::Undead));
            }
            SpellEffect::SetArmor(n) => {
                self.players[p].armor = n as i32;
            }
            SpellEffect::DiscoverHeroPower => {}
            SpellEffect::MightyDragonbreath { attack, health } => {
                let cards = &self.data.cards;
                for pass in 0..3 {
                    for o in &mut self.players[p].board {
                        let ok = match pass {
                            0 => true,
                            1 => cards.get(o.card).tribes.has(Tribe::Dragon),
                            _ => o.keywords.has(Keywords::DIVINE_SHIELD),
                        };
                        if ok {
                            buff(o, attack + da, health + dh);
                        }
                    }
                }
            }
            SpellEffect::BoonOfBeetles(n) => {
                self.players[p].boon_of_beetles =
                    self.players[p].boon_of_beetles.saturating_add(n);
            }
            SpellEffect::BroodOfNozdormu => {
                self.players[p].brood_of_nozdormu =
                    self.players[p].brood_of_nozdormu.saturating_add(1);
            }
            SpellEffect::CloningConch => {
                let tier = self.players[p].tier;
                if let Some(c) =
                    self.data.cards.random_pool_minion(Tribe::Murloc, tier, self.lobby, &mut self.rng)
                {
                    self.give_generated(p, c);
                    self.give_generated(p, c);
                }
            }
            SpellEffect::GoldenTouch => {
                let cards = &self.data.cards;
                let shop = &mut self.players[p].shop;
                let ok = |m: &Minion| !m.golden && !cards.get(m.card).fixed_stats;
                let n = shop.iter().filter(|m| ok(m)).count() as u32;
                if n > 0 {
                    let k = self.rng.below(n) as usize;
                    let m = shop.iter_mut().filter(|m| ok(m)).nth(k).unwrap();
                    let base = cards.get(m.card);
                    let (ga, gh) =
                        base.golden_stats.unwrap_or((base.attack * 2, base.health * 2));
                    m.golden = true;
                    m.attack += ga - base.attack;
                    m.health += gh - base.health;
                }
            }
            SpellEffect::SharingIsCaring => {
                self.players[p].sharing_is_caring =
                    self.players[p].sharing_is_caring.saturating_add(1);
            }
            SpellEffect::MenagerieTableware { attack, health } => {
                let cards = &self.data.cards;
                let types = Tribe::EVERY
                    .iter()
                    .filter(|t| self.players[p].board.iter().any(|m| cards.get(m.card).tribes.has(**t)))
                    .count();
                let times = 1 + types;
                for _ in 0..times {
                    for m in &mut self.players[p].board {
                        buff(m, attack + da, health + dh);
                    }
                }
            }
            SpellEffect::UpperHand => {
                self.players[p].upper_hand = self.players[p].upper_hand.saturating_add(1);
            }
            SpellEffect::BuffTavernPermanent { attack, health } => {
                let b = TavernBuff {
                    tribe: None,
                    attack: attack + da,
                    health: health + dh,
                    this_turn: false,
                };
                self.players[p].add_tavern_buff(&self.data.cards, b);
            }
            SpellEffect::MisplacedTeaSet { attack, health } => {
                // One random unbuffed minion of each type on the board; a
                // dual or all-type minion can only be picked once.
                let cards = &self.data.cards;
                let board = &mut self.players[p].board;
                let tribes: ArrayVec<Tribes, BOARD> =
                    board.iter().map(|m| cards.get(m.card).tribes).collect();
                for i in pick_one_per_type(&tribes, &mut self.rng) {
                    buff(&mut board[i], attack + da, health + dh);
                }
            }
            SpellEffect::PlanarTelescope => {
                let cards = &self.data.cards;
                let mut counts = [0u8; Tribe::EVERY.len()];
                for m in &self.players[p].board {
                    let t = cards.get(m.card).tribes;
                    for (k, tribe) in Tribe::EVERY.iter().enumerate() {
                        // only a lobby tribe can be discovered from, so only
                        // lobby tribes count toward the most common one
                        if self.lobby.has(*tribe) && t.has(*tribe) {
                            counts[k] += 1;
                        }
                    }
                }
                let Some(max) = counts.iter().copied().max().filter(|&n| n > 0) else {
                    return;
                };
                let mut tied: ArrayVec<Tribe, { Tribe::EVERY.len() }> = ArrayVec::new();
                for (k, tribe) in Tribe::EVERY.iter().enumerate() {
                    if counts[k] == max {
                        let _ = tied.try_push(*tribe);
                    }
                }
                let k = self.rng.below(tied.len() as u32) as usize;
                let _ = self.players[p]
                    .discover_queue
                    .try_push(DiscoverRequest::TribeMinion(tied[k]));
            }
            SpellEffect::StealRandomTavern => self.steal_random_tavern(p),
            SpellEffect::DiscoverLockedMinion => {
                let tier = self.players[p].tier;
                let _ = self.players[p]
                    .discover_queue
                    .try_push(DiscoverRequest::SearchMinionTier(tier));
            }
            SpellEffect::SaloonFinest => {
                let Game { data, rng, players, pool, lobby, .. } = self;
                Self::refresh_with_spells(&mut players[p], &data.cards, &data.rules, rng, pool, *lobby);
            }
            SpellEffect::RefreshWithBattlecries(price) => {
                let Game { data, rng, players, pool, lobby, .. } = self;
                Self::refresh_with_battlecries(
                    &mut players[p],
                    pool,
                    &data.cards,
                    &data.rules,
                    rng,
                    *lobby,
                    price,
                );
                // An effect that says "Refresh" is one for the cards that
                // count Refreshes (DESIGN.md, "Two kinds of Refresh").
                self.trinket_event(p, crate::trinkets::Event::Refresh, None, 1);
            }
            SpellEffect::GetCard { card, count } => {
                for _ in 0..count {
                    self.give_generated(p, card);
                }
            }
            // A Lockbox can't be cast.
            SpellEffect::Lockbox(_) => {}
            SpellEffect::BuffLeftmost { attack, health } => {
                if let Some(m) = self.players[p].board.first_mut() {
                    buff(m, attack + da, health + dh);
                }
            }
            SpellEffect::RefreshBuff { attack, health } => {
                let pl = &mut self.players[p];
                pl.refresh_buff.0 += attack + da;
                pl.refresh_buff.1 += health + dh;
            }
            SpellEffect::GetTwo { a, b } => {
                self.give_generated(p, a);
                self.give_generated(p, b);
            }
            SpellEffect::FriendlyBounty => {
                let cards = &self.data.cards;
                let mut counts = [0u8; Tribe::EVERY.len()];
                for m in &self.players[p].board {
                    let t = cards.get(m.card).tribes;
                    for (k, tribe) in Tribe::EVERY.iter().enumerate() {
                        // only a lobby tribe can be drawn from, so only lobby
                        // tribes count toward the most common one
                        if self.lobby.has(*tribe) && t.has(*tribe) {
                            counts[k] += 1;
                        }
                    }
                }
                let Some(max) = counts.iter().copied().max().filter(|&n| n > 0) else {
                    return;
                };
                let mut tied: ArrayVec<Tribe, { Tribe::EVERY.len() }> = ArrayVec::new();
                for (k, tribe) in Tribe::EVERY.iter().enumerate() {
                    if counts[k] == max {
                        let _ = tied.try_push(*tribe);
                    }
                }
                let tribe = tied[self.rng.below(tied.len() as u32) as usize];
                let tier = self.players[p].tier;
                if let Some(c) =
                    self.data.cards.random_pool_minion(tribe, tier, self.lobby, &mut self.rng)
                {
                    self.give_generated(p, c);
                }
            }
            SpellEffect::GetGems(n) => self.give_gems(p, n),
            SpellEffect::GemBonus { attack, health } => {
                let pl = &mut self.players[p];
                pl.gem_attack += attack;
                pl.gem_health += health;
            }
            SpellEffect::GemsOnBoard(n) => {
                // Gems played by an effect, not from hand: no Roogug chain.
                let (ga, gh) = (self.players[p].gem_attack, self.players[p].gem_health);
                for m in &mut self.players[p].board {
                    m.attack += ga * n as i32;
                    m.health += gh * n as i32;
                    m.gem_attack += ga * n as i32;
                    m.gem_health += gh * n as i32;
                }
            }
            SpellEffect::Barrage { attack, health } => {
                // The Gem bonus (what a Gem gives above its base +1/+1) and
                // the Tavern spell bonus land once per cast, and each cast
                // adds on top for the coming Refreshes.
                let (ga, gh) = (self.players[p].gem_attack - 1, self.players[p].gem_health - 1);
                let pl = &mut self.players[p];
                pl.barrage.0 += attack + ga + da;
                pl.barrage.1 += health + gh + dh;
            }
            SpellEffect::CastSpell { card, times } => {
                for _ in 0..times {
                    let m = self.data.cards.make(card, false);
                    self.cast_spell(p, m);
                    if self.players[p].pending_target.is_some()
                        || self.players[p].pending_choose_one.is_some()
                    {
                        break;
                    }
                }
            }
            SpellEffect::RandomTribeMinion { tribe, count } => {
                let tier = self.players[p].tier;
                for _ in 0..count {
                    if let Some(c) =
                        self.data.cards.random_pool_minion(tribe, tier, self.lobby, &mut self.rng)
                    {
                        self.give_generated(p, c);
                    }
                }
            }
            // Handled where the spell is cast (the tier lives on the copy).
            SpellEffect::TripleReward => {}
        }
    }
}
