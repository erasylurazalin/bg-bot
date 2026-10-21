//! The fight: a pure function of two boards and an RNG. The order of events
//! follows DESIGN.md, Part 1, and matches `~/projects/bg`'s engine where the
//! design leaves room (attack pointer, Windfury swings, death order), so the
//! two can be cross-checked.

use arrayvec::ArrayVec;

use crate::BOARD;
use crate::cards::{CardDb, Effect};
use crate::minion::{Keywords, Minion};
use crate::rng::Rng;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// 0 for the first board, 1 for the second, None for a tie.
    pub winner: Option<usize>,
    /// Sum of the tiers of the winner's surviving minions, 0 on a tie.
    pub survivor_tiers: i32,
    pub attacks: u32,
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
}

struct Side {
    ms: ArrayVec<Fm, BOARD>,
    /// Index of the next minion to attack.
    ptr: usize,
}

impl Side {
    fn new(board: &[Minion], uid: &mut u16) -> Side {
        let mut ms = ArrayVec::new();
        for &m in board.iter().take(BOARD) {
            ms.push(Fm { m, uid: *uid });
            *uid += 1;
        }
        Side { ms, ptr: 0 }
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

    /// Remove the dead in board order. Each one's Deathrattle runs, then its
    /// Reborn, before the next dead minion is looked at.
    fn resolve(&mut self, db: &CardDb, side: usize, uid: &mut u16, w: &mut impl Watch) -> bool {
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
            let card = db.get(dead.card);
            if let Some(Effect::Summon { card: tok, count }) = card.deathrattle {
                for _ in 0..count {
                    let m = db.make(tok, dead.golden);
                    if self.insert(i, Fm { m, uid: *uid }) {
                        *uid += 1;
                        w.summon(side, i, &m);
                        i += 1;
                    }
                }
            }
            if dead.keywords.has(Keywords::REBORN) {
                let mut m = db.make(dead.card, dead.golden);
                m.health = 1;
                m.keywords.clear(Keywords::REBORN);
                if self.insert(i, Fm { m, uid: *uid }) {
                    *uid += 1;
                    w.summon(side, i, &m);
                    i += 1;
                }
            }
        }
        changed
    }
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

pub fn fight(db: &CardDb, a: &[Minion], b: &[Minion], max_attacks: u32, rng: &mut Rng) -> Outcome {
    fight_watched(db, a, b, max_attacks, rng, &mut ())
}

pub fn fight_watched(
    db: &CardDb,
    a: &[Minion],
    b: &[Minion],
    max_attacks: u32,
    rng: &mut Rng,
    w: &mut impl Watch,
) -> Outcome {
    let mut uid = 0u16;
    let mut sides = [Side::new(a, &mut uid), Side::new(b, &mut uid)];
    let (na, nb) = (sides[0].ms.len(), sides[1].ms.len());
    let mut turn = if na > nb {
        0
    } else if nb > na {
        1
    } else {
        rng.coin() as usize
    };
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
            let (att, def) = pair(&mut sides, turn);
            let Some(ai) = att.ms.iter().position(|f| f.uid == me) else { break };
            if def.ms.is_empty() {
                break;
            }
            let di = pick_defender(def, rng);
            let (x, y) = (&mut att.ms[ai].m, &mut def.ms[di].m);
            w.attack(turn, x, y);
            let (dx, dy) = (x.attack, y.attack);
            hit(y, dx, x);
            hit(x, dy, y);
            // the attacking side's dead are handled first
            loop {
                let (att, def) = pair(&mut sides, turn);
                let c1 = att.resolve(db, turn, &mut uid, w);
                let c2 = def.resolve(db, turn ^ 1, &mut uid, w);
                if !(c1 || c2) {
                    break;
                }
            }
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
    let survivor_tiers = winner.map_or(0, |s| sides[s].ms.iter().map(|f| db.get(f.m.card).tier as i32).sum());
    Outcome { winner, survivor_tiers, attacks }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Data;

    fn side(db: &CardDb, names: &[&str]) -> (Side, u16) {
        let mut uid = 0;
        let board: Vec<Minion> = names.iter().map(|n| db.make(db.by_name(n).unwrap(), false)).collect();
        (Side::new(&board, &mut uid), uid)
    }

    fn names(db: &CardDb, s: &Side) -> Vec<String> {
        s.ms.iter().map(|f| db.get(f.m.card).name.clone()).collect()
    }

    #[test]
    fn deathrattle_summons_where_the_minion_stood() {
        let db = Data::bundled().cards;
        let (mut s, mut uid) = side(&db, &["Wolf Pup", "Den Mother", "Ram"]);
        s.ms[1].m.health = 0;
        s.resolve(&db, 0, &mut uid, &mut ());
        assert_eq!(names(&db, &s), ["Wolf Pup", "Pup", "Ram"]);
    }

    #[test]
    fn summons_past_seven_are_lost() {
        let db = Data::bundled().cards;
        let (mut s, mut uid) = side(&db, &["Ram", "Ram", "Alpha", "Ram", "Ram", "Ram", "Ram"]);
        s.ms[2].m.health = -3;
        s.resolve(&db, 0, &mut uid, &mut ());
        assert_eq!(names(&db, &s), ["Ram", "Ram", "Wolf", "Ram", "Ram", "Ram", "Ram"]);
    }

    #[test]
    fn golden_deathrattle_summons_golden_tokens() {
        let db = Data::bundled().cards;
        let mut uid = 0;
        let alpha = db.make(db.by_name("Alpha").unwrap(), true);
        let mut s = Side::new(&[alpha], &mut uid);
        s.ms[0].m.health = 0;
        s.resolve(&db, 0, &mut uid, &mut ());
        assert_eq!(s.ms.len(), 2);
        assert!(s.ms.iter().all(|f| f.m.golden && f.m.attack == 4 && f.m.health == 4));
    }

    #[test]
    fn removing_before_the_pointer_keeps_the_next_attacker() {
        let db = Data::bundled().cards;
        let (mut s, mut uid) = side(&db, &["Wolf Pup", "Ram", "Viper"]);
        s.ptr = 2; // Viper is next
        s.ms[0].m.health = 0;
        s.resolve(&db, 0, &mut uid, &mut ());
        assert_eq!(db.get(s.ms[s.next_attacker().unwrap()].m.card).name, "Viper");
    }
}
