use crate::CardDb;
use crate::rng::Rng;

pub type CardId = u16;

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
    pub const STEALTH: u8 = 64;

    /// Every keyword a minion can carry, by its name in the data files.
    /// Which of these are Bonus Keywords is a rule (`bonus_keywords` in
    /// rules.toml), not a property of this list.
    pub const ALL: [(u8, &'static str); 7] = [
        (Self::TAUNT, "taunt"),
        (Self::DIVINE_SHIELD, "divine_shield"),
        (Self::WINDFURY, "windfury"),
        (Self::VENOMOUS, "venomous"),
        (Self::REBORN, "reborn"),
        (Self::MAGNETIC, "magnetic"),
        (Self::STEALTH, "stealth"),
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

    /// A random keyword out of `mask` that this set doesn't have yet, None
    /// when it has them all.
    pub fn random_missing(self, mask: u8, rng: &mut Rng) -> Option<u8> {
        let missing = mask & !self.0;
        let n = missing.count_ones();
        if n == 0 {
            return None;
        }
        let mut k = rng.below(n);
        for bit in 0..8 {
            let b = 1u8 << bit;
            if missing & b != 0 {
                if k == 0 {
                    return Some(b);
                }
                k -= 1;
            }
        }
        None
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
    /// Gives no Triple Reward when played (Aureate Laureate, Silent
    /// Deliverer's minions).
    pub no_reward: bool,
    /// Its Divine Shield was given "until next turn" (Ichoron the
    /// Protector): it comes off at the start of the next shop phase.
    pub temp_shield: bool,
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
    /// Snapper, Patient Scout tier offset, and the count of an event
    /// trigger, which `events_left` turns into the "(N left!)" number).
    pub improvements: u16,
    /// Extra times the next Magnetization to this minion happens this turn
    /// (Drone Duplicator). Used up by that Magnetization, cleared at the end
    /// of the turn.
    pub extra_magnetize: u8,
    /// The Tavern spell a token in hand was taught (Magicfin Apprentice).
    /// Its Battlecry casts that spell.
    pub taught_spell: Option<CardId>,
    /// Turns this minion is locked in hand and can't be played (Search
    /// Through Time). It goes down by one at the start of its owner's turn.
    pub locked_turns: u8,
    /// A Tomb Turning Undead that dies right after it is played, as long as
    /// that happens in the turn it was Discovered (the flag comes off in
    /// hand at the end of the turn).
    pub doomed: bool,
    /// Whether its Activate was already used this turn. Cleared at the start
    /// of its owner's turn; a copy bought or made later starts unused.
    pub activated: bool,
    /// Attack and Health this minion has gained from Blood Gems, from every
    /// source (Gem Confiscation steals exactly these). Shop gains persist;
    /// a fight's gems are gone with it unless they were permanent.
    pub gem_attack: i32,
    pub gem_health: i32,
    /// A Choose One card with both effects combined (Fandral's Fortune's
    /// Discover): playing it runs both options instead of asking.
    pub both_effects: bool,
    pub attached: [Attached; MAX_ATTACHED],
    pub n_attached: u8,
}

impl Minion {
    /// 2 for a golden minion, else 1: what its card's numbers are multiplied by.
    pub fn mul(&self) -> i32 {
        if self.golden { 2 } else { 1 }
    }

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

    /// The "(N left!)" of this minion's event trigger, None when no part of
    /// it has one.
    pub fn events_left(&self, db: &CardDb) -> Option<u8> {
        self.parts().find_map(|(c, _)| db.get(c).on_event).map(|t| {
            t.every.saturating_sub(self.improvements.min(u8::MAX as u16) as u8).max(1)
        })
    }

    /// Yields `(card, golden)` for the minion itself followed by every
    /// effect-carrying card Magnetized into it.
    pub fn parts(&self) -> impl Iterator<Item = (CardId, bool)> + '_ {
        std::iter::once((self.card, self.golden))
            .chain(self.attached_slice().iter().map(|a| (a.card, a.golden)))
    }

    /// Merge `other` into `self` as a Magnetization, recursively bringing all
    /// its stats, keywords, magnetization counts, Volumizer counts and effects.
    pub fn magnetize(&mut self, db: &CardDb, other: Minion) {
        self.attack += other.attack;
        self.health += other.health;
        self.keywords.0 |= other.keywords.0;
        self.winners_bread = self.winners_bread.saturating_add(other.winners_bread);
        self.magnetizations = self.magnetizations.saturating_add(other.magnetizations).saturating_add(1);
        self.volumizers = self.volumizers.saturating_add(other.volumizers);
        self.gem_attack = self.gem_attack.saturating_add(other.gem_attack);
        self.gem_health = self.gem_health.saturating_add(other.gem_health);
        if db.get(other.card).carries_effect() {
            self.push_attached(Attached { card: other.card, golden: other.golden });
        }
        for &a in other.attached_slice() {
            self.push_attached(a);
        }
    }
}
