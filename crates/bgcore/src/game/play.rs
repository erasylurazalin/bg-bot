//! Playing minions from hand: Battlecries, Magnetic, on-play triggers.

use arrayvec::ArrayVec;
use crate::cards::{
    Aura,
    ChooseOption,
    OnCastSpellOnMech,
    OnPlayMech,
    OnPlayOrMagnetizeMech,
    OnSummonBeast,
    SpellEffect,
    Tribe,
};
use crate::minion::{
    CardId,
    Keywords,
    Minion,
};
use super::{
    Game,
    PendingChooseOne,
};
use super::effects::buff;

impl Game {
    /// The player played `n` more Golden minions from hand: Maritime
    /// Extortionist grows wherever it is (Tavern, hand or board).
    pub(super) fn add_golden_played(&mut self, p: usize, n: u32) {
        let cards = &self.data.cards;
        let pl = &mut self.players[p];
        pl.golden_played = pl.golden_played.saturating_add(n);
        for m in pl.shop.iter_mut().chain(&mut pl.hand).chain(&mut pl.board) {
            for (card, golden) in m.parts().collect::<ArrayVec<_, { crate::MAX_ATTACHED + crate::MAX_GIFTS + 1 }>>() {
                if let Some((a, h)) = cards.get(card).buff_per_golden_played {
                    let k = n as i32 * if golden { 2 } else { 1 };
                    m.attack += a * k;
                    m.health += h * k;
                }
            }
        }
    }

    /// Magnetize `m` into `self.players[p].board[pos]`. `from_play` is true
    /// when `m` was played from hand, and false when attached directly by an
    /// effect (Spark Snapper, Resourceful Robot, Auto Reveille, Glambot).
    pub(super) fn magnetize_to(&mut self, p: usize, pos: usize, m: Minion, from_play: bool) {
        let cards = &self.data.cards;
        if m.pool_copies > 0 && !cards.get(m.card).token && !cards.get(m.card).is_spell() {
            let slot = cards.get(m.card).pool_slot as usize;
            self.consumed[slot] += m.pool_copies as u32;
        }
        let extra = std::mem::take(&mut self.players[p].board[pos].extra_magnetize);
        self.players[p].board[pos].magnetize(cards, m);
        {
            let pl = &mut self.players[p];
            pl.magnetizations = pl.magnetizations.saturating_add(1);
        }
        if let Some((da, dh)) = cards.get(m.card).volumizer_buff {
            let mul = if m.golden { 2 } else { 1 };
            self.add_volumizer_bonus(p, da * mul, dh * mul);
        }
        // A Magnetic minion played as a Magnetization (Spell-powered Wrench
        // casts Repair Job on a random Mech).
        if from_play {
            self.trinket_event(p, crate::trinkets::Event::Magnetize, Some(m.card), 1);
        }
        // The copy a Beatboxer gets is the Magnetic as it attached, with the
        // buff the host got from Mechgnome Interpreter and the like.
        let m1 = self.with_magnetize_buffs(p, m);
        self.trigger_mech_played_or_magnetized(p, pos, from_play, false);
        self.beatboxers_copy(p, Some(pos), m1);
        // Drone Duplicator: the same Magnetization happens again, without
        // taking another copy from the pool.
        for _ in 0..extra {
            if pos >= self.players[p].board.len() {
                break;
            }
            self.players[p].board[pos].magnetize(&self.data.cards, m);
            self.note_magnetized(p);
            self.trigger_mech_played_or_magnetized(p, pos, from_play, false);
            self.beatboxers_copy(p, Some(pos), m1);
        }
    }

    /// Count one Magnetization for good: Dr. Boom's Monster's "for each time
    /// you've Magnetized this game" reads it.
    pub(super) fn note_magnetized(&mut self, p: usize) {
        let pl = &mut self.players[p];
        pl.magnetizations = pl.magnetizations.saturating_add(1);
    }

    /// `m` plus what "whenever you play or Magnetize a Mech" cards on the
    /// board give a Mech it Magnetizes to.
    fn with_magnetize_buffs(&self, p: usize, mut m: Minion) -> Minion {
        for o in &self.players[p].board {
            for (card, golden) in o.parts() {
                if let Some(OnPlayOrMagnetizeMech::Buff { attack, health }) =
                    self.data.cards.get(card).on_play_or_magnetize_mech
                {
                    let mul = if golden { 2 } else { 1 };
                    m.attack += attack * mul;
                    m.health += health * mul;
                }
            }
        }
        m
    }

    /// Polarizing Beatboxer: `m` just Magnetized to the minion in board slot
    /// `host` (None: a minion in the Tavern), so every board minion other than
    /// the host with the Beatboxer's text gets a copy of the same Magnetic
    /// (twice when golden). Each copy is a Magnetization of its own (Utility
    /// Drone counts it; a Volumizer copy raises your Volumizers again) but not
    /// a play, takes no pool copy, is not buffed again by Mechagnome
    /// Interpreter, and sends nothing on: with two Beatboxers, A's copy to B
    /// doesn't come back to A.
    fn beatboxers_copy(&mut self, p: usize, host: Option<usize>, m: Minion) {
        let len = self.players[p].board.len();
        for j in 0..len {
            if Some(j) == host {
                continue;
            }
            let copies: u32 = self.players[p].board[j]
                .parts()
                .filter(|&(c, _)| self.data.cards.get(c).aura == Some(Aura::PolarizingBeatboxer))
                .map(|(_, golden)| if golden { 2 } else { 1 })
                .sum();
            for _ in 0..copies {
                self.players[p].board[j].magnetize(&self.data.cards, m);
                self.note_magnetized(p);
                if let Some((da, dh)) = self.data.cards.get(m.card).volumizer_buff {
                    let mul = if m.golden { 2 } else { 1 };
                    self.add_volumizer_bonus(p, da * mul, dh * mul);
                }
            }
        }
    }

    /// Glambot: a spell was cast on the Mech in Tavern slot `slot`, so a
    /// Satellite is Magnetized to it for each Glambot (twice if golden) and
    /// every Beatboxer gets a copy of each. No board trigger buffs a Tavern
    /// minion.
    pub(super) fn glambot_on_shop_mech(&mut self, p: usize, slot: usize) {
        let mut sats: ArrayVec<(CardId, i32, i32), 16> = ArrayVec::new();
        for m in &self.players[p].board {
            for (card, golden) in m.parts() {
                if let Some(OnCastSpellOnMech::MagnetizeSatellite { card: sat, attack, health }) =
                    self.data.cards.get(card).on_cast_spell_on_mech
                {
                    for _ in 0..(if golden { 2 } else { 1 }) {
                        let _ = sats.try_push((sat, attack, health));
                    }
                }
            }
        }
        for (sat_id, attack, health) in sats {
            let mut sat = self.data.cards.make(sat_id, false);
            sat.attack = attack;
            sat.health = health;
            self.players[p].shop[slot].magnetize(&self.data.cards, sat);
            self.note_magnetized(p);
            self.beatboxers_copy(p, None, sat);
        }
    }

    pub(super) fn trigger_mech_played_or_magnetized(
        &mut self,
        p: usize,
        target_pos: usize,
        from_play: bool,
        exclude_target: bool,
    ) {
        let len = self.players[p].board.len();
        for i in 0..len {
            if exclude_target && i == target_pos {
                continue;
            }
            let m = self.players[p].board[i];
            for (card, golden) in m.parts() {
                let c = self.data.cards.get(card);
                if let Some(OnPlayOrMagnetizeMech::Buff { attack, health }) =
                    c.on_play_or_magnetize_mech
                {
                    let mul = if golden { 2 } else { 1 };
                    buff(&mut self.players[p].board[target_pos], attack * mul, health * mul);
                }
                if from_play
                    && let Some(OnPlayMech::MagnetizeSatellite { card: sat_id, attack: sa, health: sh }) =
                        c.on_play_mech
                {
                    let imp = self.players[p].board[i].improvements;
                    self.players[p].board[i].improvements = imp.saturating_add(1);
                    let stacks = (1 + imp as i32) * (if golden { 2 } else { 1 });
                    let mut sat = self.data.cards.make(sat_id, false);
                    sat.attack = sa * stacks;
                    sat.health = sh * stacks;
                    self.magnetize_to(p, target_pos, sat, false);
                }
            }
        }
    }

    pub(super) fn trigger_summon_beast_in_shop(&mut self, p: usize, pos: usize) {
        let len = self.players[p].board.len();
        for i in 0..len {
            if i == pos {
                continue;
            }
            let imp = self.players[p].board[i].improvements;
            let m = self.players[p].board[i];
            let mut da = 0;
            let mut dh = 0;
            let mut imp_gain = 0u16;
            for (card, golden) in m.parts() {
                if let Some(OnSummonBeast::Buff { attack, health, improve_permanently }) =
                    self.data.cards.get(card).on_summon_beast
                {
                    let mul = if golden { 2 } else { 1 };
                    let stacks = if improve_permanently { 1 + (imp + imp_gain) as i32 } else { 1 };
                    da += attack * mul * stacks;
                    dh += health * mul * stacks;
                    if improve_permanently {
                        imp_gain += 1;
                    }
                }
            }
            if imp_gain > 0 {
                self.players[p].board[i].improvements = imp.saturating_add(imp_gain);
            }
            buff(&mut self.players[p].board[pos], da, dh);
        }
    }

    /// A minion of `tribe` was just played: hand cards that grow while in
    /// hand (Bream Counter) gain their buff.
    pub(super) fn trigger_in_hand_on_play(&mut self, p: usize, tribe: Tribe) {
        let cards = &self.data.cards;
        let mul_of = |m: &Minion| if m.golden { 2 } else { 1 };
        for m in &mut self.players[p].hand {
            let Some(w) = cards.get(m.card).while_in_hand else { continue };
            if w.tribe == tribe {
                let mul = mul_of(m);
                m.attack += w.attack * mul;
                m.health += w.health * mul;
            }
        }
    }

    /// A minion of `tribes` was just played into board slot `played`: every
    /// other minion with an "after you play a minion of this tribe" trigger
    /// runs its effects (Wrath Weaver, Insatiable Ur'zul, Ultraviolet
    /// Ascendant). The one just played never counts for itself. This runs
    /// right after it lands, before its Battlecry.
    pub(super) fn fire_after_play_tribe(
        &mut self,
        p: usize,
        tribes: crate::cards::Tribes,
        played: usize,
    ) {
        let mut pos = 0;
        while pos < self.players[p].board.len() {
            let m = self.players[p].board[pos];
            for (card, golden) in m.parts() {
                let n = match self.data.cards.get(card).after_play_tribe {
                    Some(ref t)
                        if tribes.has(t.tribe) && pos != played =>
                    {
                        t.effects.len()
                    }
                    _ => 0,
                };
                for k in 0..n {
                    if pos >= self.players[p].board.len() {
                        break;
                    }
                    let Some(e) = self
                        .data
                        .cards
                        .get(card)
                        .after_play_tribe
                        .as_ref()
                        .map(|t| t.effects[k])
                    else {
                        break;
                    };
                    self.run_effect(p, &mut pos, e, golden, 1);
                }
            }
            pos += 1;
        }
    }

    /// Ichoron the Protector: a minion of its tribe was just played into
    /// board slot `pos`, so it gets Divine Shield. The shield goes at the
    /// start of the next shop phase, unless a Golden Ichoron gave it (then it
    /// stays) or the minion already had one.
    pub(super) fn give_play_shield(&mut self, p: usize, pos: usize) {
        let cards = &self.data.cards;
        let tribes = cards.tribes_of(&self.players[p].board[pos]);
        let mut any = false;
        let mut for_good = false;
        for m in &self.players[p].board {
            for (card, golden) in m.parts() {
                if let Some(t) = cards.get(card).play_shield
                    && tribes.has(t)
                {
                    any = true;
                    for_good |= golden;
                }
            }
        }
        if !any {
            return;
        }
        let m = &mut self.players[p].board[pos];
        if !m.keywords.has(Keywords::DIVINE_SHIELD) {
            m.keywords.set(Keywords::DIVINE_SHIELD);
            m.temp_shield = !for_good;
        }
    }

    pub(super) fn battlecry_or_choose_one(&mut self, p: usize, mut pos: usize) {
        // "Battlecries you have triggered this game" (Murky Sticker), and
        // the same count for the Dark Gift Battle Scars, wherever the
        // minion with it is.
        self.players[p].tp.battlecries = self.players[p].tp.battlecries.saturating_add(1);
        self.players[p].battlecries_triggered =
            self.players[p].battlecries_triggered.saturating_add(1);
        self.dark_gift_count(p, 1, |c| c.buff_per_battlecry);
        // A hook that ran just before this can have taken the slot away (a
        // triple that pulled the minion off the board): nothing to trigger.
        let Some(&m) = self.players[p].board.get(pos) else { return };
        let mul = if m.golden { 2 } else { 1 };
        let card = self.data.cards.get(m.card);
        if let Some(options) = card.choose_one {
            let scaled = options.map(|o| match o {
                ChooseOption::MinionTarget { tribe, attack, health, keyword, in_shop } => {
                    ChooseOption::MinionTarget {
                        tribe,
                        attack: attack * mul,
                        health: health * mul,
                        keyword,
                        in_shop,
                    }
                }
                ChooseOption::SpellBonus { attack, health } => ChooseOption::SpellBonus {
                    attack: attack * mul,
                    health: health * mul,
                },
                // A golden Choose One card doubles these numbers too
                // (Fearless Foodie, Sly Infiltrator, Snare Trapper, Veteran
                // Brigand, Crater Miner's Gem Days).
                ChooseOption::SpellEffect(se) => ChooseOption::SpellEffect(match se {
                    SpellEffect::GetGems(n) => SpellEffect::GetGems(n * mul as u8),
                    SpellEffect::GemsOnBoard(n) => SpellEffect::GemsOnBoard(n * mul as u8),
                    SpellEffect::GetCard { card, count } => {
                        SpellEffect::GetCard { card, count: count * mul as u8 }
                    }
                    SpellEffect::GemBonus { attack, health } => {
                        SpellEffect::GemBonus { attack: attack * mul, health: health * mul }
                    }
                    SpellEffect::FreeRolls(n) => SpellEffect::FreeRolls(n * mul as u8),
                    SpellEffect::MaxGold(n) => SpellEffect::MaxGold(n * mul as u8),
                    SpellEffect::CastSpell { card, times } => {
                        SpellEffect::CastSpell { card, times: times * mul as u8 }
                    }
                    SpellEffect::RandomTribeMinion { tribe, count } => {
                        SpellEffect::RandomTribeMinion { tribe, count: count * mul as u8 }
                    }
                    other => other,
                }),
                other => other,
            });
            // An option no state can answer (a future card with only targeted
            // options and no target): the minion is played and the choice is
            // skipped, the same guard a Choose One spell gets.
            // An option no state can answer (a future card with only targeted
            // options and no target): the minion is played and the choice is
            // skipped, the same guard a Choose One spell gets.
            if !scaled.iter().any(|&opt| self.can_choose_option(p, opt)) {
                return;
            }
            // Both effects combined: the card says so (Fandral's Fortune) or
            // a Trailblazer covers this play.
            let covered = !m.both_effects
                && (self.players[p].tp.choose_one_all || self.take_trailblazer_cover(p));
            if m.both_effects || covered {
                self.resolve_choose_options(p, Some(pos as u8), scaled, true, true);
            } else {
                self.players[p].pending_choose_one = Some(PendingChooseOne {
                    source: Some(pos as u8),
                    golden: m.golden,
                    options: scaled,
                    tavern: true,
                });
            }
            return;
        }
        let Some(bc) = card.battlecry else { return };
        // How many times the Battlecry triggers: Brann's set-or-add count,
        // War Drum's two extra on the first Battlecry of the turn and Dragon's
        // Eye's one more for a Dragon (docs/trinkets.md).
        let mut runs = self.brann_mult(p);
        runs = runs.saturating_add(self.players[p].prize.battlecry_extra_turn);
        runs = runs.saturating_add(self.take_war_drum(p));
        if self.players[p].tp.dragon_battlecry > 0
            && self.data.cards.tribes_of(&m).has(Tribe::Dragon)
        {
            runs = runs.saturating_add(self.players[p].tp.dragon_battlecry);
        }
        self.run_effect(p, &mut pos, bc, m.golden, runs);
        self.fire_after_battlecry(p, runs);
        // Dragon Skull: once per Battlecry trigger, so a Brann double fires it
        // twice.
        for _ in 0..runs {
            self.trinket_event(p, crate::trinkets::Event::Battlecry, Some(m.card), 1);
        }
    }

    /// Run the Battlecry of the minion in board slot `pos`, for a trigger
    /// that is not a play (Young Murk-Eye Sticker, Hackerfin Portrait, the
    /// Snicker-snack power). With `random`, a target the Battlecry asks for
    /// is answered at random, out of the board and the Tavern
    /// (docs/trinkets.md: "a Battlecry that needs a target picks one at
    /// random, and that can be a Tavern minion").
    pub(super) fn run_battlecry_at(&mut self, p: usize, pos: usize, random: bool) {
        if pos >= self.players[p].board.len() {
            return;
        }
        self.battlecry_or_choose_one(p, pos);
        if random {
            self.answer_target_at_random(p);
        }
    }

    /// Answer the target a Battlecry left open with a random legal minion,
    /// the board or the Tavern. Nothing legal: the target is dropped.
    pub(super) fn answer_target_at_random(&mut self, p: usize) {
        let Some(pt) = self.players[p].pending_target else { return };
        let mut board: ArrayVec<usize, { crate::BOARD }> = ArrayVec::new();
        for s in 0..self.players[p].board.len() {
            if self.valid_target(p, &pt, s) {
                let _ = board.try_push(s);
            }
        }
        let mut shop: ArrayVec<usize, { crate::SHOP }> = ArrayVec::new();
        for s in 0..self.players[p].shop.len() {
            if self.valid_battlecry_shop_target(p, &pt, s) {
                let _ = shop.try_push(s);
            }
        }
        let total = board.len() + shop.len();
        if total == 0 {
            self.players[p].pending_target = None;
            return;
        }
        let k = self.rng.below(total as u32) as usize;
        if k < board.len() {
            self.resolve_target(p, board[k]);
        } else {
            self.resolve_shop_target(p, shop[k - board.len()]);
        }
    }

    /// "After you trigger a Battlecry" effects (Kalecgos). One trigger per
    /// Battlecry run, so Brann's double Battlecry fires them twice.
    pub(super) fn fire_after_battlecry(&mut self, p: usize, times: u8) {
        let len = self.players[p].board.len();
        for i in 0..len {
            if i >= self.players[p].board.len() {
                break;
            }
            let m = self.players[p].board[i];
            for (card, golden) in m.parts() {
                if let Some(e) = self.data.cards.get(card).after_battlecry {
                    let mut pos = i;
                    self.run_effect(p, &mut pos, e, golden, times);
                }
            }
        }
    }
}
