//! The Secret zone (docs/heroes.md "Secrets", Era's rules).
//!
//! A player holds at most the four combat Secrets plus, in the same zone, the
//! Ice Block a Safety Patch gives (the trinket's save, which has nothing to do
//! with the four and does not count toward Akazamzarak's offer). The zone is
//! ordered: Secrets fire in the order they were got, so the order is data, not
//! a bit set alone. There is no allocation: five `u8` kinds and a length.
//!
//! A Secret stays until it fires. A Secret that summons waits for room and
//! stays when there is none (`fight::Side::attack_secrets`, `Side::resolve`).
//!
//! The card each kind is: `TB_Bacon_Secrets_01` (Venomstrike Trap), `_07`
//! (Autodefense Matrix), `_15` (Pack Tactics), `_10` (Redemption) and `_12`
//! (Ice Block). The other Secrets in the export (the Better versions, Snake
//! Trap, Splitting Image, Effigy, Avenge, Hand of Salvation, Competitive
//! Spirit, Reckoning) are not in the game.

/// When a friendly minion is attacked: summon a 2/3 Venomous Cobra.
pub const VENOMSTRIKE_TRAP: u8 = 0;
/// When a friendly minion is attacked: give it Divine Shield before the hit.
pub const AUTODEFENSE_MATRIX: u8 = 1;
/// When a friendly minion is attacked: summon a copy of it with 3/3 base
/// stats to its right.
pub const PACK_TACTICS: u8 = 2;
/// When a friendly minion dies: return it with 1 Health where it stood.
pub const REDEMPTION: u8 = 3;
/// Safety Patch's save. It is in the zone but it is not one of the four, so it
/// is never offered and never fires in a fight.
pub const ICE_BLOCK: u8 = 4;

/// The combat Secrets: the ones Prestidigitation offers.
pub const COMBAT_SECRETS: u8 = 4;
/// Kinds the zone can hold: the four combat Secrets and the Ice Block.
pub const MAX_ZONE: usize = 5;

/// Every kind's name, by index (`docs/heroes.md` names).
pub const NAMES: [&str; MAX_ZONE] = [
    "Venomstrike Trap",
    "Autodefense Matrix",
    "Pack Tactics",
    "Redemption",
    "Ice Block",
];

/// Every kind's card id in the export, by index.
pub const CARD_IDS: [&str; MAX_ZONE] = [
    "TB_Bacon_Secrets_01",
    "TB_Bacon_Secrets_07",
    "TB_Bacon_Secrets_15",
    "TB_Bacon_Secrets_10",
    "TB_Bacon_Secrets_12",
];

/// A player's Secret zone: the kinds it holds, in the order they were got.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Secrets {
    kinds: [u8; MAX_ZONE],
    n: u8,
}

impl Secrets {
    pub const fn new() -> Secrets {
        Secrets { kinds: [0; MAX_ZONE], n: 0 }
    }

    pub fn len(&self) -> usize {
        self.n as usize
    }

    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    /// Whether the zone holds this kind.
    pub fn has(&self, kind: u8) -> bool {
        (kind as usize) < MAX_ZONE && self.kinds[..self.len()].contains(&kind)
    }

    /// Add a kind at the end: the order Secrets were got is the order they
    /// fire in. False when it is already held or the zone is full.
    pub fn add(&mut self, kind: u8) -> bool {
        if (kind as usize) >= MAX_ZONE || self.has(kind) || self.len() >= MAX_ZONE {
            return false;
        }
        self.kinds[self.n as usize] = kind;
        self.n += 1;
        true
    }

    /// Take a kind out of the zone (a Secret that fired, the Ice Block spent).
    pub fn remove(&mut self, kind: u8) -> bool {
        let Some(i) = self.kinds[..self.len()].iter().position(|&k| k == kind) else {
            return false;
        };
        let n = self.len();
        for j in i..n - 1 {
            self.kinds[j] = self.kinds[j + 1];
        }
        self.n -= 1;
        self.kinds[self.n as usize] = 0;
        true
    }

    /// The kinds, in the order they were got (the order they fire in).
    pub fn iter(&self) -> impl Iterator<Item = u8> + '_ {
        self.kinds[..self.len()].iter().copied()
    }

    /// How many of the four combat Secrets are held (never the Ice Block).
    pub fn combat_len(&self) -> usize {
        self.iter().filter(|&k| k < COMBAT_SECRETS).count()
    }

    /// Whether the Safety Patch's Ice Block is still unspent.
    pub fn ice_block(&self) -> bool {
        self.has(ICE_BLOCK)
    }

    /// Whether every combat Secret is in the zone, so Prestidigitation has
    /// nothing left to offer.
    pub fn all_combat(&self) -> bool {
        self.combat_len() == COMBAT_SECRETS as usize
    }
}
