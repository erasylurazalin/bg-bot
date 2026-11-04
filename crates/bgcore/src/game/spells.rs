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
    TriggerWhen,
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
    AfterCastTriggers,
    CastAfterChoice,
    DiscoverRequest,
    Game,
    PendingChooseOne,
    PendingTarget,
    QueuedRepeat,
    RefreshKind,
    RepeatWhat,
    TargetKind,
    TavernBuff,
};
use super::effects::buff;
use super::shop::{draw_filtered, give_back};

impl Game {
    /// Total spell stat bonus for player `p`, combining Intrepid Botanist's
    /// permanent bonus with board auras (Enchanted Sentinel, including when
    /// Magnetized into a host, and Humon'gozz).
    pub fn total_spell_bonus(&self, p: usize) -> (i32, i32) {
        self.players[p].total_spell_bonus(&self.data.cards)
    }

    /// The board's "whenever you cast a Tavern spell" triggers, or with
    /// `after` the "after you cast a Tavern spell" ones: the slot the card
    /// sits in, the card and whether it is golden. The "after" ones are
    /// gathered as the spell is cast and fired once it has resolved, so one
    /// the spell itself takes off the board still fires (Butchering on a
    /// Forsaken Weaver, docs/notes/UNSURE.md).
    pub(crate) fn gather_on_cast_spell(&self, p: usize, after: bool) -> AfterCastTriggers {
        let mut out = AfterCastTriggers::default();
        for (i, m) in self.players[p].board.iter().enumerate() {
            for (card, golden) in m.parts() {
                let c = self.data.cards.get(card);
                let has = if after { c.on_cast_spell_after } else { c.on_cast_spell };
                if has.is_some() {
                    out.push(i as u8, card, golden);
                }
            }
        }
        out
    }

    /// One of the triggers `gather_on_cast_spell` found.
    fn apply_on_cast_spell(&mut self, p: usize, (host, card, golden): (u8, CardId, bool), after: bool) {
        let i = host as usize;
        let c = self.data.cards.get(card);
        let trigger = if after { c.on_cast_spell_after } else { c.on_cast_spell };
        match trigger {
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
                // The minion may be gone by now (a spell that destroys its
                // own caster's card): its own stats are lost with it, the
                // Deity's are not.
                let there = i < self.players[p].board.len()
                    && self.players[p].board[i].parts().any(|(o, _)| o == card);
                if there {
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

    /// The board's "whenever you cast a Tavern spell" cards fire as the spell
    /// is cast; `TriggerWhen::Resolved` fires the "after you cast a Tavern
    /// spell" cards of the board as it stands now.
    pub(super) fn trigger_on_cast_spell(&mut self, p: usize, when: TriggerWhen) {
        let after = when == TriggerWhen::Resolved;
        let triggers = self.gather_on_cast_spell(p, after);
        for entry in triggers.iter() {
            self.apply_on_cast_spell(p, entry, after);
        }
    }

    /// The "after you cast a Tavern spell" cards as they stood when the spell
    /// was cast, fired once it has resolved.
    fn fire_after_cast_spell(&mut self, p: usize, entries: &AfterCastTriggers) {
        for entry in entries.iter() {
            self.apply_on_cast_spell(p, entry, true);
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

    /// The minions' own reaction to a spell cast on the board minion in
    /// `target_pos`: the "whenever you cast a spell on a Murloc" watchers
    /// (Shamanic Tidecaller), then the target's own watcher (Twilight
    /// Tidehunter, and the Devilish Distractor's Tavern buff).
    ///
    /// It runs before the spell's own effect, which is the game's order: a
    /// Tavern buff the target's watcher gives is on the Tavern's minions when
    /// the effect picks among them (`Hearthstone_2026_10_24_15_25_26-g4`
    /// turn 11 step 23: the Distractor's +1/+2 lands before the Methodical
    /// Madness it took eats, so the log's Distractor gains +117/+121, the
    /// eaten minions at their buffed stats). The watchers also go first so a
    /// Tavern spell they cast for you (the Gatekeeper Amalgam's Misplaced Tea
    /// Set) is cast before this cast's "improve for this turn" lands: the log
    /// sizes the Tea Set with the bonus as it stood when the cast began, and
    /// only the play's next cast gets the +1/+1.
    /// `Hearthstone_2026_11_03_15_22_36-g4` turn 10 step 27 (two Gem casts
    /// from Balinda): the total is 8/9 at the play, the first Tea Set is
    /// 12/13 (4 + 8/9), the second 13/14 (4 + 9/10). The trinkets that watch
    /// a spell cast on a friendly minion (Honeycomb Ring) come after the
    /// effect instead: `trigger_trinket_on_cast_spell_on_minion`.
    pub(super) fn trigger_on_cast_spell_on_watchers(&mut self, p: usize, target_pos: usize) {
        if target_pos >= self.players[p].board.len() {
            return;
        }
        let target_tribes = self.data.cards.tribes_of(&self.players[p].board[target_pos]);
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

    /// A trinket that watches spells cast on a friendly minion (Honeycomb
    /// Ring), after the spell's own effect.
    pub(super) fn trigger_trinket_on_cast_spell_on_minion(&mut self, p: usize) {
        self.trinket_event(p, Event::CastSpellOnMinion, None, 1);
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
        if tavern {
            self.players[p].last_spell = Some(card);
        }
        let pt = PendingTarget {
            by_minion: false,
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

    /// A Blood Gem the player's own Activate casts on the friendly minion in
    /// board slot `target`: Victorious Geomant's "Cast any that don't fit on
    /// your left-most minion". It is a real cast, so the minion's own
    /// "whenever you cast a spell on this" fires (a Gatekeeper Amalgam's
    /// Misplaced Tea Set, a Glambot's Satellite) and the "after you cast a
    /// spell" counters see it (`Hearthstone_2026_10_24_17_59_26-g1` turn 12
    /// step 40: the log's Amalgam gains the four Gems and their four Tea
    /// Sets, and the Felboars eat). It is not a Gem played from the hand, so
    /// a Hot-Air Surveyor adds no cast and Surveyor Portrait's from-hand
    /// bonus does not reach it. `source` is the minion whose Activate cast
    /// it.
    ///
    /// It does target a friendly minion, so a Balinda Stonehearth casts it
    /// twice (golden: three times). Every Geomant Activate in the logs with a
    /// Balinda on the board has the leftover Gem land twice: the plain
    /// Geomant of `Hearthstone_2026_11_03_15_22_36-g4` turn 12 step 8 puts
    /// +2/+4 on a golden Bramble Tunneler with a Gem worth +1/+2, the
    /// `Hearthstone_2026_10_28_22_11_50-g1` turn 12 step 22 golden Sanguine
    /// Refiner takes +40/+72 with a Gem worth +20/+36, and the
    /// `Hearthstone_2026_10_24_17_59_26-g1` turn 13 step 28 Gatekeeper
    /// Amalgam takes +114/+186. Each is a cast of its own: the Felboar of the
    /// `22_11_50-g1` step counts them apart, so each runs the whole cast, its
    /// "after you cast a spell" triggers included.
    pub(super) fn cast_gem_on(&mut self, p: usize, target: usize, source: Option<u8>) {
        let Some(gem) = self.data.cards.blood_gem else { return };
        if target >= self.players[p].board.len() {
            return;
        }
        let Some(st) = self.data.cards.get(gem).spell_target else { return };
        // Balinda alone: a Gem is no Tavern spell, so Replica Cathedral's
        // first Tavern spell of the turn casts nothing extra on it
        // (`balinda_mult` would add that).
        let casts = self.times_count(p, |a| (a == Aura::Balinda).then_some(TimesRole::Setter));
        for _ in 0..casts {
            if target >= self.players[p].board.len() {
                return;
            }
            let pt = PendingTarget {
                source,
                kind: TargetKind::Spell(st),
                runs: 1,
                tavern: false,
                gem: true,
                by_minion: false,
            };
            // "Whenever you cast a spell" fire before the spell's own effect,
            // and "after you cast a spell" once it has resolved.
            self.fire_event(p, EventKind::AllSpellsCast, 1);
            self.trinket_event(p, Event::SpellCast, Some(gem), 1);
            self.apply_target(p, &pt, target);
            self.run_event_triggers(p, EventKind::AllSpellsCast, 1, None, TriggerWhen::Resolved);
            self.maybe_start_discover(p);
        }
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

    /// "Set versus add, as one mechanic" (docs/rules/design.md): every "how many times"
    /// count is the highest value a setter on the board sets (1 if none),
    /// plus what each adder adds. See `cards::how_many_times`.
    pub(super) fn times_count(
        &self,
        p: usize,
        role: impl Fn(Aura) -> Option<TimesRole> + Copy,
    ) -> u8 {
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

    /// The stats of `times` Blood Gems on the Tavern minion in `slot`: the
    /// player's Gem value each, tracked as Blood Gem stats. No from-hand
    /// bonus: a Tavern minion is not a friendly minion.
    pub(super) fn add_gem_stats_shop(&mut self, p: usize, slot: usize, times: u8) {
        let (ga, gh) = (self.players[p].gem_attack, self.players[p].gem_health);
        if let Some(m) = self.players[p].shop.get_mut(slot) {
            m.attack += ga * times as i32;
            m.health += gh * times as i32;
            m.gem_attack += ga * times as i32;
            m.gem_health += gh * times as i32;
        }
    }

    /// How many Blood Gems a minion plays when one lands on it (Geomagus
    /// Roogug, either part of a multi-part card). Golden doubles it.
    fn gem_reaction(&self, m: &Minion) -> Option<u8> {
        m.parts().find_map(|(c, g)| {
            self.data
                .cards
                .get(c)
                .on_gem_played_on_self
                .map(|n| n * if g { 2 } else { 1 })
        })
    }

    /// A Blood Gem played on the friendly board minion in `pos` by a minion's
    /// or a spell's effect. `from_roogug` is the mark a Gem carries when a
    /// Geomagus Roogug played it: a Roogug there reacts to every Gem but
    /// another Roogug's, so two Roogugs never loop (docs/rules/design.md
    /// "Geomagus Roogug"). A Gem the player plays from the hand gets Surveyor
    /// Portrait's bonus where it lands instead (`Player::add_gem_stats` then
    /// `Game::roogug_reacts`). The shop only: a fight plays its Gems through
    /// the fight's own `Side::gem_at`, which carries the same mark.
    fn gem_on_board(&mut self, p: usize, pos: usize, from_roogug: bool) {
        self.players[p].add_gem_stats(pos, 1, false);
        if !from_roogug {
            self.roogug_on_gem(p, pos);
        }
    }

    /// Play `times` Blood Gems on a friendly minion: the Gems a minion's or a
    /// spell's effect plays (Turbo Hogrider, Gem Confiscation, a Rally), each
    /// of them a Gem played on it for a Geomagus Roogug.
    pub(super) fn play_gems_on_board(&mut self, p: usize, pos: usize, times: u8) {
        for _ in 0..times {
            self.gem_on_board(p, pos, false);
        }
    }

    /// Every Gem played on the friendly minion in `pos` makes a Geomagus
    /// Roogug there play its own Gems, once per Gem. For a cast whose other
    /// "played on this" triggers go first.
    pub(super) fn roogug_reacts(&mut self, p: usize, pos: usize, gems: u8) {
        for _ in 0..gems {
            self.roogug_on_gem(p, pos);
        }
    }

    /// A Blood Gem was just played on the friendly minion in `target`: every
    /// Geomagus Roogug there plays its Blood Gems on a random different
    /// friendly minion. Those Gems carry the from-Roogug mark, so they never
    /// make another Roogug react.
    fn roogug_on_gem(&mut self, p: usize, target: usize) {
        let Some(times) = self.players[p].board.get(target).and_then(|m| self.gem_reaction(m))
        else {
            return;
        };
        let n = self.players[p].board.len();
        if n <= 1 {
            return;
        }
        for _ in 0..times {
            // A different friendly minion, uniformly at random.
            let k = self.rng.below((n - 1) as u32) as usize;
            let at = if k < target { k } else { k + 1 };
            self.gem_on_board(p, at, true);
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

    /// A spell the player casts: the play from the hand, plus every extra cast
    /// their own cards add (a Bounty's again, Balinda's, a Surveyor's Gem).
    pub(super) fn cast_spell(&mut self, p: usize, m: Minion) {
        let _ = self.cast_spell_as(p, m, false, 1);
    }

    /// Cast a named spell `times` times. A targeting spell asks for its
    /// target once and every cast left lands on it, and a Choose One is
    /// picked once and its casts after the first use that same option, so the
    /// loop stops at the prompt or the queue one cast took them all into.
    /// `by_minion` says whose cast it is: a spell a minion's own effect casts
    /// for the player (Gatekeeper Amalgam's Misplaced Tea Set, a Magicfin
    /// token's taught spell, an Air Revenant's Easterly Winds) happens and
    /// buffs as printed, but it is not a spell the player cast (Era
    /// 2026-11-02, docs/notes/UNSURE.md), so nothing that counts or reacts to
    /// "spells you cast" sees it, and it gets no Choose One choice: the option
    /// is picked at random (Era 2026-11-04).
    pub(super) fn cast_named_spell(
        &mut self,
        p: usize,
        card: CardId,
        times: u32,
        by_minion: bool,
    ) {
        for cast in 0..times {
            // The casts still to come all land on the target this one picks
            // (docs/rules/design.md "Set versus add"), so `cast_spell_as`
            // folds them into the prompt's own count.
            let casts_left = (times - cast).min(u8::MAX as u32) as u8;
            let m = self.data.cards.make(card, false);
            let took_all = self.cast_spell_as(p, m, by_minion, casts_left);
            if took_all
                || self.players[p].pending_target.is_some()
                || self.players[p].pending_choose_one.is_some()
            {
                break;
            }
        }
    }

    /// `repeats` is how many times this one cast happens: the Battlecry or
    /// effect that casts a named spell `times` times folds its repeats in
    /// here, so a targeting spell asks for one target and uses it every time
    /// and a Choose One asks for one option. Returns whether this cast took
    /// all `repeats` into a prompt or the repeat queue, so the caller must
    /// not cast again.
    fn cast_spell_as(&mut self, p: usize, m: Minion, by_minion: bool, repeats: u8) -> bool {
        // A Bounty cast again (Proud Privateer) is another spell cast for
        // every trigger and every "spells cast" counter.
        let c = self.data.cards.get(m.card);
        let casts = if c.is_bounty && c.spell_effect.is_some() { self.bounty_mult(p) } else { 1 };
        let tavern = c.tavern_spell;
        // A Triple Reward is "a 0-cost plain spell (not a Tavern spell)"
        // (docs/rules/design.md "Triples"), and its cast counts as a spell
        // cast for every "after you cast a spell" counter and trigger, though
        // not for the Tavern-spell ones ("cast a spell" counts every spell).
        // The game takes that cast only once the reward's Discover is
        // answered: the play shows no counter moving and the pick shows the
        // Felboar's counter and its consume both (`Hearthstone_2026_11_02_00_10_22`
        // turn 8, Felboar [1, 3, 0, 0] at the play and [3, 3, 0, 0] with a
        // Hired Mount eaten at the pick, and a Timecap'n Hooktail's
        // Tavern-spell trigger never firing; same shape in
        // `Hearthstone_2026_11_03_15_22_36-g3` turn 8, Hooktail on the board
        // again). So both halves of the cast are owed to the pick. Whether
        // Replica Cathedral's "first spell each turn" double covers a Triple
        // Reward is not settled and no log shows it, so this cast leaves it
        // unspent, as before.
        let reward = matches!(c.spell_effect, Some(SpellEffect::TripleReward));
        let took_all = self.cast_spell_inner(p, m, by_minion, repeats);
        if reward {
            if !by_minion {
                self.players[p].cast_after_choice = Some(CastAfterChoice {
                    card: m.card,
                    casts: 1,
                    tavern: false,
                    at_play: false,
                    count_owed: false,
                    after: AfterCastTriggers::default(),
                });
            }
            return took_all;
        }
        if by_minion {
            // A minion's cast for the player is not a spell they cast: none
            // of the triggers, trinket events or counters below are its. It
            // can still open a prompt, so the fizzle check is repeated here.
            if self.players[p].pending_target.is_some() && self.choice_stuck(p) {
                self.players[p].pending_target = None;
            }
            return took_all;
        }
        // Replica Cathedral: its doubling is spent by the first spell of the
        // turn, Tavern spell or not: the cast itself read it already.
        if self.players[p].tp.first_spell_double > 0 {
            self.players[p].tp.first_spell_double -= 1;
        }
        // A spell is cast, then a Choose One picks its option and a targeting
        // spell picks its target: the card's "whenever you cast a spell"
        // triggers fire as it is cast, its "after you cast a spell" ones once
        // it has resolved (docs/rules/design.md "Targets in the shop", Era
        // 2026-11-02, and "Eating a minion with a spell": Felboar's consume
        // takes the minion the same cast buffed). The counters of the cast
        // itself are taken now, as the log shows them moving on the play.
        let waits = matches!(self.players[p].pending_choose_one, Some(pco) if pco.source.is_none());
        // A repeated Choose One is cast once per repeat too: the player picks
        // once, every cast after it is still a cast (docs/rules/design.md "Set
        // versus add").
        let choosing = self.players[p]
            .pending_choose_one
            .filter(|pco| pco.source.is_none())
            .map(|pco| pco.repeats.max(1));
        // A targeting spell is cast once per target its prompt asks for
        // (Balinda, and a Gem the Hot-Air Surveyors copied).
        let targeting = self.players[p].pending_target.map(|pt| pt.runs);
        let casts = targeting.or(choosing).unwrap_or(casts);
        // Every spell is a spell cast: a Tavern spell, a Blood Gem, a Gem
        // Day, a Banana, a Spellcraft spell (docs/rules/design.md). "Tavern spell" is
        // the narrower one, and only those touch the Tavern spell counters.
        if (waits || targeting.is_some()) && tavern {
            let n = casts as u32;
            let pl = &mut self.players[p];
            pl.spells_cast = pl.spells_cast.saturating_add(n);
            pl.spells_cast_turn = pl.spells_cast_turn.saturating_add(1);
        }
        let all_casts = if tavern { casts } else { 1 };
        // The board's "after you cast a Tavern spell" cards as they stand now,
        // so they fire once this cast has resolved even if it takes them off
        // the board itself.
        let after = if tavern {
            self.gather_on_cast_spell(p, true)
        } else {
            AfterCastTriggers::default()
        };
        if waits {
            // A Choose One spell owes every trigger to the choice.
            self.players[p].cast_after_choice = Some(CastAfterChoice {
                card: m.card,
                casts,
                tavern,
                at_play: false,
                count_owed: true,
                after,
            });
        } else if targeting.is_some() {
            // A targeting spell fires its "whenever" triggers and its trinket
            // events now, and owes the "after" ones to the target.
            self.players[p].cast_after_choice = Some(CastAfterChoice {
                card: m.card,
                casts,
                tavern,
                at_play: true,
                count_owed: false,
                after,
            });
            // One cast's worth now; the loop in `apply_target` fires them
            // again before each extra cast (Balinda casts twice, so a
            // "whenever you cast a Tavern spell" trigger fires twice too).
            self.run_event_triggers(p, EventKind::AllSpellsCast, 1, None, TriggerWhen::Now);
            self.trinket_event(p, Event::SpellCast, Some(m.card), 1);
            if tavern {
                self.run_event_triggers(p, EventKind::SpellsCast, 1, None, TriggerWhen::Now);
                self.trinket_event(p, Event::CastSpell, Some(m.card), 1);
                self.dark_gift_count(p, casts as i32, |c| c.buff_per_tavern_spell);
            }
        } else if self.players[p].cast_after_choice.is_none()
            && Game::other_choice_open(&self.players[p])
        {
            // A spell whose own effect opened a prompt (Fandral's Fortune's
            // Discover, Hired Headhunter's, Unmasked Identity's hero power)
            // has not resolved yet: the pick comes first, and what waits for
            // the spell to resolve follows it inside the same action. Its
            // "whenever you cast a spell" triggers and the counters of the
            // cast are taken now, as the game takes them on the play, but the
            // "after you cast a spell" triggers and the Dark Gift count wait.
            // The raw log has both: the Spell Siphon gift ("has +2/+2 for each
            // Tavern spell you've cast this game") only grows after the pick
            // (Planar Telescope, 2026_10_27_04_30_04 turn 7: the play at
            // Power.log 68792, the pick at 69231, the growth at 69283), and
            // Fandral's Fortune's Felboar only consumes at its pick
            // (2026_10_24_17_59_26 turn 10 step 43 to 44). A "whenever"
            // trigger still fires on the play (Living Azerite, same spell as
            // a Choose One, 2026_11_02_00_10_22-g8 turn 12 step 21).
            // `maybe_start_discover` fires what is owed once nothing is left
            // to answer. An enclosing cast that already owes its own triggers
            // keeps them: they fire now, as before.
            for _ in 0..all_casts {
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
            self.players[p].cast_after_choice = Some(CastAfterChoice {
                card: m.card,
                casts: all_casts,
                tavern,
                at_play: true,
                count_owed: tavern,
                after,
            });
        } else {
            for _ in 0..all_casts {
                self.fire_event(p, EventKind::AllSpellsCast, 1);
            }
            self.trinket_event(p, Event::SpellCast, Some(m.card), 1);
            if tavern {
                for _ in 0..casts {
                    self.fire_event(p, EventKind::SpellsCast, 1);
                }
                self.players[p].spells_cast_turn = self.players[p].spells_cast_turn.saturating_add(1);
                self.trinket_event(p, Event::CastSpell, Some(m.card), 1);
                // Dark Gifts Spell Siphon counts Tavern spells, never a Blood
                // Gem or a Banana (docs/rules/design.md "Spells versus Tavern spells").
                self.dark_gift_count(p, casts as i32, |c| c.buff_per_tavern_spell);
            }
            // A spell with nothing to wait for has landed: its "after you
            // cast a spell" triggers fire now.
            for _ in 0..all_casts {
                self.run_event_triggers(p, EventKind::AllSpellsCast, 1, None, TriggerWhen::Resolved);
            }
            if tavern {
                for _ in 0..casts {
                    self.run_event_triggers(p, EventKind::SpellsCast, 1, None, TriggerWhen::Resolved);
                    self.fire_after_cast_spell(p, &after);
                }
            }
            // The cast's "after you cast" triggers have run: a Tavern buff
            // the spell's own effect gave (Staff of Enrichment) lands now,
            // never before them (`Player::add_tavern_buff_late`).
            self.players[p].flush_tavern_buffs(&self.data.cards);
        }
        // A cast never leaves a prompt nothing can answer: the spell's own
        // "after you cast" triggers can take away its last target (the only
        // way one could go missing, since the cast itself checked), and then
        // the spell simply fizzles.
        if self.players[p].pending_target.is_some() && self.choice_stuck(p) {
            self.players[p].pending_target = None;
        }
        took_all
    }

    /// Fire the triggers a Choose One or targeting spell's cast owed. A
    /// targeting spell fired its "whenever" triggers and trinket events at the
    /// play (`at_play`), so only its "after you cast a spell" triggers are
    /// left; a Choose One spell owes those too, and so does a plain spell
    /// whose own effect opened a Discover. The Dark Gift count waits the same
    /// way when the play did not take it (`count_owed`, Spell Siphon). Called
    /// once the option, the target and the Discover are answered
    /// (`maybe_start_discover`).
    pub(super) fn fire_cast_after_choice(&mut self, p: usize, what: CastAfterChoice) {
        if !what.at_play {
            for _ in 0..what.casts {
                self.run_event_triggers(p, EventKind::AllSpellsCast, 1, None, TriggerWhen::Now);
            }
            self.trinket_event(p, Event::SpellCast, Some(what.card), 1);
            if what.tavern {
                for _ in 0..what.casts {
                    self.run_event_triggers(p, EventKind::SpellsCast, 1, None, TriggerWhen::Now);
                }
                self.trinket_event(p, Event::CastSpell, Some(what.card), 1);
            }
        }
        if what.count_owed && what.tavern {
            // Dark Gifts Spell Siphon counts Tavern spells, never a Blood Gem
            // or a Banana (docs/rules/design.md "Spells versus Tavern spells").
            self.dark_gift_count(p, what.casts as i32, |c| c.buff_per_tavern_spell);
        }
        for _ in 0..what.casts {
            self.run_event_triggers(p, EventKind::AllSpellsCast, 1, None, TriggerWhen::Resolved);
        }
        if what.tavern {
            for _ in 0..what.casts {
                self.run_event_triggers(p, EventKind::SpellsCast, 1, None, TriggerWhen::Resolved);
                // "After you cast a Tavern spell" cards (Forsaken Weaver) wait
                // for the spell to resolve like every other "after" trigger.
                self.fire_after_cast_spell(p, &what.after);
            }
        }
        // The cast's "after you cast" triggers have run: a Tavern buff the
        // spell's own effect gave lands now, not before them.
        self.players[p].flush_tavern_buffs(&self.data.cards);
    }

    /// Whether a spell has anything to land on right now. A spell cast
    /// outside `Action::Play` (a taught spell, Puzzle Box's random one) can
    /// have no legal target, so it fizzles instead of waiting on a choice
    /// that could never be made.
    pub(super) fn spell_castable(&self, p: usize, card: CardId) -> bool {
        let c = self.data.cards.get(card);
        // A Choose One spell with no option the player could pick would open
        // a choice nothing can answer.
        if let Some(options) = c.choose_one
            && !options.iter().any(|&opt| self.can_choose_option(p, opt))
        {
            return false;
        }
        let Some(st) = c.spell_target else { return true };
        (0..self.players[p].board.len()).any(|s| self.valid_spell_target(p, st, s))
            || (st.can_target_shop()
                && (0..self.players[p].shop.len())
                    .any(|s| self.spell_target_ok(p, st, &self.players[p].shop[s])))
    }

    /// Puzzle Box: cast a random Tavern spell, at a random target when it
    /// needs one. A spell with nothing to land on is not picked, and the
    /// destructive ones are never cast (docs/rules/heroes.md).
    pub(super) fn cast_random_spell(&mut self, p: usize) {
        let tier = self.players[p].tier;
        let mut picked = None;
        for _ in 0..8 {
            let Some(c) = self.data.cards.random_castable_spell(tier, self.lobby, &mut self.rng) else {
                return;
            };
            if self.spell_castable(p, c) {
                picked = Some(c);
                break;
            }
        }
        let Some(card) = picked else { return };
        let m = self.players[p].make_card(&self.data.cards, card, false, 0);
        self.cast_spell(p, m);
        if self.players[p].pending_target.is_some() {
            self.answer_target_at_random(p);
        }
        // A Choose One spell picks its option at random too (Era: the spell
        // picks a random target).
        if let Some(pco) = self.players[p].pending_choose_one {
            let ok: ArrayVec<usize, 2> = (0..2)
                .filter(|&k| self.can_choose_option(p, pco.options[k]))
                .collect();
            if !ok.is_empty() {
                let k = ok[self.rng.below(ok.len() as u32) as usize];
                self.resolve_choose(p, k);
            }
        }
    }

    /// Run the spell itself: open its target or Choose One prompt, or run its
    /// effect. `by_minion` says whether a minion's effect cast it, and then no
    /// "whenever you cast a spell" card sees it and it gets no Choose One
    /// choice (its option is picked at random).
    /// `repeats` is how many times this cast happens: see `cast_spell_as`.
    /// Returns whether this one cast took all `repeats` into a prompt or the
    /// repeat queue (`true`) or has to be cast again for each repeat.
    fn cast_spell_inner(&mut self, p: usize, m: Minion, by_minion: bool, repeats: u8) -> bool {
        let c = self.data.cards.get(m.card);
        let tavern = c.tavern_spell;
        // "The last Tavern spell you cast" (Cataclysmic Harbinger, the
        // Galaxy's Lens): a spell a minion casts for the player is not the
        // player's cast (docs/rules/design.md "A spell a minion casts for
        // you"), and neither is a Blood Gem or another non-Tavern spell, so
        // neither takes the place of the one those cards copy.
        if !by_minion && tavern {
            self.players[p].last_spell = Some(m.card);
        }
        if let Some(st) = c.spell_target {
            if !self.spell_castable(p, m.card) {
                return false;
            }
            // A Blood Gem plays once per cast: Balinda sets the count and
            // each Hot-Air Surveyor adds one ("Set versus add"). A spell a
            // minion casts for the player is not the player's, so neither
            // reaches it: the log's Magicfin Apprentice casting Fortify with
            // Balinda on the board gives +3 Health, not +6
            // (`Hearthstone_2026_11_03_00_29_06-g1` turn 11 step 17).
            let (runs, gem) = if by_minion {
                (1, c.blood_gem)
            } else if c.blood_gem {
                (self.gem_casts(p), true)
            } else {
                (self.balinda_mult(p), false)
            };
            // A Battlecry's repeats (Brann, and whatever else sets or adds
            // them) cast this spell again on the one target it picks.
            let runs = runs.max(1).saturating_mul(repeats.max(1));
            self.players[p].pending_target = Some(PendingTarget {
                source: None,
                kind: TargetKind::Spell(st),
                runs,
                tavern,
                gem,
                by_minion,
            });
            return true;
        }
        if let Some(options) = c.choose_one {
            if !options.iter().any(|&opt| self.can_choose_option(p, opt)) {
                return false;
            }
            // A repeated Choose One works normally: the player picks the
            // option and its target once, and every cast after the first uses
            // the SAME option on the SAME target (docs/rules/design.md "Set
            // versus add", Era 2026-11-04). That is a Battlecry's repeats.
            // Balinda is not one of them: `apply_choose_option` runs the
            // option she doubles again, the same option on the same target
            // (log Hearthstone_2026_11_02_00_10_22-g4 turn 12 step 0: two
            // casts of Alliance Flag's second option on one minion, one
            // "spells cast").
            let runs = repeats.max(1);
            // A Trailblazer's coverage runs both effects instead of asking,
            // and Trailblazer Sticker covers every Choose One card.
            let both = self.players[p].tp.choose_one_all || self.take_trailblazer_cover(p);
            if by_minion {
                // A spell a minion casts gets no Choose One choice (Era
                // 2026-11-04): its option is picked at random, and with
                // Brann's two casts each picks its own. A target it needs is
                // still the player's pick, so the casts after this one wait
                // in the queue and land on the one picked here.
                let picked = if both {
                    None
                } else {
                    // No pickable option cannot happen (the guard above), and
                    // then the cast is simply spent.
                    let Some(k) = self.random_choose_option(p, options) else {
                        return true;
                    };
                    Some(k)
                };
                if runs > 1 {
                    let _ = self.players[p].repeats.try_push(QueuedRepeat {
                        what: RepeatWhat::ChooseOne { source: None, options, both, tavern, by_minion },
                        runs: runs - 1,
                        started: true,
                        at: None,
                        option: None,
                        random_discover: false,
                    });
                }
                match picked {
                    Some(k) => self.apply_choose_option(p, None, options[k], tavern, true),
                    None => self.resolve_choose_options(p, None, options, tavern, true, true),
                }
                return true;
            }
            if runs > 1 {
                let _ = self.players[p].repeats.try_push(QueuedRepeat {
                    what: RepeatWhat::ChooseOne { source: None, options, both, tavern, by_minion },
                    runs: runs - 1,
                    started: true,
                    at: None,
                    option: None,
                    random_discover: false,
                });
            }
            if both {
                self.resolve_choose_options(p, None, options, tavern, by_minion, true);
            } else {
                self.players[p].pending_choose_one = Some(PendingChooseOne {
                    source: None,
                    golden: false,
                    options,
                    tavern,
                    repeats: runs,
                });
            }
            return true;
        }
        if let Some(se) = c.spell_effect {
            if matches!(se, SpellEffect::TripleReward) {
                // The tier was fixed when the reward was given.
                let tier = (m.improvements as u8).clamp(1, MAX_TIER);
                let _ = self.players[p]
                    .discover_queue
                    .try_push(DiscoverRequest::MinionTier(tier));
                self.maybe_start_discover(p);
                return false;
            }
            if matches!(se, SpellEffect::DiscoverHeroPower) {
                // The exclusion list lives on the card, so the intercept runs
                // here where the card is known. Unmasked Identity replaces
                // Power of the Storm for good on the main slot.
                let excludes = self.data.heroes.hero_indices(&c.exclude_heroes);
                if tavern && !by_minion {
                    self.trigger_on_cast_spell(p, TriggerWhen::Now);
                }
                self.players[p].storm_power[0] = false;
                self.discover_hero_power(p, &excludes, false, 1);
                return false;
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
                if tavern && !by_minion {
                    self.trigger_on_cast_spell(p, TriggerWhen::Now);
                }
                self.execute_spell_effect(p, se, tavern);
            }
            self.maybe_start_discover(p);
        }
        false
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
                    // Top Shelf: one Tier higher, up to Tier 7.
                    DiscoverKind::HigherTierMinion => {
                        DiscoverRequest::MinionTier((tier + 1).min(crate::CARD_MAX_TIER))
                    }
                    DiscoverKind::LastOpponentMinion => DiscoverRequest::LastOpponentWarband,
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
            // ---- the Darkmoon Prizes (block 5) ----
            SpellEffect::DiscoverPrize(tier) => {
                let _ = self.players[p].discover_queue.try_push(DiscoverRequest::Prize(tier));
            }
            SpellEffect::DiscoverPrizesBelow(tier) => {
                // One Discover per Prize tier below this one (Big Winner!).
                for t in 1..=tier {
                    let _ = self.players[p].discover_queue.try_push(DiscoverRequest::Prize(t));
                }
            }
            SpellEffect::RandomPrize(n) => {
                for _ in 0..n {
                    let Some(card) = self.data.cards.random_prize_any(&mut self.rng) else { break };
                    self.give_generated(p, card);
                }
            }
            SpellEffect::FillHand { card } => {
                while !self.players[p].hand.is_full() {
                    self.give_generated(p, card);
                }
            }
            SpellEffect::FillHandSpells => {
                let tier = self.players[p].tier;
                while !self.players[p].hand.is_full() {
                    let Some(card) = self.data.cards.random_spell(tier, self.lobby, &mut self.rng)
                    else {
                        break;
                    };
                    self.give_generated(p, card);
                }
            }
            SpellEffect::ReturnEachTurn { card } => {
                self.players[p].prize.returns_each_turn = Some(card);
                self.players[p].gold = self.players[p].gold.saturating_add(1);
            }
            SpellEffect::EvolvingTavern => self.evolve_tavern(p),
            SpellEffect::StealAllTavern => {
                self.steal_all_tavern(p);
                self.refresh_tavern_now(p);
            }
            SpellEffect::ExtraShopMinion { attack, health } => {
                // New Recruit: one more Tavern slot for good, and the +2/+2
                // into the same "after the Tavern is Refreshed this game"
                // amount Waveling and Easterly Winds give (Era: one shared
                // buff, so they stack).
                let pl = &mut self.players[p];
                pl.prize.extra_shop = true;
                pl.refresh_buff.0 += attack;
                pl.refresh_buff.1 += health;
            }
            SpellEffect::FreeRollsPerTurn { now, per_turn } => {
                let pl = &mut self.players[p];
                pl.free_rolls = pl.free_rolls.saturating_add(now);
                pl.prize.free_rolls_per_turn =
                    pl.prize.free_rolls_per_turn.saturating_add(per_turn);
            }
            SpellEffect::RandomSpellAtLeast(cost) => {
                let tier = self.players[p].tier;
                if let Some(card) =
                    self.data.cards.random_spell_at_least(cost, tier, self.lobby, &mut self.rng)
                {
                    self.give_generated(p, card);
                }
            }
            SpellEffect::SpellDiscountThisTurn(n) => {
                let pl = &mut self.players[p];
                pl.prize.spell_discount_turn = pl.prize.spell_discount_turn.saturating_add(n);
            }
            SpellEffect::MinionPrice(n) => self.players[p].prize.minion_price = n,
            SpellEffect::BattlecryExtraThisTurn(n) => {
                let pl = &mut self.players[p];
                pl.prize.battlecry_extra_turn = pl.prize.battlecry_extra_turn.saturating_add(n);
            }
            SpellEffect::DiscoverHeroPower => {}
            SpellEffect::MightyDragonbreath { attack, health } => {
                let cards = &self.data.cards;
                for pass in 0..3 {
                    for o in &mut self.players[p].board {
                        let ok = match pass {
                            0 => true,
                            1 => cards.tribes_of(o).has(Tribe::Dragon),
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
                let ok = |m: &Minion| !m.golden && !cards.get(m.card).fixed_stats;
                let n = self.players[p].shop.iter().filter(|m| ok(m)).count() as u32;
                if n > 0 {
                    let k = self.rng.below(n) as usize;
                    let at = self.players[p]
                        .shop
                        .iter()
                        .enumerate()
                        .filter(|(_, m)| ok(m))
                        .nth(k)
                        .unwrap()
                        .0;
                    // A Tavern minion turns Golden where it stands, by the one
                    // `golden_up` rule.
                    let mut m = self.players[p].shop[at];
                    super::shop::golden_up(&self.players[p], cards, &mut m);
                    self.players[p].shop[at] = m;
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
                    .filter(|t| self.players[p].board.iter().any(|m| cards.tribes_of(m).has(**t)))
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
                // The Tavern's stats wait for the end of the cast: a Felboar
                // eating for this same cast takes its minion at its pre-spell
                // stats (`Player::add_tavern_buff_late`).
                let b = TavernBuff {
                    tribe: None,
                    attack: attack + da,
                    health: health + dh,
                    this_turn: false,
                    typed_only: false,
                };
                self.players[p].add_tavern_buff_late(&self.data.cards, b);
            }
            SpellEffect::MisplacedTeaSet { attack, health } => {
                // One random unbuffed minion of each type on the board; a
                // dual or all-type minion can only be picked once.
                let cards = &self.data.cards;
                let board = &mut self.players[p].board;
                let tribes: ArrayVec<Tribes, BOARD> =
                    board.iter().map(|m| cards.tribes_of(m)).collect();
                for i in pick_one_per_type(&tribes, &mut self.rng) {
                    buff(&mut board[i], attack + da, health + dh);
                }
            }
            SpellEffect::PlanarTelescope => {
                let cards = &self.data.cards;
                let mut counts = [0u8; Tribe::EVERY.len()];
                for m in &self.players[p].board {
                    let t = cards.tribes_of(m);
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
                // count Refreshes (docs/rules/design.md, "Two kinds of Refresh").
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
                    let t = cards.tribes_of(m);
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
            SpellEffect::SpellBonus { attack, health } => {
                let pl = &mut self.players[p];
                pl.spell_bonus.0 += attack + da;
                pl.spell_bonus.1 += health + dh;
            }
            SpellEffect::GemBonus { attack, health } => {
                let pl = &mut self.players[p];
                pl.gem_attack += attack;
                pl.gem_health += health;
            }
            SpellEffect::GemsOnBoard(n) => {
                // Gems played by an effect, not from hand: each one makes a
                // Geomagus Roogug react.
                for i in 0..self.players[p].board.len() {
                    self.play_gems_on_board(p, i, n);
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
            // Veteran Brigand's "cast Blood Gem Barrage 3 times": left as
            // the player's cast, as it was. Whether a Battlecry's cast is the
            // minion's is still open, for docs/notes/UNSURE.md.
            SpellEffect::CastSpell { card, times } => {
                self.cast_named_spell(p, card, times as u32, false);
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

    /// Evolving Tavern: every card in the Tavern becomes one from a Tier
    /// higher, the Tavern spell included. The old minions' pool copies go
    /// back, the new ones take a copy each and get the player's Tavern buffs;
    /// a spell never held a pool copy, so its slot just draws another one
    /// (Era: "every card in the Tavern, minions and the Tavern spell").
    fn evolve_tavern(&mut self, p: usize) {
        let Game { data, rng, players, pool, lobby, .. } = self;
        let cards = &data.cards;
        let pl = &mut players[p];
        // One Tier higher, and never past the Tavern's own top (a Tier 7
        // card only comes from a card that names Tier 7).
        let up = (pl.tier + 1).min(crate::MAX_TIER);
        let old: ArrayVec<Minion, { crate::SHOP }> = pl.shop.drain(..).collect();
        let mut made: ArrayVec<Minion, { crate::SHOP }> = ArrayVec::new();
        for m in old {
            give_back(pool, cards, &m);
            let Some(c) = draw_filtered(pool, cards, rng, up, up, &[], *lobby, |_| true) else {
                continue;
            };
            let mut n = pl.make_card(cards, c, false, 1);
            pl.apply_tavern_buffs(cards, &mut n);
            let _ = made.try_push(n);
        }
        pl.shop = made;
        // The spell in the Tavern's spell slot goes one Tier higher too, and
        // no Tavern spell above Tier 6 (a pool spell holds no pool copy, so
        // nothing goes back for the old one).
        if pl.shop_spell.is_some() {
            pl.shop_spell = cards.random_tier_spell(up, &[], *lobby, rng);
        }
    }

    /// Refresh the Tavern for an effect that says so (Evolving Tavern is the
    /// only card that leaves its own shape; Mindflayer Goggles refreshes after
    /// stealing every card). Not the player's own Refresh, so queued Fodders
    /// wait, but it is a Refresh for the cards that count them.
    pub(super) fn refresh_tavern_now(&mut self, p: usize) {
        {
            let Game { data, rng, players, pool, lobby, .. } = self;
            let _ = Self::refresh_shop(
                &mut players[p],
                pool,
                &data.cards,
                &data.rules,
                rng,
                *lobby,
                RefreshKind::Effect,
            );
        }
        self.trinket_event(p, crate::trinkets::Event::Refresh, None, 1);
    }
}
