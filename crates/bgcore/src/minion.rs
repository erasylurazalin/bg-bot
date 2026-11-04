use crate::CardDb;
use crate::rng::Rng;

pub type CardId = u16;

/// Maximum number of Dark Gifts one minion holds. A triple brings the gifts
/// of all three copies, a Magnetized Mech brings its own, and the button, the
/// four gift trinkets and the two heroes can each add one, so a minion in a
/// long game can collect a good few. Past this many, a new gift is not
/// attached (the minion keeps the ones it has; the offer still shows it).
pub const MAX_GIFTS: usize = 8;

/// Maximum number of effect-carrying Magnetic cards attached to a single host.
/// Cards without persistent effects (such as Satellites and Volumizers) only
/// increment `magnetizations` and take no slot here. When a host already
/// holds `MAX_ATTACHED` effect-carrying attachments, further merges still
/// grant full stats, keywords and magnetization count, and drop only the
/// excess trigger.
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
    /// Poisonous kills what it damages and is never used up (docs/rules/design.md
    /// "Keywords"). A separate keyword from Venomous, and not a Bonus Keyword:
    /// rules.toml's `bonus_keywords` leaves it out, so no random keyword and
    /// no "different Bonus Keywords" count ever reads it.
    pub const POISONOUS: u8 = 128;

    /// Every keyword a minion can carry, by its name in the data files.
    /// Which of these are Bonus Keywords is a rule (`bonus_keywords` in
    /// rules.toml), not a property of this list.
    pub const ALL: [(u8, &'static str); 8] = [
        (Self::TAUNT, "taunt"),
        (Self::DIVINE_SHIELD, "divine_shield"),
        (Self::WINDFURY, "windfury"),
        (Self::VENOMOUS, "venomous"),
        (Self::REBORN, "reborn"),
        (Self::MAGNETIC, "magnetic"),
        (Self::STEALTH, "stealth"),
        (Self::POISONOUS, "poisonous"),
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

/// One Dark Gift attached to a minion (docs/rules/darkgifts.md: a gift sits on its
/// minion like an attached card). Packed into 16 bits so `Minion` stays
/// small: the low ten bits are the gift's effect card (a token in
/// cards.toml's Dark Gifts section, ids below 1024) and the top six its own
/// small counter: a countdown (Incubation, Replication, Affinity), the turn
/// it landed (Steady Growth), or the hits its Divine Shield has left
/// (Toreth's Blessing). Every other gift keeps 0 there.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct GiftOn(u16);

/// How many bits of a `GiftOn` hold the card and how many the counter.
const GIFT_STATE_SHIFT: u32 = 10;
pub const GIFT_CARD_MAX: u16 = (1 << GIFT_STATE_SHIFT) - 1;
pub const GIFT_STATE_MAX: u8 = 63;

impl GiftOn {
    pub fn new(card: CardId, state: u8) -> GiftOn {
        debug_assert!(card <= GIFT_CARD_MAX, "gift card id {card} is too large");
        debug_assert!(state <= GIFT_STATE_MAX, "gift state {state} is too large");
        GiftOn(card | ((state.min(GIFT_STATE_MAX) as u16) << GIFT_STATE_SHIFT))
    }

    pub fn card(self) -> CardId {
        self.0 & GIFT_CARD_MAX
    }

    pub fn state(self) -> u8 {
        (self.0 >> GIFT_STATE_SHIFT) as u8
    }

    pub fn with_state(self, state: u8) -> GiftOn {
        GiftOn::new(self.card(), state)
    }
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
    /// How many Volumizers this minion is (1 for a Volumizer, 0 otherwise):
    /// the count "your Volumizers have +N/+M" scales by. A Volumizer
    /// Magnetized into a host is not counted there: the bonuses reach the
    /// Volumizers in the Tavern, the hand and on the board, never the ones
    /// already inside a host (Era, 2026-11-03).
    pub volumizers: u16,
    /// Per-minion scaling or progress counter (Lurking Leviathan, Spark
    /// Snapper, Patient Scout tier offset, and the count of an event
    /// trigger, which `events_left` turns into the "(N left!)" number).
    /// For a Living Prison it is the arms it holds: shares of the next
    /// bought minion's stats this turn (a golden's own press is worth two),
    /// which add up in a triple instead of taking the highest count.
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
    /// A Tavern tier it can't be played below (Faelin's Expedition Plans): it
    /// may be buffed in hand, but not played or summoned until the Tavern
    /// reaches that tier. 0 for none.
    pub locked_tier: u8,
    /// Gold the player must have spent this game before it unlocks (Thorim's
    /// Choose Your Champion: a Tier 7 minion in hand until 60 Gold is in).
    /// 0 for none.
    pub locked_gold: u16,
    /// A Tomb Turning Undead that dies right after it is played, as long as
    /// that happens in the turn it was Discovered (the flag comes off in
    /// hand at the end of the turn).
    pub doomed: bool,
    /// Played from hand into this slot and its "after you play an X" triggers
    /// have not fired yet: it never counts for itself, whatever its own play
    /// moved around on the board (`resolve_after_play` clears it).
    pub just_played: bool,
    /// Whether its Activate was already used this turn. Cleared at the start
    /// of its owner's turn; a copy bought or made later starts unused.
    pub activated: bool,
    /// Limited uses this minion still has this turn (a Thorned Trailblazer's
    /// combines, a Magicfin Mycologist's teachings, Malchezaar's Health
    /// Refreshes; `turn_uses` counts them). They belong to the minion: spent
    /// left to right, gone when it is sold, and a golden from a triple starts
    /// fresh while the copies take their leftovers with them.
    pub uses_left: u8,
    /// Attack and Health this minion has gained from Blood Gems, from every
    /// source (Gem Confiscation steals exactly these). Shop gains persist;
    /// a fight's gems are gone with it unless they were permanent.
    pub gem_attack: i32,
    pub gem_health: i32,
    /// A Choose One card with both effects combined (Fandral's Fortune's
    /// Discover): playing it runs both options instead of asking.
    pub both_effects: bool,
    /// Linked hand cards share a nonzero id. The high bits mark a golden
    /// four-card group and whether its first card was played.
    pub linked: u16,
    pub attached: [Attached; MAX_ATTACHED],
    pub n_attached: u8,
    /// The Dark Gifts on this minion, in the order they landed.
    pub gifts: [GiftOn; MAX_GIFTS],
    pub n_gifts: u8,
    /// A Divine Shield that takes several hits to break: the hits it has
    /// left, 0 (or 1) for an ordinary one. Set by Toreth's Blessing; it never
    /// carries out of a fight, like any other shield damage.
    pub shield_hits: u8,
    /// Dancin' Deryl's hats on this minion (Hat Trick): each is +1/+1, already
    /// counted in its stats, and they move to another friendly minion when it
    /// is sold (docs/rules/heroes.md).
    pub hats: u8,
    /// Darkmoon Tickets on this card while it sits in the Tavern (Come One,
    /// Come All!): a mark the buy takes, not a card of its own.
    pub tickets: u8,
    /// Its Reborn came from Reborn Rites and comes off at the start of the
    /// next turn. A Reborn printed on the card stays.
    pub temp_reborn: bool,
    /// The Attack a temporary swap added or took off (Spirit Swap): undone at
    /// the start of the next turn, so a later buff stays.
    pub temp_attack: i32,
    /// Frozen on its own in the Tavern (Twice as Nice freezes the two cards
    /// it names, not the whole Tavern): it stays one more turn on its own.
    pub frozen: bool,
    /// Marked by Rapid Reanimation (Teron Gorefiend) for the next combat:
    /// destroyed at Start of Combat and resummoned as an exact copy when
    /// space opens. Cleared at the start of the next turn.
    pub reanimate: bool,
    /// What this minion gains at the end of its owner's turn, for good: a
    /// Prize gave it "At the end of your turn, gain +4/+4" (Gruul Rules).
    /// (0, 0) for none.
    pub end_turn_gain: (i32, i32),
    /// Watched by a Makeshift Master this turn: whatever stats it gains
    /// outside combat the Deity gains too, wherever it is (the Tavern when it
    /// is chosen, the hand after a buy, the board after it is played). The
    /// watch ends with the turn, and with the card when it is sold.
    pub watched: bool,
    /// Tribes a portrait gave every copy of this card its owner has or will
    /// have (Bronzebeard Portrait: a Brann counts as a Murloc and a Dragon).
    /// `CardDb::tribes_of` adds them, so every tribe check in the engine
    /// sees them: board, hand, the most common type, tribe synergies and a
    /// fight.
    pub extra_tribes: crate::cards::Tribes,
}

impl Minion {
    /// 2 for a golden minion, else 1: what its card's numbers are multiplied by.
    /// Whether it is locked in hand right now, for any of the locks
    /// (docs/rules/design.md "Cards locked in hand"): for a turn or two, until the Tavern
    /// reaches its tier, or until enough Gold has been spent this game. A
    /// locked card takes a hand slot, can't be played, sold, discarded or
    /// summoned from the hand, and hand buffs still reach it.
    pub fn locked_in_hand(&self, tier: u8, gold_spent: u32) -> bool {
        self.locked_turns > 0
            || (self.locked_tier > 0 && self.locked_tier > tier)
            || (self.locked_gold > 0 && gold_spent < self.locked_gold as u32)
    }

    pub fn mul(&self) -> i32 {
        if self.golden { 2 } else { 1 }
    }

    /// The limited uses a fresh copy of this minion has each turn: a Thorned
    /// Trailblazer's combines (the base card; it cannot be Magnetized), a
    /// Magicfin Mycologist's teachings and a Malchezaar's Health Refreshes
    /// (an attached part brings its own). A real minion has at most one of
    /// these effects, so the maximum is its count.
    pub fn turn_uses(&self, db: &CardDb) -> u8 {
        let mut uses = db.get(self.card).choose_one_cover.saturating_mul(self.mul() as u8);
        for (c, golden) in self.parts() {
            let card = db.get(c);
            let mul: u8 = if golden { 2 } else { 1 };
            uses = uses
                .max(card.health_refreshes.saturating_mul(mul))
                .max(if card.teaches_spell { mul } else { 0 });
        }
        uses
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

    pub fn gift_slice(&self) -> &[GiftOn] {
        &self.gifts[..self.n_gifts as usize]
    }

    /// Attach a gift. A gift works the same on a golden minion; gifts stack
    /// (docs/rules/darkgifts.md "Gifts stack"), so the same gift can sit on a
    /// minion twice and fire twice, except the ones that set a state
    /// (`Card::gift_unique`), which `push_gift_unique` drops. Returns whether
    /// it was attached.
    pub fn push_gift(&mut self, g: GiftOn) -> bool {
        let n = self.n_gifts as usize;
        if n < MAX_GIFTS {
            self.gifts[n] = g;
            self.n_gifts += 1;
            return true;
        }
        false
    }

    /// Take the gift whose effect card is `card` off this minion, keeping the
    /// order of the rest.
    pub fn remove_gift(&mut self, card: CardId) -> Option<GiftOn> {
        let n = self.n_gifts as usize;
        let at = self.gifts[..n].iter().position(|g| g.card() == card)?;
        let g = self.gifts[at];
        for k in at..n - 1 {
            self.gifts[k] = self.gifts[k + 1];
        }
        // The vacated slot goes back to a default, so two minions with the
        // same gifts compare equal however many they had before.
        self.gifts[n - 1] = GiftOn::default();
        self.n_gifts -= 1;
        Some(g)
    }

    /// Attach a gift that does not stack: false (and nothing attached) when
    /// the minion already holds it.
    pub fn push_gift_unique(&mut self, g: GiftOn) -> bool {
        if self.gift_slice().iter().any(|o| o.card() == g.card()) {
            return false;
        }
        self.push_gift(g)
    }

    /// Set the counter of the gift at index `k`.
    pub fn set_gift_state_at(&mut self, k: usize, state: u8) -> bool {
        if let Some(g) = self.gifts.get(k) {
            self.gifts[k] = g.with_state(state);
            return k < self.n_gifts as usize;
        }
        false
    }

    /// Take the gift at index `k` off this minion.
    pub fn remove_gift_at(&mut self, k: usize) -> Option<GiftOn> {
        let n = self.n_gifts as usize;
        if k >= n {
            return None;
        }
        let g = self.gifts[k];
        for j in k..n - 1 {
            self.gifts[j] = self.gifts[j + 1];
        }
        self.gifts[n - 1] = GiftOn::default();
        self.n_gifts -= 1;
        Some(g)
    }

    /// The counter of the gift on this minion whose effect card is `card`.
    pub fn gift_state(&self, card: CardId) -> Option<u8> {
        self.gift_slice().iter().find(|g| g.card() == card).map(|g| g.state())
    }

    /// Write the counter of the gift whose effect card is `card`. False when
    /// the minion holds no such gift.
    pub fn set_gift_state(&mut self, card: CardId, state: u8) -> bool {
        let n = self.n_gifts as usize;
        for g in &mut self.gifts[..n] {
            if g.card() == card {
                *g = g.with_state(state);
                return true;
            }
        }
        false
    }

    /// Whether any gift on this minion has a card property (`pick`), which is
    /// how the engine asks "does this minion have the Efficiency gift".
    pub fn has_gift(&self, db: &CardDb, pick: impl Fn(&crate::cards::Card) -> bool) -> bool {
        if self.n_gifts == 0 {
            return false;
        }
        self.gift_slice().iter().any(|g| pick(db.get(g.card())))
    }

    /// The "(N left!)" of this minion's event trigger, None when no part of
    /// it has one.
    pub fn events_left(&self, db: &CardDb) -> Option<u8> {
        self.parts().find_map(|(c, _)| db.get(c).on_event).map(|t| {
            t.every.saturating_sub(self.improvements.min(u8::MAX as u16) as u8).max(1)
        })
    }

    /// Yields `(card, golden)` for the minion itself followed by every
    /// effect-carrying card Magnetized into it and then every Dark Gift on it.
    /// A gift never doubles: its pair is always `false`, whatever the host.
    pub fn parts(&self) -> impl Iterator<Item = (CardId, bool)> + '_ {
        std::iter::once((self.card, self.golden))
            .chain(self.attached_slice().iter().map(|a| (a.card, a.golden)))
            .chain(self.gift_slice().iter().map(|g| (g.card(), false)))
    }

    /// Merge `other` into `self` as a Magnetization, recursively bringing all
    /// its stats, keywords, magnetization counts and effects. Its Volumizer
    /// count stays behind: a Volumizer inside a host keeps the stats it
    /// brought, and later "your Volumizers" bonuses don't reach it.
    pub fn magnetize(&mut self, db: &CardDb, other: Minion) {
        self.attack += other.attack;
        self.health += other.health;
        self.extra_tribes = self.extra_tribes.with_all(other.extra_tribes);
        self.end_turn_gain.0 += other.end_turn_gain.0;
        self.end_turn_gain.1 += other.end_turn_gain.1;
        self.keywords.0 |= other.keywords.0;
        self.winners_bread = self.winners_bread.saturating_add(other.winners_bread);
        self.magnetizations = self.magnetizations.saturating_add(other.magnetizations).saturating_add(1);
        self.gem_attack = self.gem_attack.saturating_add(other.gem_attack);
        self.gem_health = self.gem_health.saturating_add(other.gem_health);
        if db.get(other.card).carries_effect() {
            self.push_attached(Attached { card: other.card, golden: other.golden });
        }
        for &a in other.attached_slice() {
            self.push_attached(a);
        }
        // A Magnetized Mech keeps its own gifts; the magnetic minion's are
        // lost (docs/rules/darkgifts.md "Magnetic minions and gifts").
    }
}
