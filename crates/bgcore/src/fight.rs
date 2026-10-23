//! The fight: a pure function of two boards, their counters, and an RNG.
//! The order of events follows DESIGN.md, Part 1, and matches `~/projects/bg`'s
//! engine where the design leaves room (attack pointer, Windfury swings, death
//! order), so the two can be cross-checked.
//!
//! Rally, "whenever" and "after a friendly minion attacks" triggers, Avenge,
//! damage milestones and Deathrattles all run the same `Effect`s, through
//! `Side::run_effect`. The Tavern attack of a Fishbait runs through here too
//! (`tavern_attack`), so everything that reacts to an attack reacts to it.

use arrayvec::ArrayVec;

use crate::cards::{
    N_COUNTERS, AttackFilter, Aura, CardDb, Deathrattle, Effect, OnSummonBeast, PermanentKeep,
    SpellEffect, StartOfCombat, Tribe,
};
use crate::minion::{CardId, Keywords, Minion};
use crate::rng::Rng;
use crate::{BOARD, HAND};

/// Per-player counters passed into a fight.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Counters {
    /// The owner's hero Health, for Holy Vanguard; None when unknown.
    pub health: Option<i32>,
    pub beetle_attack: i32,
    pub beetle_health: i32,
    /// How many times each "improve your future ..." effect has triggered
    /// (`Counter::index`), which scales its next buff.
    pub improve: [u16; N_COUNTERS],
    /// The Beast bonus the player has until next turn (Goldrinn in the
    /// shop). Beasts already on the board carry it in their stats; Beasts
    /// that show up in the fight get it.
    pub beast_attack: i32,
    pub beast_health: i32,
    /// The owner's Tavern tier (Ghastcoiler picks Deathrattles up to it).
    /// 0 means unknown, which allows any tier.
    pub tier: u8,
    pub boon_of_beetles: u8,
    pub brood_of_nozdormu: u8,
    pub upper_hand: u8,
    /// The player's Tavern spell bonus, for spells cast inside the fight
    /// (Runic Arcanist, Crimson Vindicator).
    pub spell_attack: i32,
    pub spell_health: i32,
    /// Sandbox only: "any minion" draws inside the fight (Ghastcoiler) only
    /// pick this tribe and neutral minions. None in a normal game.
    pub rig: Option<crate::cards::Tribe>,
}

/// Side effects from a fight that carry over to one player after combat.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SideOutcome {
    /// Permanent improvement stacks gained by the minion that started in each
    /// board slot (Lurking Leviathan, Treasure Parrot damage).
    pub permanent_improvements: [u16; BOARD],
    /// Increase to the player's Beetle bonus during this fight (Turquoise
    /// Skitterer, Ravaging Scorpid).
    pub beetle_attack_gain: i32,
    pub beetle_health_gain: i32,
    /// Increase to the player's improve counts (Tasty Lobster).
    pub improve_gain: [u16; N_COUNTERS],
    /// Stats kept permanently by the minion in each starting board slot
    /// (Tarecgosa, Persistent Poet). The game adds them to the board after
    /// the fight.
    pub permanent_stats: [(i32, i32); BOARD],
    /// Bonus Keywords kept permanently, by starting board slot.
    pub permanent_keywords: [u8; BOARD],
    /// Increase to the player's Tavern spell bonus this game (Blue Whelp,
    /// and any spell bonus an effect grants in a fight).
    pub spell_bonus_gain: (i32, i32),
    /// Increase to the player's Beast bonus until next turn (Goldrinn). Only
    /// a Tavern attack keeps it, a fight's is gone with its board.
    pub beast_bonus_gain: (i32, i32),
    /// Deathrattles the player triggered in this fight (Falling Sky Golem).
    pub deathrattles_triggered: u16,
    /// Generated cards added to the player's hand during this fight
    /// (Headhunter Gryphon, Conveyor Construct, Rescue Bot).
    pub cards: [CardId; HAND],
    pub n_cards: u8,
    /// Permanent stat gains for the minions in each starting hand slot
    /// (Very Hungry Winterfinner). The game adds them to the hand after the
    /// fight; combat changes to minions summoned from hand are not kept.
    pub hand_stats: [(i32, i32); HAND],
    /// Number of Boon of Beetles Beetles summoned during this fight.
    pub boon_of_beetles_used: u8,
    /// Tavern stats the player's minions gained for the rest of the game
    /// (Sacrificial Wrathguard's Deathrattle). The game applies it as an
    /// ongoing Tavern buff after the fight.
    pub tavern_buff_gain: (i32, i32),
    /// Upcoming Refreshes that get Fodders (Trapped Clapper), oldest first.
    pub fodder_refreshes: [u8; 16],
    pub n_fodder_refreshes: u8,
}

impl SideOutcome {
    pub fn cards_gained(&self) -> &[CardId] {
        &self.cards[..self.n_cards as usize]
    }

    fn push_card(&mut self, c: CardId) {
        let n = self.n_cards as usize;
        if n < HAND {
            self.cards[n] = c;
            self.n_cards += 1;
        }
    }

    /// Queue `count` upcoming Refreshes that each get `per` Fodders. It
    /// stacks: Refreshes already queued get these on top.
    fn push_fodder_refreshes(&mut self, per: u8, count: u32) {
        for i in 0..count as usize {
            if i < self.n_fodder_refreshes as usize {
                self.fodder_refreshes[i] = self.fodder_refreshes[i].saturating_add(per);
            } else if i < self.fodder_refreshes.len() {
                self.fodder_refreshes[i] = per;
                self.n_fodder_refreshes += 1;
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// 0 for the first board, 1 for the second, None for a tie.
    pub winner: Option<usize>,
    /// Sum of the tiers of the winner's surviving minions, 0 on a tie.
    pub survivor_tiers: i32,
    pub attacks: u32,
    pub sides: [SideOutcome; 2],
}

/// Watches a fight, for tests and debugging. Every method defaults to doing
/// nothing, and `()` watches nothing, so a plain fight pays nothing for it.
pub trait Watch {
    fn attack(&mut self, _side: usize, _attacker: &Minion, _defender: &Minion) {}
    fn death(&mut self, _side: usize, _pos: usize, _m: &Minion) {}
    fn summon(&mut self, _side: usize, _pos: usize, _m: &Minion) {}
}

impl Watch for () {}

#[derive(Clone, Copy)]
struct Fm {
    m: Minion,
    uid: u16,
    start_slot: Option<u8>,
    /// Friendly minions that died while this one lived (Avenge).
    avenge: u16,
    /// The uid of the minion that dealt the damage that killed this one.
    killer: Option<u16>,
}

/// Who an effect runs for and where it summons.
#[derive(Clone, Copy)]
struct Fx {
    side: usize,
    /// uid of the minion that has the effect (on the board, unless it died).
    host: u16,
    /// uid of the minion that killed the host, for a Deathrattle.
    killer: Option<u16>,
    /// uid of the minion that just attacked, for "give it +..." triggers.
    attacker: Option<u16>,
    /// Summons made right of the attacker don't attack in the same round.
    skip_ptr: bool,
}

struct Side<'a> {
    ms: ArrayVec<Fm, BOARD>,
    /// The hand as of the start of the fight, borrowed: it is empty unless
    /// something on the board (or in the hand) reads it. The fight never
    /// writes to it; combat-time gains live in `hand_bonus` and permanent
    /// ones in `SideOutcome::hand_stats`.
    hand: &'a [Minion],
    /// Combat-only stat gains on the hand, by hand slot.
    hand_bonus: [(i32, i32); HAND],
    /// Whether any minion on the board reacts to taking damage.
    damage_trigger: bool,
    /// Bit `i` marks the hand minion in slot `i` summoned for this combat
    /// only: later hand checks no longer see it.
    hand_taken: u16,
    /// Index of the next minion to attack.
    ptr: usize,
    counters: Counters,
    /// Combat-only buff to all Beasts on this side (Humming Bird, Goldrinn).
    beast_buff: (i32, i32),
    /// Buffs waiting for a minion that killed a Fishbait, by its uid. They
    /// are handed out by `resolve_deaths`, which sees both sides.
    killer_buffs: ArrayVec<(u16, i32, i32), 8>,
    /// Minions to destroy, by uid, from a "destroy the minion that killed
    /// this" Deathrattle (Leeroy). Applied by `resolve_deaths`, which sees
    /// both sides.
    killer_destroy: ArrayVec<u16, 8>,
    out: SideOutcome,
}


fn has_taunt(db: &CardDb, m: &Minion) -> bool {
    m.keywords.has(Keywords::TAUNT)
        || (m.keywords.has(Keywords::DIVINE_SHIELD)
            && m.parts().any(|(c, _)| db.get(c).aura == Some(Aura::TauntWhileShielded)))
}

fn filter_ok(db: &CardDb, f: AttackFilter, attacker: &Minion) -> bool {
    f.tribe.is_none_or(|t| db.get(attacker.card).tribes.has(t))
        && (!f.rally || attacker.parts().any(|(c, _)| db.get(c).rally.is_some()))
}

impl<'a> Side<'a> {
    fn new(
        db: &CardDb,
        board: &[Minion],
        hand: &'a [Minion],
        counters: Counters,
        uid: &mut u16,
    ) -> Side<'a> {
        let mut ms = ArrayVec::new();
        for (slot, &m) in board.iter().take(BOARD).enumerate() {
            ms.push(Fm { m, uid: *uid, start_slot: Some(slot as u8), avenge: 0, killer: None });
            *uid += 1;
        }
        let damage_trigger = ms
            .iter()
            .any(|f| f.m.parts().any(|(c, _)| db.get(c).when_damaged.is_some()));
        // Holy Vanguard: +30/+30 while the hero has 15 or less Health. The
        // Health already includes every damage reaction (a rewound hit has
        // given its Health back).
        if counters.health.is_some_and(|h| h <= 15) {
            for f in &mut ms {
                let mut bonus = 0;
                for (card, golden) in f.m.parts() {
                    if db.get(card).aura == Some(Aura::HolyVanguard) {
                        bonus = bonus.max(30 * if golden { 2 } else { 1 });
                    }
                }
                f.m.attack += bonus;
                f.m.health += bonus;
            }
        }
        Side {
            ms,
            hand: &hand[..hand.len().min(HAND)],
            hand_bonus: [(0, 0); HAND],
            damage_trigger,
            hand_taken: 0,
            ptr: 0,
            counters,
            beast_buff: (counters.beast_attack, counters.beast_health),
            killer_buffs: ArrayVec::new(),
            killer_destroy: ArrayVec::new(),
            out: SideOutcome::default(),
        }
    }

    /// A hand minion as it stands now: its card copy plus this combat's
    /// stat gains.
    fn hand_at(&self, i: usize) -> Minion {
        let mut m = self.hand[i];
        m.attack += self.hand_bonus[i].0;
        m.health += self.hand_bonus[i].1;
        m
    }

    fn hand_taken(&self, i: usize) -> bool {
        self.hand_taken & (1 << i) != 0
    }

    /// The hand minion with the highest Attack of a tribe that is still in
    /// hand; ties go to the left-most. Spells are not minions.
    fn best_in_hand(&self, db: &CardDb, tribe: Tribe) -> Option<usize> {
        let mut best: Option<usize> = None;
        for (i, m) in self.hand.iter().enumerate() {
            if self.hand_taken(i)
                || db.get(m.card).is_spell()
                || !db.get(m.card).tribes.has(tribe)
            {
                continue;
            }
            let attack = m.attack + self.hand_bonus[i].0;
            if best.is_none_or(|b| {
                attack > self.hand[b].attack + self.hand_bonus[b].0
            }) {
                best = Some(i);
            }
        }
        best
    }

    /// Summon the `n` highest-Attack minions of a tribe from hand for this
    /// combat. A summoned copy leaves the hand; it is not put back.
    #[allow(clippy::too_many_arguments)]
    fn summon_from_hand(
        &mut self,
        db: &CardDb,
        side: usize,
        at: &mut usize,
        n: u32,
        tribe: Tribe,
        skip_ptr: bool,
        uid: &mut u16,
        w: &mut impl Watch,
    ) {
        for _ in 0..n {
            if self.ms.is_full() {
                break;
            }
            let Some(i) = self.best_in_hand(db, tribe) else { break };
            let m = self.hand_at(i);
            self.hand_taken |= 1 << i;
            if !self.summon(db, side, *at, m, uid, w) {
                self.hand_taken &= !(1 << i);
                break;
            }
            if skip_ptr && *at == self.ptr {
                self.ptr += 1;
            }
            *at += 1;
        }
    }

    /// Give the minion in hand slot `i` stats, permanently (the game adds
    /// them to the real hand after the fight).
    fn hand_buff(&mut self, db: &CardDb, i: usize, da: i32, dh: i32) {
        if db.get(self.hand[i].card).fixed_stats {
            return;
        }
        self.hand_bonus[i].0 = self.hand_bonus[i].0.saturating_add(da);
        self.hand_bonus[i].1 = self.hand_bonus[i].1.saturating_add(dh);
        self.out.hand_stats[i].0 = self.out.hand_stats[i].0.saturating_add(da);
        self.out.hand_stats[i].1 = self.out.hand_stats[i].1.saturating_add(dh);
    }

    /// Give a random minion still in hand stats (Very Hungry Winterfinner).
    fn buff_random_hand(&mut self, db: &CardDb, da: i32, dh: i32, rng: &mut Rng) {
        let mut ok: ArrayVec<usize, HAND> = ArrayVec::new();
        for (i, m) in self.hand.iter().enumerate() {
            if !self.hand_taken(i) && !db.get(m.card).is_spell() {
                ok.push(i);
            }
        }
        if ok.is_empty() {
            return;
        }
        let i = ok[rng.below(ok.len() as u32) as usize];
        self.hand_buff(db, i, da, dh);
    }

    /// A friendly minion took damage while in board slot `i`.
    fn when_damaged(
        &mut self,
        db: &CardDb,
        side: usize,
        i: usize,
        uid: &mut u16,
        rng: &mut Rng,
        w: &mut impl Watch,
    ) {
        // Most minions have nothing to do here; check before copying.
        if !self.ms[i].m.parts().any(|(c, _)| db.get(c).when_damaged.is_some()) {
            return;
        }
        let m = self.ms[i].m;
        let host = self.ms[i].uid;
        for (card, golden) in m.parts() {
            if let Some(e) = db.get(card).when_damaged {
                let fx = Fx { side, host, killer: None, attacker: None, skip_ptr: false };
                let mut at = i + 1;
                self.run_effect(db, e, golden, fx, &mut at, uid, rng, w);
            }
        }
    }

    fn index_of(&self, uid: u16) -> Option<usize> {
        self.ms.iter().position(|f| f.uid == uid)
    }

    /// How much of a stat gain at board slot `i` is kept permanently: 1, 2
    /// for a golden keeper, None when nothing keeps it. Tarecgosa keeps its
    /// own gains; a Dragon keeps its when a living Persistent Poet sits next
    /// to it, checked at the moment of the gain.
    fn keep_mul(&self, db: &CardDb, i: usize) -> Option<i32> {
        let m = &self.ms[i].m;
        let mut mul = 0;
        for (card, golden) in m.parts() {
            if db.get(card).permanent_keep == Some(PermanentKeep::SelfKeep) {
                mul = mul.max(if golden { 2 } else { 1 });
            }
        }
        if db.get(m.card).tribes.has(Tribe::Dragon) {
            for j in [i.checked_sub(1), Some(i + 1)].into_iter().flatten() {
                if j >= self.ms.len() || self.ms[j].m.health <= 0 {
                    continue;
                }
                for (card, golden) in self.ms[j].m.parts() {
                    if db.get(card).permanent_keep == Some(PermanentKeep::AdjacentDragons) {
                        mul = mul.max(if golden { 2 } else { 1 });
                    }
                }
            }
        }
        (mul > 0).then_some(mul)
    }

    /// Give the minion in slot `i` stats. A keeper keeps them; Scarlet
    /// Survivor gains its keyword when the gain takes it to its threshold.
    fn gain_at(&mut self, db: &CardDb, i: usize, da: i32, dh: i32) {
        if db.get(self.ms[i].m.card).fixed_stats {
            return;
        }
        let keep = self.keep_mul(db, i);
        let mut threshold = 0u8;
        let f = &mut self.ms[i];
        f.m.attack = f.m.attack.saturating_add(da);
        f.m.health = f.m.health.saturating_add(dh);
        if let Some(t) = db.get(f.m.card).attack_threshold
            && f.m.improvements == 0
            && f.m.attack >= t.total as i32
        {
            f.m.keywords.set(t.keyword);
            f.m.improvements = 1;
            threshold = t.keyword;
        }
        let slot = f.start_slot;
        if let Some(mul) = keep
            && let Some(slot) = slot
        {
            let s = slot as usize;
            if threshold != 0 {
                self.out.permanent_keywords[s] |= threshold;
            }
            self.out.permanent_stats[s].0 =
                self.out.permanent_stats[s].0.saturating_add(da.saturating_mul(mul));
            self.out.permanent_stats[s].1 =
                self.out.permanent_stats[s].1.saturating_add(dh.saturating_mul(mul));
        }
    }

    /// Give the minion in slot `i` stats that it keeps after the fight
    /// regardless of other keepers (Devout Hellcaller).
    fn gain_permanent_at(&mut self, db: &CardDb, i: usize, da: i32, dh: i32) {
        if db.get(self.ms[i].m.card).fixed_stats {
            return;
        }
        let f = &mut self.ms[i];
        f.m.attack = f.m.attack.saturating_add(da);
        f.m.health = f.m.health.saturating_add(dh);
        if let Some(slot) = f.start_slot {
            let s = slot as usize;
            self.out.permanent_stats[s].0 = self.out.permanent_stats[s].0.saturating_add(da);
            self.out.permanent_stats[s].1 = self.out.permanent_stats[s].1.saturating_add(dh);
        }
    }

    /// A friendly Demon (`attacker_uid`) dealt damage: every other friendly
    /// minion with "After another friendly Demon deals damage" gains its
    /// stats for good (Devout Hellcaller).
    fn note_demon_damage(&mut self, db: &CardDb, attacker_uid: u16) {
        let mut hits: ArrayVec<(u16, i32, i32), BOARD> = ArrayVec::new();
        for f in &self.ms {
            if f.m.health <= 0 || f.uid == attacker_uid {
                continue;
            }
            for (card, golden) in f.m.parts() {
                if let Some((a, h)) = db.get(card).after_demon_damage {
                    let mul = if golden { 2 } else { 1 };
                    let _ = hits.try_push((f.uid, a * mul, h * mul));
                }
            }
        }
        for (host, da, dh) in hits {
            if let Some(i) = self.index_of(host) {
                self.gain_permanent_at(db, i, da, dh);
            }
        }
    }

    /// "Give your minions +attack/+health" cast inside a fight, with the
    /// player's spell bonus.
    fn cast_board_buff(&mut self, db: &CardDb, attack: i32, health: i32) {
        let (da, dh) = (attack + self.counters.spell_attack, health + self.counters.spell_health);
        for i in 0..self.ms.len() {
            if self.ms[i].m.health > 0 {
                self.gain_at(db, i, da, dh);
            }
        }
    }

    /// Mighty Dragonbreath: all minions, again for Dragons, again for
    /// minions with Divine Shield.
    fn cast_mighty_dragonbreath(&mut self, db: &CardDb, attack: i32, health: i32) {
        let (da, dh) = (attack + self.counters.spell_attack, health + self.counters.spell_health);
        for pass in 0..3 {
            for i in 0..self.ms.len() {
                if self.ms[i].m.health <= 0 {
                    continue;
                }
                let ok = match pass {
                    0 => true,
                    1 => db.get(self.ms[i].m.card).tribes.has(Tribe::Dragon),
                    _ => self.ms[i].m.keywords.has(Keywords::DIVINE_SHIELD),
                };
                if ok {
                    self.gain_at(db, i, da, dh);
                }
            }
        }
    }

    /// The minion in slot `i` gains a keyword. A keeper keeps it.
    fn gain_keyword_at(&mut self, db: &CardDb, i: usize, keyword: u8) {
        self.ms[i].m.keywords.set(keyword);
        if self.keep_mul(db, i).is_some()
            && let Some(slot) = self.ms[i].start_slot
        {
            self.out.permanent_keywords[slot as usize] |= keyword;
        }
    }

    /// A friendly minion just lost Divine Shield: every Hopebringer on this
    /// side improves by one for the rest of the game. The count lives in its
    /// `improvements` and is carried to the player's real board.
    fn note_shield_lost(&mut self, db: &CardDb) {
        for i in 0..self.ms.len() {
            if self.ms[i].m.health <= 0 {
                continue;
            }
            let n = self.ms[i]
                .m
                .parts()
                .filter(|(c, _)| {
                    matches!(db.get(*c).start_of_combat, Some(StartOfCombat::Hopebringer { .. }))
                })
                .count() as u16;
            if n == 0 {
                continue;
            }
            self.ms[i].m.improvements = self.ms[i].m.improvements.saturating_add(n);
            if let Some(slot) = self.ms[i].start_slot {
                let s = slot as usize;
                self.out.permanent_improvements[s] =
                    self.out.permanent_improvements[s].saturating_add(n);
            }
        }
    }

    fn next_attacker(&self) -> Option<usize> {
        let n = self.ms.len();
        if n == 0 {
            return None;
        }
        let p = self.ptr % n;
        (0..n).map(|k| (p + k) % n).find(|&i| self.ms[i].m.attack > 0)
    }

    fn remove(&mut self, i: usize) -> Fm {
        if i < self.ptr {
            self.ptr -= 1;
        }
        self.ms.remove(i)
    }

    /// False when the board is full and the minion is lost.
    fn insert(&mut self, i: usize, f: Fm) -> bool {
        if self.ms.is_full() {
            return false;
        }
        self.ms.insert(i, f);
        if i < self.ptr {
            self.ptr += 1;
        }
        true
    }

    fn add_beetle_buff(&mut self, db: &CardDb, da: i32, dh: i32) {
        self.counters.beetle_attack += da;
        self.counters.beetle_health += dh;
        self.out.beetle_attack_gain += da;
        self.out.beetle_health_gain += dh;
        for i in 0..self.ms.len() {
            if self.ms[i].m.health > 0 && db.get(self.ms[i].m.card).is_beetle {
                self.gain_at(db, i, da, dh);
            }
        }
    }

    fn add_combat_beast_buff(&mut self, db: &CardDb, da: i32, dh: i32) {
        self.beast_buff.0 += da;
        self.beast_buff.1 += dh;
        self.out.beast_bonus_gain.0 += da;
        self.out.beast_bonus_gain.1 += dh;
        for i in 0..self.ms.len() {
            if self.ms[i].m.health > 0 && db.get(self.ms[i].m.card).tribes.has(Tribe::Beast) {
                self.gain_at(db, i, da, dh);
            }
        }
    }

    /// A Deathrattle of this side triggered.
    fn note_deathrattle(&mut self, db: &CardDb) {
        self.out.deathrattles_triggered = self.out.deathrattles_triggered.saturating_add(1);
        for i in 0..self.ms.len() {
            if self.ms[i].m.health <= 0 {
                continue;
            }
            let m = self.ms[i].m;
            for (card, golden) in m.parts() {
                if let Some((a, h)) = db.get(card).buff_per_deathrattle {
                    let mul = if golden { 2 } else { 1 };
                    self.gain_at(db, i, a * mul, h * mul);
                }
            }
        }
    }

    fn summon(
        &mut self,
        db: &CardDb,
        side: usize,
        pos: usize,
        mut m: Minion,
        uid: &mut u16,
        w: &mut impl Watch,
    ) -> bool {
        if self.ms.is_full() {
            return false;
        }
        let pos = pos.min(self.ms.len());
        let c = db.get(m.card);
        if c.when_damaged.is_some() {
            self.damage_trigger = true;
        }
        if c.is_beetle {
            m.attack += self.counters.beetle_attack;
            m.health += self.counters.beetle_health;
        }
        self.insert(pos, Fm { m, uid: *uid, start_slot: None, avenge: 0, killer: None });
        *uid += 1;
        if c.tribes.has(Tribe::Beast) {
            self.ms[pos].m.attack += self.beast_buff.0;
            self.ms[pos].m.health += self.beast_buff.1;
            for j in 0..self.ms.len() {
                if j == pos || self.ms[j].m.health <= 0 {
                    continue;
                }
                let imp = self.ms[j].m.improvements;
                let start_slot = self.ms[j].start_slot;
                let mut da_total = 0;
                let mut dh_total = 0;
                let mut imp_gain = 0u16;
                for (card, golden) in self.ms[j].m.parts() {
                    if let Some(OnSummonBeast::Buff { attack, health, improve_permanently }) =
                        db.get(card).on_summon_beast
                    {
                        let mul = if golden { 2 } else { 1 };
                        let stacks = if improve_permanently {
                            1 + (imp + imp_gain) as i32
                        } else {
                            1
                        };
                        da_total += attack * mul * stacks;
                        dh_total += health * mul * stacks;
                        if improve_permanently {
                            imp_gain += 1;
                        }
                    }
                }
                if imp_gain > 0 {
                    self.ms[j].m.improvements = self.ms[j].m.improvements.saturating_add(imp_gain);
                    if let Some(slot) = start_slot {
                        let s = slot as usize;
                        self.out.permanent_improvements[s] =
                            self.out.permanent_improvements[s].saturating_add(imp_gain);
                    }
                }
                self.gain_at(db, pos, da_total, dh_total);
            }
            for j in 0..self.ms.len() {
                if j == pos || self.ms[j].m.health <= 0 {
                    continue;
                }
                let mj = self.ms[j].m;
                for (card, golden) in mj.parts() {
                    if matches!(db.get(card).on_summon_beast, Some(OnSummonBeast::MultiplyAttackInCombat)) {
                        let factor = if golden { 3 } else { 2 };
                        self.ms[pos].m.attack *= factor;
                    }
                }
            }
        }
        w.summon(side, pos, &self.ms[pos].m);
        true
    }

    fn spawn_boon_of_beetles(
        &mut self,
        db: &CardDb,
        side: usize,
        uid: &mut u16,
        w: &mut impl Watch,
    ) {
        if self.counters.boon_of_beetles == 0 || self.ms.is_full() {
            return;
        }
        let Some(tok) = db.beetle_token else { return };
        while self.counters.boon_of_beetles > 0 && !self.ms.is_full() {
            let mut m = db.make(tok, false);
            m.keywords.set(Keywords::TAUNT);
            let pos = self.ms.len();
            if self.summon(db, side, pos, m, uid, w) {
                self.counters.boon_of_beetles -= 1;
                self.out.boon_of_beetles_used = self.out.boon_of_beetles_used.saturating_add(1);
            } else {
                break;
            }
        }
    }

    /// How many times one Deathrattle fires for this side: once, plus one
    /// for each Titus Rivendare and two for a golden one. Every path that
    /// fires a Deathrattle asks here, so a new way to repeat them (trinkets,
    /// quest rewards) is added in this one place.
    fn deathrattle_fires(&self, db: &CardDb) -> u32 {
        let mut fires = 1;
        for f in &self.ms {
            if f.m.health > 0 {
                for (card, golden) in f.m.parts() {
                    if db.get(card).aura == Some(Aura::Titus) {
                        fires += if golden { 2 } else { 1 };
                    }
                }
            }
        }
        fires
    }

    /// Run one effect for the minion `fx.host`. `at` is where summons go
    /// and moves right with each one. Golden doubles the effect.
    #[allow(clippy::too_many_arguments)]
    fn run_effect(
        &mut self,
        db: &CardDb,
        e: Effect,
        golden: bool,
        fx: Fx,
        at: &mut usize,
        uid: &mut u16,
        rng: &mut Rng,
        w: &mut impl Watch,
    ) {
        let mul: i32 = if golden { 2 } else { 1 };
        match e {
            Effect::Summon { card, count, golden_doubles_count } => {
                let (n, g) = if golden_doubles_count {
                    (count * mul as u8, false)
                } else {
                    (count, golden)
                };
                for _ in 0..n {
                    let m = db.make(card, g);
                    if self.summon(db, fx.side, *at, m, uid, w) {
                        if fx.skip_ptr && *at == self.ptr {
                            self.ptr += 1;
                        }
                        *at += 1;
                    }
                }
            }
            Effect::BuffOthers { tribe, attack, health, .. } => {
                for i in 0..self.ms.len() {
                    if self.ms[i].uid != fx.host
                        && self.ms[i].m.health > 0
                        && db.get(self.ms[i].m.card).is(tribe)
                    {
                        self.gain_at(db, i, attack * mul, health * mul);
                    }
                }
            }
            Effect::BuffAll { tribe, attack, health } => {
                for i in 0..self.ms.len() {
                    if self.ms[i].m.health > 0 && db.get(self.ms[i].m.card).is(tribe) {
                        self.gain_at(db, i, attack * mul, health * mul);
                    }
                }
            }
            Effect::BuffSelf { attack, health } => {
                if let Some(i) = self.index_of(fx.host) {
                    self.gain_at(db, i, attack * mul, health * mul);
                }
            }
            Effect::BuffRandomFriendly { tribe, attack, health, improve } => {
                let mut ok: ArrayVec<usize, BOARD> = ArrayVec::new();
                for (i, f) in self.ms.iter().enumerate() {
                    if f.m.health > 0 && db.get(f.m.card).is(tribe) {
                        ok.push(i);
                    }
                }
                let scale = improve.map_or(1, |c| 1 + self.counters.improve[c.index()] as i32);
                if !ok.is_empty() {
                    let i = ok[rng.below(ok.len() as u32) as usize];
                    self.gain_at(db, i, attack * mul * scale, health * mul * scale);
                }
                if let Some(c) = improve {
                    let k = c.index();
                    self.counters.improve[k] = self.counters.improve[k].saturating_add(1);
                    self.out.improve_gain[k] = self.out.improve_gain[k].saturating_add(1);
                }
            }
            Effect::BuffKiller { attack, health } => {
                if let Some(k) = fx.killer {
                    let _ = self.killer_buffs.try_push((k, attack * mul, health * mul));
                }
            }
            Effect::GainKeyword { keyword } => {
                if let Some(i) = self.index_of(fx.host) {
                    self.gain_keyword_at(db, i, keyword);
                }
            }
            Effect::GetRandomMinion { tribe } => {
                for _ in 0..mul {
                    if let Some(c) = db.random_pool_minion(tribe, self.counters.tier, rng) {
                        self.out.push_card(c);
                    }
                }
            }
            Effect::AddFodderRefreshes(n) => {
                self.out.push_fodder_refreshes(mul as u8, n as u32);
            }
            Effect::SummonRandomDeathrattle { count } => {
                for _ in 0..count as i32 * mul {
                    let Some(c) = db.random_deathrattle_minion(self.counters.tier, self.counters.rig, rng) else { break };
                    let m = db.make(c, false);
                    if self.summon(db, fx.side, *at, m, uid, w) {
                        *at += 1;
                    }
                }
            }
            Effect::TriggerDeathrattle { times } => {
                for _ in 0..times as i32 * mul {
                    let Some(idx) = self.ms.iter().position(|f| {
                        f.m.health > 0 && f.m.parts().any(|(c, _)| db.get(c).deathrattle.is_some())
                    }) else {
                        break;
                    };
                    let (m, host) = (self.ms[idx].m, self.ms[idx].uid);
                    let runs = self.deathrattle_fires(db);
                    let n = self.fire_deathrattles(db, fx.side, &m, host, None, idx + 1, runs, uid, rng, w);
                    if idx < *at {
                        *at += n;
                    }
                }
            }
            Effect::BeetleBuff { attack, health } => {
                self.add_beetle_buff(db, attack * mul, health * mul);
            }
            Effect::RandomVolumizer => {
                for _ in 0..mul {
                    if let Some(v) = db.random_volumizer(rng) {
                        self.out.push_card(v);
                    }
                }
            }
            Effect::GetCard { card, count } => {
                for _ in 0..count as i32 * mul {
                    self.out.push_card(card);
                }
            }
            Effect::SpellBonus { attack, health } => {
                self.counters.spell_attack += attack * mul;
                self.counters.spell_health += health * mul;
                self.out.spell_bonus_gain.0 += attack * mul;
                self.out.spell_bonus_gain.1 += health * mul;
            }
            Effect::RandomChromadrake => {
                for _ in 0..mul {
                    if let Some(c) = db.random_chromadrake(rng) {
                        self.out.push_card(c);
                    }
                }
            }
            Effect::BuffAttacker { attack, health } => {
                if let Some(a) = fx.attacker
                    && let Some(i) = self.index_of(a)
                    && self.ms[i].m.health > 0
                {
                    self.gain_at(db, i, attack * mul, health * mul);
                }
            }
            Effect::BuffRandomOther { tribe, attack, health, keyword, count } => {
                // A golden Amber Guardian picks two Dragons, not bigger buffs.
                let n = count as u32 * mul as u32;
                let mut ok: ArrayVec<usize, BOARD> = ArrayVec::new();
                for (i, f) in self.ms.iter().enumerate() {
                    if f.uid != fx.host && f.m.health > 0 && db.get(f.m.card).is(tribe) {
                        ok.push(i);
                    }
                }
                for _ in 0..n {
                    if ok.is_empty() {
                        break;
                    }
                    let k = rng.below(ok.len() as u32) as usize;
                    let i = ok.remove(k);
                    self.gain_at(db, i, attack, health);
                    if keyword != 0 {
                        self.gain_keyword_at(db, i, keyword);
                    }
                }
            }
            Effect::SummonFromHand { tribe, count } => {
                let n = count as u32 * mul as u32;
                self.summon_from_hand(db, fx.side, at, n, tribe, fx.skip_ptr, uid, w);
            }
            Effect::ShieldDragons { count } => {
                // Dragons without Divine Shield come first; once every one
                // has it, further picks are wasted on shielded Dragons.
                let n = count as i32 * mul;
                let mut without: ArrayVec<usize, BOARD> = ArrayVec::new();
                let mut with: ArrayVec<usize, BOARD> = ArrayVec::new();
                for (i, f) in self.ms.iter().enumerate() {
                    if f.m.health <= 0 || !db.get(f.m.card).tribes.has(Tribe::Dragon) {
                        continue;
                    }
                    if f.m.keywords.has(Keywords::DIVINE_SHIELD) {
                        let _ = with.try_push(i);
                    } else {
                        let _ = without.try_push(i);
                    }
                }
                for _ in 0..n {
                    let pool = if without.is_empty() { &mut with } else { &mut without };
                    if pool.is_empty() {
                        break;
                    }
                    let k = rng.below(pool.len() as u32) as usize;
                    let i = pool.remove(k);
                    self.gain_keyword_at(db, i, Keywords::DIVINE_SHIELD);
                }
            }
            Effect::DestroyKiller => {
                if let Some(k) = fx.killer {
                    let _ = self.killer_destroy.try_push(k);
                }
            }
            Effect::BuffRandomHand { attack, health } => {
                self.buff_random_hand(db, attack * mul, health * mul, rng);
            }
            Effect::CastSpell { card, times } => {
                let Some(sp) = db.get(card).spell_effect else { return };
                for _ in 0..times as i32 * mul {
                    match sp {
                        SpellEffect::BuffBoard { attack, health, times, .. } => {
                            for _ in 0..times {
                                self.cast_board_buff(db, attack, health);
                            }
                        }
                        SpellEffect::MightyDragonbreath { attack, health } => {
                            self.cast_mighty_dragonbreath(db, attack, health);
                        }
                        _ => break,
                    }
                }
            }
            // Parsing keeps every other effect out of fights.
            _ => {}
        }
    }

    /// Run the Deathrattles of `dead`, a minion that died or whose
    /// Deathrattle was triggered. Summons go in from `at`; returns how many
    /// were inserted.
    #[allow(clippy::too_many_arguments)]
    fn fire_deathrattles(
        &mut self,
        db: &CardDb,
        side: usize,
        dead: &Minion,
        host: u16,
        killer: Option<u16>,
        at: usize,
        runs: u32,
        uid: &mut u16,
        rng: &mut Rng,
        w: &mut impl Watch,
    ) -> usize {
        let mut pos = at;
        for _ in 0..runs {
            for (card, golden) in dead.parts() {
                let Some(dr) = db.get(card).deathrattle else { continue };
                self.note_deathrattle(db);
                let mul = if golden { 2 } else { 1 };
                match dr {
                    Deathrattle::Summon { card: tok, count, golden_doubles_count, beetle_buff } => {
                        if let Some((ba, bh)) = beetle_buff {
                            self.add_beetle_buff(db, ba * mul, bh * mul);
                        }
                        let (n, tok_golden) = if golden_doubles_count {
                            (count * mul as u8, false)
                        } else {
                            (count, golden)
                        };
                        for _ in 0..n {
                            let m = db.make(tok, tok_golden);
                            if self.summon(db, side, pos, m, uid, w) {
                                pos += 1;
                            }
                        }
                    }
                    Deathrattle::SummonByAttack { card: tok } => {
                        let n = dead.attack.max(0) as usize;
                        for _ in 0..n {
                            let m = db.make(tok, golden);
                            if self.summon(db, side, pos, m, uid, w) {
                                pos += 1;
                            }
                        }
                    }
                    Deathrattle::CombatBeastBuff { attack, health } => {
                        self.add_combat_beast_buff(db, attack * mul, health * mul);
                    }
                    Deathrattle::RandomVolumizer => {
                        for _ in 0..mul {
                            if let Some(v) = db.random_volumizer(rng) {
                                self.out.push_card(v);
                            }
                        }
                    }
                    Deathrattle::GetCard { card: tok, count } => {
                        for _ in 0..count as i32 * mul {
                            self.out.push_card(tok);
                        }
                    }
                    Deathrattle::TavernBuff { attack, health, per_improvement } => {
                        // "Improve this" stacks live in the dead minion's
                        // improvements (Sacrificial Wrathguard).
                        let n = dead.improvements as i32;
                        let da = (attack + per_improvement.0 * n) * mul;
                        let dh = (health + per_improvement.1 * n) * mul;
                        self.out.tavern_buff_gain.0 += da;
                        self.out.tavern_buff_gain.1 += dh;
                    }
                    Deathrattle::Effect(e) => {
                        let fx = Fx { side, host, killer, attacker: None, skip_ptr: false };
                        self.run_effect(db, e, golden, fx, &mut pos, uid, rng, w);
                    }
                }
            }
        }
        pos - at
    }

    /// A friendly minion died: every living minion with Avenge counts it, and
    /// runs its effect when the count reaches its N.
    fn avenge_tick(&mut self, db: &CardDb, side: usize, uid: &mut u16, rng: &mut Rng, w: &mut impl Watch) {
        let mut fire: ArrayVec<(u16, Effect, bool), 32> = ArrayVec::new();
        for f in &mut self.ms {
            if f.m.health <= 0 || !f.m.parts().any(|(c, _)| db.get(c).avenge.is_some()) {
                continue;
            }
            f.avenge = f.avenge.saturating_add(1);
            for (card, golden) in f.m.parts() {
                if let Some(a) = db.get(card).avenge
                    && f.avenge % a.every as u16 == 0
                {
                    let _ = fire.try_push((f.uid, a.effect, golden));
                }
            }
        }
        for (host, e, golden) in fire {
            let Some(i) = self.index_of(host) else { continue };
            let fx = Fx { side, host, killer: None, attacker: None, skip_ptr: false };
            let mut at = i + 1;
            self.run_effect(db, e, golden, fx, &mut at, uid, rng, w);
        }
    }

    /// The minion in slot `i` dealt `dealt` damage: count it towards a damage
    /// milestone (Treasure Parrot).
    #[allow(clippy::too_many_arguments)]
    fn count_damage(&mut self, db: &CardDb, side: usize, i: usize, dealt: i32, uid: &mut u16, rng: &mut Rng, w: &mut impl Watch) {
        if dealt <= 0 {
            return;
        }
        let m = self.ms[i].m;
        let host = self.ms[i].uid;
        for (card, golden) in m.parts() {
            let Some(ms) = db.get(card).damage_milestone else { continue };
            let n = dealt.min(u16::MAX as i32) as u16;
            let before = self.ms[i].m.improvements;
            let after = before.saturating_add(n);
            self.ms[i].m.improvements = after;
            if let Some(slot) = self.ms[i].start_slot {
                let s = slot as usize;
                self.out.permanent_improvements[s] = self.out.permanent_improvements[s].saturating_add(n);
            }
            if before < ms.total && after >= ms.total {
                let fx = Fx { side, host, killer: None, attacker: None, skip_ptr: false };
                let mut at = i + 1;
                self.run_effect(db, ms.effect, golden, fx, &mut at, uid, rng, w);
            }
        }
    }

    /// Remove the dead in board order. Each one's Deathrattle runs, then
    /// Avenge counts it, then its Reborn, before the next dead minion is
    /// looked at.
    fn resolve(
        &mut self,
        db: &CardDb,
        side: usize,
        uid: &mut u16,
        rng: &mut Rng,
        w: &mut impl Watch,
    ) -> bool {
        let mut changed = false;
        let mut i = 0;
        while i < self.ms.len() {
            if self.ms[i].m.health > 0 {
                i += 1;
                continue;
            }
            changed = true;
            let fm = self.remove(i);
            let dead = fm.m;
            w.death(side, i, &dead);
            let runs = self.deathrattle_fires(db);
            i += self.fire_deathrattles(db, side, &dead, fm.uid, fm.killer, i, runs, uid, rng, w);
            self.avenge_tick(db, side, uid, rng, w);
            if dead.keywords.has(Keywords::REBORN) {
                let mut m = db.make(dead.card, dead.golden);
                m.health = 1;
                m.keywords.clear(Keywords::REBORN);
                if self.summon(db, side, i, m, uid, w) {
                    i += 1;
                }
            }
        }
        changed
    }
}

/// Deal `dmg` non-combat damage (e.g. Boom-in-a-Box) to `target`. Returns
/// the damage its Health actually lost, 0 when a shield took it.
fn deal_damage(target: &mut Minion, dmg: i32) -> i32 {
    if dmg <= 0 {
        return 0;
    }
    if target.keywords.has(Keywords::DIVINE_SHIELD) {
        target.keywords.clear(Keywords::DIVINE_SHIELD);
        return 0;
    }
    target.health -= dmg;
    dmg
}

/// `source` deals `dmg` to `target`. Divine Shield takes any hit and keeps
/// Venomous unused; 0 damage does nothing at all. Returns the damage dealt,
/// 0 when the shield took it.
fn hit(target: &mut Minion, dmg: i32, source: &mut Minion) -> i32 {
    if dmg <= 0 {
        return 0;
    }
    if target.keywords.has(Keywords::DIVINE_SHIELD) {
        target.keywords.clear(Keywords::DIVINE_SHIELD);
        return 0;
    }
    target.health -= dmg;
    if source.keywords.has(Keywords::VENOMOUS) {
        target.health = target.health.min(0);
        source.keywords.clear(Keywords::VENOMOUS);
    }
    dmg
}

fn targetable(f: &Fm) -> bool {
    !f.m.keywords.has(Keywords::STEALTH)
}

/// The defender is random among the enemies. Taunt minions come first and
/// Stealth minions can never be picked. None when every enemy has Stealth:
/// the attacking side then loses its turn.
fn pick_defender(db: &CardDb, def: &Side, rng: &mut Rng) -> Option<usize> {
    let eligible = |f: &&Fm| targetable(f);
    let taunts = def.ms.iter().filter(eligible).filter(|f| has_taunt(db, &f.m)).count();
    if taunts == 0 {
        let n = def.ms.iter().filter(eligible).count() as u32;
        if n == 0 {
            return None;
        }
        let k = rng.below(n) as usize;
        return Some(def.ms.iter().enumerate().filter(|(_, f)| eligible(f)).nth(k).unwrap().0);
    }
    let k = rng.below(taunts as u32) as usize;
    def.ms
        .iter()
        .enumerate()
        .filter(|(_, f)| eligible(f) && has_taunt(db, &f.m))
        .nth(k)
        .map(|(i, _)| i)
}

fn pair<'s, 'a>(
    sides: &'s mut [Side<'a>; 2],
    turn: usize,
) -> (&'s mut Side<'a>, &'s mut Side<'a>) {
    let [a, b] = sides;
    if turn == 0 { (a, b) } else { (b, a) }
}

/// Hand out the buffs Fishbait Deathrattles gave to their killers.
fn give_killer_buffs(sides: &mut [Side<'_>; 2], db: &CardDb) {
    for s in 0..2 {
        while let Some((killer, da, dh)) = sides[s].killer_buffs.pop() {
            for t in 0..2 {
                if let Some(i) = sides[t].ms.iter().position(|f| f.uid == killer && f.m.health > 0)
                {
                    sides[t].gain_at(db, i, da, dh);
                }
            }
        }
    }
}

/// Hand out the "destroy the minion that killed this" queue (Leeroy). A
/// destroy ignores Divine Shield, and does nothing when the killer is gone.
fn apply_killer_destroys(sides: &mut [Side<'_>; 2]) -> bool {
    let mut queued: ArrayVec<u16, 16> = ArrayVec::new();
    for s in 0..2 {
        while let Some(u) = sides[s].killer_destroy.pop() {
            let _ = queued.try_push(u);
        }
    }
    let mut any = false;
    for u in queued {
        for t in 0..2 {
            if let Some(i) = sides[t].ms.iter().position(|f| f.uid == u && f.m.health > 0) {
                sides[t].ms[i].m.health = 0;
                any = true;
            }
        }
    }
    any
}

fn resolve_deaths(
    sides: &mut [Side<'_>; 2],
    first_side: usize,
    db: &CardDb,
    uid: &mut u16,
    rng: &mut Rng,
    w: &mut impl Watch,
) {
    loop {
        let (att, def) = pair(sides, first_side);
        let c1 = att.resolve(db, first_side, uid, rng, w);
        let c2 = def.resolve(db, first_side ^ 1, uid, rng, w);
        give_killer_buffs(sides, db);
        let c3 = apply_killer_destroys(sides);
        if !(c1 || c2 || c3) {
            break;
        }
    }
    let (att, def) = pair(sides, first_side);
    att.spawn_boon_of_beetles(db, first_side, uid, w);
    def.spawn_boon_of_beetles(db, first_side ^ 1, uid, w);
}

fn run_start_of_combat(
    sides: &mut [Side<'_>; 2],
    db: &CardDb,
    uid: &mut u16,
    rng: &mut Rng,
    w: &mut impl Watch,
) {
    for s in 0..2 {
        sides[s].spawn_boon_of_beetles(db, s, uid, w);
    }
    // Flighty Scout triggers from the hand, so it comes before the board's
    // Start of Combat effects. One copy each, in hand order, if there is
    // board space; the card stays in hand.
    for s in 0..2 {
        for i in 0..sides[s].hand.len() {
            if sides[s].hand_taken(i)
                || !matches!(
                    db.get(sides[s].hand[i].card).start_of_combat,
                    Some(StartOfCombat::FlightyScout)
                )
            {
                continue;
            }
            let m = sides[s].hand_at(i);
            let at = sides[s].ms.len();
            sides[s].summon(db, s, at, m, uid, w);
        }
    }
    for s in 0..2 {
        for _ in 0..sides[s].counters.brood_of_nozdormu {
            if let Some(f) = sides[s].ms.first_mut() {
                f.m.attack *= 2;
            }
        }
        for _ in 0..sides[s].counters.upper_hand {
            let opp = s ^ 1;
            let n = sides[opp].ms.len();
            if n > 0 {
                let k = rng.below(n as u32) as usize;
                sides[opp].ms[k].m.health = 1;
            }
        }
    }
    let mut soc: [ArrayVec<u16, BOARD>; 2] = [ArrayVec::new(), ArrayVec::new()];
    for s in 0..2 {
        for f in &sides[s].ms {
            if f.m.parts().any(|(c, _)| db.get(c).start_of_combat.is_some()) {
                soc[s].push(f.uid);
            }
        }
    }
    if soc[0].is_empty() && soc[1].is_empty() {
        return;
    }
    let mut turn = rng.coin() as usize;
    let mut idx = [0usize; 2];
    while idx[0] < soc[0].len() || idx[1] < soc[1].len() {
        if sides[0].ms.is_empty() || sides[1].ms.is_empty() {
            break;
        }
        if idx[turn] >= soc[turn].len() {
            turn ^= 1;
        }
        let me = soc[turn][idx[turn]];
        idx[turn] += 1;
        let Some(mi) = sides[turn].ms.iter().position(|f| f.uid == me) else {
            if idx[turn ^ 1] < soc[turn ^ 1].len() {
                turn ^= 1;
            }
            continue;
        };
        let m = sides[turn].ms[mi].m;
        let mut shields_lost = [false; 2];
        for (card, golden) in m.parts() {
            let mul = if golden { 2 } else { 1 };
            match db.get(card).start_of_combat {
                Some(StartOfCombat::CombatBeastBuff { attack, health }) => {
                    sides[turn].add_combat_beast_buff(db, attack * mul, health * mul);
                }
                Some(StartOfCombat::DamageAllOthers(dmg)) => {
                    let waves = if golden { 2 } else { 1 };
                    for _ in 0..waves {
                        for s in [turn, turn ^ 1] {
                            for i in 0..sides[s].ms.len() {
                                if sides[s].ms[i].uid == me {
                                    continue;
                                }
                                let had = sides[s].ms[i].m.keywords.has(Keywords::DIVINE_SHIELD);
                                let dealt = deal_damage(&mut sides[s].ms[i].m, dmg);
                                if had && !sides[s].ms[i].m.keywords.has(Keywords::DIVINE_SHIELD)
                                {
                                    shields_lost[s] = true;
                                }
                                if dealt > 0 && sides[s].damage_trigger {
                                    sides[s].when_damaged(db, s, i, uid, rng, w);
                                }
                            }
                        }
                    }
                }
                Some(StartOfCombat::Effect(e)) => {
                    let fx = Fx { side: turn, host: me, killer: None, attacker: None, skip_ptr: false };
                    let mut at = mi + 1;
                    sides[turn].run_effect(db, e, golden, fx, &mut at, uid, rng, w);
                }
                Some(StartOfCombat::AttackImmediately) => {
                    for _ in 0..mul {
                        attack_swings(sides, turn, me, db, uid, rng, w);
                    }
                }
                Some(StartOfCombat::Hopebringer { attack, health, improve }) => {
                    let Some(mi) = sides[turn].index_of(me) else { break };
                    let imp = sides[turn].ms[mi].m.improvements as i32;
                    // The improvement is part of the buff, so a golden
                    // Hopebringer doubles it too (+8/+6 per lost shield).
                    let da = attack * mul + imp * improve.0 * mul;
                    let dh = health * mul + imp * improve.1 * mul;
                    for i in 0..sides[turn].ms.len() {
                        if sides[turn].ms[i].m.health > 0 {
                            sides[turn].gain_at(db, i, da, dh);
                        }
                    }
                }
                Some(StartOfCombat::FlightyScout) => {
                    // It triggers from the hand, handled before this loop.
                }
                Some(StartOfCombat::GainHighestHandAttack) => {
                    let Some(mi) = sides[turn].index_of(me) else { break };
                    let mut best = 0;
                    for (i, m) in sides[turn].hand.iter().enumerate() {
                        if !sides[turn].hand_taken(i) && !db.get(m.card).is_spell() {
                            best = best.max(m.attack + sides[turn].hand_bonus[i].0);
                        }
                    }
                    sides[turn].gain_at(db, mi, best * mul, 0);
                }
                Some(StartOfCombat::GainHandStats) => {
                    let Some(mi) = sides[turn].index_of(me) else { break };
                    let (mut da, mut dh) = (0, 0);
                    for (i, m) in sides[turn].hand.iter().enumerate() {
                        if !sides[turn].hand_taken(i) && !db.get(m.card).is_spell() {
                            da += m.attack + sides[turn].hand_bonus[i].0;
                            dh += m.health + sides[turn].hand_bonus[i].1;
                        }
                    }
                    sides[turn].gain_at(db, mi, da * mul, dh * mul);
                }
                None => {}
            }
        }
        for s in 0..2 {
            if shields_lost[s] {
                sides[s].note_shield_lost(db);
            }
        }
        resolve_deaths(sides, turn, db, uid, rng, w);
        if idx[turn ^ 1] < soc[turn ^ 1].len() {
            turn ^= 1;
        }
    }
}

/// Run the attack triggers of one kind for the minion `me`, which attacks:
/// Rally (for `me` only) and "whenever a friendly minion attacks" before the
/// damage, or "after a friendly minion attacks" after it.
#[allow(clippy::too_many_arguments)]
fn attack_triggers(
    sides: &mut [Side<'_>; 2],
    turn: usize,
    me: u16,
    attacker: &Minion,
    target: Option<u16>,
    after: bool,
    db: &CardDb,
    uid: &mut u16,
    rng: &mut Rng,
    w: &mut impl Watch,
) {
    let trigger = |c: &crate::cards::Card| {
        if after { c.after_friendly_attacks } else { c.whenever_friendly_attacks }
    };
    let mut hosts: ArrayVec<u16, BOARD> = ArrayVec::new();
    for f in &sides[turn].ms {
        if !after && f.m.health <= 0 {
            continue;
        }
        let relevant = f.m.parts().any(|(c, _)| {
            let c = db.get(c);
            (!after && f.uid == me && c.rally.is_some())
                || trigger(c).is_some_and(|t| {
                    (!t.filter.exclude_self || f.uid != me) && filter_ok(db, t.filter, attacker)
                })
        });
        if relevant {
            hosts.push(f.uid);
        }
    }
    for host in hosts {
        let Some(hi) = sides[turn].index_of(host) else { continue };
        let m = sides[turn].ms[hi].m;
        for (card, golden) in m.parts() {
            let c = db.get(card);
            if !after
                && host == me
                && let Some(e) = c.rally
            {
                match e {
                    // These two read or change the minion being attacked,
                    // which is on the other side.
                    Effect::GainTargetAttack => {
                        if let Some(t) = target
                            && let Some(di) = sides[turn ^ 1].index_of(t)
                        {
                            let stat = sides[turn ^ 1].ms[di].m.attack;
                            let mul = if golden { 2 } else { 1 };
                            if let Some(hi) = sides[turn].index_of(host) {
                                sides[turn].gain_at(db, hi, stat * mul, 0);
                            }
                        }
                    }
                    Effect::StripTargetKeywords { mask } => {
                        if let Some(t) = target
                            && let Some(di) = sides[turn ^ 1].index_of(t)
                        {
                            sides[turn ^ 1].ms[di].m.keywords.0 &= !mask;
                        }
                    }
                    _ => {
                        let Some(hi) = sides[turn].index_of(host) else { break };
                        let fx = Fx {
                            side: turn,
                            host,
                            killer: None,
                            attacker: None,
                            skip_ptr: true,
                        };
                        let mut at = hi + 1;
                        sides[turn].run_effect(db, e, golden, fx, &mut at, uid, rng, w);
                    }
                }
            }
            if let Some(t) = trigger(c)
                && (!t.filter.exclude_self || host != me)
                && filter_ok(db, t.filter, attacker)
            {
                let Some(hi) = sides[turn].index_of(host) else { break };
                let fx = Fx {
                    side: turn,
                    host,
                    killer: None,
                    attacker: Some(me),
                    skip_ptr: false,
                };
                let mut at = hi + 1;
                sides[turn].run_effect(db, t.effect, golden, fx, &mut at, uid, rng, w);
            }
        }
    }
}

/// The minion `me` of side `turn` attacks: one swing, or two with Windfury,
/// each ending in the deaths it causes.
fn attack_swings(
    sides: &mut [Side<'_>; 2],
    turn: usize,
    me: u16,
    db: &CardDb,
    uid: &mut u16,
    rng: &mut Rng,
    w: &mut impl Watch,
) {
    let windfury = sides[turn]
        .index_of(me)
        .is_some_and(|i| sides[turn].ms[i].m.keywords.has(Keywords::WINDFURY));
    for _ in 0..(if windfury { 2 } else { 1 }) {
        let Some(ai) = sides[turn].index_of(me) else { break };
        if sides[turn].ms[ai].m.attack <= 0 || sides[turn ^ 1].ms.is_empty() {
            break;
        }
        let Some(di) = pick_defender(db, &sides[turn ^ 1], rng) else { break };
        let def_uid = sides[turn ^ 1].ms[di].uid;

        // Attacking ends Stealth.
        sides[turn].ms[ai].m.keywords.clear(Keywords::STEALTH);

        // Rally and "whenever a friendly minion attacks" trigger when the
        // minion attacks, before damage.
        let attacker = sides[turn].ms[ai].m;
        attack_triggers(sides, turn, me, &attacker, Some(def_uid), false, db, uid, rng, w);

        let Some(ai) = sides[turn].index_of(me) else { break };
        let Some(di) = sides[turn ^ 1].index_of(def_uid) else { break };
        let attacker = sides[turn].ms[ai].m;
        let (att, def) = pair(sides, turn);
        let (x_alive, y_alive) = (att.ms[ai].m.health > 0, def.ms[di].m.health > 0);
        let (x_shielded, y_shielded) = (
            att.ms[ai].m.keywords.has(Keywords::DIVINE_SHIELD),
            def.ms[di].m.keywords.has(Keywords::DIVINE_SHIELD),
        );
        w.attack(turn, &att.ms[ai].m, &def.ms[di].m);
        let (dx, dy) = (att.ms[ai].m.attack, def.ms[di].m.attack);
        let (xf, yf) = (&mut att.ms[ai], &mut def.ms[di]);
        let dealt_x = hit(&mut yf.m, dx, &mut xf.m);
        let dealt_y = hit(&mut xf.m, dy, &mut yf.m);
        let x_lost = x_shielded && !xf.m.keywords.has(Keywords::DIVINE_SHIELD);
        let y_lost = y_shielded && !yf.m.keywords.has(Keywords::DIVINE_SHIELD);
        if y_alive && yf.m.health <= 0 && dealt_x > 0 {
            yf.killer = Some(xf.uid);
        }
        if x_alive && xf.m.health <= 0 && dealt_y > 0 {
            xf.killer = Some(yf.uid);
        }
        att.count_damage(db, turn, ai, dealt_x, uid, rng, w);
        let Some(di) = def.index_of(def_uid) else { continue };
        def.count_damage(db, turn ^ 1, di, dealt_y, uid, rng, w);
        // Devout Hellcaller: another friendly Demon dealt damage. The
        // attacker's tribe is a cheap check, so a side without Demons pays
        // nothing extra here.
        if dealt_x > 0 && db.get(attacker.card).tribes.has(Tribe::Demon) {
            sides[turn].note_demon_damage(db, me);
        }
        // "Whenever this takes damage" (Very Hungry Winterfinner). It fires
        // even when the damage is lethal.
        if dealt_x > 0 && sides[turn ^ 1].damage_trigger {
            sides[turn ^ 1].when_damaged(db, turn ^ 1, di, uid, rng, w);
        }
        if dealt_y > 0
            && sides[turn].damage_trigger
            && let Some(ai) = sides[turn].index_of(me)
        {
            sides[turn].when_damaged(db, turn, ai, uid, rng, w);
        }

        // "After a friendly minion attacks" triggers (Ravaging Scorpid,
        // Deathstrider).
        attack_triggers(sides, turn, me, &attacker, Some(def_uid), true, db, uid, rng, w);

        // Hopebringer improves when a friendly minion loses Divine Shield.
        if x_lost {
            sides[turn].note_shield_lost(db);
        }
        if y_lost {
            sides[turn ^ 1].note_shield_lost(db);
        }

        // The attacking side's dead are handled first.
        resolve_deaths(sides, turn, db, uid, rng, w);
    }
}

pub fn fight(db: &CardDb, a: &[Minion], b: &[Minion], max_attacks: u32, rng: &mut Rng) -> Outcome {
    fight_with(db, a, b, [Counters::default(); 2], max_attacks, rng, &mut ())
}

pub fn fight_watched(
    db: &CardDb,
    a: &[Minion],
    b: &[Minion],
    max_attacks: u32,
    rng: &mut Rng,
    w: &mut impl Watch,
) -> Outcome {
    fight_with(db, a, b, [Counters::default(); 2], max_attacks, rng, w)
}

pub fn fight_with(
    db: &CardDb,
    a: &[Minion],
    b: &[Minion],
    counters: [Counters; 2],
    max_attacks: u32,
    rng: &mut Rng,
    w: &mut impl Watch,
) -> Outcome {
    fight_with_hands(db, a, b, [&[], &[]], counters, max_attacks, rng, w)
}

/// A fight where each side also brings a hand. Only hand-reading cards look
/// at it; passing empty hands is the same as `fight_with`.
#[allow(clippy::too_many_arguments)]
pub fn fight_with_hands(
    db: &CardDb,
    a: &[Minion],
    b: &[Minion],
    hands: [&[Minion]; 2],
    counters: [Counters; 2],
    max_attacks: u32,
    rng: &mut Rng,
    w: &mut impl Watch,
) -> Outcome {
    let mut uid = 0u16;
    let mut sides = [
        Side::new(db, a, hands[0], counters[0], &mut uid),
        Side::new(db, b, hands[1], counters[1], &mut uid),
    ];
    let (na, nb) = (sides[0].ms.len(), sides[1].ms.len());
    let mut turn = if na > nb {
        0
    } else if nb > na {
        1
    } else {
        rng.coin() as usize
    };
    run_start_of_combat(&mut sides, db, &mut uid, rng, w);

    let mut attacks = 0;
    let mut skips = 0;
    while !sides[0].ms.is_empty() && !sides[1].ms.is_empty() && attacks < max_attacks {
        // nothing to attack when every enemy has Stealth: the turn passes
        let can_target = sides[turn ^ 1].ms.iter().any(targetable);
        let att = &mut sides[turn];
        let Some(i) = att.next_attacker().filter(|_| can_target) else {
            skips += 1;
            if skips >= 2 {
                break; // neither side can attack
            }
            turn ^= 1;
            continue;
        };
        skips = 0;
        att.ptr = i + 1;
        let me = att.ms[i].uid;
        attack_swings(&mut sides, turn, me, db, &mut uid, rng, w);
        attacks += 1;
        turn ^= 1;
    }
    let (ea, eb) = (sides[0].ms.is_empty(), sides[1].ms.is_empty());
    let winner = match (ea, eb) {
        (false, true) => Some(0),
        (true, false) => Some(1),
        _ => None,
    };
    let survivor_tiers =
        winner.map_or(0, |s| sides[s].ms.iter().map(|f| db.get(f.m.card).tier as i32).sum());
    Outcome { winner, survivor_tiers, attacks, sides: [sides[0].out, sides[1].out] }
}

/// What a Tavern attack leaves behind.
#[derive(Clone, Debug)]
pub struct TavernAttack {
    /// The attacking player's board afterwards, with everything the attack
    /// changed on it (Rally summons and buffs, stats taken from the target).
    pub board: ArrayVec<Minion, BOARD>,
    /// The target if it survived.
    pub target: Option<Minion>,
    /// Cards, Beetle and "future" bonuses and Deathrattles that carry over.
    /// Its `permanent_improvements` are already in `board`.
    pub side: SideOutcome,
}

/// The minion in slot `attacker` of `board` attacks `target`, a minion in the
/// Tavern (Fishbait), once. Rally, "whenever" and "after" triggers, the
/// target's Deathrattle and everything they cause run as in a fight.
pub fn tavern_attack(
    db: &CardDb,
    board: &[Minion],
    attacker: usize,
    target: Minion,
    counters: Counters,
    rng: &mut Rng,
) -> TavernAttack {
    let mut uid = 0u16;
    let counters = Counters { boon_of_beetles: 0, brood_of_nozdormu: 0, upper_hand: 0, ..counters };
    let mut sides = [
        Side::new(db, board, &[], counters, &mut uid),
        Side::new(db, &[target], &[], Counters::default(), &mut uid),
    ];
    let me = sides[0].ms[attacker].uid;
    attack_swings(&mut sides, 0, me, db, &mut uid, rng, &mut ());
    TavernAttack {
        board: sides[0].ms.iter().map(|f| f.m).collect(),
        target: sides[1].ms.first().map(|f| f.m),
        side: sides[0].out,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v01_db() -> CardDb {
        CardDb::from_toml(include_str!("../tests/data/v01-cards.toml")).unwrap()
    }

    fn side(db: &CardDb, names: &[&str]) -> (Side<'static>, u16) {
        let mut uid = 0;
        let board: Vec<Minion> =
            names.iter().map(|n| db.make(db.by_name(n).unwrap(), false)).collect();
        (Side::new(db, &board, &[], Counters::default(), &mut uid), uid)
    }

    fn names(db: &CardDb, s: &Side) -> Vec<String> {
        s.ms.iter().map(|f| db.get(f.m.card).name.clone()).collect()
    }

    #[test]
    fn deathrattle_summons_where_the_minion_stood() {
        let db = v01_db();
        let (mut s, mut uid) = side(&db, &["Wolf Pup", "Den Mother", "Ram"]);
        let mut rng = Rng::new(1);
        s.ms[1].m.health = 0;
        s.resolve(&db, 0, &mut uid, &mut rng, &mut ());
        assert_eq!(names(&db, &s), ["Wolf Pup", "Pup", "Ram"]);
    }

    #[test]
    fn summons_past_seven_are_lost() {
        let db = v01_db();
        let (mut s, mut uid) = side(&db, &["Ram", "Ram", "Alpha", "Ram", "Ram", "Ram", "Ram"]);
        let mut rng = Rng::new(1);
        s.ms[2].m.health = -3;
        s.resolve(&db, 0, &mut uid, &mut rng, &mut ());
        assert_eq!(names(&db, &s), ["Ram", "Ram", "Wolf", "Ram", "Ram", "Ram", "Ram"]);
    }

    #[test]
    fn golden_deathrattle_summons_golden_tokens() {
        let db = v01_db();
        let mut uid = 0;
        let mut rng = Rng::new(1);
        let alpha = db.make(db.by_name("Alpha").unwrap(), true);
        let mut s = Side::new(&db, &[alpha], &[], Counters::default(), &mut uid);
        s.ms[0].m.health = 0;
        s.resolve(&db, 0, &mut uid, &mut rng, &mut ());
        assert_eq!(s.ms.len(), 2);
        assert!(s.ms.iter().all(|f| f.m.golden && f.m.attack == 4 && f.m.health == 4));
    }

    #[test]
    fn removing_before_the_pointer_keeps_the_next_attacker() {
        let db = v01_db();
        let (mut s, mut uid) = side(&db, &["Wolf Pup", "Ram", "Viper"]);
        let mut rng = Rng::new(1);
        s.ptr = 2; // Viper is next
        s.ms[0].m.health = 0;
        s.resolve(&db, 0, &mut uid, &mut rng, &mut ());
        assert_eq!(db.get(s.ms[s.next_attacker().unwrap()].m.card).name, "Viper");
    }

    fn s2_db() -> CardDb {
        crate::Data::bundled().cards
    }

    fn side2(db: &CardDb, names: &[&str]) -> (Side<'static>, u16) {
        side(db, names)
    }

    #[test]
    fn tasty_lobster_buff_grows_with_each_trigger() {
        let db = s2_db();
        let (mut s, mut uid) = side2(&db, &["Tasty Lobster", "Cage Gnawer", "Cord Puller"]);
        let mut rng = Rng::new(1);
        s.ms[0].m.health = 0;
        s.resolve(&db, 0, &mut uid, &mut rng, &mut ());
        // Cage Gnawer is the only other Beast: 2/7 becomes 4/8, the Mech is left alone
        assert_eq!((s.ms[0].m.attack, s.ms[0].m.health), (4, 8));
        assert_eq!((s.ms[1].m.attack, s.ms[1].m.health), (1, 1));
        assert_eq!(s.counters.improve, [1]);
        assert_eq!(s.out.improve_gain, [1]);

        // a second Lobster, made plain, now gives +4/+2
        let lobster = db.make(db.by_name("Tasty Lobster").unwrap(), false);
        s.summon(&db, 0, 0, lobster, &mut uid, &mut ());
        assert_eq!((s.ms[0].m.attack, s.ms[0].m.health), (2, 1));
        s.ms[0].m.health = 0;
        s.resolve(&db, 0, &mut uid, &mut rng, &mut ());
        assert_eq!((s.ms[0].m.attack, s.ms[0].m.health), (8, 10));
        assert_eq!(s.out.improve_gain, [2]);

        // golden at count 0: twice the buff, and the count still goes up by one
        let gold = db.make(db.by_name("Tasty Lobster").unwrap(), true);
        let (mut s, mut uid) = side2(&db, &["Cage Gnawer"]);
        s.ms.insert(0, Fm { m: gold, uid: 99, start_slot: None, avenge: 0, killer: None });
        s.ms[0].m.health = 0;
        s.resolve(&db, 0, &mut uid, &mut rng, &mut ());
        assert_eq!((s.ms[0].m.attack, s.ms[0].m.health), (2 + 4, 7 + 2));
        assert_eq!(s.out.improve_gain, [1]);
    }

    #[test]
    fn lobster_count_is_live_with_titus_and_with_deathstrider() {
        let db = s2_db();
        let mut rng = Rng::new(1);
        // Titus makes the Lobster's Deathrattle trigger twice: +2/+1 then +4/+2
        let (mut s, mut uid) = side2(&db, &["Titus Rivendare", "Tasty Lobster", "Cage Gnawer"]);
        s.ms[1].m.health = 0;
        s.resolve(&db, 0, &mut uid, &mut rng, &mut ());
        assert_eq!((s.ms[1].m.attack, s.ms[1].m.health), (2 + 2 + 4, 7 + 1 + 2));
        assert_eq!(s.out.improve_gain, [2]);

        // Deathstrider's trigger, then the same Lobster dying: +2/+1 then +4/+2. The
        // living Lobster is a Beast too, so the first buff may land on it.
        for seed in 0..8 {
            let mut rng = Rng::new(seed);
            let (mut s, mut uid) = side2(&db, &["Tasty Lobster", "Cage Gnawer"]);
            let fx = Fx { side: 0, host: s.ms[1].uid, killer: None, attacker: None, skip_ptr: false };
            let mut at = 0;
            let e = Effect::TriggerDeathrattle { times: 1 };
            s.run_effect(&db, e, false, fx, &mut at, &mut uid, &mut rng, &mut ());
            let first_on_gnawer = s.ms[1].m.attack == 4;
            assert_eq!(s.ms[0].m.attack + s.ms[1].m.attack, 2 + 2 + 2);
            assert_eq!(s.counters.improve, [1]);
            s.ms[0].m.health = 0;
            s.resolve(&db, 0, &mut uid, &mut rng, &mut ());
            let (a, h) = if first_on_gnawer { (2 + 2 + 4, 7 + 1 + 2) } else { (2 + 4, 7 + 2) };
            assert_eq!((s.ms[0].m.attack, s.ms[0].m.health), (a, h));
            assert_eq!(s.out.improve_gain, [2]);
        }

        // a count carried in from an earlier fight: base +4/+2, golden +8/+4
        let mut uid = 0;
        let counters = Counters { improve: [1], ..Counters::default() };
        let gold = db.make(db.by_name("Tasty Lobster").unwrap(), true);
        let gnawer = db.make(db.by_name("Cage Gnawer").unwrap(), false);
        let mut s = Side::new(&db, &[gold, gnawer], &[], counters, &mut uid);
        s.ms[0].m.health = 0;
        s.resolve(&db, 0, &mut uid, &mut rng, &mut ());
        assert_eq!((s.ms[0].m.attack, s.ms[0].m.health), (2 + 8, 7 + 4));
        assert_eq!(s.counters.improve, [2]);
    }

    #[test]
    fn relentless_deflector_avenges_every_third_friendly_death() {
        let db = s2_db();
        let (mut s, mut uid) = side2(&db, &["Relentless Deflector", "Microbot", "Microbot", "Microbot"]);
        let mut rng = Rng::new(1);
        let deflector = |s: &Side| s.ms[0].m;
        // no Divine Shield, so no Taunt either
        assert!(!has_taunt(&db, &deflector(&s)));
        for k in 1..=3 {
            s.ms[1].m.health = 0;
            s.resolve(&db, 0, &mut uid, &mut rng, &mut ());
            assert_eq!(deflector(&s).keywords.has(Keywords::DIVINE_SHIELD), k == 3, "after death {k}");
        }
        // while it has Divine Shield it has Taunt
        assert!(has_taunt(&db, &deflector(&s)));
        // the count goes on: three more deaths give the shield back
        s.ms[0].m.keywords.clear(Keywords::DIVINE_SHIELD);
        assert!(!has_taunt(&db, &deflector(&s)));
        for k in 1..=3 {
            let m = db.make(db.by_name("Microbot").unwrap(), false);
            s.summon(&db, 0, 1, m, &mut uid, &mut ());
            s.ms[1].m.health = 0;
            s.resolve(&db, 0, &mut uid, &mut rng, &mut ());
            assert_eq!(deflector(&s).keywords.has(Keywords::DIVINE_SHIELD), k == 3, "after death {}", k + 3);
        }
        // its own death, or an enemy's, is not counted: only friendly deaths
        let (mut s, mut uid) = side2(&db, &["Relentless Deflector"]);
        s.ms[0].m.health = 0;
        s.resolve(&db, 0, &mut uid, &mut rng, &mut ());
        assert!(s.ms.is_empty());
    }

    #[test]
    fn ghastcoiler_summons_random_deathrattle_minions() {
        let db = s2_db();
        for seed in 0..40 {
            let mut rng = Rng::new(seed);
            let (mut s, mut uid) = side2(&db, &["Ghastcoiler"]);
            s.ms[0].m.health = 0;
            s.resolve(&db, 0, &mut uid, &mut rng, &mut ());
            assert_eq!(s.ms.len(), 2);
            assert!(s.ms.iter().all(|f| db.get(f.m.card).deathrattle.is_some() && !f.m.golden));

            let gold = db.make(db.by_name("Ghastcoiler").unwrap(), true);
            let mut uid = 0;
            let mut s = Side::new(&db, &[gold], &[], Counters::default(), &mut uid);
            s.ms[0].m.health = 0;
            s.resolve(&db, 0, &mut uid, &mut rng, &mut ());
            assert_eq!(s.ms.len(), 4);
        }
        // only Deathrattle minions up to the owner's Tavern tier
        for tier in 1..=3u8 {
            for seed in 0..40 {
                let mut uid = 0;
                let gold = db.make(db.by_name("Ghastcoiler").unwrap(), true);
                let counters = Counters { tier, ..Counters::default() };
                let mut s = Side::new(&db, &[gold], &[], counters, &mut uid);
                s.ms[0].m.health = 0;
                s.resolve(&db, 0, &mut uid, &mut Rng::new(seed), &mut ());
                assert_eq!(s.ms.len(), 4);
                assert!(s.ms.iter().all(|f| db.get(f.m.card).tier <= tier), "tier {tier}");
            }
        }
        // a full board loses the extra summons
        let (mut s, mut uid) = side2(&db, &["Ghastcoiler", "Cord Puller", "Cord Puller", "Cord Puller", "Cord Puller", "Cord Puller", "Cord Puller"]);
        s.ms[0].m.health = 0;
        s.resolve(&db, 0, &mut uid, &mut Rng::new(3), &mut ());
        assert_eq!(s.ms.len(), 7);
    }

    #[test]
    fn falling_sky_golem_grows_with_every_deathrattle() {
        let db = s2_db();
        let (mut s, mut uid) = side2(&db, &["Falling Sky Golem", "Cord Puller", "Buzzing Vermin"]);
        let mut rng = Rng::new(1);
        s.ms[1].m.health = 0;
        s.resolve(&db, 0, &mut uid, &mut rng, &mut ());
        // Cord Puller's Deathrattle: the Golem is 4/2 + 4/2
        assert_eq!((s.ms[0].m.attack, s.ms[0].m.health), (8, 4));
        assert_eq!(s.out.deathrattles_triggered, 1);
        // a minion without a Deathrattle changes nothing
        let (mut s, mut uid) = side2(&db, &["Falling Sky Golem", "Annoy-o-Module"]);
        s.ms[1].m.health = 0;
        s.resolve(&db, 0, &mut uid, &mut rng, &mut ());
        assert_eq!((s.ms[0].m.attack, s.ms[0].m.health), (4, 2));
        assert_eq!(s.out.deathrattles_triggered, 0);
        // Titus makes each Deathrattle trigger twice, and each one counts
        let (mut s, mut uid) = side2(&db, &["Falling Sky Golem", "Titus Rivendare", "Cord Puller"]);
        s.ms[2].m.health = 0;
        s.resolve(&db, 0, &mut uid, &mut rng, &mut ());
        assert_eq!((s.ms[0].m.attack, s.ms[0].m.health), (12, 6));
        assert_eq!(s.out.deathrattles_triggered, 2);
    }

    #[test]
    fn fishbait_deathrattle_gives_its_killer_plus_five() {
        let db = s2_db();
        let fishbait = db.make(db.by_name("Fishbait").unwrap(), false);
        let beast = db.make(db.by_name("Buzzing Vermin").unwrap(), false);
        let counters = Counters::default();
        // the Beast kills it and gets +5/+5
        let out = tavern_attack(&db, &[beast], 0, fishbait, counters, &mut Rng::new(1));
        assert!(out.target.is_none());
        assert_eq!((out.board[0].attack, out.board[0].health), (1 + 5, 1 + 5));
        // golden Fishbait has 2 Health: one point of damage leaves it alive and gives nothing
        let gold = db.make(db.by_name("Fishbait").unwrap(), true);
        assert_eq!((gold.attack, gold.health), (0, 2));
        let out = tavern_attack(&db, &[beast], 0, gold, counters, &mut Rng::new(1));
        assert_eq!(out.target.map(|t| t.health), Some(1));
        assert_eq!((out.board[0].attack, out.board[0].health), (1, 1));
        // killing the golden one gives +10/+10
        let strong = db.make(db.by_name("Turquoise Skitterer").unwrap(), false);
        let out = tavern_attack(&db, &[strong], 0, gold, counters, &mut Rng::new(1));
        assert!(out.target.is_none());
        assert_eq!((out.board[0].attack, out.board[0].health), (5 + 10, 5 + 10));
        // a Beast with 0 Attack can't attack
        let mut weak = beast;
        weak.attack = 0;
        let out = tavern_attack(&db, &[weak], 0, fishbait, counters, &mut Rng::new(1));
        assert!(out.target.is_some());
    }
}
