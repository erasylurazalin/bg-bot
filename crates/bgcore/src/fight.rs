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

use crate::cards::{N_COUNTERS, AttackFilter, Aura, CardDb, Deathrattle, Effect, OnSummonBeast, StartOfCombat, Tribe};
use crate::minion::{CardId, Keywords, Minion};
use crate::rng::Rng;
use crate::{BOARD, HAND};

/// Per-player counters passed into a fight.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Counters {
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
    /// Increase to the player's Beast bonus until next turn (Goldrinn). Only
    /// a Tavern attack keeps it, a fight's is gone with its board.
    pub beast_bonus_gain: (i32, i32),
    /// Deathrattles the player triggered in this fight (Falling Sky Golem).
    pub deathrattles_triggered: u16,
    /// Generated cards added to the player's hand during this fight
    /// (Headhunter Gryphon, Conveyor Construct, Rescue Bot).
    pub cards: [CardId; HAND],
    pub n_cards: u8,
    /// Number of Boon of Beetles Beetles summoned during this fight.
    pub boon_of_beetles_used: u8,
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
    /// Summons made right of the attacker don't attack in the same round.
    skip_ptr: bool,
}

struct Side {
    ms: ArrayVec<Fm, BOARD>,
    /// Index of the next minion to attack.
    ptr: usize,
    counters: Counters,
    /// Combat-only buff to all Beasts on this side (Humming Bird, Goldrinn).
    beast_buff: (i32, i32),
    /// Buffs waiting for a minion that killed a Fishbait, by its uid. They
    /// are handed out by `resolve_deaths`, which sees both sides.
    killer_buffs: ArrayVec<(u16, i32, i32), 8>,
    out: SideOutcome,
}

/// Give a minion stats, unless its card says it can't gain any.
fn gain(db: &CardDb, f: &mut Fm, da: i32, dh: i32) {
    if !db.get(f.m.card).fixed_stats {
        f.m.attack += da;
        f.m.health += dh;
    }
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

impl Side {
    fn new(board: &[Minion], counters: Counters, uid: &mut u16) -> Side {
        let mut ms = ArrayVec::new();
        for (slot, &m) in board.iter().take(BOARD).enumerate() {
            ms.push(Fm { m, uid: *uid, start_slot: Some(slot as u8), avenge: 0, killer: None });
            *uid += 1;
        }
        Side {
            ms,
            ptr: 0,
            counters,
            beast_buff: (counters.beast_attack, counters.beast_health),
            killer_buffs: ArrayVec::new(),
            out: SideOutcome::default(),
        }
    }

    fn index_of(&self, uid: u16) -> Option<usize> {
        self.ms.iter().position(|f| f.uid == uid)
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
        for f in &mut self.ms {
            if f.m.health > 0 && db.get(f.m.card).is_beetle {
                gain(db, f, da, dh);
            }
        }
    }

    fn add_combat_beast_buff(&mut self, db: &CardDb, da: i32, dh: i32) {
        self.beast_buff.0 += da;
        self.beast_buff.1 += dh;
        self.out.beast_bonus_gain.0 += da;
        self.out.beast_bonus_gain.1 += dh;
        for f in &mut self.ms {
            if f.m.health > 0 && db.get(f.m.card).tribes.has(Tribe::Beast) {
                gain(db, f, da, dh);
            }
        }
    }

    /// A Deathrattle of this side triggered.
    fn note_deathrattle(&mut self, db: &CardDb) {
        self.out.deathrattles_triggered = self.out.deathrattles_triggered.saturating_add(1);
        for f in &mut self.ms {
            if f.m.health <= 0 {
                continue;
            }
            let m = f.m;
            for (card, golden) in m.parts() {
                if let Some((a, h)) = db.get(card).buff_per_deathrattle {
                    let mul = if golden { 2 } else { 1 };
                    gain(db, f, a * mul, h * mul);
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
                gain(db, &mut self.ms[pos], da_total, dh_total);
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

    fn titus_extra(&self, db: &CardDb) -> u32 {
        let mut extra = 0;
        for f in &self.ms {
            if f.m.health > 0 {
                for (card, golden) in f.m.parts() {
                    if db.get(card).aura == Some(Aura::Titus) {
                        extra += if golden { 2 } else { 1 };
                    }
                }
            }
        }
        extra
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
                for f in &mut self.ms {
                    if f.uid != fx.host && f.m.health > 0 && db.get(f.m.card).is(tribe) {
                        gain(db, f, attack * mul, health * mul);
                    }
                }
            }
            Effect::BuffAll { tribe, attack, health } => {
                for f in &mut self.ms {
                    if f.m.health > 0 && db.get(f.m.card).is(tribe) {
                        gain(db, f, attack * mul, health * mul);
                    }
                }
            }
            Effect::BuffSelf { attack, health } => {
                if let Some(i) = self.index_of(fx.host) {
                    gain(db, &mut self.ms[i], attack * mul, health * mul);
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
                    gain(db, &mut self.ms[i], attack * mul * scale, health * mul * scale);
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
                    self.ms[i].m.keywords.set(keyword);
                }
            }
            Effect::GetRandomMinion { tribe } => {
                for _ in 0..mul {
                    if let Some(c) = db.random_pool_minion(tribe, rng) {
                        self.out.push_card(c);
                    }
                }
            }
            Effect::SummonRandomDeathrattle { count } => {
                for _ in 0..count as i32 * mul {
                    let Some(c) = db.random_deathrattle_minion(self.counters.tier, rng) else { break };
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
                    let runs = 1 + self.titus_extra(db);
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
                    Deathrattle::Effect(e) => {
                        let fx = Fx { side, host, killer, skip_ptr: false };
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
            let fx = Fx { side, host, killer: None, skip_ptr: false };
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
                let fx = Fx { side, host, killer: None, skip_ptr: false };
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
            let runs = 1 + self.titus_extra(db);
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

/// Deal `dmg` non-combat damage (e.g. Boom-in-a-Box) to `target`.
fn deal_damage(target: &mut Minion, dmg: i32) {
    if dmg <= 0 {
        return;
    }
    if target.keywords.has(Keywords::DIVINE_SHIELD) {
        target.keywords.clear(Keywords::DIVINE_SHIELD);
        return;
    }
    target.health -= dmg;
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

fn pair(sides: &mut [Side; 2], turn: usize) -> (&mut Side, &mut Side) {
    let [a, b] = sides;
    if turn == 0 { (a, b) } else { (b, a) }
}

/// Hand out the buffs Fishbait Deathrattles gave to their killers.
fn give_killer_buffs(sides: &mut [Side; 2], db: &CardDb) {
    for s in 0..2 {
        while let Some((killer, da, dh)) = sides[s].killer_buffs.pop() {
            for t in 0..2 {
                if let Some(f) = sides[t].ms.iter_mut().find(|f| f.uid == killer && f.m.health > 0) {
                    gain(db, f, da, dh);
                }
            }
        }
    }
}

fn resolve_deaths(
    sides: &mut [Side; 2],
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
        if !(c1 || c2) {
            break;
        }
    }
    let (att, def) = pair(sides, first_side);
    att.spawn_boon_of_beetles(db, first_side, uid, w);
    def.spawn_boon_of_beetles(db, first_side ^ 1, uid, w);
}

fn run_start_of_combat(
    sides: &mut [Side; 2],
    db: &CardDb,
    uid: &mut u16,
    rng: &mut Rng,
    w: &mut impl Watch,
) {
    for s in 0..2 {
        sides[s].spawn_boon_of_beetles(db, s, uid, w);
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
        for (card, golden) in m.parts() {
            match db.get(card).start_of_combat {
                Some(StartOfCombat::CombatBeastBuff { attack, health }) => {
                    let mul = if golden { 2 } else { 1 };
                    sides[turn].add_combat_beast_buff(db, attack * mul, health * mul);
                }
                Some(StartOfCombat::DamageAllOthers(dmg)) => {
                    let waves = if golden { 2 } else { 1 };
                    for _ in 0..waves {
                        for s in [turn, turn ^ 1] {
                            for f in &mut sides[s].ms {
                                if f.uid != me {
                                    deal_damage(&mut f.m, dmg);
                                }
                            }
                        }
                    }
                }
                None => {}
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
    sides: &mut [Side; 2],
    turn: usize,
    me: u16,
    attacker: &Minion,
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
                || trigger(c).is_some_and(|t| filter_ok(db, t.filter, attacker))
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
                let Some(hi) = sides[turn].index_of(host) else { break };
                let fx = Fx { side: turn, host, killer: None, skip_ptr: true };
                let mut at = hi + 1;
                sides[turn].run_effect(db, e, golden, fx, &mut at, uid, rng, w);
            }
            if let Some(t) = trigger(c)
                && filter_ok(db, t.filter, attacker)
            {
                let Some(hi) = sides[turn].index_of(host) else { break };
                let fx = Fx { side: turn, host, killer: None, skip_ptr: false };
                let mut at = hi + 1;
                sides[turn].run_effect(db, t.effect, golden, fx, &mut at, uid, rng, w);
            }
        }
    }
}

/// The minion `me` of side `turn` attacks: one swing, or two with Windfury,
/// each ending in the deaths it causes.
fn attack_swings(
    sides: &mut [Side; 2],
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
        attack_triggers(sides, turn, me, &attacker, false, db, uid, rng, w);

        let Some(ai) = sides[turn].index_of(me) else { break };
        let Some(di) = sides[turn ^ 1].index_of(def_uid) else { break };
        let attacker = sides[turn].ms[ai].m;
        let (att, def) = pair(sides, turn);
        let (x_alive, y_alive) = (att.ms[ai].m.health > 0, def.ms[di].m.health > 0);
        w.attack(turn, &att.ms[ai].m, &def.ms[di].m);
        let (dx, dy) = (att.ms[ai].m.attack, def.ms[di].m.attack);
        let (xf, yf) = (&mut att.ms[ai], &mut def.ms[di]);
        let dealt_x = hit(&mut yf.m, dx, &mut xf.m);
        let dealt_y = hit(&mut xf.m, dy, &mut yf.m);
        if y_alive && yf.m.health <= 0 && dealt_x > 0 {
            yf.killer = Some(xf.uid);
        }
        if x_alive && xf.m.health <= 0 && dealt_y > 0 {
            xf.killer = Some(yf.uid);
        }
        att.count_damage(db, turn, ai, dealt_x, uid, rng, w);
        let Some(di) = def.index_of(def_uid) else { continue };
        def.count_damage(db, turn ^ 1, di, dealt_y, uid, rng, w);

        // "After a friendly minion attacks" triggers (Ravaging Scorpid,
        // Deathstrider).
        attack_triggers(sides, turn, me, &attacker, true, db, uid, rng, w);

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
    let mut uid = 0u16;
    let mut sides = [Side::new(a, counters[0], &mut uid), Side::new(b, counters[1], &mut uid)];
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
    let mut sides =
        [Side::new(board, counters, &mut uid), Side::new(&[target], Counters::default(), &mut uid)];
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

    fn side(db: &CardDb, names: &[&str]) -> (Side, u16) {
        let mut uid = 0;
        let board: Vec<Minion> =
            names.iter().map(|n| db.make(db.by_name(n).unwrap(), false)).collect();
        (Side::new(&board, Counters::default(), &mut uid), uid)
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
        let mut s = Side::new(&[alpha], Counters::default(), &mut uid);
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

    fn side2(db: &CardDb, names: &[&str]) -> (Side, u16) {
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
            let fx = Fx { side: 0, host: s.ms[1].uid, killer: None, skip_ptr: false };
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
        let mut s = Side::new(&[gold, gnawer], counters, &mut uid);
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
            let mut s = Side::new(&[gold], Counters::default(), &mut uid);
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
                let mut s = Side::new(&[gold], counters, &mut uid);
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
