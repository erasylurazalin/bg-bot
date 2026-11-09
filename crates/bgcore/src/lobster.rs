//! The lobster search's own mode (docs/briefs/lobster-search.md): the Tavern
//! and Headhunter Gryphon's random Beasts come from streams fixed by a seed
//! and an index, so a line is reproducible and the search sees its future.
//! The mode is off in every normal game.

use crate::cards::{CardDb, Tribe, Tribes};
use crate::minion::CardId;
use crate::game::Game;
use crate::rng::Rng;
use crate::rules::Rules;

const TAG_TAVERN: u64 = 0x0000_7461_7665_726e;
const TAG_TAVERN_SPELL: u64 = 0x0000_0073_7065_6c6c;
const TAG_BEAST: u64 = 0x0000_0062_6561_7374;

/// One player's stream state. A copy lives in `Player`, so every search copy
/// carries its own position in the streams and clones exactly.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LobsterMode {
    pub seed: u64,
    /// The Shark knob in per mille: 0, 100, 250 or 500 for p = 0, 0.1, 0.25,
    /// 0.5.
    pub shark_per_mille: u16,
    /// The turn the streams belong to, reset at the start of every turn.
    pub turn: u32,
    /// The next Tavern to draw in this turn: 0 is the turn's own roll, then
    /// each explicit Refresh or Refresh effect (a sold Snarky Shark) takes the
    /// next one.
    pub refresh: u32,
    /// The next random Beast the Gryphon's Rally gives in this turn.
    pub beast: u32,
    /// Every Fishbait the left-most Beast has attacked in this mode, all game.
    pub fishbait_attacks: u32,
    /// Every Deathstrider deathrattle trigger fired in this mode, all game:
    /// a golden Deathstrider's second trigger counts twice.
    pub deathstrider_fires: u32,
    /// Whether the Tavern draw uses the fixed stream. The logged-turn replay
    /// turns this off and puts the log's own Tavern in place instead.
    pub stream_tavern: bool,
    /// Whether the Gryphon's random Beasts use the fixed stream. The replay
    /// turns this off and uses the engine RNG, as the shop replay does.
    pub stream_beasts: bool,
}

impl LobsterMode {
    pub fn new(seed: u64, shark_per_mille: u16, turn: u32) -> LobsterMode {
        LobsterMode {
            seed,
            shark_per_mille,
            turn,
            refresh: 0,
            beast: 0,
            fishbait_attacks: 0,
            deathstrider_fires: 0,
            stream_tavern: true,
            stream_beasts: true,
        }
    }

    /// A private stream for one index. Every draw is a pure function of the
    /// seed and the index, never of how many other draws happened, so the
    /// Tavern at refresh N is the same whatever path reached it.
    fn rng(&self, tag: u64, index: u32, slot: u32) -> Rng {
        Rng::new(mix(self.seed, tag, self.turn as u64, index as u64, slot as u64))
    }

    fn shark_or_lionfish(
        &self,
        cards: &CardDb,
        rng: &mut Rng,
    ) -> Option<CardId> {
        if self.shark_per_mille == 0 || rng.below(1000) >= self.shark_per_mille as u32 {
            return None;
        }
        // Equal odds, one coin.
        if rng.coin() {
            cards.by_name("Snarky Shark")
        } else {
            cards.by_name("Lurking Lionfish")
        }
    }

    /// The minion in Tavern slot `slot` of refresh `refresh`. A normal draw is
    /// weighted like a fresh pool (each card by its tier's copy count), so the
    /// mode's normal stream matches what a full pool would offer; the extra
    /// Shark or Lionfish is generated and takes no pool copy.
    pub fn tavern_minion(
        &self,
        cards: &CardDb,
        rules: &Rules,
        lobby: Tribes,
        tier: u8,
        refresh: u32,
        slot: u32,
    ) -> Option<CardId> {
        let mut rng = self.rng(TAG_TAVERN, refresh, slot);
        if let Some(c) = self.shark_or_lionfish(cards, &mut rng) {
            return Some(c);
        }
        let high = (tier as usize).min(crate::CARD_MAX_TIER as usize);
        let ids = cards.ids_up_to_tier.get(high)?;
        let mut total = 0u32;
        for &c in ids {
            let card = cards.get(c);
            if card.pool_slot as usize != c as usize || card.is_spell() || !card.lobby_ok(lobby) {
                continue;
            }
            let copies = rules.copies(card.tier).max(1) as u32;
            total += copies;
        }
        if total == 0 {
            return None;
        }
        let mut r = rng.below(total);
        for &c in ids {
            let card = cards.get(c);
            if card.pool_slot as usize != c as usize || card.is_spell() || !card.lobby_ok(lobby) {
                continue;
            }
            let n = rules.copies(card.tier).max(1) as u32;
            if r < n {
                return Some(c);
            }
            r -= n;
        }
        None
    }

    /// The Tavern's spell slot for refresh `refresh`, a normal spell draw.
    pub fn tavern_spell(
        &self,
        cards: &CardDb,
        lobby: Tribes,
        tier: u8,
        refresh: u32,
    ) -> Option<CardId> {
        let mut rng = self.rng(TAG_TAVERN_SPELL, refresh, 0);
        cards.random_spell(tier, lobby, &mut rng)
    }

    /// The next random Beast for the Gryphon's Rally in this turn. A normal
    /// draw is uniform over the pool's Beasts at or below the player's tier,
    /// the same as the engine's `random_pool_minion`. The Shark knob also
    /// applies here, with the same equal odds.
    pub fn next_beast(&mut self, cards: &CardDb, lobby: Tribes, tier: u8) -> Option<CardId> {
        let index = self.beast;
        self.beast = self.beast.saturating_add(1);
        let mut rng = self.rng(TAG_BEAST, index, 0);
        if let Some(c) = self.shark_or_lionfish(cards, &mut rng) {
            return Some(c);
        }
        let high = tier.min(crate::MAX_TIER);
        let ok = |c: usize| {
            let card = cards.get(c as CardId);
            card.pool_slot as usize == c
                && card.tribes.has(Tribe::Beast)
                && card.tier <= high
                && card.lobby_ok(lobby)
        };
        let count = (0..cards.pool_cards).filter(|&c| ok(c)).count() as u32;
        if count == 0 {
            return None;
        }
        let k = rng.below(count) as usize;
        (0..cards.pool_cards).filter(|&c| ok(c)).nth(k).map(|c| c as CardId)
    }
}

impl Game {
    /// Turn the mode on for player `p`. The Tavern is not rolled here: the
    /// caller seats the start position's own board and hand first, then calls
    /// `seat_lobster_tavern`.
    pub fn enable_lobster_mode(&mut self, p: usize, seed: u64, shark_per_mille: u16) {
        let turn = self.turn;
        self.players[p].lobster = Some(LobsterMode::new(seed, shark_per_mille, turn));
        // The mode's streams do not read or consume the pool, and the extra
        // Sharks and Lionfish are generated: the copy accounting is not the
        // normal game's, so the invariant checker is told.
        self.pool_untracked = true;
    }

    /// The next Gryphon Beast in this turn's stream, advancing the stream.
    pub fn lobster_next_beast(&mut self, p: usize) -> Option<CardId> {
        let mut mode = self.players[p].lobster?;
        let tier = self.players[p].tier;
        let card = mode.next_beast(&self.data.cards, self.lobby, tier);
        self.players[p].lobster = Some(mode);
        card
    }
}

/// SplitMix64's finalizer, used to spread a stream's index fields over a seed.
fn mix(seed: u64, tag: u64, turn: u64, index: u64, slot: u64) -> u64 {
    let mut x = seed;
    for v in [tag, turn, index, slot] {
        x = x.wrapping_add(v).wrapping_add(0x9e37_79b9_7f4a_7c15);
        x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        x ^= x >> 31;
    }
    x
}
