use crate::CardDb;

pub type CardId = u8;

/// Maximum number of effect-carrying Magnetic cards attached to a single host.
/// Cards without persistent effects (such as Satellites and Volumizers) only
/// increment `magnetizations` and `volumizers` and take no slot here. When a
/// host already holds `MAX_ATTACHED` effect-carrying attachments, further
/// merges still grant full stats, keywords, magnetization count and Volumizer
/// count, and drop only the excess trigger.
pub const MAX_ATTACHED: usize = 16;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Keywords(pub u8);

impl Keywords {
    pub const TAUNT: u8 = 1;
    pub const DIVINE_SHIELD: u8 = 2;
    pub const WINDFURY: u8 = 4;
    pub const VENOMOUS: u8 = 8;
    pub const REBORN: u8 = 16;
    pub const MAGNETIC: u8 = 32;

    pub const ALL: [(u8, &'static str); 6] = [
        (Self::TAUNT, "taunt"),
        (Self::DIVINE_SHIELD, "divine_shield"),
        (Self::WINDFURY, "windfury"),
        (Self::VENOMOUS, "venomous"),
        (Self::REBORN, "reborn"),
        (Self::MAGNETIC, "magnetic"),
    ];

    pub fn has(self, k: u8) -> bool {
        self.0 & k != 0
    }

    pub fn set(&mut self, k: u8) {
        self.0 |= k;
    }

    pub fn clear(&mut self, k: u8) {
        self.0 &= !k;
    }
}

/// An effect-carrying card Magnetized into a host Mech.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Attached {
    pub card: CardId,
    pub golden: bool,
}

/// A minion in a shop, hand, board or fight. Health is current health; in
/// the shop phase it is also the maximum, since damage never carries over.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Minion {
    pub card: CardId,
    pub attack: i32,
    pub health: i32,
    pub keywords: Keywords,
    pub golden: bool,
    /// Pool copies held by this minion (1 when drawn from the pool, up to 3
    /// when tripled from pool copies, 0 when generated).
    pub pool_copies: u8,
    /// Pending Winner's Bread triggers waiting on the next combat's result.
    pub winners_bread: u8,
    /// Total Magnetizations on this minion. Merging a card with N
    /// Magnetizations adds N + 1.
    pub magnetizations: u16,
    /// Number of Volumizers this minion is or carries inside it.
    pub volumizers: u16,
    /// Per-minion scaling or progress counter (Lurking Leviathan, Spark
    /// Snapper, Patient Scout tier offset, Auto Reveille buy counter).
    pub improvements: u16,
    pub attached: [Attached; MAX_ATTACHED],
    pub n_attached: u8,
}

impl Minion {
    pub fn attached_slice(&self) -> &[Attached] {
        &self.attached[..self.n_attached as usize]
    }

    pub fn push_attached(&mut self, a: Attached) {
        let n = self.n_attached as usize;
        if n < MAX_ATTACHED {
            self.attached[n] = a;
            self.n_attached += 1;
        }
    }

    /// Yields `(card, golden)` for the minion itself followed by every
    /// effect-carrying card Magnetized into it.
    pub fn parts(&self) -> impl Iterator<Item = (CardId, bool)> + '_ {
        std::iter::once((self.card, self.golden))
            .chain(self.attached_slice().iter().map(|a| (a.card, a.golden)))
    }

    /// Merge `other` into `self` as a Magnetization, recursively bringing all
    /// stats, keywords, magnetization counts, Volumizer counts and effects.
    pub fn magnetize(&mut self, db: &CardDb, other: Minion) {
        self.attack += other.attack;
        self.health += other.health;
        self.keywords.0 |= other.keywords.0;
        self.winners_bread = self.winners_bread.saturating_add(other.winners_bread);
        self.magnetizations = self.magnetizations.saturating_add(other.magnetizations).saturating_add(1);
        self.volumizers = self.volumizers.saturating_add(other.volumizers);
        if db.get(other.card).carries_effect() {
            self.push_attached(Attached { card: other.card, golden: other.golden });
        }
        for &a in other.attached_slice() {
            self.push_attached(a);
        }
    }
}
