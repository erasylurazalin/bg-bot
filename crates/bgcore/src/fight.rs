//! The fight: a pure function of two boards, their counters, and an RNG.
//! The order of events follows DESIGN.md, Part 1, and matches `~/projects/bg`'s
//! engine where the design leaves room (attack pointer, Windfury swings, death
//! order), so the two can be cross-checked.

use arrayvec::ArrayVec;

use crate::cards::{
    AfterFriendlyAttack, Aura, CardDb, Deathrattle, OnSummonBeast, Rally, StartOfCombat, Tribe,
};
use crate::minion::{CardId, Keywords, Minion};
use crate::rng::Rng;
use crate::{BOARD, HAND};

/// Per-player counters passed into a fight.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Counters {
    pub beetle_attack: i32,
    pub beetle_health: i32,
}

/// Side effects from a fight that carry over to one player after combat.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SideOutcome {
    /// Permanent improvement stacks gained by the minion that started in each
    /// board slot (Lurking Leviathan).
    pub permanent_improvements: [u16; BOARD],
    /// Increase to the player's Beetle bonus during this fight (Turquoise
    /// Skitterer, Ravaging Scorpid).
    pub beetle_attack_gain: i32,
    pub beetle_health_gain: i32,
    /// Generated cards added to the player's hand during this fight
    /// (Headhunter Gryphon, Conveyor Construct).
    pub cards: [CardId; HAND],
    pub n_cards: u8,
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
}

struct Side {
    ms: ArrayVec<Fm, BOARD>,
    /// Index of the next minion to attack.
    ptr: usize,
    counters: Counters,
    /// Combat-only buff to all Beasts on this side (Humming Bird, Goldrinn).
    beast_buff: (i32, i32),
    out: SideOutcome,
}

impl Side {
    fn new(board: &[Minion], counters: Counters, uid: &mut u16) -> Side {
        let mut ms = ArrayVec::new();
        for (slot, &m) in board.iter().take(BOARD).enumerate() {
            ms.push(Fm { m, uid: *uid, start_slot: Some(slot as u8) });
            *uid += 1;
        }
        Side { ms, ptr: 0, counters, beast_buff: (0, 0), out: SideOutcome::default() }
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
                f.m.attack += da;
                f.m.health += dh;
            }
        }
    }

    fn add_combat_beast_buff(&mut self, db: &CardDb, da: i32, dh: i32) {
        self.beast_buff.0 += da;
        self.beast_buff.1 += dh;
        for f in &mut self.ms {
            if f.m.health > 0 && db.get(f.m.card).tribe == Tribe::Beast {
                f.m.attack += da;
                f.m.health += dh;
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
        let c = db.get(m.card);
        if c.is_beetle {
            m.attack += self.counters.beetle_attack;
            m.health += self.counters.beetle_health;
        }
        self.insert(pos, Fm { m, uid: *uid, start_slot: None });
        *uid += 1;
        if c.tribe == Tribe::Beast {
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
                self.ms[pos].m.attack += da_total;
                self.ms[pos].m.health += dh_total;
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

    /// Remove the dead in board order. Each one's Deathrattle runs, then its
    /// Reborn, before the next dead minion is looked at.
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
            let dead = self.remove(i).m;
            w.death(side, i, &dead);
            let runs = 1 + self.titus_extra(db);
            for _ in 0..runs {
                for (card, golden) in dead.parts() {
                    match db.get(card).deathrattle {
                        Some(Deathrattle::Summon {
                            card: tok,
                            count,
                            golden_doubles_count,
                            beetle_buff,
                        }) => {
                            let mul = if golden { 2 } else { 1 };
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
                                if self.summon(db, side, i, m, uid, w) {
                                    i += 1;
                                }
                            }
                        }
                        Some(Deathrattle::SummonByAttack { card: tok }) => {
                            let n = dead.attack.max(0) as usize;
                            for _ in 0..n {
                                let m = db.make(tok, golden);
                                if self.summon(db, side, i, m, uid, w) {
                                    i += 1;
                                }
                            }
                        }
                        Some(Deathrattle::CombatBeastBuff { attack, health }) => {
                            let mul = if golden { 2 } else { 1 };
                            self.add_combat_beast_buff(db, attack * mul, health * mul);
                        }
                        Some(Deathrattle::RandomVolumizer) => {
                            let n = if golden { 2 } else { 1 };
                            for _ in 0..n {
                                if let Some(v) = db.random_volumizer(rng) {
                                    self.out.push_card(v);
                                }
                            }
                        }
                        None => {}
                    }
                }
            }
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
/// Venomous unused; 0 damage does nothing at all.
fn hit(target: &mut Minion, dmg: i32, source: &mut Minion) {
    if dmg <= 0 {
        return;
    }
    if target.keywords.has(Keywords::DIVINE_SHIELD) {
        target.keywords.clear(Keywords::DIVINE_SHIELD);
        return;
    }
    target.health -= dmg;
    if source.keywords.has(Keywords::VENOMOUS) {
        target.health = target.health.min(0);
        source.keywords.clear(Keywords::VENOMOUS);
    }
}

fn pick_defender(def: &Side, rng: &mut Rng) -> usize {
    let taunts = def.ms.iter().filter(|f| f.m.keywords.has(Keywords::TAUNT)).count();
    if taunts == 0 {
        return rng.below(def.ms.len() as u32) as usize;
    }
    let k = rng.below(taunts as u32) as usize;
    def.ms
        .iter()
        .enumerate()
        .filter(|(_, f)| f.m.keywords.has(Keywords::TAUNT))
        .nth(k)
        .unwrap()
        .0
}

fn pair(sides: &mut [Side; 2], turn: usize) -> (&mut Side, &mut Side) {
    let [a, b] = sides;
    if turn == 0 { (a, b) } else { (b, a) }
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
        if !(c1 || c2) {
            break;
        }
    }
}

fn run_start_of_combat(
    sides: &mut [Side; 2],
    db: &CardDb,
    uid: &mut u16,
    rng: &mut Rng,
    w: &mut impl Watch,
) {
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
        let att = &mut sides[turn];
        let Some(i) = att.next_attacker() else {
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
        let swings = if att.ms[i].m.keywords.has(Keywords::WINDFURY) { 2 } else { 1 };
        for _ in 0..swings {
            let Some(ai) = sides[turn].ms.iter().position(|f| f.uid == me) else { break };
            if sides[turn].ms[ai].m.attack <= 0 || sides[turn ^ 1].ms.is_empty() {
                break;
            }
            let di = pick_defender(&sides[turn ^ 1], rng);
            let def_uid = sides[turn ^ 1].ms[di].uid;

            // Rally triggers when the minion attacks, before damage.
            let att_m = sides[turn].ms[ai].m;
            let mut spawn_pos = ai + 1;
            for (card, golden) in att_m.parts() {
                match db.get(card).rally {
                    Some(Rally::Summon { card: tok, count, golden_doubles_count }) => {
                        let mul = if golden { 2 } else { 1 };
                        let (n, tok_golden) = if golden_doubles_count {
                            (count * mul as u8, false)
                        } else {
                            (count, golden)
                        };
                        for _ in 0..n {
                            let m = db.make(tok, tok_golden);
                            if sides[turn].summon(db, turn, spawn_pos, m, &mut uid, w) {
                                if spawn_pos == sides[turn].ptr {
                                    sides[turn].ptr += 1;
                                }
                                spawn_pos += 1;
                            }
                        }
                    }
                    Some(Rally::RandomMinion(tribe)) => {
                        let n = if golden { 2 } else { 1 };
                        for _ in 0..n {
                            if let Some(c) = db.random_pool_minion(tribe, rng) {
                                sides[turn].out.push_card(c);
                            }
                        }
                    }
                    None => {}
                }
            }

            let Some(ai) = sides[turn].ms.iter().position(|f| f.uid == me) else { break };
            let Some(di) = sides[turn ^ 1].ms.iter().position(|f| f.uid == def_uid) else { break };
            let (att, def) = pair(&mut sides, turn);
            let (x, y) = (&mut att.ms[ai].m, &mut def.ms[di].m);
            w.attack(turn, x, y);
            let (dx, dy) = (x.attack, y.attack);
            hit(y, dx, x);
            hit(x, dy, y);

            // "After a friendly minion attacks" triggers (Ravaging Scorpid).
            let n_att = sides[turn].ms.len();
            for j in 0..n_att {
                let m = sides[turn].ms[j].m;
                for (card, golden) in m.parts() {
                    if let Some(AfterFriendlyAttack::BeetleBuff { attack, health }) =
                        db.get(card).after_friendly_attack
                    {
                        let mul = if golden { 2 } else { 1 };
                        sides[turn].add_beetle_buff(db, attack * mul, health * mul);
                    }
                }
            }

            // The attacking side's dead are handled first.
            resolve_deaths(&mut sides, turn, db, &mut uid, rng, w);
        }
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
}
