//! One 8-player game: shop phase, pairing, fights, knockouts.
//!
//! The shop phase is simultaneous in the real game. Here the players act one
//! action at a time, in a random order that changes every turn, until each
//! has ended their turn. `current_player` says whose action is next.

use std::sync::Arc;

use arrayvec::ArrayVec;

use crate::action::{Action, N_ACTIONS};
use crate::cards::{
    N_COUNTERS, Aura, Card, CardDb, ChooseOption, ConsumeBy, ConsumePick, DiscoverKind, Effect,
    EndOfTurn, EventKind, OnCastSpell, OnCastSpellOnMech, OnPlayMech, OnPlayOrMagnetizeMech,
    OnDiscard, OnSell, OnSummonBeast, SpellEffect, SpellTarget, StartOfTurn, TimesRole, Tribe,
    Tribes, how_many_times, pick_one_per_type,
};
use crate::fight::{Counters, Deity, SeenCounters, SideOutcome, fight_with_hands};
use crate::minion::{CardId, Keywords, Minion};
use crate::rng::Rng;
use crate::rules::Rules;
use crate::{BOARD, DISCOVER, Data, HAND, MAX_TIER, PLAYERS, SHOP, TRIBES};

/// Upper bound on pool minions, for the fixed-size copy count in
/// `check_invariants`. The card parser refuses a card list past it.
pub const MAX_POOL_CARDS: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Opponent {
    Player(u8),
    /// The ghost of a knocked-out player.
    Ghost(u8),
}

/// What a player last saw of an opponent's board, in their last fight.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Seen {
    pub board: ArrayVec<Minion, BOARD>,
    pub tier: u8,
    /// The turn of that fight, 0 for never.
    pub turn: u32,
    /// Their Deity at the first attack of that fight, after Start of
    /// Combat effects.
    pub deity: Option<Deity>,
    /// Their Blood Gem value at that moment (what the client shows during
    /// combat, so it is "as last seen").
    pub gem: (i32, i32),
    /// Their Tavern spell bonus at that moment.
    pub spell: (i32, i32),
}

impl Seen {
    /// Both sides of a fight as the other player sees it: the board that went
    /// in, and the counters at the first attack, after Start of Combat.
    fn at_fight(board: ArrayVec<Minion, BOARD>, tier: u8, turn: u32, c: SeenCounters) -> Seen {
        Seen {
            board,
            tier,
            turn,
            deity: c.deity,
            gem: c.gem,
            spell: c.spell,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TargetKind {
    MinionBuff {
        tribe: Option<Tribe>,
        attack: i32,
        health: i32,
        keyword: u8,
        /// Also give a random Bonus Keyword the minion doesn't have.
        random_keyword: bool,
    },
    /// Pick the friendly minion that consumes `count` Tavern minions.
    /// `stat_mul` doubles the stats taken by a golden consumer.
    Consume {
        tribe: Option<Tribe>,
        pick: ConsumePick,
        keywords: bool,
        count: u8,
        stat_mul: u8,
    },
    Spell(SpellTarget),
    /// Trigger the Rally of the chosen minion in the shop (Sky-hatch
    /// Runaway), `runs` times.
    TriggerRally,
    /// Trigger the Battlecry of the chosen minion (Kelp Keeper), `runs`
    /// times.
    TriggerBattlecry,
    /// Buff the chosen minion on the board or in the Tavern (Suspicious
    /// Prisonguard).
    AnyBuff { attack: i32, health: i32 },
    /// Set the chosen minion's (board or Tavern) stats (Tyrael).
    AnySetStats { attack: i32, health: i32 },
    /// Make the chosen friendly minion (of a tribe, up to a tier) Golden on
    /// the board (Elite Navigator, Captain Sanders). The pending target's
    /// `runs` is how many are still to be made.
    MakeGolden { tribe: Option<Tribe>, max_tier: u8 },
    /// Destroy the chosen friendly minion of a tribe (not this one), then
    /// Discover `discover` minions of that tribe (Maw Caster).
    DestroyDiscover { tribe: Tribe, discover: u8 },
    /// Destroy the chosen friendly minion of a tribe (not this one), then
    /// get `copies` plain copies of it (Disguised Graverobber).
    DestroyCopy { tribe: Tribe, copies: u8 },
    /// Give the chosen friendly minion of a tribe, not a copy of this card,
    /// Reborn and destroy it; this minion gains the stats (Dead Bellringer).
    RebornDestroy { tribe: Tribe, attack: i32, health: i32 },
}

impl TargetKind {
    /// Whether the player may pick a minion in the Tavern for this target.
    pub fn takes_shop(self) -> bool {
        match self {
            TargetKind::AnyBuff { .. } | TargetKind::AnySetStats { .. } => true,
            TargetKind::Spell(st) => st.can_target_shop(),
            _ => false,
        }
    }
}

/// A targeted Battlecry, Choose One option, or Tavern spell waiting for the
/// player to pick its target on the board.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PendingTarget {
    /// Board slot of the minion whose effect it is, or None for a spell.
    pub source: Option<u8>,
    pub kind: TargetKind,
    pub runs: u8,
    /// The spell being cast is a Tavern spell: only those fire "Tavern spell"
    /// triggers and spell counters, and get spell power.
    pub tavern: bool,
    /// The spell being cast is a Blood Gem: its buff is the player's Blood
    /// Gem value, the stats it gives are Blood Gem stats, and each cast is a
    /// Gem played on the target (Geomagus Roogug reacts to those from hand).
    pub gem: bool,
}

/// A chosen consume waiting for the player to pick the Tavern minion.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PendingShopTarget {
    /// Board slot of the minion whose effect it is.
    pub source: u8,
    pub kind: ShopTargetKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShopTargetKind {
    Consume {
        keywords: bool,
        /// How many more Tavern minions it consumes.
        remaining: u8,
        /// Doubles the stats taken by a golden consumer.
        stat_mul: u8,
    },
    /// Replace the Tavern minion with this card (golden or not), then have the
    /// left-most Beast attack it.
    Fishbait { card: CardId, golden: bool },
}

/// A buff to Tavern minions that also reaches every minion that shows up in
/// the player's Tavern later.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TavernBuff {
    pub tribe: Option<Tribe>,
    pub attack: i32,
    pub health: i32,
    /// Ends when the player's turn does. Otherwise it lasts the game.
    pub this_turn: bool,
}

/// One buff per (tribe filter, duration): 10 tribes and no filter, times 2.
pub const MAX_TAVERN_BUFFS: usize = 22;

/// A Choose One minion or spell waiting for the player to pick one of its
/// two options.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PendingChooseOne {
    pub source: Option<u8>,
    pub golden: bool,
    pub options: [ChooseOption; 2],
    /// The spell being cast is a Tavern spell (see `PendingTarget::tavern`).
    pub tavern: bool,
}

/// One option of a "both effects combined" Choose One that still has to run,
/// after the option before it is done waiting on its own target.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct QueuedOption {
    pub source: Option<u8>,
    pub option: ChooseOption,
    /// Whether the Choose One spell was a Tavern spell.
    pub tavern: bool,
}

/// "Discard a card" (Mangled Bandit's Activate) waiting for the player to
/// pick the card in hand to throw away.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PendingDiscard {
    /// Board slot of the minion whose effect it is.
    pub source: u8,
    /// What the discard earns, run after the card is gone.
    pub reward: Effect,
    /// Whether the minion with the effect is golden (doubles the reward).
    pub golden: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DiscoverRequest {
    MinionTier(u8),
    /// A minion of exactly this tier, locked in hand for a turn (Search
    /// Through Time).
    SearchMinionTier(u8),
    SpellTier(u8),
    /// A Tavern spell of this tier or lower (Rodeo Performer).
    SpellMaxTier(u8),
    DeathrattleMinion(u8),
    BattlecryMinion(u8),
    TribeMinion(Tribe),
    /// A minion of the tribe, up to the tier, that dies if it is played in
    /// the turn it was Discovered (Tomb Turning).
    DoomedTribeMinion(Tribe),
    /// A Demon of Tier `max_tier` or lower; the pick damages the hero by its
    /// Tier, dealt by the friendly minion in board slot `source` (Imposing
    /// Percussionist).
    DemonMinion { max_tier: u8, source: u8 },
    /// A Choose One minion, any tribe, up to the tier; the pick has both
    /// effects combined (Fandral's Fortune).
    ChooseOneMinion,
}

/// Why the Tavern is filled. Only the player's own Refresh action (paid with
/// Gold, Health or free) is a Refresh for card purposes, so only it takes
/// queued Fodders. The automatic roll at the start of a turn and a card
/// effect that says "Refresh" (Snarky Shark) leave the queue alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RefreshKind {
    TurnStart,
    Explicit,
    Effect,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Player {
    pub health: i32,
    pub alive: bool,
    /// 1 to 8 once decided, 0 while still playing.
    pub placement: u8,
    /// Turn the player reached each tier (1 to 6), 0 if never reached.
    pub tier_turns: [u8; MAX_TIER as usize],
    /// Counts of (Beast, Mech, neutral) minions on the player's final board.
    pub final_tribes: [u8; TRIBES],
    pub tavern_buffs: ArrayVec<TavernBuff, MAX_TAVERN_BUFFS>,
    /// Damage the hero took last, what "rewind it" gives back.
    pub last_damage: i32,
    pub gold: u8,
    pub gold_next_turn: u8,
    pub extra_max_gold: u8,
    pub free_rolls: u8,
    pub tier: u8,
    pub upgrade_cost: u8,
    pub frozen: bool,
    pub lost_last_combat: bool,
    pub won_last_combat: bool,
    pub overconfidence_win: u8,
    pub overconfidence_tie: u8,
    pub winners_bread_buff: (i32, i32),
    pub next_turn_board_buffs: ArrayVec<(i32, i32, u8), 8>,
    pub boon_of_beetles: u8,
    pub brood_of_nozdormu: u8,
    pub upper_hand: u8,
    pub sharing_is_caring: u8,
    pub volumizer_bonus: (i32, i32),
    pub beetle_bonus: (i32, i32),
    /// How many times each "improve your future ..." effect has triggered
    /// this game (Tasty Lobster).
    pub improve: [u16; N_COUNTERS],
    /// Beast bonus until next turn (Goldrinn triggered in the shop). Every
    /// Beast on the board carries it in its stats and it comes off at the
    /// start of the next turn.
    pub beast_bonus: (i32, i32),
    /// Deathrattles triggered this game, which Falling Sky Golem reads.
    pub deathrattles_triggered: u32,
    /// Golden minions played from hand to the board this game (Maritime
    /// Extortionist, Hooktusk). A minion made Golden on the board isn't one.
    pub golden_played: u32,
    /// Gold spent this turn (Lovesick Balladist). Resets every turn.
    pub gold_spent_turn: u16,
    /// Elementals played from hand this turn (Conflagration). Resets every
    /// turn.
    pub elementals_played_turn: u16,
    /// What "after the Tavern is Refreshed this game" gives: the sum of every
    /// Waveling, En-Djinn Blazer and Easterly Winds so far. Each explicit
    /// Refresh gives the whole amount to one random Tavern minion.
    pub refresh_buff: (i32, i32),
    pub spell_bonus: (i32, i32),
    pub spell_discount: u8,
    pub last_spell: Option<CardId>,
    pub shop_spell: Option<CardId>,
    pub shop: ArrayVec<Minion, SHOP>,
    pub hand: ArrayVec<Minion, HAND>,
    pub board: ArrayVec<Minion, BOARD>,
    pub pending_choose_one: Option<PendingChooseOne>,
    pub pending_target: Option<PendingTarget>,
    pub pending_shop_target: Option<PendingShopTarget>,
    /// "Discard a card" waiting for the player's pick.
    pub pending_discard: Option<PendingDiscard>,
    /// Choose One cards played whose choice is still open: Turbo Hogrider
    /// reacts once the card is fully resolved (`resolve_hogrider`).
    pub hogrider_pending: u8,
    /// Options of a "both effects combined" Choose One that still have to
    /// run, after the one before them is done waiting on its target.
    pub choose_queue: ArrayVec<QueuedOption, 2>,
    /// Discover options waiting for a choice. Minion cards here are out of the pool.
    pub discover: ArrayVec<CardId, DISCOVER>,
    /// Queued Discover requests to offer once pending choices finish.
    pub discover_queue: ArrayVec<DiscoverRequest, 8>,
    /// The board slot of the Demon whose Discover makes the hero take damage
    /// equal to the chosen card's Tier (Imposing Percussionist), if any.
    pub discover_damage_from: Option<u8>,
    /// Whether the Discover now waiting locks its minion in hand for a turn
    /// (Search Through Time).
    pub discover_locked: bool,
    /// Whether the Discover now waiting marks its minion to die when played
    /// this turn (Tomb Turning).
    pub discover_doomed: bool,
    /// Your Undead's Attack and Health "this game, wherever they are". Every
    /// Undead you own carries it in its stats (board, hand, Tavern), and
    /// every Undead made later gets it.
    pub undead_bonus: (i32, i32),
    /// Eternal Knights that died this game, which every Knight reads.
    pub knight_deaths: u32,
    /// Upcoming Refreshes and how many Fodders each one gets, oldest first
    /// (Laboratory Assistant, Trapped Clapper, Twisted Wrathguard).
    pub fodder_refreshes: ArrayVec<u8, 64>,
    /// What one of this player's Blood Gems gives, raised by "your Blood Gems
    /// give an extra +X/+Y this game". Starts at +1/+1.
    pub gem_attack: i32,
    pub gem_health: i32,
    /// Blood Gem Barrage casts: every Refresh gives this much, as Blood
    /// Gems, to every minion in the Tavern (the turn-start roll included
    /// when the Tavern changed).
    pub barrage: (i32, i32),
    /// Whether the Discover now waiting marks its Choose One pick as having
    /// both effects (Fandral's Fortune).
    pub discover_both: bool,
    pub deity: Option<Deity>,
    pub discarded: u32,
    pub spells_cast: u32,
    pub next_link: u16,
    pub actions: u16,
    pub done: bool,
    pub opponent: Option<Opponent>,
    pub last_opponent: Option<Opponent>,
    pub seen: [Seen; PLAYERS],
}

impl Player {
    fn new(data: &Data) -> Player {
        Player {
            health: data.rules.starting_health,
            alive: true,
            placement: 0,
            tier_turns: [1, 0, 0, 0, 0, 0],
            final_tribes: [0; TRIBES],
            tavern_buffs: ArrayVec::new(),
            last_damage: 0,
            gold: 0,
            gold_next_turn: 0,
            extra_max_gold: 0,
            free_rolls: 0,
            tier: 1,
            upgrade_cost: data.rules.upgrade_cost(1),
            frozen: false,
            lost_last_combat: false,
            won_last_combat: false,
            overconfidence_win: 0,
            overconfidence_tie: 0,
            winners_bread_buff: (4, 6),
            next_turn_board_buffs: ArrayVec::new(),
            boon_of_beetles: 0,
            brood_of_nozdormu: 0,
            upper_hand: 0,
            sharing_is_caring: 0,
            volumizer_bonus: (0, 0),
            beetle_bonus: (0, 0),
            improve: [0; N_COUNTERS],
            beast_bonus: (0, 0),
            deathrattles_triggered: 0,
            golden_played: 0,
            gold_spent_turn: 0,
            elementals_played_turn: 0,
            refresh_buff: (0, 0),
            spell_bonus: (0, 0),
            spell_discount: 0,
            last_spell: None,
            shop_spell: None,
            shop: ArrayVec::new(),
            hand: ArrayVec::new(),
            board: ArrayVec::new(),
            pending_choose_one: None,
            pending_target: None,
            pending_shop_target: None,
            pending_discard: None,
            hogrider_pending: 0,
            choose_queue: ArrayVec::new(),
            discover: ArrayVec::new(),
            discover_queue: ArrayVec::new(),
            discover_damage_from: None,
            discover_locked: false,
            discover_doomed: false,
            discover_both: false,
            undead_bonus: (0, 0),
            knight_deaths: 0,
            fodder_refreshes: ArrayVec::new(),
            gem_attack: 1,
            gem_health: 1,
            barrage: (0, 0),
            deity: None,
            discarded: 0,
            spells_cast: 0,
            next_link: 1,
            actions: 0,
            done: false,
            opponent: None,
            last_opponent: None,
            seen: Default::default(),
        }
    }

    /// Cost of the spell currently in the shop, after Ominous Seer discounts.
    pub fn spell_cost(&self, cards: &CardDb) -> Option<u8> {
        let id = self.shop_spell?;
        Some(cards.get(id).cost.saturating_sub(self.spell_discount))
    }

    /// Highest discover tier of any Patient Scout currently on the board, or 0
    /// when none is present.
    pub fn patient_scout_tier(&self, cards: &CardDb) -> u8 {
        let mut best = 0u8;
        for m in &self.board {
            for (card, _) in m.parts() {
                if matches!(cards.get(card).on_sell, Some(OnSell::DiscoverByTier)) {
                    let tier = (1 + m.improvements as u8).min(MAX_TIER);
                    best = best.max(tier);
                }
            }
        }
        best
    }

    /// Counts per `TRIBES` slot (the ten tribes in `Tribe::EVERY` order, then
    /// neutral) of the minions currently on this player's board. A
    /// multi-tribe minion counts once in each of its tribes.
    pub fn board_tribes(&self, cards: &CardDb) -> [u8; TRIBES] {
        let mut out = [0u8; TRIBES];
        for m in &self.board {
            let t = cards.get(m.card).tribes;
            if t.is_neutral() {
                out[Tribe::EVERY.len()] += 1;
            }
            for (slot, tribe) in Tribe::EVERY.into_iter().enumerate() {
                if t.has(tribe) {
                    out[slot] += 1;
                }
            }
        }
        out
    }

    /// How many different Bonus Keywords the minions on the board have
    /// between them.
    pub fn distinct_bonus_keywords(&self, mask: u8) -> u32 {
        (self.board.iter().fold(0, |acc, m| acc | m.keywords.0) & mask).count_ones()
    }

    /// Queue `per` Fodders for each of the next `count` Refreshes. It stacks:
    /// a Refresh that already gets Fodders gets these on top.
    fn add_fodder_refreshes(&mut self, per: u8, count: u32) {
        for i in 0..count as usize {
            if !self.add_fodder_to_refresh(i, per) {
                break;
            }
        }
    }

    /// Add `per` Fodders to the Refresh `index` queues from now (0 is the
    /// next one), filling any gap with a Refresh that gets none. Returns
    /// false when the queue is full.
    fn add_fodder_to_refresh(&mut self, index: usize, per: u8) -> bool {
        if per == 0 {
            return true;
        }
        while self.fodder_refreshes.len() < index {
            if self.fodder_refreshes.try_push(0).is_err() {
                return false;
            }
        }
        match self.fodder_refreshes.get_mut(index) {
            Some(n) => {
                *n = n.saturating_add(per);
                true
            }
            None => self.fodder_refreshes.try_push(per).is_ok(),
        }
    }

    /// Take the Fodders the next Refresh gets, if any are queued.
    fn take_fodder_for_refresh(&mut self) -> u8 {
        if self.fodder_refreshes.is_empty() {
            0
        } else {
            self.fodder_refreshes.remove(0)
        }
    }

    /// Health Refreshes left this turn, from the uses on the Malchezaars on
    /// the board. Uses belong to the minion: a golden from a triple starts
    /// fresh, and sold copies take theirs along.
    pub fn health_refreshes_left(&self, cards: &CardDb) -> u8 {
        self.board
            .iter()
            .filter(|m| m.parts().any(|(c, _)| cards.get(c).health_refreshes > 0))
            .map(|m| m.uses_left)
            .fold(0u8, u8::saturating_add)
    }

    /// Spend one Health Refresh: the first Malchezaar left to right that
    /// still has one loses it.
    pub fn spend_health_refresh(&mut self, cards: &CardDb) {
        let Some(i) = (0..self.board.len()).find(|&i| {
            self.board[i].uses_left > 0
                && self.board[i].parts().any(|(c, _)| cards.get(c).health_refreshes > 0)
        }) else { return };
        self.board[i].uses_left -= 1;
    }

    /// Whether a minion on the board undoes hero damage ("rewind it").
    pub fn can_rewind(&self, cards: &CardDb) -> bool {
        self.board.iter().any(|m| {
            m.parts().any(|(card, _)| cards.get(card).after_hero_damage.contains(&Effect::RewindDamage))
        })
    }

    /// Whether a Health cost can be paid: it can't kill the hero, unless a
    /// rewind on the board gives the Health straight back.
    pub fn can_pay_health(&self, cost: i32, cards: &CardDb) -> bool {
        self.health - cost >= 1 || self.can_rewind(cards)
    }

    /// Whether the next Refresh would be paid in Health.
    pub fn can_refresh_with_health(&self, cards: &CardDb, rules: &Rules) -> bool {
        self.health_refreshes_left(cards) > 0 && self.can_pay_health(rules.health_refresh_cost, cards)
    }

    /// Add a Tavern buff: it goes onto the minions in the Tavern now and is
    /// remembered for the ones that show up later.
    pub fn add_tavern_buff(&mut self, cards: &CardDb, b: TavernBuff) {
        match self
            .tavern_buffs
            .iter_mut()
            .find(|o| o.tribe == b.tribe && o.this_turn == b.this_turn)
        {
            Some(o) => {
                o.attack += b.attack;
                o.health += b.health;
            }
            None => self.tavern_buffs.push(b),
        }
        for m in &mut self.shop {
            if !cards.get(m.card).is_spell() && cards.get(m.card).is(b.tribe) {
                buff(m, b.attack, b.health);
            }
        }
    }

    /// Put every Tavern buff that applies on a minion that just showed up in
    /// the Tavern.
    pub fn apply_tavern_buffs(&self, cards: &CardDb, m: &mut Minion) {
        if cards.get(m.card).is_spell() {
            return;
        }
        for b in &self.tavern_buffs {
            if cards.get(m.card).is(b.tribe) {
                buff(m, b.attack, b.health);
            }
        }
    }

    /// The player's turn is over: "this turn" Tavern buffs come off the
    /// minions still in the Tavern. Bought minions keep what they had.
    pub fn end_turn_tavern_buffs(&mut self, cards: &CardDb) {
        for b in self.tavern_buffs.iter().filter(|b| b.this_turn) {
            for m in &mut self.shop {
                if !cards.get(m.card).is_spell() && cards.get(m.card).is(b.tribe) {
                    buff(m, -b.attack, -b.health);
                }
            }
        }
        self.tavern_buffs.retain(|b| !b.this_turn);
    }

    /// Create a copy of `id` with this player's current Volumizer and Beetle
    /// bonuses applied.
    pub fn make_card(&self, cards: &CardDb, id: CardId, golden: bool, pool_copies: u8) -> Minion {
        let mut m = cards.make(id, golden);
        m.pool_copies = pool_copies;
        if m.volumizers > 0 {
            m.attack += self.volumizer_bonus.0 * m.volumizers as i32;
            m.health += self.volumizer_bonus.1 * m.volumizers as i32;
        }
        let c = cards.get(id);
        if c.is_beetle {
            m.attack += self.beetle_bonus.0;
            m.health += self.beetle_bonus.1;
        }
        if let Some((a, h)) = c.buff_per_deathrattle {
            let n = self.deathrattles_triggered as i32 * m.mul();
            m.attack += a * n;
            m.health += h * n;
        }
        if let Some((a, h)) = c.buff_per_golden_played {
            let n = self.golden_played as i32 * m.mul();
            m.attack += a * n;
            m.health += h * n;
        }
        if c.tribes.has(Tribe::Undead) && !c.fixed_stats {
            m.attack += self.undead_bonus.0;
            m.health += self.undead_bonus.1;
        }
        if let Some((a, h)) = c.buff_per_kind_death {
            let n = self.knight_deaths as i32 * m.mul();
            m.attack += a * n;
            m.health += h * n;
        }
        m
    }

    /// The player's Tavern spell stat bonus: permanent gains plus board
    /// auras (Enchanted Sentinel, including when Magnetized into a host,
    /// and Humon'gozz).
    pub fn total_spell_bonus(&self, cards: &CardDb) -> (i32, i32) {
        let (mut da, mut dh) = self.spell_bonus;
        for m in &self.board {
            for (card, golden) in m.parts() {
                if let Some((a, h)) = cards.get(card).spell_buff {
                    let mul = if golden { 2 } else { 1 };
                    da += a * mul;
                    dh += h * mul;
                }
            }
        }
        (da, dh)
    }

    pub fn counters(&self, cards: &CardDb, lobby: Tribes) -> Counters {
        let (spell_attack, spell_health) = self.total_spell_bonus(cards);
        Counters {
            health: Some(self.health),
            beetle_attack: self.beetle_bonus.0,
            beetle_health: self.beetle_bonus.1,
            improve: self.improve,
            beast_attack: self.beast_bonus.0,
            beast_health: self.beast_bonus.1,
            tier: self.tier,
            boon_of_beetles: self.boon_of_beetles,
            brood_of_nozdormu: self.brood_of_nozdormu,
            upper_hand: self.upper_hand,
            sharing_is_caring: self.sharing_is_caring,
            spell_attack,
            spell_health,
            undead_attack: self.undead_bonus.0,
            undead_health: self.undead_bonus.1,
            knight_deaths: self.knight_deaths,
            gem_attack: self.gem_attack,
            gem_health: self.gem_health,
            deity: self.deity,
            spells_cast: self.spells_cast,
            lobby,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Ghost {
    pub player: u8,
    pub board: ArrayVec<Minion, BOARD>,
    pub tier: u8,
    pub counters: Counters,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IllegalAction;

#[derive(Clone, Debug)]
pub struct Game {
    pub data: Arc<Data>,
    pub rng: Rng,
    /// The five tribes of this game's lobby (DESIGN.md "The lobby's
    /// tribes"): the pool and every draw are filtered to them.
    pub lobby: Tribes,
    pub turn: u32,
    pub players: [Player; PLAYERS],
    /// Copies left of each pool card, indexed by card `pool_slot`.
    pub pool: Vec<u8>,
    /// Pool copies permanently consumed into hosts via Magnetize, indexed by
    /// card `pool_slot`.
    pub consumed: Vec<u32>,
    pub ghost: Option<Ghost>,
    pub over: bool,
    /// Set only by `Game::new_sandbox`: one player, a rigged Tavern and a
    /// fight against a copy of the player's own board. None in a normal game.
    pub sandbox: Option<Sandbox>,
    order: ArrayVec<u8, PLAYERS>,
    cursor: usize,
}

/// Settings and results of a sandbox game (see `Game::new_sandbox`).
#[derive(Clone, Debug)]
pub struct Sandbox {
    /// Free Refreshes the player has at the start of every turn.
    pub free_rolls: u8,
    /// The latest mirror fight, for whoever records the game.
    pub last_fight: Option<SandboxFight>,
}

/// One mirror fight in a sandbox game: what fought, who won and what the
/// player's side carried over.
#[derive(Clone, Debug)]
pub struct SandboxFight {
    pub turn: u32,
    pub tier: u8,
    pub health: i32,
    /// The board and the hand as they went into the fight (after the
    /// end-of-turn effects).
    pub board: ArrayVec<Minion, BOARD>,
    pub hand: ArrayVec<Minion, HAND>,
    /// Some(0) the player won, Some(1) the mirror won, None a tie.
    pub winner: Option<usize>,
    /// What the player's side carried over, as the game applied it.
    pub side: SideOutcome,
}

/// Draw one card from the pool, weighted by copies left, among cards of
/// tier `lo..=hi` that satisfy `pred` and aren't in `exclude`. When the shared
/// Volumizer slot is drawn, picks a random Volumizer color not in `exclude`.
#[allow(clippy::too_many_arguments)]
fn draw_filtered(
    pool: &mut [u8],
    cards: &CardDb,
    rng: &mut Rng,
    lo: u8,
    hi: u8,
    exclude: &[CardId],
    lobby: Tribes,
    pred: impl Fn(&Card) -> bool,
) -> Option<CardId> {
    // A card with no copies left (or one that is not a pool slot, which never
    // holds copies) adds nothing, so the cheap checks run first.
    let ok = |c: usize| {
        if pool[c] == 0 {
            return false;
        }
        let t = cards.tiers[c];
        if !(lo <= t && t <= hi) {
            return false;
        }
        let card = &cards.cards[c];
        if card.pool_slot as usize != c || !card.lobby_ok(lobby) || !pred(card) {
            return false;
        }
        if card.is_volumizer {
            cards.volumizers.iter().any(|v| !exclude.contains(v))
        } else {
            exclude.is_empty() || !exclude.contains(&(c as CardId))
        }
    };
    let mut eligible: ArrayVec<(u8, u8), MAX_POOL_CARDS> = ArrayVec::new();
    let mut total = 0u32;
    // Ids in ascending order, so the walk below sees cards in the same order
    // as a scan of the whole pool; `ok` still checks everything.
    for &c in &cards.ids_up_to_tier[(hi as usize).min(crate::CARD_MAX_TIER as usize)] {
        let c = c as usize;
        if ok(c) {
            total += pool[c] as u32;
            eligible.push((c as u8, pool[c]));
        }
    }
    if total == 0 {
        return None;
    }
    let mut r = rng.below(total);
    for &(c, n) in &eligible {
        let c = c as usize;
        let n = n as u32;
        if r < n {
            pool[c] -= 1;
            if cards.cards[c].is_volumizer {
                let mut avail: ArrayVec<CardId, 3> = ArrayVec::new();
                for &v in &cards.volumizers {
                    if !exclude.contains(&v) {
                        let _ = avail.try_push(v);
                    }
                }
                let k = rng.below(avail.len() as u32) as usize;
                return Some(avail[k]);
            }
            return Some(c as CardId);
        }
        r -= n;
    }
    unreachable!()
}

/// Draw one Golden minion from the pool: a card of tier `lo..=hi` (with a
/// type when `typed`), weighted by copies left, taking the three copies a
/// Golden minion stands for (fewer when the pool has fewer; cards with all
/// three left are preferred). The lobby filters it.
fn draw_golden(
    pool: &mut [u8],
    cards: &CardDb,
    rng: &mut Rng,
    lo: u8,
    hi: u8,
    typed: bool,
    lobby: Tribes,
) -> Option<(CardId, u8)> {
    for min in [3u8, 1] {
        let ok = |c: usize| {
            let card = &cards.cards[c];
            pool[c] >= min
                && card.pool_slot as usize == c
                && lo <= card.tier
                && card.tier <= hi
                && (!typed || !card.tribes.is_neutral())
                && card.lobby_ok(lobby)
        };
        let total: u32 = (0..cards.pool_cards).filter(|&c| ok(c)).map(|c| pool[c] as u32).sum();
        if total == 0 {
            continue;
        }
        let mut r = rng.below(total);
        for c in 0..cards.pool_cards {
            if !ok(c) {
                continue;
            }
            let n = pool[c] as u32;
            if r < n {
                let take = pool[c].min(3);
                pool[c] -= take;
                let id = if cards.cards[c].is_volumizer {
                    cards.volumizers[rng.below(cards.volumizers.len() as u32) as usize]
                } else {
                    c as CardId
                };
                return Some((id, take));
            }
            r -= n;
        }
    }
    None
}

/// Make a minion Golden where it stands: its stats go to the golden card's
/// (double the printed ones), keeping every buff.
fn golden_up(cards: &CardDb, m: &mut Minion) {
    if m.golden {
        return;
    }
    let base = cards.get(m.card);
    let (ga, gh) = base.golden_stats.unwrap_or((base.attack * 2, base.health * 2));
    m.golden = true;
    m.attack += ga - base.attack;
    m.health += gh - base.health;
}

fn give_back(pool: &mut [u8], cards: &CardDb, m: &Minion) {
    let c = cards.get(m.card);
    if m.pool_copies > 0 && !c.token && !c.is_spell() {
        let slot = c.pool_slot as usize;
        pool[slot] += m.pool_copies;
    }
}

fn buff(m: &mut Minion, attack: i32, health: i32) {
    m.attack += attack;
    m.health += health;
}

/// What turns Golden in a triple: the card, how many of its own copies go in,
/// and a stand-in card (Elemental of Surprise) with how many of those.
type TriplePlan = (CardId, usize, Option<(CardId, usize)>);

impl Game {
    pub fn new(data: Arc<Data>, seed: u64) -> Game {
        let mut g = Game::unstarted(data, seed, None);
        g.start_turn();
        g
    }

    /// A normal game with a fixed lobby: for tests and the sandbox, which
    /// need known tribes. Games that roll one pick exactly 5 of the 10
    /// (DESIGN.md); a test may pass more to set up its cards.
    pub fn new_with_lobby(data: Arc<Data>, seed: u64, lobby: Tribes) -> Game {
        let mut g = Game::unstarted(data, seed, Some(lobby));
        g.start_turn();
        g
    }

    /// A one-player practice game for the sandbox: only player 0 plays, no
    /// pairing and no opponents. With a `lobby`, the game's tribes are fixed
    /// to it (the sandbox's --tribe); None rolls the normal 5. The player
    /// gets `free_rolls` free Refreshes each turn, and each turn ends with a
    /// fight against a copy of the player's own board that never damages the
    /// hero. Shop-phase damage still counts and ends the game at 0 Health.
    /// Normal games never set any of this.
    pub fn new_sandbox(data: Arc<Data>, seed: u64, lobby: Option<Tribes>, free_rolls: u8) -> Game {
        let mut g = Game::unstarted(data, seed, lobby);
        for p in 1..PLAYERS {
            g.players[p].alive = false;
            g.players[p].placement = p as u8 + 1;
        }
        g.sandbox = Some(Sandbox { free_rolls, last_fight: None });
        g.start_turn();
        g
    }

    fn unstarted(data: Arc<Data>, seed: u64, fixed: Option<Tribes>) -> Game {
        let mut rng = Rng::new(seed);
        let lobby = fixed.unwrap_or_else(|| Tribes::random_lobby(5, &mut rng));
        let pool = (0..data.cards.pool_cards)
            .map(|c| {
                let card = &data.cards.cards[c];
                if card.pool_slot as usize == c && card.lobby_ok(lobby) {
                    data.rules.copies(card.tier)
                } else {
                    0
                }
            })
            .collect();
        let consumed = vec![0u32; data.cards.pool_cards];
        let mut players: [Player; PLAYERS] = std::array::from_fn(|_| Player::new(&data));
        let gods: ArrayVec<CardId, 2> = data.cards.cards.iter().enumerate()
            .filter(|(_, c)| c.deity.is_some())
            .map(|(id, _)| id as CardId)
            .collect();
        // The Deity only exists in lobbies with Aberrations (DESIGN.md).
        let god = (lobby.has(Tribe::Aberration) && !gods.is_empty())
            .then(|| gods[rng.below(gods.len() as u32) as usize]);
        if let Some(card) = god {
            for pl in &mut players {
                pl.deity = Some(Deity { card, attack: 1, health: 1, golden: false });
            }
        }
        Game {
            data,
            rng,
            lobby,
            turn: 0,
            players,
            pool,
            consumed,
            ghost: None,
            over: false,
            sandbox: None,
            order: ArrayVec::new(),
            cursor: 0,
        }
    }

    /// Whose action is next, None once the game is over.
    pub fn current_player(&self) -> Option<usize> {
        if self.over { None } else { Some(self.order[self.cursor] as usize) }
    }

    pub fn alive(&self) -> usize {
        self.players.iter().filter(|p| p.alive).count()
    }

    pub fn placements(&self) -> [u8; PLAYERS] {
        std::array::from_fn(|i| self.players[i].placement)
    }

    /// Whether the current player may take `a`.
    pub fn is_legal(&self, a: Action) -> bool {
        let Some(p) = self.current_player() else { return false };
        let pl = &self.players[p];
        let r = &self.data.rules;
        if let Some(pt) = pl.pending_target {
            if pt.kind.takes_shop() {
                return match a {
                    Action::Target(s) => self.valid_target(p, &pt, s as usize),
                    Action::TargetShop(s) => self.valid_shop_target(p, &pt, s as usize),
                    _ => false,
                };
            }
            return matches!(a, Action::Target(s) if self.valid_target(p, &pt, s as usize));
        }
        if pl.pending_shop_target.is_some() {
            return matches!(a, Action::TargetShop(s) if (s as usize) < pl.shop.len());
        }
        if pl.pending_discard.is_some() {
            return matches!(a, Action::Discard(s) if self.valid_discard(p, s as usize));
        }
        if let Some(pco) = pl.pending_choose_one {
            return matches!(a, Action::Choose(k) if k < 2 && self.can_choose_option(p, pco.options[k as usize]));
        }
        if !pl.discover.is_empty() {
            return matches!(a, Action::Choose(k) if (k as usize) < pl.discover.len());
        }
        match a {
            Action::Buy(s) => {
                let s = s as usize;
                if s >= pl.shop.len() || pl.hand.is_full() {
                    return false;
                }
                let card = self.data.cards.get(pl.shop[s].card);
                if card.is_spell() {
                    if card.pay_health {
                        pl.can_pay_health(card.cost as i32, &self.data.cards)
                    } else {
                        let cost = card.cost.saturating_sub(pl.spell_discount);
                        pl.gold >= cost
                    }
                } else {
                    pl.gold >= r.buy_cost
                }
            }
            Action::BuySpell => {
                let Some(id) = pl.shop_spell else { return false };
                let card = self.data.cards.get(id);
                if pl.hand.is_full() {
                    return false;
                }
                if card.pay_health {
                    pl.can_pay_health(card.cost as i32, &self.data.cards)
                } else {
                    pl.gold >= card.cost.saturating_sub(pl.spell_discount)
                }
            }
            Action::Sell(s) => (s as usize) < pl.board.len(),
            Action::Activate(s) => {
                (s as usize) < pl.board.len()
                    && !pl.board[s as usize].activated
                    && self.activate_of(p, s as usize).is_some_and(|(act, _)| {
                        pl.gold >= act.cost
                            && (!act.discard || self.has_discardable(p))
                            && (!act.effect.needs_shop_slot() || self.shop_has_minion(p))
                            && (!act.effect.needs_any_minion()
                                || pl.board.len() > 1
                                || self.shop_has_minion(p))
                            && (!act.effect.needs_shop_minion() || !pl.hand.is_full())
                            && act
                                .effect
                                .needs_different_of_tribe()
                                .is_none_or(|t| self.has_different_of_tribe(p, s as usize, t))
                            && (!act.effect.needs_rally_target()
                                || pl.board.iter().enumerate().any(|(i, m)| {
                                    i != s as usize
                                        && m.parts().any(|(c, _)| self.data.cards.get(c).rally.is_some())
                                }))
                            && (!act.effect.needs_battlecry_target()
                                || pl.board.iter().enumerate().any(|(i, m)| {
                                    i != s as usize
                                        && m.parts().any(|(c, _)| {
                                            let card = self.data.cards.get(c);
                                            card.battlecry.is_some() || card.choose_one.is_some()
                                        })
                                }))
                    })
            }
            Action::Play { hand, pos } => {
                let h = hand as usize;
                let pos = pos as usize;
                if h >= pl.hand.len() {
                    return false;
                }
                let card_id = pl.hand[h].card;
                if pl.hand[h].locked_turns > 0 {
                    false
                } else if self.data.cards.get(card_id).is_spell() {
                    pos == 0 && self.can_cast_spell(p, card_id)
                } else if pos > pl.board.len() {
                    false
                } else if pl.hand[h].keywords.has(Keywords::MAGNETIC)
                    && pos < pl.board.len()
                    && self.can_magnetize(card_id, pl.board[pos].card)
                {
                    true
                } else {
                    !pl.board.is_full()
                }
            }
            Action::Target(_) | Action::TargetShop(_) | Action::Choose(_) | Action::Discard(_) => {
                false
            }
            Action::Move { from, to } => {
                (from as usize) < pl.board.len() && (to as usize) < pl.board.len() && from != to
            }
            Action::Roll => {
                pl.free_rolls > 0
                    || pl.can_refresh_with_health(&self.data.cards, r)
                    || pl.gold >= r.roll_cost
            }
            Action::Freeze | Action::EndTurn => true,
            Action::Level => pl.tier < MAX_TIER && pl.gold >= pl.upgrade_cost,
        }
    }

    /// Fills `mask` with `is_legal` for every action. Only the index ranges
    /// that can be legal in the current state are asked (a slot past the end
    /// of the board, hand or shop never is), which gives the same mask as
    /// asking all of them.
    pub fn legal_mask(&self, mask: &mut [bool; N_ACTIONS]) {
        mask.fill(false);
        let Some(p) = self.current_player() else { return };
        let pl = &self.players[p];
        let mut ask = |a: Action| mask[a.index()] = self.is_legal(a);
        if let Some(pt) = pl.pending_target {
            for s in 0..BOARD as u8 {
                ask(Action::Target(s));
            }
            if pt.kind.takes_shop() {
                for s in 0..SHOP as u8 {
                    ask(Action::TargetShop(s));
                }
            }
        } else if pl.pending_shop_target.is_some() {
            for s in 0..pl.shop.len() as u8 {
                ask(Action::TargetShop(s));
            }
        } else if pl.pending_discard.is_some() {
            for s in 0..pl.hand.len() as u8 {
                ask(Action::Discard(s));
            }
        } else if pl.pending_choose_one.is_some() {
            for k in 0..2 {
                ask(Action::Choose(k));
            }
        } else if !pl.discover.is_empty() {
            for k in 0..pl.discover.len() as u8 {
                ask(Action::Choose(k));
            }
        } else {
            for s in 0..pl.shop.len() as u8 {
                ask(Action::Buy(s));
            }
            ask(Action::BuySpell);
            for s in 0..pl.board.len() as u8 {
                ask(Action::Sell(s));
                ask(Action::Activate(s));
            }
            for hand in 0..pl.hand.len() as u8 {
                for pos in 0..=BOARD as u8 {
                    ask(Action::Play { hand, pos });
                }
            }
            for from in 0..pl.board.len() as u8 {
                for to in 0..pl.board.len() as u8 {
                    ask(Action::Move { from, to });
                }
            }
            ask(Action::Roll);
            ask(Action::Freeze);
            ask(Action::Level);
            ask(Action::EndTurn);
        }
    }

    /// Whether the Magnetic card `hand_card` can merge into the board minion
    /// `board_card`: a Mech, or an Undead for a card that says so
    /// (Prosthetic Hand).
    fn can_magnetize(&self, hand_card: CardId, board_card: CardId) -> bool {
        let cards = &self.data.cards;
        let t = cards.get(board_card).tribes;
        t.has(Tribe::Mech) || (cards.get(hand_card).magnetize_to_undead && t.has(Tribe::Undead))
    }

    /// Whether the board minion in slot `s` has another friendly minion of
    /// `tribe` that is not a copy of it to point at.
    fn has_different_of_tribe(&self, p: usize, s: usize, tribe: Tribe) -> bool {
        let board = &self.players[p].board;
        let cards = &self.data.cards;
        board.iter().enumerate().any(|(i, m)| {
            i != s && m.card != board[s].card && cards.get(m.card).tribes.has(tribe)
        })
    }

    /// Whether the card in hand slot `s` may be discarded: any playable card
    /// (a Blood Gem included), never an unplayable one (the Lockbox) or a
    /// minion locked in hand.
    fn valid_discard(&self, p: usize, s: usize) -> bool {
        let Some(m) = self.players[p].hand.get(s) else { return false };
        if m.locked_turns > 0 {
            return false;
        }
        !matches!(self.data.cards.get(m.card).spell_effect, Some(SpellEffect::Lockbox(_)))
    }

    /// Whether the player holds any card a discard could take.
    fn has_discardable(&self, p: usize) -> bool {
        (0..self.players[p].hand.len()).any(|s| self.valid_discard(p, s))
    }

    /// The Activate ability of the board minion in slot `s` (the first part
    /// of it that has one) and whether that part is golden.
    fn activate_of(&self, p: usize, s: usize) -> Option<(crate::cards::Activate, bool)> {
        self.players[p].board[s]
            .parts()
            .find_map(|(card, golden)| self.data.cards.get(card).activate.map(|a| (a, golden)))
    }

    /// Whether the Tavern holds a minion (not a spell) for an effect that
    /// needs one.
    fn shop_has_minion(&self, p: usize) -> bool {
        self.players[p]
            .shop
            .iter()
            .any(|m| !self.data.cards.get(m.card).is_spell())
    }

    fn spell_target_ok(&self, st: SpellTarget, m: &Minion) -> bool {
        let c = self.data.cards.get(m.card);
        if c.is_spell() {
            return false;
        }
        match st {
            SpellTarget::Buff { .. }
            | SpellTarget::BuffPerBonusKeyword { .. }
            | SpellTarget::SetStats { .. }
            | SpellTarget::TrickyTrousers { .. }
            | SpellTarget::RobustEvolution => true,
            SpellTarget::BuffSameTribe { .. }
            | SpellTarget::SameTribeMinion
            | SpellTarget::TavernBuffType { .. }
            | SpellTarget::RefreshWithType => !c.tribes.is_neutral(),
            SpellTarget::Consume { tribe, .. } => c.is(tribe),
            SpellTarget::MakeGolden { max_tier } => !m.golden && c.tier <= max_tier,
            SpellTarget::Devourer => true,
            SpellTarget::HalfOfHighestTavern { tribe } => c.tribes.has(tribe),
            SpellTarget::BuffPerTribePlayed { .. } => true,
            SpellTarget::Butchering { .. } => c.tribes.has(Tribe::Undead),
            SpellTarget::Confiscation { .. } => true,
        }
    }

    fn valid_spell_target(&self, p: usize, st: SpellTarget, s: usize) -> bool {
        let b = &self.players[p].board;
        s < b.len() && self.spell_target_ok(st, &b[s])
    }

    /// Whether the pending target may be taken on the Tavern minion in
    /// shop slot `s`.
    fn valid_shop_target(&self, p: usize, pt: &PendingTarget, s: usize) -> bool {
        let pl = &self.players[p];
        if s >= pl.shop.len() || self.data.cards.get(pl.shop[s].card).is_spell() {
            return false;
        }
        match pt.kind {
            TargetKind::AnyBuff { .. } | TargetKind::AnySetStats { .. } => true,
            TargetKind::Spell(st) => st.can_target_shop() && self.spell_target_ok(st, &pl.shop[s]),
            _ => false,
        }
    }

    fn can_choose_option(&self, p: usize, opt: ChooseOption) -> bool {
        match opt {
            ChooseOption::SpellTarget(st) => {
                let pl = &self.players[p];
                (0..pl.board.len()).any(|s| self.valid_spell_target(p, st, s))
                    || (st.can_target_shop()
                        && (0..pl.shop.len())
                            .any(|s| self.spell_target_ok(st, &pl.shop[s])))
            }
            ChooseOption::MinionTarget { .. }
            | ChooseOption::SpellBonus { .. }
            | ChooseOption::SpellEffect(_) => true,
        }
    }

    pub fn can_cast_spell(&self, p: usize, card_id: CardId) -> bool {
        let c = self.data.cards.get(card_id);
        if let Some(st) = c.spell_target {
            let pl = &self.players[p];
            (0..pl.board.len()).any(|s| self.valid_spell_target(p, st, s))
                || (st.can_target_shop()
                    && (0..pl.shop.len()).any(|s| self.spell_target_ok(st, &pl.shop[s])))
        } else if let Some(opts) = c.choose_one {
            opts.iter().any(|&opt| self.can_choose_option(p, opt))
        } else {
            // Playing the spell frees its own hand slot, so these only need
            // something to steal; they are never cast from the Tavern.
            match c.spell_effect {
                Some(SpellEffect::StealRandomTavern) => self.shop_has_minion(p),
                // A Lockbox is unplayable.
                Some(SpellEffect::Lockbox(_)) => false,
                Some(_) => true,
                None => false,
            }
        }
    }

    fn valid_target(&self, p: usize, pt: &PendingTarget, s: usize) -> bool {
        let b = &self.players[p].board;
        if s >= b.len() {
            return false;
        }
        if let Some(src) = pt.source
            && s == src as usize
        {
            return false;
        }
        match pt.kind {
            TargetKind::MinionBuff { tribe, .. } | TargetKind::Consume { tribe, .. } => {
                self.data.cards.get(b[s].card).is(tribe)
            }
            TargetKind::AnyBuff { .. } | TargetKind::AnySetStats { .. } => true,
            TargetKind::Spell(st) => self.valid_spell_target(p, st, s),
            TargetKind::MakeGolden { tribe, max_tier } => {
                let c = self.data.cards.get(b[s].card);
                !b[s].golden && c.tier <= max_tier && c.is(tribe)
            }
            TargetKind::DestroyDiscover { tribe, .. } | TargetKind::DestroyCopy { tribe, .. } => {
                self.data.cards.get(b[s].card).tribes.has(tribe)
            }
            TargetKind::RebornDestroy { tribe, .. } => {
                let host = pt.source.map(|src| b[src as usize].card);
                self.data.cards.get(b[s].card).tribes.has(tribe) && Some(b[s].card) != host
            }
            TargetKind::TriggerRally => {
                b[s].parts().any(|(c, _)| self.data.cards.get(c).rally.is_some())
            }
            TargetKind::TriggerBattlecry => b[s]
                .parts()
                .any(|(c, _)| {
                    let card = self.data.cards.get(c);
                    card.battlecry.is_some() || card.choose_one.is_some()
                }),
        }
    }

    /// The Attack and Health of the highest-Health minion in the player's
    /// Tavern, a random one of them on a Health tie (no random number is
    /// drawn without a tie). None with no minion there.
    fn highest_tavern_stats(&mut self, p: usize) -> Option<(i32, i32)> {
        let cards = &self.data.cards;
        let mut best: ArrayVec<(i32, i32), { crate::SHOP }> = ArrayVec::new();
        for m in &self.players[p].shop {
            if cards.get(m.card).is_spell() {
                continue;
            }
            if best.first().is_none_or(|&(_, h)| m.health > h) {
                best.clear();
            }
            if best.first().is_none_or(|&(_, h)| m.health == h) {
                best.push((m.attack, m.health));
            }
        }
        match best.len() {
            0 => None,
            1 => Some(best[0]),
            n => Some(best[self.rng.below(n as u32) as usize]),
        }
    }

    /// Take an action for the current player.
    pub fn step(&mut self, a: Action) -> Result<(), IllegalAction> {
        if !self.is_legal(a) {
            return Err(IllegalAction);
        }
        let p = self.current_player().unwrap();
        self.apply(p, a);
        self.resolve_doomed(p);
        self.resolve_hogrider(p);
        // A hero knocked out mid-action may still collect cards from effects
        // that were running when it died.
        for q in 0..PLAYERS {
            self.settle_dead(q);
        }
        if self.over {
            #[cfg(debug_assertions)]
            self.check_invariants().unwrap();
            return Ok(());
        }
        // A hero knocked out by its own effect damage takes no more actions.
        if self.players[p].alive {
            self.check_attack_thresholds(p);
            let pl = &mut self.players[p];
            pl.actions += 1;
            if a == Action::EndTurn || pl.actions >= self.data.rules.max_actions_per_turn {
                self.finish_shop(p);
            }
        }
        self.advance();
        #[cfg(debug_assertions)]
        self.check_invariants().unwrap();
        Ok(())
    }

    fn apply(&mut self, p: usize, a: Action) {
        match a {
            Action::Buy(s) => {
                let slot = s as usize;
                let id = self.players[p].shop[slot].card;
                let c = self.data.cards.get(id);
                if c.is_spell() {
                    // A Saloon's Finest slot holds a spell: it is bought at
                    // its own cost, like the spell slot.
                    let (pay_health, cost) = (c.pay_health, c.cost);
                    let m = self.players[p].shop.remove(slot);
                    self.add_to_hand(p, m);
                    self.pay_for_spell(p, pay_health, cost);
                    self.magicfin_teach(p, id);
                    self.fire_event(p, EventKind::CardsBought, 1);
                } else {
                    let buy_cost = self.data.rules.buy_cost;
                    let pl = &mut self.players[p];
                    let m = pl.shop.remove(slot);
                    pl.gold -= buy_cost;
                    self.add_to_hand(p, m);
                    self.after_buy_effects(p);
                    self.check_triple(p, m.card);
                    self.fire_event(p, EventKind::GoldSpent, buy_cost as u32);
                    self.fire_event(p, EventKind::CardsBought, 1);
                }
            }
            Action::BuySpell => {
                let id = self.players[p].shop_spell.take().unwrap();
                let c = self.data.cards.get(id);
                let (pay_health, cost) = (c.pay_health, c.cost);
                let pl = &mut self.players[p];
                let m = pl.make_card(&self.data.cards, id, false, 0);
                self.add_to_hand(p, m);
                self.pay_for_spell(p, pay_health, cost);
                self.magicfin_teach(p, id);
                self.fire_event(p, EventKind::CardsBought, 1);
            }
            Action::Sell(s) => {
                self.sell_minion(p, s as usize);
            }
            Action::Activate(s) => {
                let (act, golden) = self.activate_of(p, s as usize).unwrap();
                self.players[p].gold -= act.cost;
                self.players[p].board[s as usize].activated = true;
                if act.discard {
                    // The reward runs once the player has picked the card.
                    self.players[p].pending_discard = Some(PendingDiscard {
                        source: s,
                        reward: act.effect,
                        golden,
                    });
                } else {
                    let mut pos = s as usize;
                    self.run_effect(p, &mut pos, act.effect, golden, 1);
                    self.maybe_start_discover(p);
                }
                self.fire_event(p, EventKind::GoldSpent, act.cost as u32);
            }
            Action::Play { hand, pos } => {
                let m = self.players[p].hand.remove(hand as usize);
                if m.linked != 0 {
                    let group = m.linked & 0x3fff;
                    if m.linked & 0xc000 == 0x8000 {
                        for other in &mut self.players[p].hand {
                            if other.linked & 0x3fff == group {
                                other.linked |= 0x4000;
                            }
                        }
                    } else {
                        while let Some(slot) = self.players[p].hand.iter()
                            .position(|other| other.linked & 0x3fff == group)
                        {
                            self.discard_card(p, slot);
                        }
                    }
                }
                let card = self.data.cards.get(m.card);
                if card.is_spell() {
                    let choose_one = card.choose_one.is_some();
                    self.cast_spell(p, m);
                    if choose_one {
                        // Turbo Hogrider reacts once the card is resolved.
                        self.players[p].hogrider_pending += 1;
                    }
                    return;
                }
                let pos = pos as usize;
                let is_merge = m.keywords.has(Keywords::MAGNETIC)
                    && pos < self.players[p].board.len()
                    && self.can_magnetize(m.card, self.players[p].board[pos].card);
                let mut choose_one_play = false;
                if is_merge {
                    self.magnetize_to(p, pos, m, true);
                    if m.golden && !m.no_reward {
                        self.give_triple_reward(p);
                    }
                } else {
                    let mut m = m;
                    if self.data.cards.get(m.card).tribes.has(Tribe::Beast) {
                        let (ba, bh) = self.players[p].beast_bonus;
                        m.attack += ba;
                        m.health += bh;
                    }
                    // A minion landing on the board starts with its limited
                    // uses for the turn (a Trailblazer's combines, a
                    // Mycologist's teachings, Malchezaar's Health Refreshes).
                    m.uses_left = m.turn_uses(&self.data.cards);
                    self.players[p].board.insert(pos, m);
                    let t = self.data.cards.get(m.card).tribes;
                    // The Triple Reward comes before anything the Battlecry
                    // adds to the hand (and is lost on a full hand).
                    if m.golden && !m.no_reward {
                        self.give_triple_reward(p);
                    }
                    choose_one_play = self.data.cards.get(m.card).choose_one.is_some();
                    if t.has(Tribe::Elemental) {
                        self.players[p].elementals_played_turn =
                            self.players[p].elementals_played_turn.saturating_add(1);
                        self.give_play_shield(p, pos);
                    }
                    self.fire_after_play_tribe(p, t, pos);
                    if t.has(Tribe::Elemental) {
                        self.fire_event_skipping(p, EventKind::ElementalsPlayed, 1, Some(pos));
                    }
                    if let Some((da, dh)) = self.data.cards.get(m.card).volumizer_buff {
                        let mul = if m.golden { 2 } else { 1 };
                        self.add_volumizer_bonus(p, da * mul, dh * mul);
                    }
                    if t.has(Tribe::Beast) {
                        self.trigger_summon_beast_in_shop(p, pos);
                    }
                    if t.has(Tribe::Mech) {
                        self.trigger_mech_played_or_magnetized(p, pos, true, true);
                    }
                    self.battlecry_or_choose_one(p, pos);
                    self.maybe_start_discover(p);
                    if t.has(Tribe::Murloc) {
                        self.trigger_in_hand_on_play(p, Tribe::Murloc);
                    }
                }
                if m.golden {
                    // A Golden minion played from hand counts for Maritime
                    // Extortionist and Hooktusk.
                    self.add_golden_played(p, 1);
                }
                if choose_one_play {
                    self.players[p].hogrider_pending += 1;
                }
            }
            Action::Target(s) => self.resolve_target(p, s as usize),
            Action::TargetShop(s) => self.resolve_shop_target(p, s as usize),
            Action::Choose(k) => self.resolve_choose(p, k as usize),
            Action::Discard(s) => self.resolve_discard(p, s as usize),
            Action::Move { from, to } => {
                let pl = &mut self.players[p];
                let m = pl.board.remove(from as usize);
                pl.board.insert(to as usize, m);
            }
            Action::Roll => {
                let Game { data, rng, players, pool, lobby, .. } = self;
                let (cards, rules) = (&data.cards, &data.rules);
                let pl = &mut players[p];
                let mut gold_spent = 0;
                let mut health_paid = 0;
                if pl.free_rolls > 0 {
                    pl.free_rolls -= 1;
                } else if pl.can_refresh_with_health(cards, rules) {
                    pl.spend_health_refresh(cards);
                    health_paid = rules.health_refresh_cost;
                } else {
                    pl.gold -= rules.roll_cost;
                    gold_spent = rules.roll_cost;
                }
                Self::refresh_shop(pl, pool, cards, rules, rng, *lobby, RefreshKind::Explicit);
                if health_paid > 0 {
                    // A Health cost is not a Demon dealing damage.
                    self.take_hero_damage(p, health_paid, None);
                }
                self.fire_event(p, EventKind::GoldSpent, gold_spent as u32);
            }
            Action::Freeze => self.players[p].frozen = !self.players[p].frozen,
            Action::Level => {
                let turn = self.turn as u8;
                let pl = &mut self.players[p];
                let cost = pl.upgrade_cost;
                pl.gold -= cost;
                pl.tier += 1;
                pl.upgrade_cost = self.data.rules.upgrade_cost(pl.tier);
                pl.tier_turns[pl.tier as usize - 1] = turn;
                self.fire_event(p, EventKind::GoldSpent, cost as u32);
            }
            Action::EndTurn => {}
        }
    }

    /// Throw the Tavern back and draw a new one. Unfreezes it. `kind` says
    /// whether this is the player's own Refresh (which takes queued
    /// Fodders) or an effect that merely refreshes the Tavern.
    fn refresh_shop(
        pl: &mut Player,
        pool: &mut [u8],
        cards: &CardDb,
        rules: &Rules,
        rng: &mut Rng,
        lobby: Tribes,
        kind: RefreshKind,
    ) {
        pl.frozen = false;
        for m in pl.shop.drain(..) {
            give_back(pool, cards, &m);
        }
        pl.shop_spell = None;
        Self::fill_shop(pl, pool, cards, rules.shop_size(pl.tier), rng, lobby, kind);
        Self::apply_refresh_buff(pl, cards, rng);
        Self::apply_barrage(pl, cards);
    }

    /// "After the Tavern is Refreshed this game" (Waveling, En-Djinn Blazer,
    /// Easterly Winds): the whole amount goes to one random minion in the
    /// Tavern. Called by every Refresh, the turn-start roll included when
    /// the Tavern changed (see "Two kinds of Refresh" in DESIGN.md). It
    /// draws no random number when there is nothing to give.
    fn apply_refresh_buff(pl: &mut Player, cards: &CardDb, rng: &mut Rng) {
        let (a, h) = pl.refresh_buff;
        if a == 0 && h == 0 {
            return;
        }
        let ok = |m: &Minion| !cards.get(m.card).is_spell() && !cards.get(m.card).fixed_stats;
        let n = pl.shop.iter().filter(|m| ok(m)).count() as u32;
        if n == 0 {
            return;
        }
        let k = rng.below(n) as usize;
        if let Some(m) = pl.shop.iter_mut().filter(|m| ok(m)).nth(k) {
            buff(m, a, h);
        }
    }

    /// Blood Gem Barrage: after every Refresh (the turn-start roll included
    /// when the Tavern changed), every minion in the Tavern gains the whole
    /// amount cast so far, as Blood Gem stats.
    fn apply_barrage(pl: &mut Player, cards: &CardDb) {
        let (a, h) = pl.barrage;
        if a == 0 && h == 0 {
            return;
        }
        for m in &mut pl.shop {
            if !cards.get(m.card).is_spell() && !cards.get(m.card).fixed_stats {
                buff(m, a, h);
                m.gem_attack += a;
                m.gem_health += h;
            }
        }
    }

    /// Fill the Tavern: `size` minion slots and the spell slot. Only the
    /// player's own Refresh takes Fodders queued for upcoming Refreshes; the
    /// automatic roll at the start of a turn and an effect that refreshes
    /// the Tavern leave the queue alone. The first `fodders` slots hold
    /// Demon Fodders (they are not extra slots). A Fodder can also take the
    /// spell slot, which then has no spell this Refresh; Fodders beyond all
    /// slots are lost.
    #[allow(clippy::too_many_arguments)]
    fn fill_shop(
        pl: &mut Player,
        pool: &mut [u8],
        cards: &CardDb,
        size: usize,
        rng: &mut Rng,
        lobby: Tribes,
        kind: RefreshKind,
    ) {
        let fodders = match kind {
            RefreshKind::Explicit => pl.take_fodder_for_refresh(),
            RefreshKind::TurnStart | RefreshKind::Effect => 0,
        };
        let slots = size + 1;
        let fodders = (fodders as usize).min(slots);
        let normal = size.saturating_sub(fodders);
        while pl.shop.len() < normal {
            match draw_filtered(pool, cards, rng, 1, pl.tier, &[], lobby, |_| true) {
                Some(c) => {
                    let mut m = pl.make_card(cards, c, false, 1);
                    pl.apply_tavern_buffs(cards, &mut m);
                    pl.shop.push(m);
                }
                None => break,
            }
        }
        if let Some(tok) = cards.fodder_token {
            let mut made: ArrayVec<Minion, SHOP> = ArrayVec::new();
            for _ in 0..fodders {
                if pl.shop.len() + made.len() >= slots {
                    break;
                }
                let mut m = pl.make_card(cards, tok, false, 0);
                pl.apply_tavern_buffs(cards, &mut m);
                made.push(m);
            }
            for (k, m) in made.into_iter().enumerate() {
                pl.shop.insert(k, m);
            }
        }
        // The spell slot is free unless Fodders took it too; a frozen Tavern
        // keeps the spell it already had.
        if pl.shop.len() <= size {
            if pl.shop_spell.is_none() {
                pl.shop_spell = cards.random_spell(pl.tier, lobby, rng);
            }
        } else {
            pl.shop_spell = None;
        }
        // Only an explicit Refresh feeds the Fodders it just made.
        if kind == RefreshKind::Explicit {
            Self::feed_fodders(pl, pool, cards, rng, lobby);
        }
    }

    /// Saloon's Finest: every Tavern slot becomes a Tavern spell up to the
    /// player's tier. Spells have no pool, so repeats are allowed. This is a
    /// card effect, not the player's own Refresh, so no Fodders.
    fn refresh_with_spells(
        pl: &mut Player,
        cards: &CardDb,
        rules: &Rules,
        rng: &mut Rng,
        pool: &mut [u8],
        lobby: Tribes,
    ) {
        pl.frozen = false;
        for m in pl.shop.drain(..) {
            give_back(pool, cards, &m);
        }
        pl.shop_spell = None;
        let size = rules.shop_size(pl.tier);
        for _ in 0..size {
            match cards.random_spell(pl.tier, lobby, rng) {
                Some(id) => {
                    let m = pl.make_card(cards, id, false, 0);
                    pl.shop.push(m);
                }
                None => break,
            }
        }
        pl.shop_spell = cards.random_spell(pl.tier, lobby, rng);
        Self::apply_refresh_buff(pl, cards, rng);
        Self::apply_barrage(pl, cards);
    }

    /// Lost Staff of Hamuul: refresh the Tavern with minions of one type,
    /// up to the player's tier. A card effect, not the player's own
    /// Refresh, so no Fodders.
    #[allow(clippy::too_many_arguments)]
    fn refresh_with_tribe(
        pl: &mut Player,
        pool: &mut [u8],
        cards: &CardDb,
        rules: &Rules,
        rng: &mut Rng,
        lobby: Tribes,
        tribe: Tribe,
    ) {
        pl.frozen = false;
        for m in pl.shop.drain(..) {
            give_back(pool, cards, &m);
        }
        pl.shop_spell = None;
        let size = rules.shop_size(pl.tier);
        while pl.shop.len() < size {
            match draw_filtered(pool, cards, rng, 1, pl.tier, &[], lobby, |c| c.tribes.has(tribe)) {
                Some(c) => {
                    let mut m = pl.make_card(cards, c, false, 1);
                    pl.apply_tavern_buffs(cards, &mut m);
                    pl.shop.push(m);
                }
                None => break,
            }
        }
        pl.shop_spell = cards.random_spell(pl.tier, lobby, rng);
        Self::apply_refresh_buff(pl, cards, rng);
        Self::apply_barrage(pl, cards);
    }

    /// The Fodders that just appeared in a Refresh feed themselves if the
    /// player has a Demon at that moment: each goes to a random friendly
    /// Demon, which gains its current stats, and its slot gets a new card
    /// from the pool. Call this only when the Tavern was just refreshed; a
    /// Fodder that waits because there was no Demon never feeds later, even
    /// when a Demon lands, and it is a plain 2/2 Demon until the shop is
    /// refreshed again.
    fn feed_fodders(pl: &mut Player, pool: &mut [u8], cards: &CardDb, rng: &mut Rng, lobby: Tribes) {
        let is_demon = |m: &Minion| cards.get(m.card).tribes.has(Tribe::Demon);
        if !pl.board.iter().any(is_demon) {
            return;
        }
        let demons = pl.board.iter().filter(|m| is_demon(m)).count() as u32;
        let mut i = 0;
        while i < pl.shop.len() {
            if !cards.get(pl.shop[i].card).is_fodder {
                i += 1;
                continue;
            }
            let fodder = pl.shop.remove(i);
            let k = rng.below(demons) as usize;
            let di = pl
                .board
                .iter()
                .enumerate()
                .filter(|(_, m)| is_demon(m))
                .nth(k)
                .map(|(j, _)| j)
                .unwrap();
            buff(&mut pl.board[di], fodder.attack, fodder.health);
            if let Some(c) = draw_filtered(pool, cards, rng, 1, pl.tier, &[], lobby, |_| true) {
                let mut m = pl.make_card(cards, c, false, 1);
                pl.apply_tavern_buffs(cards, &mut m);
                pl.shop.insert(i, m);
                i += 1;
            }
        }
    }

    /// Total spell stat bonus for player `p`, combining Intrepid Botanist's
    /// permanent bonus with board auras (Enchanted Sentinel, including when
    /// Magnetized into a host, and Humon'gozz).
    pub fn total_spell_bonus(&self, p: usize) -> (i32, i32) {
        self.players[p].total_spell_bonus(&self.data.cards)
    }

    /// The player played `n` more Golden minions from hand: Maritime
    /// Extortionist grows wherever it is (Tavern, hand or board).
    fn add_golden_played(&mut self, p: usize, n: u32) {
        let cards = &self.data.cards;
        let pl = &mut self.players[p];
        pl.golden_played = pl.golden_played.saturating_add(n);
        for m in pl.shop.iter_mut().chain(&mut pl.hand).chain(&mut pl.board) {
            for (card, golden) in m.parts().collect::<ArrayVec<_, { crate::MAX_ATTACHED + 1 }>>() {
                if let Some((a, h)) = cards.get(card).buff_per_golden_played {
                    let k = n as i32 * if golden { 2 } else { 1 };
                    m.attack += a * k;
                    m.health += h * k;
                }
            }
        }
    }

    fn add_volumizer_bonus(&mut self, p: usize, da: i32, dh: i32) {
        let pl = &mut self.players[p];
        pl.volumizer_bonus.0 += da;
        pl.volumizer_bonus.1 += dh;
        for m in pl.shop.iter_mut().chain(&mut pl.hand).chain(&mut pl.board) {
            if m.volumizers > 0 {
                let k = m.volumizers as i32;
                m.attack += da * k;
                m.health += dh * k;
            }
        }
    }

    fn add_beetle_bonus(&mut self, p: usize, da: i32, dh: i32) {
        self.add_beetle_bonus_to(p, da, dh, true);
    }

    /// Give the player's Beetles +da/+dh this game; `board` says whether the
    /// ones on the board get it too (a Tavern attack already gave it to them).
    fn add_beetle_bonus_to(&mut self, p: usize, da: i32, dh: i32, board: bool) {
        let cards = &self.data.cards;
        let pl = &mut self.players[p];
        pl.beetle_bonus.0 += da;
        pl.beetle_bonus.1 += dh;
        let on_board = if board { &mut pl.board[..] } else { &mut [] };
        for m in pl.shop.iter_mut().chain(&mut pl.hand).chain(on_board) {
            if cards.get(m.card).is_beetle {
                m.attack += da;
                m.health += dh;
            }
        }
    }

    /// Your Undead have +da/+dh this game, wherever they are: the ones in the
    /// Tavern and the hand now (and the board when `board`), and every Undead
    /// made later. A fight or shop destroy that already gave the board its
    /// share passes `board = false`.
    fn add_undead_bonus(&mut self, p: usize, da: i32, dh: i32, board: bool) {
        if da == 0 && dh == 0 {
            return;
        }
        let cards = &self.data.cards;
        let pl = &mut self.players[p];
        pl.undead_bonus.0 += da;
        pl.undead_bonus.1 += dh;
        let on_board = if board { &mut pl.board[..] } else { &mut [] };
        for m in pl.shop.iter_mut().chain(&mut pl.hand).chain(on_board) {
            let c = cards.get(m.card);
            if !c.is_spell() && !c.fixed_stats && c.tribes.has(Tribe::Undead) {
                m.attack += da;
                m.health += dh;
            }
        }
    }

    /// `n` more of the player's Eternal Knights died: every Knight, in the
    /// Tavern, the hand and (when `board`) on the board, grows.
    fn add_knight_deaths(&mut self, p: usize, n: u32, board: bool) {
        if n == 0 {
            return;
        }
        let cards = &self.data.cards;
        let pl = &mut self.players[p];
        pl.knight_deaths = pl.knight_deaths.saturating_add(n);
        let on_board = if board { &mut pl.board[..] } else { &mut [] };
        for m in pl.shop.iter_mut().chain(&mut pl.hand).chain(on_board) {
            if let Some((a, h)) = cards.get(m.card).buff_per_kind_death {
                let k = n as i32 * m.mul();
                m.attack += a * k;
                m.health += h * k;
            }
        }
    }

    /// Destroy the board minions in `victims` in the shop phase: they die for
    /// real (Deathrattles, Reborn, Knight deaths, Banshee and Phantom). Returns
    /// where the `track`ed board slot ended up.
    fn destroy_in_shop(&mut self, p: usize, victims: &[usize], track: Option<usize>) -> Option<usize> {
        let Game { data, rng, players, pool, lobby, .. } = self;
        let pl = &mut players[p];
        let out = crate::fight::shop_destroy(
            &data.cards,
            &pl.board,
            victims,
            track,
            pl.counters(&data.cards, *lobby),
            rng,
        );
        // The minions that died leave the warband for good, and take their
        // pool copies back to the pool.
        for m in &out.dead {
            give_back(pool, &data.cards, m);
        }
        pl.board.clear();
        pl.board.extend(out.board);
        self.apply_side_outcome(p, &out.side, false);
        out.tracked
    }

    /// The player triggered `n` more Deathrattles: Falling Sky Golem and its
    /// like grow, wherever they are.
    fn add_deathrattles_triggered(&mut self, p: usize, n: u32, board: bool) {
        if n == 0 {
            return;
        }
        let cards = &self.data.cards;
        let pl = &mut self.players[p];
        pl.deathrattles_triggered = pl.deathrattles_triggered.saturating_add(n);
        let on_board = if board { &mut pl.board[..] } else { &mut [] };
        for m in pl.shop.iter_mut().chain(&mut pl.hand).chain(on_board) {
            for (card, golden) in m.parts().collect::<ArrayVec<_, { crate::MAX_ATTACHED + 1 }>>() {
                if let Some((a, h)) = cards.get(card).buff_per_deathrattle {
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
    fn magnetize_to(&mut self, p: usize, pos: usize, m: Minion, from_play: bool) {
        let cards = &self.data.cards;
        if m.pool_copies > 0 && !cards.get(m.card).token && !cards.get(m.card).is_spell() {
            let slot = cards.get(m.card).pool_slot as usize;
            self.consumed[slot] += m.pool_copies as u32;
        }
        let extra = std::mem::take(&mut self.players[p].board[pos].extra_magnetize);
        self.players[p].board[pos].magnetize(cards, m);
        if let Some((da, dh)) = cards.get(m.card).volumizer_buff {
            let mul = if m.golden { 2 } else { 1 };
            self.add_volumizer_bonus(p, da * mul, dh * mul);
        }
        // The copy a Beatboxer gets is the Magnetic as it attached, with the
        // buff the host got from Mechagnome Interpreter and the like.
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
            self.trigger_mech_played_or_magnetized(p, pos, from_play, false);
            self.beatboxers_copy(p, Some(pos), m1);
        }
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
    fn glambot_on_shop_mech(&mut self, p: usize, slot: usize) {
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
            self.beatboxers_copy(p, None, sat);
        }
    }

    fn trigger_mech_played_or_magnetized(
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

    fn trigger_on_cast_spell(&mut self, p: usize) {
        let len = self.players[p].board.len();
        for i in 0..len {
            let m = self.players[p].board[i];
            for (card, golden) in m.parts() {
                match self.data.cards.get(card).on_cast_spell {
                    Some(OnCastSpell::BuffDivineShield { attack, health }) => {
                        let mul = if golden { 2 } else { 1 };
                        for o in &mut self.players[p].board {
                            if o.keywords.has(Keywords::DIVINE_SHIELD) {
                                buff(o, attack * mul, health * mul);
                            }
                        }
                    }
                    Some(OnCastSpell::UndeadBonus { attack, health }) => {
                        let mul = if golden { 2 } else { 1 };
                        self.add_undead_bonus(p, attack * mul, health * mul, true);
                    }
                    Some(OnCastSpell::BuffSelfAndDeity { attack, health }) => {
                        let mul = if golden { 2 } else { 1 };
                        buff(&mut self.players[p].board[i], attack * mul, health * mul);
                        if let Some(deity) = &mut self.players[p].deity {
                            deity.attack += attack * mul;
                            deity.health += health * mul;
                        }
                    }
                    Some(OnCastSpell::BuffAllAndDeity { attack, health }) => {
                        let mul = if golden { 2 } else { 1 };
                        for o in &mut self.players[p].board {
                            buff(o, attack * mul, health * mul);
                        }
                        if let Some(deity) = &mut self.players[p].deity {
                            deity.attack += attack * mul;
                            deity.health += health * mul;
                        }
                    }
                    Some(OnCastSpell::BuffAll { attack, health }) => {
                        // A golden one gives the same buff twice.
                        let times = if golden { 2 } else { 1 };
                        for _ in 0..times {
                            for o in &mut self.players[p].board {
                                buff(o, attack, health);
                            }
                        }
                    }
                    None => {}
                }
            }
        }
    }

    fn trigger_on_cast_spell_on_mech(&mut self, p: usize, target_pos: usize) {
        let len = self.players[p].board.len();
        for i in 0..len {
            let m = self.players[p].board[i];
            for (card, golden) in m.parts() {
                if let Some(OnCastSpellOnMech::MagnetizeSatellite {
                    card: sat_id,
                    attack,
                    health,
                }) = self.data.cards.get(card).on_cast_spell_on_mech
                {
                    let count = if golden { 2 } else { 1 };
                    for _ in 0..count {
                        let mut sat = self.data.cards.make(sat_id, false);
                        sat.attack = attack;
                        sat.health = health;
                        self.magnetize_to(p, target_pos, sat, false);
                    }
                }
            }
        }
    }

    /// A spell was cast on the board minion in `target_pos`: run the
    /// "whenever you cast a spell on a Murloc" watchers (Shamanic
    /// Tidecaller), then the target's own watcher (Twilight Tidehunter).
    fn trigger_on_cast_spell_on_target(&mut self, p: usize, target_pos: usize) {
        if target_pos >= self.players[p].board.len() {
            return;
        }
        let target_tribes = self.data.cards.get(self.players[p].board[target_pos].card).tribes;
        let len = self.players[p].board.len();
        for i in 0..len {
            if i >= self.players[p].board.len() {
                break;
            }
            let m = self.players[p].board[i];
            for (card, golden) in m.parts() {
                let (on_tribe, on_self) = {
                    let c = self.data.cards.get(card);
                    (c.on_cast_spell_on_tribe, c.on_cast_spell_on_self)
                };
                if let Some((tribe, e)) = on_tribe
                    && target_tribes.has(tribe)
                {
                    let mut pos = i;
                    self.run_effect(p, &mut pos, e, golden, 1);
                }
                if i == target_pos
                    && let Some(e) = on_self
                {
                    let mut pos = i;
                    self.run_effect(p, &mut pos, e, golden, 1);
                }
            }
        }
    }

    fn trigger_summon_beast_in_shop(&mut self, p: usize, pos: usize) {
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

    /// After buying a Tavern spell, a Magicfin Mycologist on the board
    /// teaches a 1/1 Murloc token that spell, once per turn each: the first
    /// one left to right that still has its use for this turn.
    fn magicfin_teach(&mut self, p: usize, spell: CardId) {
        let Some(token) = self.data.cards.taught_token else { return };
        let cards = &self.data.cards;
        let Some(i) = (0..self.players[p].board.len()).find(|&i| {
            self.players[p].board[i].uses_left > 0
                && self.players[p].board[i].parts().any(|(c, _)| cards.get(c).teaches_spell)
        }) else { return };
        if self.players[p].hand.is_full() {
            return;
        }
        self.players[p].board[i].uses_left -= 1;
        let mut m = self.players[p].make_card(cards, token, false, 0);
        m.taught_spell = Some(spell);
        self.add_to_hand(p, m);
    }

    /// A minion of `tribe` was just played: hand cards that grow while in
    /// hand (Bream Counter) gain their buff.
    fn trigger_in_hand_on_play(&mut self, p: usize, tribe: Tribe) {
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
    fn fire_after_play_tribe(
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
    fn give_play_shield(&mut self, p: usize, pos: usize) {
        let cards = &self.data.cards;
        let tribes = cards.get(self.players[p].board[pos].card).tribes;
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

    /// A minion was just bought into the last hand slot (before it can
    /// triple): every Stone Age Slab that has not fired this turn buffs and
    /// multiplies it, left to right, then an armed Living Prison gains its
    /// stats (the Slab buffs included).
    fn after_buy_effects(&mut self, p: usize) {
        let cards = &self.data.cards;
        let pl = &mut self.players[p];
        let Some(last) = pl.hand.len().checked_sub(1) else { return };
        if cards.get(pl.hand[last].card).is_spell() {
            return;
        }
        if !cards.get(pl.hand[last].card).fixed_stats {
            for i in 0..pl.board.len() {
                let slab = pl.board[i].parts().find_map(|(card, golden)| {
                    cards.get(card).after_buy.map(|a| (a, golden))
                });
                if let Some((a, golden)) = slab
                    && !pl.board[i].activated
                {
                    pl.board[i].activated = true;
                    let b = &mut pl.hand[last];
                    b.attack += a.attack;
                    b.health += a.health;
                    let times = if golden { a.golden_multiply } else { a.multiply };
                    b.attack = b.attack.saturating_mul(times);
                    b.health = b.health.saturating_mul(times);
                }
            }
        }
        let bought = pl.hand[last];
        for m in &mut pl.board {
            if m.improvements == 0 {
                continue;
            }
            let mut mul = 0;
            for (card, golden) in m.parts() {
                if matches!(cards.get(card).activate, Some(a) if a.effect == Effect::ArmNextBuy) {
                    mul = mul.max(if golden { 2 } else { 1 });
                }
            }
            if mul > 0 {
                m.improvements = 0;
                buff(m, bought.attack * mul, bought.health * mul);
            }
        }
    }

    /// "Set versus add, as one mechanic" (DESIGN.md): every "how many times"
    /// count is the highest value a setter on the board sets (1 if none),
    /// plus what each adder adds. See `cards::how_many_times`.
    fn times_count(&self, p: usize, role: impl Fn(Aura) -> Option<TimesRole> + Copy) -> u8 {
        let cards = &self.data.cards;
        let parts = self.players[p].board.iter().flat_map(|m| m.parts());
        how_many_times(cards, parts, role)
    }

    fn brann_mult(&self, p: usize) -> u8 {
        self.times_count(p, |a| (a == Aura::Brann).then_some(TimesRole::Setter))
    }

    fn drakkari_mult(&self, p: usize) -> u8 {
        self.times_count(p, |a| (a == Aura::Drakkari).then_some(TimesRole::Setter))
    }

    /// How many times the player's Bounties cast: once, twice with a Proud
    /// Privateer, three times with a Golden one. "Twice" is a fixed count, so
    /// two Privateers don't stack (the highest wins).
    fn bounty_mult(&self, p: usize) -> u8 {
        self.times_count(p, |a| (a == Aura::ProudPrivateer).then_some(TimesRole::Setter))
    }

    fn balinda_mult(&self, p: usize) -> u8 {
        self.times_count(p, |a| (a == Aura::Balinda).then_some(TimesRole::Setter))
    }

    /// How many times a Blood Gem cast from hand on a friendly minion plays:
    /// Balinda sets the count (golden: 3), each Hot-Air Surveyor adds one
    /// (golden: two). Balinda plus one Surveyor: 3 casts.
    fn gem_casts(&self, p: usize) -> u8 {
        self.times_count(p, |a| match a {
            Aura::Balinda => Some(TimesRole::Setter),
            Aura::HotAirSurveyor => Some(TimesRole::Adder),
            _ => None,
        })
    }

    /// Spend one Thorned Trailblazer combine: the first one left to right
    /// that still has a use this turn, so none is wasted.
    fn take_trailblazer_cover(&mut self, p: usize) -> bool {
        let cards = &self.data.cards;
        let Some(i) = (0..self.players[p].board.len()).find(|&i| {
            self.players[p].board[i].uses_left > 0
                && cards.get(self.players[p].board[i].card).choose_one_cover > 0
        }) else { return false };
        self.players[p].board[i].uses_left -= 1;
        true
    }

    /// A Golden minion played from hand puts a Triple Reward spell in hand,
    /// a 0-cost plain spell whose Discover tier is fixed now: one above the
    /// player's tier, tier 6 at most. Lost on a full hand.
    fn give_triple_reward(&mut self, p: usize) {
        let id = self
            .data
            .cards
            .triple_reward
            .expect("the card data has no Triple Reward spell");
        if self.players[p].hand.is_full() {
            return;
        }
        let tier = (self.players[p].tier + 1).min(MAX_TIER);
        let mut m = self.players[p].make_card(&self.data.cards, id, false, 0);
        m.improvements = tier as u16;
        self.add_to_hand(p, m);
    }

    /// Get `n` Blood Gems in hand; the ones past a full hand are lost.
    fn give_gems(&mut self, p: usize, n: u8) {
        let Some(gem) = self.data.cards.blood_gem else { return };
        for _ in 0..n {
            self.give_generated(p, gem);
        }
    }

    /// Play `times` Blood Gems on a friendly minion (the player's Gem value,
    /// tracked as Blood Gem stats). Shop only: a fight plays its Gems through
    /// the fight's own counters.
    fn play_gems_on_board(&mut self, p: usize, pos: usize, times: u8) {
        let (ga, gh) = (self.players[p].gem_attack, self.players[p].gem_health);
        if let Some(m) = self.players[p].board.get_mut(pos) {
            m.attack += ga * times as i32;
            m.health += gh * times as i32;
            m.gem_attack += ga * times as i32;
            m.gem_health += gh * times as i32;
        }
    }

    /// A Blood Gem was just cast from hand onto the friendly minion in
    /// `target`: every Geomagus Roogug there plays its Blood Gems on a random
    /// different friendly minion. Those Gems are not from hand, so two
    /// Roogugs never loop.
    fn roogug_on_gem(&mut self, p: usize, target: usize) {
        let Some((count, golden)) = self.players[p].board.get(target).and_then(|m| {
            m.parts().find_map(|(c, g)| self.data.cards.get(c).on_gem_played_on_self.map(|n| (n, g)))
        }) else {
            return;
        };
        let times = count * (if golden { 2 } else { 1 });
        let n = self.players[p].board.len();
        if n <= 1 {
            return;
        }
        for _ in 0..times {
            // A different friendly minion, uniformly at random.
            let k = self.rng.below((n - 1) as u32) as usize;
            let at = if k < target { k } else { k + 1 };
            self.play_gems_on_board(p, at, 1);
        }
    }

    /// A Choose One card was just played: every Turbo Hogrider plays its
    /// Blood Gems on all friendly Quilboar.
    fn after_choose_one_played(&mut self, p: usize) {
        let cards = &self.data.cards;
        let hogs: ArrayVec<(u8, bool), BOARD> = self.players[p]
            .board
            .iter()
            .filter_map(|m| {
                m.parts().find_map(|(c, g)| {
                    cards.get(c).after_choose_one_play.map(|n| (n, g))
                })
            })
            .collect();
        if hogs.is_empty() {
            return;
        }
        for (count, golden) in hogs {
            let times = count * (if golden { 2 } else { 1 });
            let (ga, gh) = (self.players[p].gem_attack, self.players[p].gem_health);
            for m in &mut self.players[p].board {
                if cards.get(m.card).tribes.has(Tribe::Quilboar) {
                    m.attack += ga * times as i32;
                    m.health += gh * times as i32;
                    m.gem_attack += ga * times as i32;
                    m.gem_health += gh * times as i32;
                }
            }
        }
    }

    /// The player picked the card in hand slot `s` to discard (Mangled
    /// Bandit's Activate): it is gone, a pool copy goes back to the pool, and
    /// the reward runs.
    fn resolve_discard(&mut self, p: usize, s: usize) {
        let Some(pd) = self.players[p].pending_discard.take() else { return };
        if !self.valid_discard(p, s) {
            return;
        }
        self.discard_card(p, s);
        let mut pos = pd.source as usize;
        self.run_effect(p, &mut pos, pd.reward, pd.golden, 1);
        self.maybe_start_discover(p);
    }

    fn discard_card(&mut self, p: usize, s: usize) -> Option<Minion> {
        if s >= self.players[p].hand.len() {
            return None;
        }
        let m = self.players[p].hand.remove(s);
        give_back(&mut self.pool, &self.data.cards, &m);
        self.players[p].discarded = self.players[p].discarded.saturating_add(1);
        self.fire_event(p, EventKind::Discarded, 1);
        match self.data.cards.get(m.card).on_discard {
            Some(OnDiscard::CastSpell(times)) => {
                for _ in 0..times {
                    self.cast_spell(p, m);
                }
            }
            Some(OnDiscard::MaxGold(n)) => {
                self.players[p].extra_max_gold = self.players[p].extra_max_gold.saturating_add(n);
            }
            None => {}
        }
        Some(m)
    }

    /// Sell the board minion in `slot`: Gold, the copy back to the pool and
    /// the sell triggers, exactly as the Sell action does (Channel the
    /// Devourer sells through here too).
    fn sell_minion(&mut self, p: usize, slot: usize) -> Minion {
        let m = self.players[p].board.remove(slot);
        let mut sell_gold = self.data.rules.sell_gold;
        if self.players[p].lost_last_combat {
            for (card, golden) in m.parts() {
                if let Some(OnSell::GoldIfLost(g)) = self.data.cards.get(card).on_sell {
                    let mul = if golden { 2 } else { 1 };
                    sell_gold = g * mul;
                }
            }
        }
        self.players[p].gold = self.players[p].gold.saturating_add(sell_gold);
        give_back(&mut self.pool, &self.data.cards, &m);
        for (card, golden) in m.parts() {
            match self.data.cards.get(card).on_sell {
                Some(OnSell::GetCard { card: tok, count }) => {
                    let n = count * (if golden { 2 } else { 1 });
                    for _ in 0..n {
                        self.give_generated(p, tok);
                    }
                }
                Some(OnSell::DiscoverByTier) => {
                    let tier = (1 + m.improvements as u8).min(MAX_TIER);
                    let n = if golden { 2 } else { 1 };
                    for _ in 0..n {
                        let _ = self.players[p]
                            .discover_queue
                            .try_push(DiscoverRequest::MinionTier(tier));
                    }
                    self.maybe_start_discover(p);
                }
                Some(OnSell::Effect(e)) => {
                    let mut pos = 0;
                    self.run_effect(p, &mut pos, e, golden, 1);
                }
                Some(OnSell::DiscoverTier7IfGolden) => {
                    if golden {
                        self.discover_tier7_minion(p);
                    }
                }
                Some(OnSell::GoldIfLost(_)) | None => {}
            }
        }
        self.fire_event(p, EventKind::MinionsSold, 1);
        m
    }

    /// Pay for a spell bought from the Tavern. A spell bought with Health
    /// (Hasty Excavation) is hero damage, not Gold; a Gold purchase uses up
    /// the next-spell discount.
    fn pay_for_spell(&mut self, p: usize, pay_health: bool, cost: u8) {
        if pay_health {
            self.take_shop_damage(p, cost as i32, None);
        } else {
            let paid = cost.saturating_sub(self.players[p].spell_discount);
            self.players[p].gold = self.players[p].gold.saturating_sub(paid);
            self.players[p].spell_discount = 0;
            self.fire_event(p, EventKind::GoldSpent, paid as u32);
        }
    }

    fn cast_spell(&mut self, p: usize, m: Minion) {
        // A Bounty cast again (Proud Privateer) is another spell cast for
        // every trigger and every "spells cast" counter. Only a Tavern spell
        // counts as a spell cast at all (DESIGN.md): a Blood Gem, a Gem Day
        // and a Triple Reward do not.
        let c = self.data.cards.get(m.card);
        let casts = if c.is_bounty && c.spell_effect.is_some() { self.bounty_mult(p) } else { 1 };
        let tavern = c.tavern_spell;
        self.cast_spell_inner(p, m);
        if tavern {
            for _ in 0..casts {
                self.fire_event(p, EventKind::SpellsCast, 1);
            }
        }
    }

    fn cast_spell_inner(&mut self, p: usize, m: Minion) {
        self.players[p].last_spell = Some(m.card);
        let c = self.data.cards.get(m.card);
        let tavern = c.tavern_spell;
        if let Some(st) = c.spell_target {
            // A spell cast outside `Action::Play` (a taught spell) can have no
            // legal target; it fizzles instead of waiting on a choice that
            // could never be made.
            let castable =
                (0..self.players[p].board.len()).any(|s| self.valid_spell_target(p, st, s))
                    || (st.can_target_shop()
                        && (0..self.players[p].shop.len())
                            .any(|s| self.spell_target_ok(st, &self.players[p].shop[s])));
            if !castable {
                return;
            }
            // A Blood Gem plays once per cast: Balinda sets the count and
            // each Hot-Air Surveyor adds one ("Set versus add").
            let (runs, gem) = if c.blood_gem {
                (self.gem_casts(p), true)
            } else {
                (self.balinda_mult(p), false)
            };
            self.players[p].pending_target = Some(PendingTarget {
                source: None,
                kind: TargetKind::Spell(st),
                runs,
                tavern,
                gem,
            });
        } else if let Some(options) = c.choose_one {
            if !options.iter().any(|&opt| self.can_choose_option(p, opt)) {
                return;
            }
            // A Trailblazer's coverage runs both effects instead of asking.
            if self.take_trailblazer_cover(p) {
                self.resolve_choose_options(p, None, options, tavern, true);
            } else {
                self.players[p].pending_choose_one = Some(PendingChooseOne {
                    source: None,
                    golden: false,
                    options,
                    tavern,
                });
            }
        } else if let Some(se) = c.spell_effect {
            if matches!(se, SpellEffect::TripleReward) {
                // The tier was fixed when the reward was given.
                let tier = (m.improvements as u8).clamp(1, MAX_TIER);
                let _ = self.players[p]
                    .discover_queue
                    .try_push(DiscoverRequest::MinionTier(tier));
                self.maybe_start_discover(p);
                return;
            }
            // Proud Privateer: a Bounty casts again, each cast a spell cast.
            let casts = if c.is_bounty { self.bounty_mult(p) } else { 1 };
            for _ in 0..casts {
                if tavern {
                    self.trigger_on_cast_spell(p);
                }
                self.execute_spell_effect(p, se, tavern);
            }
            self.maybe_start_discover(p);
        }
    }

    /// Run a spell effect (or a Choose One option). `tavern` says whether the
    /// spell cast is a Tavern spell: only those get spell power.
    fn execute_spell_effect(&mut self, p: usize, se: SpellEffect, tavern: bool) {
        let (da, dh) = if tavern { self.total_spell_bonus(p) } else { (0, 0) };
        match se {
            SpellEffect::BuffDeity { attack, health } => {
                if let Some(deity) = &mut self.players[p].deity {
                    deity.attack += attack + da;
                    deity.health += health + dh;
                }
            }
            SpellEffect::GainGold(g) => {
                self.players[p].gold = self.players[p].gold.saturating_add(g);
            }
            SpellEffect::GoldNextTurn(g) => {
                self.players[p].gold_next_turn = self.players[p].gold_next_turn.saturating_add(g);
            }
            SpellEffect::MaxGold(g) => {
                self.players[p].extra_max_gold = self.players[p].extra_max_gold.saturating_add(g);
            }
            SpellEffect::FreeRolls(n) => {
                self.players[p].free_rolls = self.players[p].free_rolls.saturating_add(n);
            }
            SpellEffect::Overconfidence { win, tie } => {
                self.players[p].overconfidence_win =
                    self.players[p].overconfidence_win.saturating_add(win);
                self.players[p].overconfidence_tie =
                    self.players[p].overconfidence_tie.saturating_add(tie);
            }
            SpellEffect::BuffShop { attack, health } => {
                let cards = &self.data.cards;
                for m in &mut self.players[p].shop {
                    if !cards.get(m.card).is_spell() {
                        buff(m, attack + da, health + dh);
                    }
                }
            }
            SpellEffect::BuffBoard { attack, health, times, golden_extra } => {
                for _ in 0..times {
                    for m in &mut self.players[p].board {
                        buff(m, attack + da, health + dh);
                        if golden_extra && m.golden {
                            buff(m, attack + da, health + dh);
                        }
                    }
                }
            }
            SpellEffect::BuffRandomBoard { count, attack, health } => {
                let mut indices: ArrayVec<usize, BOARD> =
                    (0..self.players[p].board.len()).collect();
                self.rng.shuffle(&mut indices);
                for &i in indices.iter().take(count as usize) {
                    buff(&mut self.players[p].board[i], attack + da, health + dh);
                }
            }
            SpellEffect::NextTurnBoardBuff { attack, health, times } => {
                let _ = self.players[p]
                    .next_turn_board_buffs
                    .try_push((attack, health, times));
            }
            SpellEffect::RandomTierMinion(tier) => {
                if let Some(c) = self.data.cards.random_tier_minion(tier, self.lobby, &mut self.rng) {
                    self.give_generated(p, c);
                }
            }
            SpellEffect::Discover(kind) => {
                let tier = self.players[p].tier;
                let req = match kind {
                    DiscoverKind::Tier1Minion => DiscoverRequest::MinionTier(1),
                    DiscoverKind::MyTierMinion => DiscoverRequest::MinionTier(tier),
                    DiscoverKind::MyTierSpell => DiscoverRequest::SpellTier(tier),
                    DiscoverKind::DeathrattleMinion => DiscoverRequest::DeathrattleMinion(tier),
                    DiscoverKind::BattlecryMinion => DiscoverRequest::BattlecryMinion(tier),
                    DiscoverKind::Tier7Minion => DiscoverRequest::MinionTier(crate::CARD_MAX_TIER),
                    DiscoverKind::ChooseOneMinion => DiscoverRequest::ChooseOneMinion,
                };
                let _ = self.players[p].discover_queue.try_push(req);
            }
            SpellEffect::TombTurning => {
                let _ = self.players[p]
                    .discover_queue
                    .try_push(DiscoverRequest::DoomedTribeMinion(Tribe::Undead));
            }
            SpellEffect::MightyDragonbreath { attack, health } => {
                let cards = &self.data.cards;
                for pass in 0..3 {
                    for o in &mut self.players[p].board {
                        let ok = match pass {
                            0 => true,
                            1 => cards.get(o.card).tribes.has(Tribe::Dragon),
                            _ => o.keywords.has(Keywords::DIVINE_SHIELD),
                        };
                        if ok {
                            buff(o, attack + da, health + dh);
                        }
                    }
                }
            }
            SpellEffect::BoonOfBeetles(n) => {
                self.players[p].boon_of_beetles =
                    self.players[p].boon_of_beetles.saturating_add(n);
            }
            SpellEffect::BroodOfNozdormu => {
                self.players[p].brood_of_nozdormu =
                    self.players[p].brood_of_nozdormu.saturating_add(1);
            }
            SpellEffect::CloningConch => {
                let tier = self.players[p].tier;
                if let Some(c) =
                    self.data.cards.random_pool_minion(Tribe::Murloc, tier, self.lobby, &mut self.rng)
                {
                    self.give_generated(p, c);
                    self.give_generated(p, c);
                }
            }
            SpellEffect::GoldenTouch => {
                let cards = &self.data.cards;
                let shop = &mut self.players[p].shop;
                let ok = |m: &Minion| !m.golden && !cards.get(m.card).fixed_stats;
                let n = shop.iter().filter(|m| ok(m)).count() as u32;
                if n > 0 {
                    let k = self.rng.below(n) as usize;
                    let m = shop.iter_mut().filter(|m| ok(m)).nth(k).unwrap();
                    let base = cards.get(m.card);
                    let (ga, gh) =
                        base.golden_stats.unwrap_or((base.attack * 2, base.health * 2));
                    m.golden = true;
                    m.attack += ga - base.attack;
                    m.health += gh - base.health;
                }
            }
            SpellEffect::SharingIsCaring => {
                self.players[p].sharing_is_caring =
                    self.players[p].sharing_is_caring.saturating_add(1);
            }
            SpellEffect::MenagerieTableware { attack, health } => {
                let cards = &self.data.cards;
                let types = Tribe::EVERY
                    .iter()
                    .filter(|t| self.players[p].board.iter().any(|m| cards.get(m.card).tribes.has(**t)))
                    .count();
                let times = 1 + types;
                for _ in 0..times {
                    for m in &mut self.players[p].board {
                        buff(m, attack + da, health + dh);
                    }
                }
            }
            SpellEffect::UpperHand => {
                self.players[p].upper_hand = self.players[p].upper_hand.saturating_add(1);
            }
            SpellEffect::BuffTavernPermanent { attack, health } => {
                let b = TavernBuff {
                    tribe: None,
                    attack: attack + da,
                    health: health + dh,
                    this_turn: false,
                };
                self.players[p].add_tavern_buff(&self.data.cards, b);
            }
            SpellEffect::MisplacedTeaSet { attack, health } => {
                // One random unbuffed minion of each type on the board; a
                // dual or all-type minion can only be picked once.
                let cards = &self.data.cards;
                let board = &mut self.players[p].board;
                let tribes: ArrayVec<Tribes, BOARD> =
                    board.iter().map(|m| cards.get(m.card).tribes).collect();
                for i in pick_one_per_type(&tribes, &mut self.rng) {
                    buff(&mut board[i], attack + da, health + dh);
                }
            }
            SpellEffect::PlanarTelescope => {
                let cards = &self.data.cards;
                let mut counts = [0u8; Tribe::EVERY.len()];
                for m in &self.players[p].board {
                    let t = cards.get(m.card).tribes;
                    for (k, tribe) in Tribe::EVERY.iter().enumerate() {
                        // only a lobby tribe can be discovered from, so only
                        // lobby tribes count toward the most common one
                        if self.lobby.has(*tribe) && t.has(*tribe) {
                            counts[k] += 1;
                        }
                    }
                }
                let Some(max) = counts.iter().copied().max().filter(|&n| n > 0) else {
                    return;
                };
                let mut tied: ArrayVec<Tribe, { Tribe::EVERY.len() }> = ArrayVec::new();
                for (k, tribe) in Tribe::EVERY.iter().enumerate() {
                    if counts[k] == max {
                        let _ = tied.try_push(*tribe);
                    }
                }
                let k = self.rng.below(tied.len() as u32) as usize;
                let _ = self.players[p]
                    .discover_queue
                    .try_push(DiscoverRequest::TribeMinion(tied[k]));
            }
            SpellEffect::StealRandomTavern => self.steal_random_tavern(p),
            SpellEffect::DiscoverLockedMinion => {
                let tier = self.players[p].tier;
                let _ = self.players[p]
                    .discover_queue
                    .try_push(DiscoverRequest::SearchMinionTier(tier));
            }
            SpellEffect::SaloonFinest => {
                let Game { data, rng, players, pool, lobby, .. } = self;
                Self::refresh_with_spells(&mut players[p], &data.cards, &data.rules, rng, pool, *lobby);
            }
            SpellEffect::GetCard { card, count } => {
                for _ in 0..count {
                    self.give_generated(p, card);
                }
            }
            // A Lockbox can't be cast.
            SpellEffect::Lockbox(_) => {}
            SpellEffect::BuffLeftmost { attack, health } => {
                if let Some(m) = self.players[p].board.first_mut() {
                    buff(m, attack + da, health + dh);
                }
            }
            SpellEffect::RefreshBuff { attack, health } => {
                let pl = &mut self.players[p];
                pl.refresh_buff.0 += attack + da;
                pl.refresh_buff.1 += health + dh;
            }
            SpellEffect::GetTwo { a, b } => {
                self.give_generated(p, a);
                self.give_generated(p, b);
            }
            SpellEffect::FriendlyBounty => {
                let cards = &self.data.cards;
                let mut counts = [0u8; Tribe::EVERY.len()];
                for m in &self.players[p].board {
                    let t = cards.get(m.card).tribes;
                    for (k, tribe) in Tribe::EVERY.iter().enumerate() {
                        // only a lobby tribe can be drawn from, so only lobby
                        // tribes count toward the most common one
                        if self.lobby.has(*tribe) && t.has(*tribe) {
                            counts[k] += 1;
                        }
                    }
                }
                let Some(max) = counts.iter().copied().max().filter(|&n| n > 0) else {
                    return;
                };
                let mut tied: ArrayVec<Tribe, { Tribe::EVERY.len() }> = ArrayVec::new();
                for (k, tribe) in Tribe::EVERY.iter().enumerate() {
                    if counts[k] == max {
                        let _ = tied.try_push(*tribe);
                    }
                }
                let tribe = tied[self.rng.below(tied.len() as u32) as usize];
                let tier = self.players[p].tier;
                if let Some(c) =
                    self.data.cards.random_pool_minion(tribe, tier, self.lobby, &mut self.rng)
                {
                    self.give_generated(p, c);
                }
            }
            SpellEffect::GetGems(n) => self.give_gems(p, n),
            SpellEffect::GemBonus { attack, health } => {
                let pl = &mut self.players[p];
                pl.gem_attack += attack;
                pl.gem_health += health;
            }
            SpellEffect::GemsOnBoard(n) => {
                // Gems played by an effect, not from hand: no Roogug chain.
                let (ga, gh) = (self.players[p].gem_attack, self.players[p].gem_health);
                for m in &mut self.players[p].board {
                    m.attack += ga * n as i32;
                    m.health += gh * n as i32;
                    m.gem_attack += ga * n as i32;
                    m.gem_health += gh * n as i32;
                }
            }
            SpellEffect::Barrage { attack, health } => {
                // The Gem bonus (what a Gem gives above its base +1/+1) and
                // the Tavern spell bonus land once per cast, and each cast
                // adds on top for the coming Refreshes.
                let (ga, gh) = (self.players[p].gem_attack - 1, self.players[p].gem_health - 1);
                let pl = &mut self.players[p];
                pl.barrage.0 += attack + ga + da;
                pl.barrage.1 += health + gh + dh;
            }
            SpellEffect::CastSpell { card, times } => {
                for _ in 0..times {
                    let m = self.data.cards.make(card, false);
                    self.cast_spell(p, m);
                    if self.players[p].pending_target.is_some()
                        || self.players[p].pending_choose_one.is_some()
                    {
                        break;
                    }
                }
            }
            SpellEffect::RandomTribeMinion { tribe, count } => {
                let tier = self.players[p].tier;
                for _ in 0..count {
                    if let Some(c) =
                        self.data.cards.random_pool_minion(tribe, tier, self.lobby, &mut self.rng)
                    {
                        self.give_generated(p, c);
                    }
                }
            }
            // Handled where the spell is cast (the tier lives on the copy).
            SpellEffect::TripleReward => {}
        }
    }

    fn battlecry_or_choose_one(&mut self, p: usize, mut pos: usize) {
        let m = self.players[p].board[pos];
        let mul = if m.golden { 2 } else { 1 };
        let card = self.data.cards.get(m.card);
        if let Some(options) = card.choose_one {
            let scaled = options.map(|o| match o {
                ChooseOption::MinionTarget { tribe, attack, health, keyword } => {
                    ChooseOption::MinionTarget {
                        tribe,
                        attack: attack * mul,
                        health: health * mul,
                        keyword,
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
            // Both effects combined: the card says so (Fandral's Fortune) or
            // a Trailblazer covers this play.
            let covered = !m.both_effects && self.take_trailblazer_cover(p);
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
        let runs = self.brann_mult(p);
        self.run_effect(p, &mut pos, bc, m.golden, runs);
        self.fire_after_battlecry(p, runs);
    }

    /// Run one option of a Choose One (the shared body of `resolve_choose`
    /// and the both-effects paths).
    fn apply_choose_option(&mut self, p: usize, source: Option<u8>, opt: ChooseOption, tavern: bool) {
        match opt {
            ChooseOption::MinionTarget { tribe, attack, health, keyword } => {
                let pt = PendingTarget {
                    source,
                    kind: TargetKind::MinionBuff {
                        tribe,
                        attack,
                        health,
                        keyword,
                        random_keyword: false,
                    },
                    runs: 1,
                    tavern: true,
                    gem: false,
                };
                self.offer_target(p, pt);
            }
            ChooseOption::SpellBonus { attack, health } => {
                self.players[p].spell_bonus.0 += attack;
                self.players[p].spell_bonus.1 += health;
            }
            ChooseOption::SpellTarget(st) => {
                let runs = self.balinda_mult(p);
                // offer_target, not a raw set: the shop may have changed since
                // the choice was offered (Saloon's Finest), and a prompt with
                // no legal target could never be answered.
                self.offer_target(
                    p,
                    PendingTarget {
                        source: None,
                        kind: TargetKind::Spell(st),
                        runs,
                        tavern,
                        gem: false,
                    },
                );
            }
            ChooseOption::SpellEffect(se) => {
                if tavern {
                    self.trigger_on_cast_spell(p);
                }
                self.execute_spell_effect(p, se, tavern);
            }
        }
    }

    /// A Choose One with both effects combined: the first option runs now,
    /// the second right after it, or through the queue when the first leaves
    /// a target prompt open. `source.is_some()` means the Choose One is a
    /// minion's Battlecry, so "after you trigger a Battlecry" fires once.
    fn resolve_choose_options(
        &mut self,
        p: usize,
        source: Option<u8>,
        options: [ChooseOption; 2],
        tavern: bool,
        both: bool,
    ) {
        if !both {
            self.players[p].pending_choose_one = Some(PendingChooseOne {
                source,
                golden: false,
                options,
                tavern,
            });
            return;
        }
        self.apply_choose_option(p, source, options[0], tavern);
        let blocked = self.players[p].pending_target.is_some()
            || self.players[p].pending_shop_target.is_some()
            || self.players[p].pending_choose_one.is_some()
            || self.players[p].pending_discard.is_some()
            || !self.players[p].discover.is_empty();
        if blocked {
            let _ = self.players[p]
                .choose_queue
                .try_push(QueuedOption { source, option: options[1], tavern });
        } else {
            self.apply_choose_option(p, source, options[1], tavern);
        }
        if source.is_some() {
            self.fire_after_battlecry(p, 1);
        }
        self.maybe_start_discover(p);
    }

    /// "After you trigger a Battlecry" effects (Kalecgos). One trigger per
    /// Battlecry run, so Brann's double Battlecry fires them twice.
    fn fire_after_battlecry(&mut self, p: usize, times: u8) {
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

    /// Run a generic effect for the minion in board slot `*pos`. `golden`
    /// doubles it and `runs` repeats it (Brann). `pos` follows the minion
    /// when a triple takes board minions away to its left.
    fn run_effect(&mut self, p: usize, pos: &mut usize, e: Effect, golden: bool, runs: u8) {
        let mul = if golden { 2 } else { 1 };
        let bonus_mask = self.data.rules.bonus_mask;
        match e {
            Effect::BuffDeity { attack, health, per_spell } => {
                let n = self.players[p].spells_cast as i32;
                if let Some(deity) = &mut self.players[p].deity {
                    deity.attack += (attack + per_spell.0 * n) * mul * runs as i32;
                    deity.health += (health + per_spell.1 * n) * mul * runs as i32;
                }
            }
            Effect::BuffSelfAndDeity { attack, health } => {
                if let Some(m) = self.players[p].board.get_mut(*pos) {
                    buff(m, attack * mul * runs as i32, health * mul * runs as i32);
                }
                if let Some(deity) = &mut self.players[p].deity {
                    deity.attack += attack * mul * runs as i32;
                    deity.health += health * mul * runs as i32;
                }
            }
            Effect::BuffDeityAndImprove { attack, health } => {
                for _ in 0..runs {
                    let imp = self.players[p].board.get(*pos).map_or(0, |m| m.improvements as i32);
                    if let Some(deity) = &mut self.players[p].deity {
                        deity.attack += attack * (1 + imp) * mul;
                        deity.health += health * (1 + imp) * mul;
                    }
                    if let Some(m) = self.players[p].board.get_mut(*pos) {
                        m.improvements = m.improvements.saturating_add(1);
                    }
                }
            }
            Effect::BuffLeftmostPerDiscard { attack, health } => {
                let n = 1 + self.players[p].discarded as i32;
                if let Some(m) = self.players[p].board.first_mut() {
                    buff(m, attack * n * mul * runs as i32, health * n * mul * runs as i32);
                }
            }
            Effect::GetRandomSpellUpTo { cost, count } => {
                let tier = self.players[p].tier;
                for _ in 0..count.saturating_mul(mul as u8).saturating_mul(runs) {
                    let card = match cost {
                        Some(cost) => {
                            self.data.cards.random_spell_of_cost_up_to(cost, tier, self.lobby, &mut self.rng)
                        }
                        None => self.data.cards.random_spell(tier, self.lobby, &mut self.rng),
                    };
                    if let Some(card) = card {
                        self.give_generated_tracking(p, card, pos);
                    }
                }
            }
            Effect::GetLinkedSpells { count } => {
                let tier = self.players[p].tier;
                let mut cards = ArrayVec::<CardId, HAND>::new();
                for _ in 0..count.saturating_mul(mul as u8).saturating_mul(runs) {
                    if let Some(card) = self.data.cards.random_spell(tier, self.lobby, &mut self.rng) {
                        let _ = cards.try_push(card);
                    }
                }
                self.give_linked(p, &cards, golden);
            }
            Effect::GetLinkedMinions { tribe, count } => {
                let tier = self.players[p].tier;
                let mut cards = ArrayVec::<CardId, HAND>::new();
                for _ in 0..count.saturating_mul(mul as u8).saturating_mul(runs) {
                    if let Some(card) = self.data.cards.random_pool_minion(tribe, tier, self.lobby, &mut self.rng) {
                        let _ = cards.try_push(card);
                    }
                }
                self.give_linked(p, &cards, golden);
            }
            Effect::DiscardLeftmostSpells { count, attack, health } => {
                for _ in 0..count.saturating_mul(mul as u8).saturating_mul(runs) {
                    let cards = &self.data.cards;
                    let slot = self.players[p].hand.iter()
                        .position(|m| cards.get(m.card).tavern_spell);
                    let Some(slot) = slot else { break };
                    self.discard_card(p, slot);
                    if let Some(m) = self.players[p].board.get_mut(*pos) {
                        buff(m, attack, health);
                    }
                }
            }
            Effect::SummonAndBuffAll { .. }
            | Effect::SummonDeadAberrations { .. }
            | Effect::DamageHighestHealth => {}
            Effect::BuffOthers { tribe, attack, health, per_keyword } => {
                let cards = &self.data.cards;
                let pl = &mut self.players[p];
                let k = pl.distinct_bonus_keywords(bonus_mask) as i32;
                let (da, dh) = (attack + per_keyword.0 * k, health + per_keyword.1 * k);
                for _ in 0..runs {
                    for (i, o) in pl.board.iter_mut().enumerate() {
                        if i != *pos && cards.get(o.card).is(tribe) {
                            buff(o, da * mul, dh * mul);
                        }
                    }
                }
            }
            Effect::BuffSelf { attack, health } => {
                if let Some(m) = self.players[p].board.get_mut(*pos) {
                    buff(m, attack * mul * runs as i32, health * mul * runs as i32);
                }
            }
            Effect::BuffChosen { tribe, attack, health, per_keyword, random_keyword } => {
                let k = self.players[p].distinct_bonus_keywords(bonus_mask) as i32;
                let pt = PendingTarget {
                    source: Some(*pos as u8),
                    kind: TargetKind::MinionBuff {
                        tribe,
                        attack: (attack + per_keyword.0 * k) * mul,
                        health: (health + per_keyword.1 * k) * mul,
                        keyword: 0,
                        random_keyword,
                    },
                    runs,
                        tavern: true,
                        gem: false,
                };
                self.offer_target(p, pt);
            }
            Effect::RandomKeyword => {
                for _ in 0..mul as u8 * runs {
                    self.give_random_keyword(p, *pos);
                }
            }
            Effect::Consume { by, pick, keywords, count } => {
                // A golden consumer takes double stats; repeated Battlecries
                // (Brann) consume once more each.
                let count = count.saturating_mul(runs);
                let stat_mul = mul as u8;
                match by {
                    ConsumeBy::This => self.start_consume(p, *pos, pick, keywords, count, stat_mul),
                    ConsumeBy::Chosen(tribe) => {
                        let pt = PendingTarget {
                            source: Some(*pos as u8),
                            kind: TargetKind::Consume { tribe, pick, keywords, count, stat_mul },
                            runs: 1,
                                tavern: true,
                                gem: false,
                        };
                        self.offer_target(p, pt);
                    }
                }
            }
            Effect::TavernBuff { tribe, attack, health, this_turn } => {
                let n = mul * runs as i32;
                let b = TavernBuff { tribe, attack: attack * n, health: health * n, this_turn };
                self.players[p].add_tavern_buff(&self.data.cards, b);
            }
            Effect::TavernBuffRandomKeyword { attack, health, mask } => {
                let bits = mask.count_ones();
                if bits == 0 {
                    return;
                }
                for _ in 0..runs {
                    let mut k = self.rng.below(bits);
                    let mut keyword = 0u8;
                    for bit in 0..8 {
                        let b = 1u8 << bit;
                        if mask & b != 0 {
                            if k == 0 {
                                keyword = b;
                                break;
                            }
                            k -= 1;
                        }
                    }
                    let cards = &self.data.cards;
                    for m in &mut self.players[p].shop {
                        if cards.get(m.card).is_spell() {
                            continue;
                        }
                        buff(m, attack * mul, health * mul);
                        m.keywords.set(keyword);
                    }
                }
            }
            Effect::AddFodderRefreshes(n) => {
                let per = mul as u8;
                let times = (n as u32).saturating_mul(runs as u32);
                self.players[p].add_fodder_refreshes(per, times);
            }
            Effect::AddFodderToNextRefresh(n) => {
                let add = n.saturating_mul(mul as u8).saturating_mul(runs);
                if let Some(first) = self.players[p].fodder_refreshes.first_mut() {
                    *first = first.saturating_add(add);
                } else {
                    let _ = self.players[p].fodder_refreshes.try_push(add);
                }
            }
            Effect::DemonsConsume { count } => {
                let n = count.saturating_mul(runs);
                let cards = &self.data.cards;
                let demons: ArrayVec<usize, BOARD> = self.players[p]
                    .board
                    .iter()
                    .enumerate()
                    .filter(|(_, m)| cards.get(m.card).tribes.has(Tribe::Demon))
                    .map(|(i, _)| i)
                    .collect();
                for i in demons {
                    self.start_consume(p, i, ConsumePick::Random, false, n, mul as u8);
                }
            }
            Effect::ImproveSelf(n) => {
                // The count is Activates, so a golden Activate adds one too.
                // Its Deathrattle doubles the per-Activate number instead.
                if let Some(m) = self.players[p].board.get_mut(*pos) {
                    let add = n.saturating_mul(runs);
                    m.improvements = m.improvements.saturating_add(add as u16);
                }
            }
            Effect::DiscoverDemonTierDamage(n) => {
                let times =
                    (n as u32).saturating_mul(mul as u32).saturating_mul(runs as u32);
                let tier = self.players[p].tier;
                let source = *pos as u8;
                for _ in 0..times {
                    let _ = self.players[p]
                        .discover_queue
                        .try_push(DiscoverRequest::DemonMinion { max_tier: tier, source });
                }
            }
            Effect::BuffAnyMinion { attack, health } => {
                let pt = PendingTarget {
                    source: Some(*pos as u8),
                    kind: TargetKind::AnyBuff {
                        attack: attack * mul,
                        health: health * mul,
                    },
                    runs,
                        tavern: true,
                        gem: false,
                };
                self.offer_target(p, pt);
            }
            Effect::SetStatsAnyMinion { attack, health } => {
                let pt = PendingTarget {
                    source: Some(*pos as u8),
                    kind: TargetKind::AnySetStats {
                        attack: attack * mul,
                        health: health * mul,
                    },
                    runs,
                        tavern: true,
                        gem: false,
                };
                self.offer_target(p, pt);
            }
            Effect::StealTavern { count } => {
                let n = count.saturating_mul(mul as u8).saturating_mul(runs);
                self.steal_highest_tavern(p, n);
            }
            Effect::DiscoverSpell { count } => {
                let n = count.saturating_mul(mul as u8).saturating_mul(runs);
                let tier = self.players[p].tier;
                for _ in 0..n {
                    let _ = self.players[p]
                        .discover_queue
                        .try_push(DiscoverRequest::SpellMaxTier(tier));
                }
            }
            Effect::DamageHero(n) => {
                self.take_shop_damage(p, n * mul * runs as i32, Some(*pos));
            }
            Effect::RewindDamage => {
                let pl = &mut self.players[p];
                pl.health += pl.last_damage;
                pl.last_damage = 0;
            }
            Effect::MagnetizeVolumizerAndCopy => {
                for _ in 0..runs {
                    if let Some(v_id) = self.data.cards.random_volumizer(self.lobby, &mut self.rng) {
                        let v = self.players[p].make_card(&self.data.cards, v_id, false, 0);
                        self.magnetize_to(p, *pos, v, false);
                        for _ in 0..mul {
                            self.give_generated_tracking(p, v_id, pos);
                        }
                    }
                }
            }
            Effect::BeetleBuff { attack, health } => {
                for _ in 0..runs {
                    self.add_beetle_bonus(p, attack * mul, health * mul);
                }
            }
            Effect::GoldNextTurn(g) => {
                let gain = g.saturating_mul(mul as u8).saturating_mul(runs);
                self.players[p].gold_next_turn =
                    self.players[p].gold_next_turn.saturating_add(gain);
            }
            Effect::RandomVolumizer => {
                let total = (mul as u8).saturating_mul(runs);
                for _ in 0..total {
                    if let Some(v) = self.data.cards.random_volumizer(self.lobby, &mut self.rng) {
                        self.give_generated_tracking(p, v, pos);
                    }
                }
            }
            Effect::SpellDiscount(d) => {
                let gain = d.saturating_mul(mul as u8).saturating_mul(runs);
                self.players[p].spell_discount =
                    self.players[p].spell_discount.saturating_add(gain);
            }
            Effect::GetCard { card: tok, count } => {
                let total = count.saturating_mul(mul as u8).saturating_mul(runs);
                for _ in 0..total {
                    self.give_generated_tracking(p, tok, pos);
                }
            }
            Effect::BuffAll { tribe, attack, health } => {
                let cards = &self.data.cards;
                for _ in 0..runs {
                    for o in &mut self.players[p].board {
                        if cards.get(o.card).is(tribe) && !cards.get(o.card).fixed_stats {
                            buff(o, attack * mul, health * mul);
                        }
                    }
                }
            }
            Effect::BuffRandomFriendly { tribe, attack, health, improve } => {
                for _ in 0..runs {
                    let scale = improve.map_or(1, |c| 1 + self.players[p].improve[c.index()] as i32);
                    let cards = &self.data.cards;
                    let board = &mut self.players[p].board;
                    let ok = |m: &Minion| cards.get(m.card).is(tribe) && !cards.get(m.card).fixed_stats;
                    let n = board.iter().filter(|m| ok(m)).count() as u32;
                    if n > 0 {
                        let k = self.rng.below(n) as usize;
                        let m = board.iter_mut().filter(|m| ok(m)).nth(k).unwrap();
                        buff(m, attack * mul * scale, health * mul * scale);
                    }
                    if let Some(c) = improve {
                        let n = &mut self.players[p].improve[c.index()];
                        *n = n.saturating_add(1);
                    }
                }
            }
            Effect::GainKeyword { keyword } => {
                if let Some(m) = self.players[p].board.get_mut(*pos) {
                    m.keywords.set(keyword);
                }
            }
            Effect::GetRandomMinion { tribe } => {
                let tier = self.players[p].tier;
                for _ in 0..mul * runs as i32 {
                    if let Some(c) =
                        self.data.cards.random_pool_minion(tribe, tier, self.lobby, &mut self.rng)
                    {
                        self.give_generated_tracking(p, c, pos);
                    }
                }
            }
            Effect::GetRandomSpell { cost, count } => {
                let total = count as i32 * mul * runs as i32;
                for _ in 0..total {
                    if let Some(c) = self.data.cards.random_spell_of_cost(cost, self.lobby, &mut self.rng) {
                        self.give_generated_tracking(p, c, pos);
                    }
                }
            }
            Effect::ReplaceWithFishbait { card } => {
                self.start_fishbait(p, *pos, card, golden);
            }
            Effect::RefreshWithFishbait { card } => {
                for _ in 0..runs {
                    self.refresh_with_fishbait(p, card, golden);
                }
            }
            Effect::ExtraMagnetization(n) => {
                if let Some(m) = self.players[p].board.get_mut(*pos) {
                    let add = (n as i32 * mul) as u8 * runs;
                    m.extra_magnetize = m.extra_magnetize.saturating_add(add);
                }
            }
            // A Rally triggered in the shop can summon (Sky-hatch Runaway
            // triggering Flittering Bat): the copies go next to the minion.
            Effect::Summon { card, count, golden_doubles_count } => {
                let (n, g) = if golden_doubles_count {
                    (count * mul as u8, false)
                } else {
                    (count, golden)
                };
                let mut at = *pos + 1;
                for _ in 0..n {
                    if self.players[p].board.is_full() || at > self.players[p].board.len() {
                        break;
                    }
                    let mut m = self.players[p].make_card(&self.data.cards, card, g, 0);
                    if self.data.cards.get(card).tribes.has(Tribe::Beast) {
                        let (ba, bh) = self.players[p].beast_bonus;
                        m.attack += ba;
                        m.health += bh;
                    }
                    self.players[p].board.insert(at, m);
                    at += 1;
                }
            }
            Effect::SpellBonus { attack, health } => {
                let n = mul * runs as i32;
                let pl = &mut self.players[p];
                pl.spell_bonus.0 += attack * n;
                pl.spell_bonus.1 += health * n;
            }
            Effect::RandomChromadrake => {
                for _ in 0..mul as u8 * runs {
                    if let Some(c) = self.data.cards.random_chromadrake(self.lobby, &mut self.rng) {
                        self.give_generated_tracking(p, c, pos);
                    }
                }
            }
            // A spell cast by a minion (Gatekeeper Amalgam) is a real cast:
            // it gets the Tavern spell bonus and every "whenever you cast a
            // spell" card sees it.
            Effect::CastSpell { card, times } => {
                for _ in 0..times as u32 * mul as u32 * runs as u32 {
                    let m = self.data.cards.make(card, false);
                    self.cast_spell(p, m);
                    if self.players[p].pending_target.is_some()
                        || self.players[p].pending_choose_one.is_some()
                    {
                        break;
                    }
                }
            }
            Effect::TriggerRally => {
                let pt = PendingTarget {
                    source: Some(*pos as u8),
                    kind: TargetKind::TriggerRally,
                    runs: (mul as u8).saturating_mul(runs),
                    tavern: true,
                    gem: false,
                };
                self.offer_target(p, pt);
            }
            Effect::TriggerBattlecry => {
                let pt = PendingTarget {
                    source: Some(*pos as u8),
                    kind: TargetKind::TriggerBattlecry,
                    runs: (mul as u8).saturating_mul(runs),
                    tavern: true,
                    gem: false,
                };
                self.offer_target(p, pt);
            }
            Effect::BuffLeftmostHand { attack, health } => {
                let n = mul * runs as i32;
                let cards = &self.data.cards;
                let slot = self.players[p].hand.iter().position(|m| !cards.get(m.card).is_spell());
                if let Some(i) = slot {
                    let m = &mut self.players[p].hand[i];
                    m.attack += attack * n;
                    m.health += health * n;
                }
            }
            Effect::BuffTribeBoardAndHand { tribe, attack, health } => {
                let n = mul * runs as i32;
                let cards = &self.data.cards;
                for o in &mut self.players[p].board {
                    if cards.get(o.card).tribes.has(tribe) {
                        buff(o, attack * n, health * n);
                    }
                }
                for o in &mut self.players[p].hand {
                    if cards.get(o.card).tribes.has(tribe) {
                        buff(o, attack * n, health * n);
                    }
                }
            }
            Effect::DiscoverTribeIfAnother { tribe, count } => {
                let n = count.saturating_mul(mul as u8).saturating_mul(runs);
                let cards = &self.data.cards;
                let another = self.players[p]
                    .board
                    .iter()
                    .enumerate()
                    .any(|(i, m)| i != *pos && cards.get(m.card).tribes.has(tribe));
                if another {
                    for _ in 0..n {
                        let _ = self.players[p]
                            .discover_queue
                            .try_push(DiscoverRequest::TribeMinion(tribe));
                    }
                }
            }
            Effect::TriggerAdjacentBattlecries(count) => {
                let times = count.saturating_mul(mul as u8).saturating_mul(runs);
                let here = *pos;
                for neighbour in [here.checked_sub(1), Some(here + 1)].into_iter().flatten() {
                    for _ in 0..times {
                        if neighbour >= self.players[p].board.len() {
                            break;
                        }
                        self.battlecry_or_choose_one(p, neighbour);
                        if self.players[p].pending_target.is_some()
                            || self.players[p].pending_choose_one.is_some()
                        {
                            break;
                        }
                    }
                }
                self.maybe_start_discover(p);
            }
            Effect::CastTaughtSpell => {
                let Some(sp) = self.players[p].board.get(*pos).and_then(|m| m.taught_spell)
                else {
                    return;
                };
                for _ in 0..mul as u8 * runs {
                    let m = self.data.cards.make(sp, false);
                    self.cast_spell(p, m);
                    if self.players[p].pending_target.is_some()
                        || self.players[p].pending_choose_one.is_some()
                    {
                        break;
                    }
                }
                self.maybe_start_discover(p);
            }
            Effect::GetRandomTierMinion { tier } => {
                for _ in 0..mul * runs as i32 {
                    if let Some(c) =
                        self.data.cards.random_tier_minion(tier, self.lobby, &mut self.rng)
                    {
                        self.give_generated_tracking(p, c, pos);
                    }
                }
            }
            // A Rally triggered in the shop (Sky-hatch Runaway): the buffs
            // are permanent in the shop anyway.
            Effect::BuffEachType { attack, health } => {
                for _ in 0..mul * runs as i32 {
                    let cards = &self.data.cards;
                    let board = &mut self.players[p].board;
                    let tribes: ArrayVec<Tribes, BOARD> =
                        board.iter().map(|m| cards.get(m.card).tribes).collect();
                    for i in pick_one_per_type(&tribes, &mut self.rng) {
                        if !cards.get(board[i].card).fixed_stats {
                            buff(&mut board[i], attack, health);
                        }
                    }
                }
            }
            Effect::GiveOwnStatsToLeftmostHand => {
                let Some(host) = self.players[p].board.get(*pos).copied() else { return };
                let n = mul * runs as i32;
                let cards = &self.data.cards;
                let slot = self.players[p].hand.iter().position(|m| !cards.get(m.card).is_spell());
                if let Some(i) = slot {
                    buff(&mut self.players[p].hand[i], host.attack * n, host.health * n);
                }
            }
            Effect::GetLockbox { sooner } => {
                for _ in 0..runs {
                    self.give_lockbox(p, sooner.saturating_mul(mul as u8));
                }
            }
            Effect::GetRandomBounty => {
                for _ in 0..mul as u8 * runs {
                    if let Some(c) = self.data.cards.random_bounty(self.lobby, &mut self.rng) {
                        self.give_generated_tracking(p, c, pos);
                    }
                }
            }
            Effect::BuffAdjacentPerGolden { attack } => {
                // Once, and again for each friendly Golden minion.
                let golden_count = self.players[p].board.iter().filter(|m| m.golden).count() as i32;
                let n = (1 + golden_count) * runs as i32;
                let here = *pos;
                let board = &mut self.players[p].board;
                for j in [here.checked_sub(1), Some(here + 1)].into_iter().flatten() {
                    if let Some(o) = board.get_mut(j) {
                        buff(o, attack * mul * n, 0);
                    }
                }
            }
            Effect::BuffChosenPerGold { tribe, attack, health, per_gold } => {
                let g = self.players[p].gold_spent_turn as i32;
                let pt = PendingTarget {
                    source: Some(*pos as u8),
                    kind: TargetKind::MinionBuff {
                        tribe,
                        attack: (attack + per_gold.0 * g) * mul,
                        health: (health + per_gold.1 * g) * mul,
                        keyword: 0,
                        random_keyword: false,
                    },
                    runs,
                        tavern: true,
                        gem: false,
                };
                self.offer_target(p, pt);
            }
            Effect::MakeGoldenChosen { tribe, max_tier, count } => {
                let pt = PendingTarget {
                    source: Some(*pos as u8),
                    kind: TargetKind::MakeGolden { tribe, max_tier },
                    runs: count.saturating_mul(mul as u8).saturating_mul(runs),
                        tavern: true,
                        gem: false,
                };
                self.offer_target(p, pt);
            }
            Effect::GetGoldenMinion { tier, count } => {
                let n = count.saturating_mul(mul as u8).saturating_mul(runs);
                for _ in 0..n {
                    if !self.give_golden_minion(p, tier, tier, false, true) {
                        break;
                    }
                }
            }
            Effect::BuffOthersPerGolden { tribe, attack, health } => {
                let k = 1 + self.players[p].golden_played as i32;
                let cards = &self.data.cards;
                let pl = &mut self.players[p];
                for _ in 0..runs {
                    for (i, o) in pl.board.iter_mut().enumerate() {
                        if i != *pos && cards.get(o.card).is(tribe) {
                            buff(o, attack * mul * k, health * mul * k);
                        }
                    }
                }
            }
            Effect::RefreshBuff { attack, health } => {
                let n = mul * runs as i32;
                let pl = &mut self.players[p];
                pl.refresh_buff.0 += attack * n;
                pl.refresh_buff.1 += health * n;
            }
            Effect::FreeRefreshes(n) => {
                let gain = n.saturating_mul(mul as u8).saturating_mul(runs);
                self.players[p].free_rolls = self.players[p].free_rolls.saturating_add(gain);
            }
            Effect::BuffAllImproving { attack, health, counter } => {
                for _ in 0..runs {
                    let k = 1 + self.players[p].improve[counter.index()] as i32;
                    let cards = &self.data.cards;
                    for o in &mut self.players[p].board {
                        if !cards.get(o.card).fixed_stats {
                            buff(o, attack * mul * k, health * mul * k);
                        }
                    }
                    let n = &mut self.players[p].improve[counter.index()];
                    *n = n.saturating_add(1);
                }
            }
            Effect::ArmNextBuy => {
                if let Some(m) = self.players[p].board.get_mut(*pos) {
                    m.improvements = 1;
                }
            }
            Effect::GainHighestTavernStats => {
                let cards = &self.data.cards;
                let pl = &mut self.players[p];
                let mut best: Option<(i32, i32)> = None;
                let mut best_health = i32::MIN;
                for m in &pl.shop {
                    if !cards.get(m.card).is_spell() && m.health > best_health {
                        best_health = m.health;
                        best = Some((m.attack, m.health));
                    }
                }
                if let Some((a, h)) = best
                    && let Some(host) = pl.board.get_mut(*pos)
                    && !cards.get(host.card).fixed_stats
                {
                    let n = mul * runs as i32;
                    buff(host, a * n, h * n);
                }
            }
            Effect::UndeadBonus { attack, health, outside_attack } => {
                let a = if outside_attack != 0 { outside_attack } else { attack };
                let n = mul * runs as i32;
                self.add_undead_bonus(p, a * n, health * n, true);
            }
            Effect::DestroyChosenDiscover { tribe, count } => {
                let pt = PendingTarget {
                    source: Some(*pos as u8),
                    kind: TargetKind::DestroyDiscover {
                        tribe,
                        discover: count.saturating_mul(mul as u8).saturating_mul(runs),
                    },
                    runs: 1,
                        tavern: true,
                        gem: false,
                };
                self.offer_target(p, pt);
            }
            Effect::DestroyChosenGetCopy { tribe, copies } => {
                let pt = PendingTarget {
                    source: Some(*pos as u8),
                    kind: TargetKind::DestroyCopy {
                        tribe,
                        copies: copies.saturating_mul(mul as u8).saturating_mul(runs),
                    },
                    runs: 1,
                        tavern: true,
                        gem: false,
                };
                self.offer_target(p, pt);
            }
            Effect::GiveRebornDestroy { tribe, attack, health } => {
                let pt = PendingTarget {
                    source: Some(*pos as u8),
                    kind: TargetKind::RebornDestroy {
                        tribe,
                        attack: attack * mul,
                        health: health * mul,
                    },
                    runs: 1,
                    tavern: true,
                    gem: false,
                };
                self.offer_target(p, pt);
            }
            Effect::GetGems(n) => {
                let total = n.saturating_mul(mul as u8).saturating_mul(runs);
                self.give_gems(p, total);
            }
            Effect::GetGemsCastOverflow(n) => {
                let total = n.saturating_mul(mul as u8).saturating_mul(runs);
                let Some(gem) = self.data.cards.blood_gem else { return };
                for _ in 0..total {
                    if self.players[p].hand.is_full() {
                        // Cast on the left-most friendly minion instead.
                        self.play_gems_on_board(p, 0, 1);
                    } else {
                        let m = self.players[p].make_card(&self.data.cards, gem, false, 0);
                        self.add_to_hand(p, m);
                    }
                }
            }
            // A Rally triggered in the shop (Sky-hatch Runaway): the Gems are
            // permanent in the shop anyway.
            Effect::GemSelf { count, permanent: _ } => {
                let times = count.saturating_mul(mul as u8).saturating_mul(runs);
                self.play_gems_on_board(p, *pos, times);
            }
            Effect::GemOthers(count) => {
                let times = count.saturating_mul(mul as u8).saturating_mul(runs);
                let board_len = self.players[p].board.len();
                for i in 0..board_len {
                    if i != *pos {
                        self.play_gems_on_board(p, i, times);
                    }
                }
            }
            Effect::GemBonus { attack, health } => {
                let n = mul * runs as i32;
                let pl = &mut self.players[p];
                pl.gem_attack += attack * n;
                pl.gem_health += health * n;
            }
            Effect::GetRandomChooseOne(n) => {
                let total = n as i32 * mul * runs as i32;
                let tier = self.players[p].tier;
                for _ in 0..total {
                    if let Some(c) =
                        self.data.cards.random_choose_one_minion(tier, self.lobby, &mut self.rng)
                    {
                        self.give_generated_tracking(p, c, pos);
                    }
                }
            }
            // The shop copy of Jailbird Juggernaut's Rally (Sky-hatch
            // Runaway): summon the stat copy; there is no target to attack.
            Effect::SummonStatCopyAttacker { card } => {
                let Some(host) = self.players[p].board.get(*pos) else { return };
                let mut m = self.players[p].make_card(&self.data.cards, card, false, 0);
                m.attack = host.attack * mul;
                m.health = host.health * mul;
                let at = *pos + 1;
                if !self.players[p].board.is_full() && at <= self.players[p].board.len() {
                    self.players[p].board.insert(at, m);
                }
            }
            // These only happen in fights.
            Effect::GiveRebornRandom { .. }
            | Effect::SummonAttacker { .. }
            | Effect::GetPlainCopy
            | Effect::GetRandomMinionHeld { .. }
            | Effect::SummonHeld
            | Effect::DestroyLeftNeighbor
            | Effect::SummonSalvaged
            | Effect::BuffOthersPerImprovement { .. }
            | Effect::BuffAllKeepGolden { .. }
            | Effect::DamageTargetAndNeighbor
            | Effect::BuffKiller { .. }
            | Effect::BuffAttacker { .. }
            | Effect::BuffRandomOther { .. }
            | Effect::SummonRandomDeathrattle { .. }
            | Effect::TriggerDeathrattle { .. }
            | Effect::SummonFromHand { .. }
            | Effect::BuffRandomHand { .. }
            | Effect::GainTargetAttack
            | Effect::StripTargetKeywords { .. }
            | Effect::DestroyKiller
            | Effect::ShieldDragons { .. }
            | Effect::GemAttacker(_) => {}
        }
    }

    /// Replace the Tavern card in `slot` with a Fishbait and have the
    /// left-most Beast attack it.
    fn put_fishbait(&mut self, p: usize, slot: usize, card: CardId, golden: bool) {
        let Game { data, players, pool, .. } = self;
        let pl = &mut players[p];
        let old = std::mem::replace(&mut pl.shop[slot], data.cards.make(card, golden));
        give_back(pool, &data.cards, &old);
        self.tavern_attack(p, slot);
    }

    /// A Fishbait effect that picks the Tavern card: ask, unless there is
    /// only one card.
    fn start_fishbait(&mut self, p: usize, source: usize, card: CardId, golden: bool) {
        match self.players[p].shop.len() {
            0 => {}
            1 => self.put_fishbait(p, 0, card, golden),
            _ => {
                self.players[p].pending_shop_target = Some(PendingShopTarget {
                    source: source as u8,
                    kind: ShopTargetKind::Fishbait { card, golden },
                });
            }
        }
    }

    /// Refresh the Tavern for free with a Fishbait in its left-most slot.
    /// An effect that says "Refresh" is not the player's own Refresh, so it
    /// leaves Fodders queued for upcoming Refreshes alone.
    fn refresh_with_fishbait(&mut self, p: usize, card: CardId, golden: bool) {
        let Game { data, rng, players, pool, lobby, .. } = self;
        Self::refresh_shop(
            &mut players[p],
            pool,
            &data.cards,
            &data.rules,
            rng,
            *lobby,
            RefreshKind::Effect,
        );
        if !players[p].shop.is_empty() {
            self.put_fishbait(p, 0, card, golden);
        }
    }

    /// The left-most Beast on the player's board attacks the Tavern minion in
    /// `slot`. It dies, or stays if the Beast can't hurt it.
    fn tavern_attack(&mut self, p: usize, slot: usize) {
        let Game { data, rng, players, lobby, .. } = self;
        let pl = &mut players[p];
        let Some(ai) = pl.board.iter().position(|m| data.cards.get(m.card).tribes.has(Tribe::Beast))
        else {
            return;
        };
        let out = crate::fight::tavern_attack(
            &data.cards,
            &pl.board,
            ai,
            pl.shop[slot],
            pl.counters(&data.cards, *lobby),
            rng,
        );
        pl.board.clear();
        pl.board.extend(out.board);
        match out.target {
            Some(t) => pl.shop[slot] = t,
            None => {
                pl.shop.remove(slot);
            }
        }
        self.apply_side_outcome(p, &out.side, false);
    }

    /// Scarlet Survivor: once its Attack reaches the threshold, it gains the
    /// keyword (once; `improvements` records that it fired).
    fn check_attack_thresholds(&mut self, p: usize) {
        let cards = &self.data.cards;
        for m in &mut self.players[p].board {
            if m.improvements != 0 {
                continue;
            }
            let t = m.parts().find_map(|(c, _)| cards.get(c).attack_threshold);
            if let Some(t) = t
                && m.attack >= t.total as i32
            {
                m.keywords.set(t.keyword);
                m.improvements = 1;
            }
        }
    }

    /// Give the minion in board slot `pos` a random Bonus Keyword it doesn't
    /// have yet, if there is one.
    fn give_random_keyword(&mut self, p: usize, pos: usize) {
        let mask = self.data.rules.bonus_mask;
        let Some(m) = self.players[p].board.get_mut(pos) else { return };
        if let Some(k) = m.keywords.random_missing(mask, &mut self.rng) {
            m.keywords.set(k);
        }
    }

    /// Steal the `n` highest-Attack Tavern minions into hand (Decoy
    /// Conjurer); ties go to the left-most. The copies leave the pool like a
    /// buy and come back when sold.
    fn steal_highest_tavern(&mut self, p: usize, n: u8) {
        for _ in 0..n {
            if self.players[p].hand.is_full() {
                break;
            }
            let cards = &self.data.cards;
            let shop = &self.players[p].shop;
            let mut best: Option<usize> = None;
            for (i, m) in shop.iter().enumerate() {
                if cards.get(m.card).is_spell() {
                    continue;
                }
                if best.is_none_or(|b| m.attack > shop[b].attack) {
                    best = Some(i);
                }
            }
            let Some(i) = best else { break };
            let m = self.players[p].shop.remove(i);
            let card = m.card;
            self.add_to_hand(p, m);
            self.check_triple(p, card);
        }
    }

    /// Steal a random Tavern minion into hand (Enchanted Lasso).
    fn steal_random_tavern(&mut self, p: usize) {
        if self.players[p].hand.is_full() {
            return;
        }
        let cards = &self.data.cards;
        let mut ok: ArrayVec<usize, SHOP> = ArrayVec::new();
        for (i, m) in self.players[p].shop.iter().enumerate() {
            if !cards.get(m.card).is_spell() {
                let _ = ok.try_push(i);
            }
        }
        if ok.is_empty() {
            return;
        }
        let k = self.rng.below(ok.len() as u32) as usize;
        let m = self.players[p].shop.remove(ok[k]);
        let card = m.card;
        self.add_to_hand(p, m);
        self.check_triple(p, card);
    }

    /// The minion in board slot `pos` consumes `count` minions from the
    /// Tavern. A chosen one waits for the player to pick (or takes the only
    /// choice there is).
    fn start_consume(
        &mut self,
        p: usize,
        pos: usize,
        pick: ConsumePick,
        keywords: bool,
        count: u8,
        stat_mul: u8,
    ) {
        for left in (1..=count).rev() {
            let shop_len = self.players[p].shop.len();
            if shop_len == 0 || pos >= self.players[p].board.len() {
                return;
            }
            let slot = match pick {
                ConsumePick::Random => self.rng.below(shop_len as u32) as usize,
                ConsumePick::HighestHealth => {
                    let shop = &self.players[p].shop;
                    // the first of equals, so the leftmost
                    (0..shop_len).fold(0, |best, i| if shop[i].health > shop[best].health { i } else { best })
                }
                ConsumePick::Chosen if shop_len == 1 => 0,
                ConsumePick::Chosen => {
                    self.players[p].pending_shop_target = Some(PendingShopTarget {
                        source: pos as u8,
                        kind: ShopTargetKind::Consume { keywords, remaining: left, stat_mul },
                    });
                    return;
                }
            };
            self.consume(p, pos, slot, keywords, stat_mul);
        }
    }

    /// The minion in board slot `pos` takes the Tavern minion in `slot`: its
    /// stats (`stat_mul` times), and its Bonus Keywords if `keywords`. The
    /// copy goes back to the pool (a generated Fodder has no copy to return).
    fn consume(&mut self, p: usize, pos: usize, slot: usize, keywords: bool, stat_mul: u8) {
        let Game { data, players, pool, .. } = self;
        let pl = &mut players[p];
        if slot >= pl.shop.len() {
            return;
        }
        let eaten = pl.shop.remove(slot);
        give_back(pool, &data.cards, &eaten);
        let m = &mut pl.board[pos];
        let k = stat_mul.max(1) as i32;
        buff(m, eaten.attack * k, eaten.health * k);
        if keywords {
            m.keywords.0 |= eaten.keywords.0 & data.rules.bonus_mask;
        }
    }

    fn resolve_shop_target(&mut self, p: usize, slot: usize) {
        if let Some(pst) = self.players[p].pending_shop_target.take() {
            match pst.kind {
                ShopTargetKind::Consume { keywords, remaining, stat_mul } => {
                    self.consume(p, pst.source as usize, slot, keywords, stat_mul);
                    if remaining > 1 {
                        self.start_consume(
                            p,
                            pst.source as usize,
                            ConsumePick::Chosen,
                            keywords,
                            remaining - 1,
                            stat_mul,
                        );
                    }
                }
                ShopTargetKind::Fishbait { card, golden } => {
                    self.put_fishbait(p, slot, card, golden)
                }
            }
            self.maybe_start_discover(p);
            return;
        }
        let Some(pt) = self.players[p].pending_target.take() else { return };
        self.apply_target_shop(p, &pt, slot);
        self.maybe_start_discover(p);
    }

    /// Resolve a pending target that the player picked in the Tavern.
    fn apply_target_shop(&mut self, p: usize, pt: &PendingTarget, slot: usize) {
        if slot >= self.players[p].shop.len()
            || self.data.cards.get(self.players[p].shop[slot].card).is_spell()
        {
            return;
        }
        match pt.kind {
            TargetKind::AnyBuff { attack, health } => {
                for _ in 0..pt.runs {
                    buff(&mut self.players[p].shop[slot], attack, health);
                }
            }
            TargetKind::AnySetStats { attack, health } => {
                for _ in 0..pt.runs {
                    let m = &mut self.players[p].shop[slot];
                    m.attack = attack;
                    m.health = health;
                }
            }
            TargetKind::Spell(st) => {
                let target = slot;
                // A Tavern minion is not a friendly minion, so Balinda's
                // double cast does not apply to it.
                for _ in 0..1 {
                    if target >= self.players[p].shop.len()
                        || self.data.cards.get(self.players[p].shop[target].card).is_spell()
                    {
                        break;
                    }
                    let (da, dh) = if pt.tavern { self.total_spell_bonus(p) } else { (0, 0) };
                    let mut generated: Option<CardId> = None;
                    let mut stop_after = false;
                    match st {
                        SpellTarget::Buff { attack, health, keyword, times, if_win_next_turn_buff } => {
                            if pt.gem {
                                // A Blood Gem on a Tavern minion: one cast,
                                // the player's Gem value, tracked as Blood
                                // Gem stats.
                                let (ga, gh) =
                                    (self.players[p].gem_attack, self.players[p].gem_health);
                                let m = &mut self.players[p].shop[target];
                                m.attack += ga;
                                m.health += gh;
                                m.gem_attack += ga;
                                m.gem_health += gh;
                            } else {
                                let m = &mut self.players[p].shop[target];
                                for _ in 0..times {
                                    // A keyword-only spell (Sacred Gift) gets no spell bonus.
                                    let (ba, bh) = if attack == 0 && health == 0 { (0, 0) } else { (attack + da, health + dh) };
                                    buff(m, ba, bh);
                                }
                                if keyword != 0 {
                                    m.keywords.set(keyword);
                                }
                                if let Some((wb_a, wb_h)) = if_win_next_turn_buff {
                                    m.winners_bread = m.winners_bread.saturating_add(1);
                                    self.players[p].winners_bread_buff = (wb_a, wb_h);
                                }
                            }
                        }
                        SpellTarget::BuffPerBonusKeyword { attack, health } => {
                            let k = self.players[p]
                                .distinct_bonus_keywords(self.data.rules.bonus_mask)
                                as i32;
                            let m = &mut self.players[p].shop[target];
                            for _ in 0..1 + k {
                                buff(m, attack + da, health + dh);
                            }
                        }
                        SpellTarget::BuffSameTribe { attack, health } => {
                            let t = self.data.cards.get(self.players[p].shop[target].card).tribes;
                            if let Some(tribe) = t.random_type(&mut self.rng) {
                                for o in &mut self.players[p].board {
                                    if self.data.cards.get(o.card).tribes.has(tribe) {
                                        buff(o, attack + da, health + dh);
                                    }
                                }
                                if self.data.cards.get(self.players[p].shop[target].card).tribes.has(tribe) {
                                    buff(&mut self.players[p].shop[target], attack + da, health + dh);
                                }
                            }
                        }
                        SpellTarget::SameTribeMinion => {
                            let c_id = self.players[p].shop[target].card;
                            let t = self.data.cards.get(c_id).tribes;
                            let tier = self.players[p].tier;
                            generated = self.data.cards.random_different_minion(t, c_id, tier, self.lobby, &mut self.rng);
                        }
                        SpellTarget::SetStats { attack, health } => {
                            let m = &mut self.players[p].shop[target];
                            m.attack = attack;
                            m.health = health;
                        }
                        SpellTarget::TrickyTrousers { attack, health } => {
                            let m = &mut self.players[p].shop[target];
                            buff(m, attack + da, health + dh);
                            if m.keywords.has(Keywords::TAUNT) {
                                m.keywords.clear(Keywords::TAUNT);
                            } else {
                                m.keywords.set(Keywords::TAUNT);
                            }
                        }
                        SpellTarget::RobustEvolution => {
                            self.robust_evolution_shop(p, target);
                            stop_after = true;
                        }
                        SpellTarget::TavernBuffType { attack, health } => {
                            let t = self.data.cards.get(self.players[p].shop[target].card).tribes;
                            if let Some(tribe) = t.random_type(&mut self.rng) {
                                let b = TavernBuff {
                                    tribe: Some(tribe),
                                    attack: attack + da,
                                    health: health + dh,
                                    this_turn: false,
                                };
                                self.players[p].add_tavern_buff(&self.data.cards, b);
                            }
                        }
                        SpellTarget::RefreshWithType => {
                            let t = self.data.cards.get(self.players[p].shop[target].card).tribes;
                            if let Some(tribe) = t.random_type(&mut self.rng) {
                                let Game { data, rng, players, pool, lobby, .. } = self;
                                Self::refresh_with_tribe(
                                    &mut players[p],
                                    pool,
                                    &data.cards,
                                    &data.rules,
                                    rng,
                                    *lobby,
                                    tribe,
                                );
                            }
                            stop_after = true;
                        }
                        SpellTarget::Confiscation { gems } => {
                            // 3 Gems on the Tavern minion, then its
                            // neighbours' Blood Gem stats move onto it.
                            let (ga, gh) =
                                (self.players[p].gem_attack, self.players[p].gem_health);
                            {
                                let m = &mut self.players[p].shop[target];
                                m.attack += ga * gems as i32;
                                m.health += gh * gems as i32;
                                m.gem_attack += ga * gems as i32;
                                m.gem_health += gh * gems as i32;
                            }
                            for n in [target.checked_sub(1), Some(target + 1)]
                                .into_iter()
                                .flatten()
                            {
                                let (sa, sh) = match self.players[p].shop.get(n) {
                                    Some(m) if !self.data.cards.get(m.card).is_spell() => {
                                        (m.gem_attack, m.gem_health)
                                    }
                                    _ => continue,
                                };
                                if sa == 0 && sh == 0 {
                                    continue;
                                }
                                let nm = &mut self.players[p].shop[n];
                                nm.attack -= sa;
                                nm.health -= sh;
                                nm.gem_attack = 0;
                                nm.gem_health = 0;
                                let m = &mut self.players[p].shop[target];
                                m.attack += sa;
                                m.health += sh;
                                m.gem_attack += sa;
                                m.gem_health += sh;
                            }
                        }
                        // The rest target a friendly board minion only.
                        SpellTarget::Consume { .. }
                        | SpellTarget::MakeGolden { .. }
                        | SpellTarget::HalfOfHighestTavern { .. }
                        | SpellTarget::Butchering { .. }
                        | SpellTarget::Devourer => {}
                        SpellTarget::BuffPerTribePlayed { attack, health, per, .. } => {
                            let n = self.players[p].elementals_played_turn as i32;
                            buff(
                                &mut self.players[p].shop[target],
                                attack + per.0 * n + da,
                                health + per.1 * n + dh,
                            );
                        }
                    }
                    if pt.tavern {
                        self.trigger_on_cast_spell(p);
                    }
                    if !stop_after
                        && target < self.players[p].shop.len()
                        && self.data.cards.get(self.players[p].shop[target].card).tribes.has(Tribe::Mech)
                    {
                        self.glambot_on_shop_mech(p, target);
                    }
                    if let Some(gen_c) = generated {
                        let mut dummy = 0;
                        self.give_generated_tracking(p, gen_c, &mut dummy);
                    }
                    if stop_after {
                        break;
                    }
                }
            }
            _ => {}
        }
    }

    /// Robust Evolution on a board minion: a random pool minion exactly one
    /// tier higher (a tier 6 into another tier 6), keeping only its Attack
    /// and Health. The old copy goes back to the pool, the new comes from it.
    fn robust_evolution_board(&mut self, p: usize, slot: usize) {
        let old = self.players[p].board[slot];
        let Some(new_id) = self.robust_evolution_draw(p, old) else { return };
        let mut m = self.data.cards.make(new_id, false);
        m.pool_copies = 1;
        m.attack = old.attack;
        m.health = old.health;
        self.players[p].board[slot] = m;
    }

    fn robust_evolution_shop(&mut self, p: usize, slot: usize) {
        let old = self.players[p].shop[slot];
        let Some(new_id) = self.robust_evolution_draw(p, old) else { return };
        let mut m = self.data.cards.make(new_id, false);
        m.pool_copies = 1;
        m.attack = old.attack;
        m.health = old.health;
        self.players[p].shop[slot] = m;
    }

    /// Return the old copies to the pool and draw the new one. The result is
    /// never golden; a golden old minion keeps its Attack and Health and
    /// returns its copies (an assumption, see DESIGN.md).
    fn robust_evolution_draw(&mut self, _p: usize, old: Minion) -> Option<CardId> {
        let old_tier = self.data.cards.get(old.card).tier;
        // Tier 6 and tier 7 minions stay on their tier: no minion is ever
        // evolved up to tier 7.
        let new_tier = if old_tier >= MAX_TIER { old_tier } else { old_tier + 1 };
        let new_id = draw_filtered(
            &mut self.pool,
            &self.data.cards,
            &mut self.rng,
            new_tier,
            new_tier,
            &[],
            self.lobby,
            |_| true,
        )?;
        let (slot, copies, token) = {
            let c = self.data.cards.get(old.card);
            (c.pool_slot, old.pool_copies, c.token || c.is_spell())
        };
        if copies > 0 && !token {
            self.pool[slot as usize] += copies;
        }
        Some(new_id)
    }

    /// Channel the Devourer: sell the chosen board minion (a normal sale)
    /// and give its stats to a random other friendly minion.
    fn channel_the_devourer(&mut self, p: usize, slot: usize) {
        if slot >= self.players[p].board.len() {
            return;
        }
        let stats = (self.players[p].board[slot].attack, self.players[p].board[slot].health);
        self.sell_minion(p, slot);
        let n = self.players[p].board.len();
        if n == 0 {
            return;
        }
        let k = self.rng.below(n as u32) as usize;
        buff(&mut self.players[p].board[k], stats.0, stats.1);
    }

    /// Run every event trigger of `kind` on the player's board. Each adds
    /// `amount` to its count and fires once for every full `every` in it,
    /// keeping the remainder.
    fn fire_event(&mut self, p: usize, kind: EventKind, amount: u32) {
        self.fire_event_skipping(p, kind, amount, None);
    }

    /// `fire_event`, leaving out the board minion in slot `skip` (a card
    /// never counts its own play).
    fn fire_event_skipping(&mut self, p: usize, kind: EventKind, amount: u32, skip: Option<usize>) {
        if amount == 0 {
            return;
        }
        if kind == EventKind::GoldSpent {
            let pl = &mut self.players[p];
            pl.gold_spent_turn = pl.gold_spent_turn.saturating_add(amount.min(u16::MAX as u32) as u16);
        }
        if kind == EventKind::SpellsCast {
            self.players[p].spells_cast = self.players[p].spells_cast.saturating_add(amount);
        }
        let mut pos = 0;
        while pos < self.players[p].board.len() {
            if skip == Some(pos) {
                pos += 1;
                continue;
            }
            let m = self.players[p].board[pos];
            for (card, golden) in m.parts() {
                let Some(t) = self.data.cards.get(card).on_event else { continue };
                if t.kind != kind || pos >= self.players[p].board.len() {
                    continue;
                }
                let imp = &mut self.players[p].board[pos].improvements;
                // An every-1 trigger has no remainder to keep, so it leaves
                // `improvements` alone: an effect that improves itself on
                // every fire (Harbinger Aph'lass) stores its level there.
                let mut count = if t.every == 1 { amount } else { *imp as u32 + amount };
                let mut fires = 0;
                while count >= t.every as u32 {
                    count -= t.every as u32;
                    fires += 1;
                }
                if t.every > 1 {
                    *imp = count as u16;
                }
                for _ in 0..fires {
                    self.run_effect(p, &mut pos, t.effect, golden, 1);
                }
            }
            pos += 1;
        }
    }

    /// The hero takes `amount` damage, then the "after your hero takes
    /// damage" effects and the damage event triggers run. They fire even
    /// when a rewind among them gives the Health back. Combat damage doesn't
    /// come through here (no minion reacts to it) and is settled by
    /// `knockouts` after the fights; Health costs are checked by
    /// `can_pay_health` before they're paid, so they can't kill.
    ///
    /// `source` is the board slot of the friendly Demon that dealt it, if
    /// any: a Health cost (Malchezaar) or the combat damage to the hero is
    /// not a Demon dealing damage, so they pass `None`.
    fn take_hero_damage(&mut self, p: usize, amount: i32, source: Option<usize>) {
        let pl = &mut self.players[p];
        if amount <= 0 {
            return;
        }
        pl.health -= amount;
        pl.last_damage = amount;
        self.note_shop_demon_damage(p, source);
        let mut pos = 0;
        while pos < self.players[p].board.len() {
            let m = self.players[p].board[pos];
            for (card, golden) in m.parts() {
                for k in 0..self.data.cards.get(card).after_hero_damage.len() {
                    if pos >= self.players[p].board.len() {
                        break;
                    }
                    let e = self.data.cards.get(card).after_hero_damage[k];
                    self.run_effect(p, &mut pos, e, golden, 1);
                }
            }
            pos += 1;
        }
        self.fire_event(p, EventKind::HeroDamage, amount as u32);
    }

    /// Damage to the hero in the shop phase (Wrath Weaver, Imposing
    /// Percussionist). Once its reactions have run, a hero at 0 Health or
    /// below is knocked out on the spot: a rewind on the board has already
    /// had its chance to give the Health back.
    fn take_shop_damage(&mut self, p: usize, amount: i32, source: Option<usize>) {
        self.take_hero_damage(p, amount, source);
        if self.players[p].alive && self.players[p].health <= 0 {
            self.knock_out_mid_turn(p);
        }
    }

    /// Knock `p` out in the middle of its turn. It takes the worst place
    /// still open, gives its cards back and stops acting; its board becomes
    /// the ghost, and the players still alive are paired again without it.
    fn knock_out_mid_turn(&mut self, p: usize) {
        let place = self.alive() as u8;
        {
            let Game { data, players, ghost, lobby, .. } = self;
            let pl = &mut players[p];
            pl.alive = false;
            pl.placement = place;
            pl.final_tribes = pl.board_tribes(&data.cards);
            *ghost = Some(Ghost {
                player: p as u8,
                board: pl.board.clone(),
                tier: pl.tier,
                counters: pl.counters(&data.cards, *lobby),
            });
            pl.opponent = None;
            pl.done = true;
        }
        // The cards go back at the end of the action: effects that were
        // already running may still touch this board.
        let left: ArrayVec<u8, PLAYERS> =
            (0..PLAYERS as u8).filter(|&q| self.players[q as usize].alive).collect();
        if left.len() <= 1 {
            for &q in &left {
                let cards = &self.data.cards;
                let pl = &mut self.players[q as usize];
                pl.placement = 1;
                pl.final_tribes = pl.board_tribes(cards);
            }
            self.over = true;
            return;
        }
        self.pair();
    }

    /// A dead player holds no cards. Effects that were still running when
    /// the hero died can give it some, so the end of every action gives
    /// them back and clears anything waiting on a choice.
    fn settle_dead(&mut self, p: usize) {
        let Game { data, players, pool, .. } = self;
        let pl = &mut players[p];
        if pl.alive {
            return;
        }
        let idle = pl.shop.is_empty()
            && pl.hand.is_empty()
            && pl.board.is_empty()
            && pl.discover.is_empty()
            && pl.shop_spell.is_none()
            && pl.discover_queue.is_empty()
            && pl.pending_target.is_none()
            && pl.pending_shop_target.is_none()
            && pl.pending_choose_one.is_none()
            && pl.pending_discard.is_none()
            && pl.choose_queue.is_empty();
        if idle {
            return;
        }
        pl.shop_spell = None;
        for m in pl.shop.drain(..).chain(pl.hand.drain(..)).chain(pl.board.drain(..)) {
            give_back(pool, &data.cards, &m);
        }
        for c in pl.discover.drain(..) {
            if !data.cards.get(c).is_spell() {
                let slot = data.cards.get(c).pool_slot as usize;
                pool[slot] += 1;
            }
        }
        pl.pending_choose_one = None;
        pl.pending_target = None;
        pl.pending_shop_target = None;
        pl.pending_discard = None;
        pl.choose_queue.clear();
        pl.discover_damage_from = None;
        pl.discover_locked = false;
        pl.discover_queue.clear();
        pl.tavern_buffs.clear();
    }

    /// A friendly Demon dealt damage in the shop phase: every other friendly
    /// minion with "After another friendly Demon deals damage" gains its
    /// stats for good right away (Devout Hellcaller). A damage source that
    /// is no longer a friendly Demon does not count.
    fn note_shop_demon_damage(&mut self, p: usize, source: Option<usize>) {
        let Some(src) = source else { return };
        let cards = &self.data.cards;
        let Some(src_card) = self.players[p].board.get(src).map(|m| m.card) else { return };
        if !cards.get(src_card).tribes.has(Tribe::Demon) {
            return;
        }
        let mut hits: ArrayVec<(usize, i32, i32), BOARD> = ArrayVec::new();
        for (i, m) in self.players[p].board.iter().enumerate() {
            if i == src {
                continue;
            }
            for (card, golden) in m.parts() {
                if let Some((a, h)) = cards.get(card).after_demon_damage {
                    let mul = if golden { 2 } else { 1 };
                    let _ = hits.try_push((i, a * mul, h * mul));
                }
            }
        }
        for (i, da, dh) in hits {
            buff(&mut self.players[p].board[i], da, dh);
        }
    }

    fn offer_target(&mut self, p: usize, pt: PendingTarget) {
        let mut board: ArrayVec<usize, BOARD> = ArrayVec::new();
        for s in 0..self.players[p].board.len() {
            if self.valid_target(p, &pt, s) {
                let _ = board.try_push(s);
            }
        }
        let mut shop: ArrayVec<usize, SHOP> = ArrayVec::new();
        if pt.kind.takes_shop() {
            for s in 0..self.players[p].shop.len() {
                if self.valid_shop_target(p, &pt, s) {
                    let _ = shop.try_push(s);
                }
            }
        }
        if board.len() + shop.len() != 1 {
            if board.is_empty() && shop.is_empty() {
                return;
            }
            self.players[p].pending_target = Some(pt);
            return;
        }
        if let Some(&s) = board.first() {
            self.apply_target(p, &pt, s);
        } else if let Some(&s) = shop.first() {
            self.apply_target_shop(p, &pt, s);
        }
    }

    fn apply_target(&mut self, p: usize, pt: &PendingTarget, s: usize) {
        match pt.kind {
            TargetKind::MinionBuff { attack, health, keyword, random_keyword, .. } => {
                for _ in 0..pt.runs {
                    let m = &mut self.players[p].board[s];
                    buff(m, attack, health);
                    if keyword != 0 {
                        m.keywords.set(keyword);
                    }
                    if random_keyword {
                        self.give_random_keyword(p, s);
                    }
                }
            }
            TargetKind::MakeGolden { .. } => {
                // A minion made Golden on the board gives no Triple Reward
                // and doesn't count as a Golden minion played.
                golden_up(&self.data.cards, &mut self.players[p].board[s]);
                if pt.runs > 1 {
                    let again = PendingTarget { runs: pt.runs - 1, ..*pt };
                    self.offer_target(p, again);
                }
            }
            TargetKind::AnyBuff { attack, health } => {
                for _ in 0..pt.runs {
                    buff(&mut self.players[p].board[s], attack, health);
                }
            }
            TargetKind::DestroyDiscover { tribe, discover } => {
                self.destroy_in_shop(p, &[s], None);
                for _ in 0..discover {
                    let _ = self.players[p]
                        .discover_queue
                        .try_push(DiscoverRequest::TribeMinion(tribe));
                }
            }
            TargetKind::DestroyCopy { copies, .. } => {
                let card = self.players[p].board[s].card;
                self.destroy_in_shop(p, &[s], None);
                for _ in 0..copies {
                    self.give_generated(p, card);
                }
            }
            TargetKind::RebornDestroy { attack, health, .. } => {
                let host = pt.source.map(|x| x as usize);
                self.players[p].board[s].keywords.set(Keywords::REBORN);
                let tracked = self.destroy_in_shop(p, &[s], host);
                if let Some(h) = tracked {
                    buff(&mut self.players[p].board[h], attack, health);
                }
            }
            TargetKind::AnySetStats { attack, health } => {
                for _ in 0..pt.runs {
                    let m = &mut self.players[p].board[s];
                    m.attack = attack;
                    m.health = health;
                }
            }
            TargetKind::Consume { pick, keywords, count, stat_mul, .. } => {
                self.start_consume(p, s, pick, keywords, count, stat_mul);
            }
            TargetKind::TriggerRally => {
                for _ in 0..pt.runs {
                    let Some(m) = self.players[p].board.get(s).copied() else { break };
                    for (card, golden) in m.parts() {
                        let Some(e) = self.data.cards.get(card).rally else { continue };
                        let mut pos = s;
                        self.run_effect(p, &mut pos, e, golden, 1);
                    }
                }
            }
            TargetKind::TriggerBattlecry => {
                for _ in 0..pt.runs {
                    if s >= self.players[p].board.len() {
                        break;
                    }
                    self.battlecry_or_choose_one(p, s);
                    // A targeting Battlecry or a Choose One waits for its
                    // own choice; don't run it again.
                    if self.players[p].pending_target.is_some()
                        || self.players[p].pending_choose_one.is_some()
                    {
                        break;
                    }
                }
                self.maybe_start_discover(p);
            }
            TargetKind::Spell(st) => {
                let mut target_pos = s;
                for k in 0..pt.runs {
                    if target_pos >= self.players[p].board.len() {
                        break;
                    }
                    // Every extra cast (Balinda) is another spell cast. Only a
                    // Tavern spell counts (a Blood Gem does not).
                    if k > 0 && pt.tavern {
                        self.fire_event(p, EventKind::SpellsCast, 1);
                        if target_pos >= self.players[p].board.len() {
                            break;
                        }
                    }
                    let (da, dh) = if pt.tavern { self.total_spell_bonus(p) } else { (0, 0) };
                    let mut generated: Option<CardId> = None;
                    // A spell that removes or transforms its target can't
                    // usefully be cast again on the same slot (Balinda).
                    let mut stop_after = false;
                    let mut target_removed = false;
                    match st {
                        SpellTarget::Buff {
                            attack,
                            health,
                            keyword,
                            times,
                            if_win_next_turn_buff,
                        } => {
                            if pt.gem {
                                // A Blood Gem: the player's Gem value, no
                                // spell power, tracked as Blood Gem stats.
                                let (ga, gh) =
                                    (self.players[p].gem_attack, self.players[p].gem_health);
                                let m = &mut self.players[p].board[target_pos];
                                m.attack += ga;
                                m.health += gh;
                                m.gem_attack += ga;
                                m.gem_health += gh;
                            } else {
                                let m = &mut self.players[p].board[target_pos];
                                for _ in 0..times {
                                    // A keyword-only spell (Sacred Gift) gets no spell bonus.
                                    let (ba, bh) = if attack == 0 && health == 0 { (0, 0) } else { (attack + da, health + dh) };
                                    buff(m, ba, bh);
                                }
                                if keyword != 0 {
                                    m.keywords.set(keyword);
                                }
                                if let Some((wb_a, wb_h)) = if_win_next_turn_buff {
                                    m.winners_bread = m.winners_bread.saturating_add(1);
                                    self.players[p].winners_bread_buff = (wb_a, wb_h);
                                }
                            }
                        }
                        SpellTarget::BuffPerBonusKeyword { attack, health } => {
                            let k = self.players[p]
                                .distinct_bonus_keywords(self.data.rules.bonus_mask)
                                as i32;
                            let m = &mut self.players[p].board[target_pos];
                            for _ in 0..1 + k {
                                buff(m, attack + da, health + dh);
                            }
                        }
                        SpellTarget::BuffSameTribe { attack, health } => {
                            let t = self.data.cards.get(self.players[p].board[target_pos].card).tribes;
                            // DESIGN.md "Types": a dual or all-type minion
                            // uses one of its types, picked at random.
                            if let Some(tribe) = t.random_type(&mut self.rng) {
                                for o in &mut self.players[p].board {
                                    if self.data.cards.get(o.card).tribes.has(tribe) {
                                        buff(o, attack + da, health + dh);
                                    }
                                }
                            }
                        }
                        SpellTarget::SameTribeMinion => {
                            let c_id = self.players[p].board[target_pos].card;
                            let t = self.data.cards.get(c_id).tribes;
                            let tier = self.players[p].tier;
                            generated = self.data.cards.random_different_minion(
                                t,
                                c_id,
                                tier,
                                self.lobby,
                                &mut self.rng,
                            );
                        }
                        SpellTarget::SetStats { attack, health } => {
                            let m = &mut self.players[p].board[target_pos];
                            m.attack = attack;
                            m.health = health;
                        }
                        SpellTarget::Consume { pick, keywords, count, .. } => {
                            self.start_consume(p, target_pos, pick, keywords, count, 1);
                        }
                        SpellTarget::MakeGolden { .. } => {
                            let m = &mut self.players[p].board[target_pos];
                            if !m.golden {
                                let base = self.data.cards.get(m.card);
                                let (ga, gh) = base
                                    .golden_stats
                                    .unwrap_or((base.attack * 2, base.health * 2));
                                m.golden = true;
                                m.attack += ga - base.attack;
                                m.health += gh - base.health;
                            }
                        }
                        SpellTarget::TrickyTrousers { attack, health } => {
                            let m = &mut self.players[p].board[target_pos];
                            buff(m, attack + da, health + dh);
                            if m.keywords.has(Keywords::TAUNT) {
                                m.keywords.clear(Keywords::TAUNT);
                            } else {
                                m.keywords.set(Keywords::TAUNT);
                            }
                        }
                        SpellTarget::RobustEvolution => {
                            // Balinda casts it again on the minion the first
                            // cast produced, in the same slot.
                            self.robust_evolution_board(p, target_pos);
                        }
                        SpellTarget::TavernBuffType { attack, health } => {
                            let t = self.data.cards.get(self.players[p].board[target_pos].card).tribes;
                            if let Some(tribe) = t.random_type(&mut self.rng) {
                                let b = TavernBuff {
                                    tribe: Some(tribe),
                                    attack: attack + da,
                                    health: health + dh,
                                    this_turn: false,
                                };
                                self.players[p].add_tavern_buff(&self.data.cards, b);
                            }
                        }
                        SpellTarget::RefreshWithType => {
                            let t = self.data.cards.get(self.players[p].board[target_pos].card).tribes;
                            if let Some(tribe) = t.random_type(&mut self.rng) {
                                let Game { data, rng, players, pool, lobby, .. } = self;
                                Self::refresh_with_tribe(
                                    &mut players[p],
                                    pool,
                                    &data.cards,
                                    &data.rules,
                                    rng,
                                    *lobby,
                                    tribe,
                                );
                            }
                            // Balinda casts it again: the Tavern refreshes
                            // twice and both casts' on-cast triggers fire.
                        }
                        SpellTarget::Devourer => {
                            self.channel_the_devourer(p, target_pos);
                            stop_after = true;
                            target_removed = true;
                        }
                        SpellTarget::HalfOfHighestTavern { .. } => {
                            // Half, rounded up, of the highest-Health Tavern
                            // minion. An empty Tavern gives nothing.
                            if let Some((a, h)) = self.highest_tavern_stats(p) {
                                buff(
                                    &mut self.players[p].board[target_pos],
                                    (a + 1) / 2 + da,
                                    (h + 1) / 2 + dh,
                                );
                            }
                        }
                        SpellTarget::BuffPerTribePlayed { attack, health, per, .. } => {
                            let n = self.players[p].elementals_played_turn as i32;
                            buff(
                                &mut self.players[p].board[target_pos],
                                attack + per.0 * n + da,
                                health + per.1 * n + dh,
                            );
                        }
                        SpellTarget::Butchering { attack } => {
                            // The bonus lands first, so a Reborn copy comes
                            // back with it. Like Channel the Devourer, a
                            // second cast (Balinda) does nothing: its
                            // target is gone.
                            self.add_undead_bonus(p, attack + da, dh, true);
                            self.destroy_in_shop(p, &[target_pos], None);
                            stop_after = true;
                            target_removed = true;
                        }
                        SpellTarget::Confiscation { gems } => {
                            // 3 Gems on the target, then the neighbours'
                            // Blood Gem stats move onto it: they lose them,
                            // it gains them.
                            let (ga, gh) =
                                (self.players[p].gem_attack, self.players[p].gem_health);
                            {
                                let m = &mut self.players[p].board[target_pos];
                                m.attack += ga * gems as i32;
                                m.health += gh * gems as i32;
                                m.gem_attack += ga * gems as i32;
                                m.gem_health += gh * gems as i32;
                            }
                            for n in [target_pos.checked_sub(1), Some(target_pos + 1)]
                                .into_iter()
                                .flatten()
                            {
                                let (sa, sh) = match self.players[p].board.get(n) {
                                    Some(m) => (m.gem_attack, m.gem_health),
                                    None => continue,
                                };
                                if sa == 0 && sh == 0 {
                                    continue;
                                }
                                let nm = &mut self.players[p].board[n];
                                nm.attack -= sa;
                                nm.health -= sh;
                                nm.gem_attack = 0;
                                nm.gem_health = 0;
                                let m = &mut self.players[p].board[target_pos];
                                m.attack += sa;
                                m.health += sh;
                                m.gem_attack += sa;
                                m.gem_health += sh;
                            }
                        }
                    }
                    if pt.tavern {
                        self.trigger_on_cast_spell(p);
                    }
                    if !target_removed && target_pos < self.players[p].board.len() {
                        self.trigger_on_cast_spell_on_target(p, target_pos);
                        if pt.gem {
                            // A Gem was played from hand on the target.
                            self.roogug_on_gem(p, target_pos);
                        }
                        let is_mech = self
                            .data
                            .cards
                            .get(self.players[p].board[target_pos].card)
                            .tribes
                            .has(Tribe::Mech);
                        if is_mech {
                            self.trigger_on_cast_spell_on_mech(p, target_pos);
                        }
                    }
                    if let Some(gen_c) = generated {
                        self.give_generated_tracking(p, gen_c, &mut target_pos);
                    }
                    if stop_after {
                        break;
                    }
                }
            }
        }
    }

    fn resolve_target(&mut self, p: usize, s: usize) {
        let pt = self.players[p].pending_target.take().unwrap();
        self.apply_target(p, &pt, s);
        self.maybe_start_discover(p);
    }

    fn maybe_start_discover(&mut self, p: usize) {
        loop {
            {
                let pl = &self.players[p];
                if pl.pending_target.is_some()
                    || pl.pending_shop_target.is_some()
                    || pl.pending_choose_one.is_some()
                    || pl.pending_discard.is_some()
                    || !pl.discover.is_empty()
                {
                    return;
                }
            }
            // A both-effects Choose One option waiting for the one before it.
            if let Some(q) = self.players[p].choose_queue.first().copied() {
                self.players[p].choose_queue.remove(0);
                self.apply_choose_option(p, q.source, q.option, q.tavern);
                continue;
            }
            if self.players[p].discover_queue.is_empty() {
                return;
            }
            let req = self.players[p].discover_queue.remove(0);
            let damage_from = match req {
                DiscoverRequest::DemonMinion { source, .. } => Some(source),
                _ => None,
            };
            let locked = matches!(req, DiscoverRequest::SearchMinionTier(_));
            let doomed = matches!(req, DiscoverRequest::DoomedTribeMinion(_));
            let both = matches!(req, DiscoverRequest::ChooseOneMinion);
            let Game { data, rng, players, pool, lobby, .. } = self;
            let pl = &mut players[p];
            pl.discover_damage_from = damage_from;
            pl.discover_locked = locked;
            pl.discover_doomed = doomed;
            pl.discover_both = both;
            while !pl.discover.is_full() {
                let next = match req {
                    DiscoverRequest::MinionTier(t)
                    | DiscoverRequest::SearchMinionTier(t) => {
                        draw_filtered(pool, &data.cards, rng, t, t, &pl.discover, *lobby, |_| true)
                    }
                    DiscoverRequest::SpellTier(t) => {
                        data.cards.random_tier_spell(t, &pl.discover, *lobby, rng)
                    }
                    DiscoverRequest::SpellMaxTier(t) => {
                        data.cards.random_spell_up_to(t, &pl.discover, *lobby, rng)
                    }
                    DiscoverRequest::DeathrattleMinion(max_t) => draw_filtered(
                        pool,
                        &data.cards,
                        rng,
                        1,
                        max_t,
                        &pl.discover,
                        *lobby,
                        |c| c.deathrattle.is_some(),
                    ),
                    DiscoverRequest::BattlecryMinion(max_t) => draw_filtered(
                        pool,
                        &data.cards,
                        rng,
                        1,
                        max_t,
                        &pl.discover,
                        *lobby,
                        |c| c.battlecry.is_some(),
                    ),
                    DiscoverRequest::ChooseOneMinion => draw_filtered(
                        pool,
                        &data.cards,
                        rng,
                        1,
                        pl.tier,
                        &pl.discover,
                        *lobby,
                        |c| c.choose_one.is_some(),
                    ),
                    DiscoverRequest::TribeMinion(t) | DiscoverRequest::DoomedTribeMinion(t) => {
                        // a "Discover a Murloc" only offers up to the tier
                        draw_filtered(pool, &data.cards, rng, 1, pl.tier, &pl.discover, *lobby, |c| {
                            c.tribes.has(t)
                        })
                    }
                    DiscoverRequest::DemonMinion { max_tier: max_t, .. } => draw_filtered(
                        pool,
                        &data.cards,
                        rng,
                        1,
                        max_t,
                        &pl.discover,
                        *lobby,
                        |c| c.tribes.has(Tribe::Demon),
                    ),
                };
                match next {
                    Some(c) => pl.discover.push(c),
                    None => break,
                }
            }
        }
    }

    fn resolve_choose(&mut self, p: usize, k: usize) {
        if let Some(pco) = self.players[p].pending_choose_one.take() {
            self.apply_choose_option(p, pco.source, pco.options[k], pco.tavern);
            // A minion's Choose One is its Battlecry.
            if pco.source.is_some() {
                self.fire_after_battlecry(p, 1);
            }
            self.maybe_start_discover(p);
            return;
        }
        let Game { data, players, pool, .. } = self;
        let pl = &mut players[p];
        let locked = pl.discover_locked;
        pl.discover_locked = false;
        let doomed = std::mem::take(&mut pl.discover_doomed);
        let both = std::mem::take(&mut pl.discover_both);
        let c = pl.discover[k];
        for (i, &o) in pl.discover.iter().enumerate() {
            if i != k && !data.cards.get(o).is_spell() {
                let slot = data.cards.get(o).pool_slot as usize;
                pool[slot] += 1;
            }
        }
        pl.discover.clear();
        // Imposing Percussionist: the hero takes damage equal to the Tier of
        // the Demon chosen.
        let damage = pl
            .discover_damage_from
            .take()
            .map(|source| (source, data.cards.get(c).tier as i32));
        let is_spell = data.cards.get(c).is_spell();
        if !pl.hand.is_full() {
            let pool_copies = if is_spell { 0 } else { 1 };
            let mut m = pl.make_card(&data.cards, c, false, pool_copies);
            if locked && !is_spell {
                m.locked_turns = 1;
            }
            if doomed && !is_spell {
                m.doomed = true;
            }
            if both && !is_spell {
                m.both_effects = true;
            }
            self.add_to_hand(p, m);
            if !is_spell {
                self.check_triple(p, c);
            }
        } else if !is_spell {
            let slot = self.data.cards.get(c).pool_slot as usize;
            self.pool[slot] += 1;
        }
        if let Some((source, d)) = damage {
            self.take_shop_damage(p, d, Some(source as usize));
        }
        self.fire_after_discover(p);
        self.maybe_start_discover(p);
    }

    /// Draw a Golden minion for the player (from the pool, three copies) and
    /// put it in hand. `no_reward` marks it as giving no Triple Reward.
    /// False when the hand is full or nothing is left.
    fn give_golden_minion(&mut self, p: usize, lo: u8, hi: u8, typed: bool, no_reward: bool) -> bool {
        if self.players[p].hand.is_full() {
            return false;
        }
        let Some(m) = self.draw_golden_minion(p, lo, hi, typed, no_reward) else { return false };
        self.add_to_hand(p, m);
        true
    }

    fn draw_golden_minion(&mut self, p: usize, lo: u8, hi: u8, typed: bool, no_reward: bool) -> Option<Minion> {
        let Game { data, rng, players, pool, lobby, .. } = self;
        let pl = &mut players[p];
        let (id, copies) = draw_golden(pool, &data.cards, rng, lo, hi, typed, *lobby)?;
        let mut m = pl.make_card(&data.cards, id, true, copies);
        m.no_reward |= no_reward;
        Some(m)
    }

    /// Get a Lockbox. With one already in hand it opens `sooner` turns
    /// sooner instead (and opens now if that takes it to 0). With a full
    /// hand and no Lockbox the new one is lost.
    fn give_lockbox(&mut self, p: usize, sooner: u8) {
        let cards = &self.data.cards;
        let Some(id) = cards.lockbox else { return };
        let pl = &mut self.players[p];
        if let Some(i) = pl.hand.iter().position(|m| m.card == id) {
            let left = pl.hand[i].improvements.saturating_sub(sooner as u16);
            pl.hand[i].improvements = left;
            if left == 0 {
                self.open_lockbox(p, i);
            }
            return;
        }
        if pl.hand.is_full() {
            return;
        }
        let m = pl.make_card(cards, id, false, 0);
        self.add_to_hand(p, m);
    }

    /// The Lockbox in hand slot `i` breaks open: it becomes a random Golden
    /// minion with a type, up to the player's tier, taken from the pool.
    fn open_lockbox(&mut self, p: usize, i: usize) {
        let tier = self.players[p].tier;
        match self.draw_golden_minion(p, 1, tier, true, false) {
            Some(m) => self.players[p].hand[i] = m,
            None => {
                self.players[p].hand.remove(i);
            }
        }
    }

    /// End of the owner's turn: every Lockbox in hand counts down `times`
    /// (Drakkari doubles it) and opens at 0.
    fn tick_lockboxes(&mut self, p: usize, times: u8) {
        let Some(id) = self.data.cards.lockbox else { return };
        for _ in 0..times {
            let mut i = 0;
            while i < self.players[p].hand.len() {
                if self.players[p].hand[i].card == id {
                    let m = &mut self.players[p].hand[i];
                    m.improvements = m.improvements.saturating_sub(1);
                    if m.improvements == 0 {
                        let before = self.players[p].hand.len();
                        self.open_lockbox(p, i);
                        if self.players[p].hand.len() < before {
                            continue;
                        }
                    }
                }
                i += 1;
            }
        }
    }

    /// After the player Discovers a card: Hooktusk and its like run.
    fn fire_after_discover(&mut self, p: usize) {
        let mut pos = 0;
        while pos < self.players[p].board.len() {
            let m = self.players[p].board[pos];
            for (card, golden) in m.parts() {
                if let Some(e) = self.data.cards.get(card).after_discover
                    && pos < self.players[p].board.len()
                {
                    self.run_effect(p, &mut pos, e, golden, 1);
                }
            }
            pos += 1;
        }
    }

    /// Get a random tier 7 minion, as a generated card (no pool copy), for
    /// "Get a Tier 7 minion". False when the hand is full or none exists.
    /// Nothing else in the lobby makes a tier 7 minion; the Pirate run's
    /// Greedy Conniver is the first card that will use this.
    pub fn give_tier7_minion(&mut self, p: usize) -> bool {
        let Some(c) = self.data.cards.random_tier7_minion(self.lobby, &mut self.rng) else {
            return false;
        };
        if self.players[p].hand.is_full() {
            return false;
        }
        self.give_generated(p, c);
        true
    }

    /// Queue "Discover a Tier 7 minion": three different tier 7 minions from
    /// the pool, the pick takes its copy (the same as every Discover) and
    /// the rest go back. Opens at once unless a choice is already open.
    pub fn discover_tier7_minion(&mut self, p: usize) {
        let _ = self.players[p]
            .discover_queue
            .try_push(DiscoverRequest::MinionTier(crate::CARD_MAX_TIER));
        self.maybe_start_discover(p);
    }

    fn give_generated(&mut self, p: usize, card: CardId) {
        let mut dummy = 0;
        self.give_generated_tracking(p, card, &mut dummy);
    }

    fn add_to_hand(&mut self, p: usize, m: Minion) -> bool {
        if self.players[p].hand.is_full() {
            return false;
        }
        self.players[p].hand.push(m);
        for i in 0..self.players[p].board.len() {
            let board = self.players[p].board[i];
            for (card, golden) in board.parts() {
                if let Some((attack, health)) = self.data.cards.get(card).on_card_added {
                    let mul = if golden { 2 } else { 1 };
                    if let Some(deity) = &mut self.players[p].deity {
                        deity.attack += attack * mul;
                        deity.health += health * mul;
                    }
                }
            }
        }
        true
    }

    fn give_linked(&mut self, p: usize, cards: &[CardId], golden: bool) {
        if cards.is_empty() {
            return;
        }
        let id = self.players[p].next_link & 0x3fff;
        self.players[p].next_link = if id == 0x3fff { 1 } else { id + 1 };
        let linked = id | if golden { 0x8000 } else { 0 };
        for &card in cards {
            let mut m = self.players[p].make_card(&self.data.cards, card, false, 0);
            m.linked = linked;
            if self.add_to_hand(p, m) && !self.data.cards.get(card).is_spell() {
                self.check_triple(p, card);
            }
        }
    }

    fn give_generated_tracking(&mut self, p: usize, card: CardId, tracked_pos: &mut usize) {
        let m = self.players[p].make_card(&self.data.cards, card, false, 0);
        if self.add_to_hand(p, m) && !self.data.cards.get(card).is_spell() {
            self.check_triple_tracking(p, card, tracked_pos);
        }
    }

    /// Three copies of a minion in hand and on board become a golden one in
    /// hand, with double printed stats plus the buffs and attachments of all
    /// three. Spells never triple.
    fn check_triple(&mut self, p: usize, card: CardId) {
        let mut dummy = 0;
        self.check_triple_tracking(p, card, &mut dummy);
    }

    fn check_triple_tracking(&mut self, p: usize, card: CardId, tracked_pos: &mut usize) {
        let cards = &self.data.cards;
        if cards.get(card).is_spell() || cards.get(card).no_triple {
            return;
        }
        let pl = &self.players[p];
        // Plain copies in hand and on the board; a Golden minion never counts.
        let copies = |id: CardId| {
            pl.hand.iter().chain(&pl.board).filter(|m| m.card == id && !m.golden).count()
        };
        // Which card turns Golden and which cards it is made of: `x` of its
        // own copies and `w` of one wildcard card (Elemental of Surprise
        // stands in for any Elemental). Three plain copies come first.
        // A stand-in counts even when it is Golden (the only card that
        // triples while already golden).
        let stand_ins = |id: CardId| {
            pl.hand.iter().chain(&pl.board).filter(|m| m.card == id).count()
        };
        let wild_of = |id: CardId| cards.get(id).triple_wildcard;
        let can_stand_in = |w: CardId, x: CardId| {
            w != x && wild_of(w).is_some_and(|t| cards.get(x).tribes.has(t))
        };
        let mut plan: Option<TriplePlan> = None;
        if copies(card) >= 3 {
            plan = Some((card, 3, None));
        } else if wild_of(card).is_none() {
            // Two of the card plus one stand-in, or one plus two stand-ins.
            let n = copies(card);
            let ids = 0..cards.cards.len() as CardId;
            if n >= 2
                && let Some(w) = ids.clone().find(|&w| can_stand_in(w, card) && stand_ins(w) >= 1)
            {
                plan = Some((card, 2, Some((w, 1))));
            } else if n >= 1
                && let Some(w) = ids.clone().find(|&w| can_stand_in(w, card) && stand_ins(w) >= 2)
            {
                plan = Some((card, 1, Some((w, 2))));
            }
        } else {
            // The new card is a wildcard: find a card it completes. Two of
            // the card plus this wildcard first, then one plus two wildcards.
            let n_w = stand_ins(card);
            let pool_ids = 0..cards.cards.len() as CardId;
            let x = pool_ids
                .clone()
                .find(|&x| can_stand_in(card, x) && copies(x) >= 2 && n_w >= 1)
                .map(|x| (x, 2usize, 1usize))
                .or_else(|| {
                    pool_ids
                        .clone()
                        .find(|&x| can_stand_in(card, x) && copies(x) >= 1 && n_w >= 2)
                        .map(|x| (x, 1usize, 2usize))
                });
            if let Some((x, nx, nw)) = x {
                plan = Some((x, nx, Some((card, nw))));
            }
        }
        let Some((x_card, x_count, wild)) = plan else { return };
        let pl = &mut self.players[p];
        let mut gold = pl.make_card(cards, x_card, true, 0);
        // a Beast on the board carries the until-next-turn Beast bonus; the
        // golden one in hand gets it when it is played, not twice
        let beast_bonus = pl.beast_bonus;
        let mut returned: ArrayVec<Minion, 2> = ArrayVec::new();
        let mut take_from = |pl: &mut Player, id: CardId, n: usize, same: bool, gold: &mut Minion| {
            for _ in 0..n {
                let is_copy = |m: &Minion| m.card == id && (!m.golden || !same);
                let (m, from_board) = if let Some(i) = pl.hand.iter().position(is_copy) {
                    (pl.hand.remove(i), false)
                } else {
                    let i = pl.board.iter().position(is_copy).unwrap();
                    if i < *tracked_pos {
                        *tracked_pos -= 1;
                    }
                    (pl.board.remove(i), true)
                };
                let base = pl.make_card(cards, id, m.golden, 0);
                let bonus = if from_board && cards.get(id).tribes.has(Tribe::Beast) {
                    beast_bonus
                } else {
                    (0, 0)
                };
                gold.attack += m.attack - bonus.0 - base.attack;
                gold.health += m.health - bonus.1 - base.health;
                gold.winners_bread = gold.winners_bread.saturating_add(m.winners_bread);
                gold.magnetizations = gold.magnetizations.saturating_add(m.magnetizations);
                gold.volumizers =
                    gold.volumizers.saturating_add(m.volumizers.saturating_sub(base.volumizers));
                // Blood Gem stats are buffs like any other: they add up.
                gold.gem_attack = gold.gem_attack.saturating_add(m.gem_attack);
                gold.gem_health = gold.gem_health.saturating_add(m.gem_health);
                for &a in m.attached_slice() {
                    gold.push_attached(a);
                }
                if same {
                    gold.keywords.0 |= m.keywords.0;
                    gold.pool_copies = gold.pool_copies.saturating_add(m.pool_copies);
                    // `improvements` is one minion's own count (Leviathan
                    // summons, Snapper growth, Hopebringer shields, Wrathguard
                    // Activates, Patient Scout tier, Treasure Parrot damage,
                    // Scarlet Survivor's flag, event counters): the golden
                    // copy takes the highest of the three, never the sum.
                    gold.improvements = gold.improvements.max(m.improvements);
                } else {
                    // A stand-in brings its buffs and its Divine Shield (the
                    // point of the card); its pool copies go back to the pool.
                    gold.keywords.0 |= m.keywords.0;
                    let _ = returned.try_push(m);
                }
            }
        };
        take_from(pl, x_card, x_count, true, &mut gold);
        if let Some((w, n)) = wild {
            take_from(pl, w, n, false, &mut gold);
        }
        // An armed Living Prison's arm is a leftover of its copies, not a
        // progress count: the golden starts unarmed (uses belong to the
        // minion, "a golden from a triple starts fresh").
        if matches!(cards.get(x_card).activate, Some(a) if a.effect == Effect::ArmNextBuy) {
            gold.improvements = 0;
        }
        self.add_to_hand(p, gold);
        for m in returned {
            give_back(&mut self.pool, &self.data.cards, &m);
        }
    }

    fn run_end_of_turn(&mut self, p: usize) {
        let runs = self.drakkari_mult(p);
        let mut pos = 0;
        while pos < self.players[p].board.len() {
            let m = self.players[p].board[pos];
            for _ in 0..runs {
                for (card, golden) in m.parts() {
                    let mul = if golden { 2 } else { 1 };
                    match self.data.cards.get(card).end_of_turn {
                        Some(EndOfTurn::BuffSelf { attack, health }) => {
                            buff(&mut self.players[p].board[pos], attack * mul, health * mul);
                        }
                        Some(EndOfTurn::ResourcefulRobot) => {
                            let n = if golden { 2 } else { 1 };
                            for _ in 0..n {
                                if let Some(v_id) = self.data.cards.random_volumizer(self.lobby, &mut self.rng)
                                {
                                    let v =
                                        self.players[p].make_card(&self.data.cards, v_id, false, 0);
                                    self.magnetize_to(p, pos, v, false);
                                    self.give_generated_tracking(p, v_id, &mut pos);
                                }
                            }
                        }
                        Some(EndOfTurn::BuffPerMagnetization { attack, health }) => {
                            for o in &mut self.players[p].board {
                                let mag = o.magnetizations as i32;
                                if mag > 0 {
                                    buff(o, attack * mul * mag, health * mul * mag);
                                }
                            }
                        }
                        Some(EndOfTurn::ImproveTier) => {
                            let imp = &mut self.players[p].board[pos].improvements;
                            *imp = (*imp + 1).min((MAX_TIER - 1) as u16);
                        }
                        Some(EndOfTurn::GetCard { card: tok, count }) => {
                            let n = count * (mul as u8);
                            for _ in 0..n {
                                self.give_generated_tracking(p, tok, &mut pos);
                            }
                        }
                        Some(EndOfTurn::Effect(e)) => {
                            self.run_effect(p, &mut pos, e, golden, 1);
                        }
                        Some(EndOfTurn::CopyLastSpell) => {
                            if let Some(sp) = self.players[p].last_spell {
                                let n = mul as u8;
                                for _ in 0..n {
                                    self.give_generated_tracking(p, sp, &mut pos);
                                }
                            }
                        }
                        None => {}
                    }
                }
            }
            pos += 1;
        }
        // A Lockbox is an end of turn countdown too, so Drakkari doubles it.
        self.tick_lockboxes(p, runs);
    }

    /// The player is done for this turn. Anything still waiting on a choice
    /// takes the first option, then end-of-turn effects run.
    /// A played Tomb Turning Undead dies once its Battlecry, and whatever it
    /// asked the player to choose or Discover, is done.
    fn choice_open(pl: &Player) -> bool {
        pl.pending_target.is_some()
            || pl.pending_shop_target.is_some()
            || pl.pending_choose_one.is_some()
            || pl.pending_discard.is_some()
            || !pl.discover.is_empty()
            || !pl.discover_queue.is_empty()
            || !pl.choose_queue.is_empty()
    }

    /// Turbo Hogrider fires for each Choose One card played once nothing
    /// that card started is still waiting for a pick, so the minion that was
    /// played (Intrepid Botanist...) is on the board with its choice done.
    fn resolve_hogrider(&mut self, p: usize) {
        let pl = &self.players[p];
        if pl.hogrider_pending == 0 || !pl.alive || Self::choice_open(pl) {
            return;
        }
        let n = std::mem::take(&mut self.players[p].hogrider_pending);
        for _ in 0..n {
            self.after_choose_one_played(p);
        }
    }

    fn resolve_doomed(&mut self, p: usize) {
        let pl = &self.players[p];
        if !pl.alive || Self::choice_open(pl) {
            return;
        }
        let victims: ArrayVec<usize, BOARD> =
            pl.board.iter().enumerate().filter(|(_, m)| m.doomed).map(|(i, _)| i).collect();
        if victims.is_empty() {
            return;
        }
        for m in &mut self.players[p].board {
            m.doomed = false;
        }
        self.destroy_in_shop(p, &victims, None);
    }

    fn finish_shop(&mut self, p: usize) {
        loop {
            let pl = &self.players[p];
            if let Some(pt) = pl.pending_target {
                if let Some(s) = (0..pl.board.len()).find(|&s| self.valid_target(p, &pt, s)) {
                    self.resolve_target(p, s);
                } else if pt.kind.takes_shop()
                    && let Some(s) = (0..pl.shop.len())
                        .find(|&s| self.valid_shop_target(p, &pt, s))
                {
                    self.resolve_shop_target(p, s);
                } else {
                    self.players[p].pending_target = None;
                }
            } else if pl.pending_shop_target.is_some() {
                self.resolve_shop_target(p, 0);
            } else if pl.pending_discard.is_some() {
                // Throw away the first card a discard may take.
                match (0..pl.hand.len()).find(|&s| self.valid_discard(p, s)) {
                    Some(s) => self.resolve_discard(p, s),
                    None => self.players[p].pending_discard = None,
                }
            } else if let Some(pco) = pl.pending_choose_one {
                let k = (0..2).find(|&k| self.can_choose_option(p, pco.options[k])).unwrap();
                self.resolve_choose(p, k);
            } else if !pl.discover.is_empty() {
                self.resolve_choose(p, 0);
            } else if !pl.choose_queue.is_empty() {
                // Both-effects Choose One options still waiting.
                self.maybe_start_discover(p);
            } else {
                break;
            }
        }
        self.resolve_doomed(p);
        self.resolve_hogrider(p);
        // A Tomb Turning Undead that wasn't played this turn is a normal one.
        for m in &mut self.players[p].hand {
            m.doomed = false;
        }
        self.run_end_of_turn(p);
        let cards = &self.data.cards;
        for m in &mut self.players[p].board {
            m.extra_magnetize = 0;
            // An armed Living Prison that found no minion to buy is wasted.
            if m.improvements > 0
                && m.parts().any(|(c, _)| {
                    matches!(cards.get(c).activate, Some(a) if a.effect == Effect::ArmNextBuy)
                })
            {
                m.improvements = 0;
            }
        }
        self.players[p].end_turn_tavern_buffs(&self.data.cards);
        self.players[p].done = true;
    }

    fn advance(&mut self) {
        if self.over {
            return;
        }
        let n = self.order.len();
        for k in 1..=n {
            let c = (self.cursor + k) % n;
            if !self.players[self.order[c] as usize].done {
                self.cursor = c;
                return;
            }
        }
        self.combat();
        if !self.over {
            self.start_turn();
        }
    }

    fn start_turn(&mut self) {
        self.turn += 1;
        let turn = self.turn;
        let base_turn_gold = self.data.rules.gold(turn);
        for p in 0..PLAYERS {
            if !self.players[p].alive {
                continue;
            }
            let (da, dh) = self.total_spell_bonus(p);
            let Game { data, rng, players, pool, sandbox, .. } = self;
            let pl = &mut players[p];
            if let Some(sb) = sandbox {
                pl.free_rolls = pl.free_rolls.max(sb.free_rolls);
            }
            // the Beast bonus lasts until now; it never kills a Beast
            let (ba, bh) = std::mem::take(&mut pl.beast_bonus);
            if (ba, bh) != (0, 0) {
                for m in &mut pl.board {
                    if data.cards.get(m.card).tribes.has(Tribe::Beast) {
                        m.attack = (m.attack - ba).max(0);
                        m.health = (m.health - bh).max(1);
                    }
                }
            }
            if turn > 1 {
                pl.upgrade_cost = pl.upgrade_cost.saturating_sub(1);
            }
            let base_gold = base_turn_gold.saturating_add(pl.extra_max_gold);
            pl.gold = base_gold.saturating_add(pl.gold_next_turn);
            pl.gold_next_turn = 0;
            if turn > 1 {
                if pl.won_last_combat {
                    pl.gold = pl.gold.saturating_add(pl.overconfidence_win);
                } else if !pl.lost_last_combat {
                    pl.gold = pl.gold.saturating_add(pl.overconfidence_tie);
                }
            }
            pl.overconfidence_win = 0;
            pl.overconfidence_tie = 0;
            for m in &pl.board {
                for (card, golden) in m.parts() {
                    if let Some(StartOfTurn::GainGold(g)) = data.cards.get(card).start_of_turn {
                        let gain = g.saturating_mul(if golden { 2 } else { 1 });
                        pl.gold = pl.gold.saturating_add(gain);
                    }
                }
            }
            for (ba, bh, times) in pl.next_turn_board_buffs.drain(..) {
                for _ in 0..times {
                    for m in &mut pl.board {
                        buff(m, ba + da, bh + dh);
                    }
                }
            }
            let (wb_a, wb_h) = pl.winners_bread_buff;
            let won = pl.won_last_combat;
            for m in &mut pl.board {
                m.activated = false;
                // The limited uses of a new turn: a Trailblazer's combines,
                // a Mycologist's teachings, Malchezaar's Health Refreshes.
                m.uses_left = m.turn_uses(&data.cards);
                // Ichoron's Divine Shield lasts until the next shop phase.
                if m.temp_shield {
                    m.keywords.clear(Keywords::DIVINE_SHIELD);
                    m.temp_shield = false;
                }
                if m.winners_bread > 0 {
                    if won {
                        let n = m.winners_bread as i32;
                        buff(m, (wb_a + da) * n, (wb_h + dh) * n);
                    }
                    m.winners_bread = 0;
                }
            }
            for m in &mut pl.hand {
                m.winners_bread = 0;
            }
            for m in &mut pl.hand {
                m.locked_turns = m.locked_turns.saturating_sub(1);
            }
            pl.done = false;
            pl.actions = 0;
            pl.gold_spent_turn = 0;
            pl.elementals_played_turn = 0;
            let was_frozen = pl.frozen;
            if pl.frozen {
                pl.frozen = false;
            } else {
                for m in pl.shop.drain(..) {
                    give_back(pool, &data.cards, &m);
                }
                pl.shop_spell = None;
            }
            let size = data.rules.shop_size(pl.tier);
            // The roll at the start of a turn counts as a Refresh for
            // Waveling's amount and Barrage when the Tavern changed: always
            // when it was not frozen, and for a frozen one only when the roll
            // had to fill a slot (a bought card, a consumed or stolen one, a
            // new slot from levelling, an empty spell slot). It is still not
            // the player's own Refresh, so queued Fodders wait for an
            // explicit one.
            let refreshed = !was_frozen || pl.shop.len() < size || pl.shop_spell.is_none();
            Self::fill_shop(pl, pool, &data.cards, size, rng, self.lobby, RefreshKind::TurnStart);
            if refreshed {
                Self::apply_refresh_buff(pl, &data.cards, rng);
                Self::apply_barrage(pl, &data.cards);
            }
        }
        for p in 0..PLAYERS {
            if self.players[p].alive {
                self.check_attack_thresholds(p);
            }
        }
        self.pair();
        self.order = (0..PLAYERS as u8).filter(|&p| self.players[p as usize].alive).collect();
        self.rng.shuffle(&mut self.order);
        self.cursor = 0;
    }

    /// Pair the living players at random, avoiding last turn's opponent
    /// when a few shuffles can. With an odd count, one fights the ghost.
    fn pair(&mut self) {
        let mut alive: ArrayVec<u8, PLAYERS> =
            (0..PLAYERS as u8).filter(|&p| self.players[p as usize].alive).collect();
        let repeats = |order: &[u8], players: &[Player; PLAYERS]| {
            order
                .chunks_exact(2)
                .filter(|c| players[c[0] as usize].last_opponent == Some(Opponent::Player(c[1])))
                .count()
        };
        let mut best = alive.clone();
        let mut best_repeats = usize::MAX;
        for _ in 0..16 {
            self.rng.shuffle(&mut alive);
            let r = repeats(&alive, &self.players);
            if r < best_repeats {
                best_repeats = r;
                best = alive.clone();
            }
            if r == 0 {
                break;
            }
        }
        for c in best.chunks(2) {
            if let [a, b] = *c {
                self.players[a as usize].opponent = Some(Opponent::Player(b));
                self.players[b as usize].opponent = Some(Opponent::Player(a));
            } else {
                let ghost = self.ghost.as_ref().map(|g| Opponent::Ghost(g.player));
                self.players[c[0] as usize].opponent = ghost;
            }
        }
    }

    /// Take what a fight (or a Tavern attack, with `after_fight` false)
    /// leaves behind. After a Tavern attack the board is already up to date,
    /// so only the hand and the Tavern take the bonuses.
    fn apply_side_outcome(&mut self, p: usize, so: &SideOutcome, after_fight: bool) {
        if after_fight {
            for (slot, &imp) in so.permanent_improvements.iter().enumerate() {
                if imp > 0 && slot < self.players[p].board.len() {
                    self.players[p].board[slot].improvements =
                        self.players[p].board[slot].improvements.saturating_add(imp);
                }
            }
            // Tarecgosa and Persistent Poet: stats and Bonus Keywords kept
            // from the fight.
            for (slot, &(da, dh)) in so.permanent_stats.iter().enumerate() {
                if (da != 0 || dh != 0) && slot < self.players[p].board.len() {
                    let m = &mut self.players[p].board[slot];
                    m.attack += da;
                    m.health += dh;
                }
            }
            for (slot, &kw) in so.permanent_keywords.iter().enumerate() {
                if kw != 0 && slot < self.players[p].board.len() {
                    self.players[p].board[slot].keywords.0 |= kw;
                }
            }
            // Very Hungry Winterfinner's hand buffs are permanent.
            for (slot, &(da, dh)) in so.hand_stats.iter().enumerate() {
                if (da != 0 || dh != 0) && slot < self.players[p].hand.len() {
                    self.players[p].hand[slot].attack += da;
                    self.players[p].hand[slot].health += dh;
                }
            }
            // Razorfen Vineweaver's permanent Blood Gems: the stats come as
            // permanent_stats, and the same gain is Blood Gem stats.
            for (slot, &(da, dh)) in so.permanent_gems.iter().enumerate() {
                if (da != 0 || dh != 0) && slot < self.players[p].board.len() {
                    let m = &mut self.players[p].board[slot];
                    m.gem_attack += da;
                    m.gem_health += dh;
                }
            }
        }
        if let Some(deity) = &mut self.players[p].deity {
            deity.attack += so.deity_gain.0;
            deity.health += so.deity_gain.1;
        }
        self.players[p].spells_cast = self.players[p].spells_cast.saturating_add(so.spells_cast_gain);
        let (sa, sh) = so.spell_bonus_gain;
        if sa != 0 || sh != 0 {
            self.players[p].spell_bonus.0 += sa;
            self.players[p].spell_bonus.1 += sh;
        }
        if so.beetle_attack_gain != 0 || so.beetle_health_gain != 0 {
            self.add_beetle_bonus_to(p, so.beetle_attack_gain, so.beetle_health_gain, after_fight);
        }
        for (n, &g) in self.players[p].improve.iter_mut().zip(&so.improve_gain) {
            *n = n.saturating_add(g);
        }
        if !after_fight {
            let pl = &mut self.players[p];
            pl.beast_bonus.0 += so.beast_bonus_gain.0;
            pl.beast_bonus.1 += so.beast_bonus_gain.1;
        }
        self.add_deathrattles_triggered(p, so.deathrattles_triggered as u32, after_fight);
        self.add_undead_bonus(p, so.undead_gain.0, so.undead_gain.1, after_fight);
        self.add_knight_deaths(p, so.knight_deaths, after_fight);
        // A Sanguine Refiner Rally in a fight raises the Gem value for good.
        if so.gem_gain != (0, 0) {
            let pl = &mut self.players[p];
            pl.gem_attack += so.gem_gain.0;
            pl.gem_health += so.gem_gain.1;
        }
        // Sacrificial Wrathguard's Deathrattle: a Tavern buff for the game.
        if so.tavern_buff_gain != (0, 0) {
            let b = TavernBuff {
                tribe: None,
                attack: so.tavern_buff_gain.0,
                health: so.tavern_buff_gain.1,
                this_turn: false,
            };
            self.players[p].add_tavern_buff(&self.data.cards, b);
        }
        // Waveling's Deathrattle: more for the Refreshes to come.
        self.players[p].refresh_buff.0 += so.refresh_buff_gain.0;
        self.players[p].refresh_buff.1 += so.refresh_buff_gain.1;
        // Trapped Clapper's Deathrattle: Fodders for upcoming Refreshes. A
        // fight's list is one entry per upcoming Refresh, and they stack on
        // any Refreshes the player already queued.
        for k in 0..so.n_fodder_refreshes as usize {
            self.players[p].add_fodder_to_refresh(k, so.fodder_refreshes[k]);
        }
        if after_fight {
            self.players[p].boon_of_beetles =
                self.players[p].boon_of_beetles.saturating_sub(so.boon_of_beetles_used);
            self.players[p].brood_of_nozdormu = 0;
            self.players[p].upper_hand = 0;
            self.players[p].sharing_is_caring = 0;
        }
        for &c in so.cards_gained() {
            self.give_generated(p, c);
        }
        for &n in so.lockboxes_gained() {
            self.give_lockbox(p, n);
        }
    }

    fn combat(&mut self) {
        if self.sandbox.is_some() {
            self.sandbox_combat();
            return;
        }
        let alive = self.alive();
        let max_attacks = self.data.rules.max_attacks;
        let cap = self.data.rules.damage_cap(self.turn, alive);
        let capped = |d: i32| cap.map_or(d, |c| d.min(c));
        let mut damage = [0i32; PLAYERS];
        for p in 0..PLAYERS {
            if !self.players[p].alive {
                continue;
            }
            // The hand only goes into the fight when something on the board
            // or in it reads the hand, so most fights skip it entirely.
            let cards = &self.data.cards;
            let reads_board = |b: &[Minion]| {
                b.iter().any(|m| m.parts().any(|(c, _)| cards.get(c).reads_hand()))
            };
            let reads_hand =
                |h: &[Minion]| h.iter().any(|m| cards.get(m.card).reads_hand());
            let (them, tier, their_counters, their_hand) = match self.players[p].opponent {
                Some(Opponent::Player(q)) if p < q as usize => {
                    let q = &self.players[q as usize];
                    let hand: &[Minion] = if reads_board(&q.board) || reads_hand(&q.hand) {
                        &q.hand
                    } else {
                        &[]
                    };
                    (q.board.clone(), q.tier, q.counters(cards, self.lobby), hand)
                }
                Some(Opponent::Ghost(_)) => {
                    let g = self.ghost.as_ref().unwrap();
                    (g.board.clone(), g.tier, g.counters, &[] as &[Minion])
                }
                _ => continue,
            };
            let my_hand: &[Minion] = if reads_board(&self.players[p].board)
                || reads_hand(&self.players[p].hand)
            {
                &self.players[p].hand
            } else {
                &[]
            };
            let me = self.players[p].board.clone();
            let my_counters = self.players[p].counters(cards, self.lobby);
            let turn = self.turn;
            let my_tier = self.players[p].tier;
            let hands: [&[Minion]; 2] = [my_hand, their_hand];
            let out = fight_with_hands(
                &self.data.cards,
                &me,
                &them,
                hands,
                [my_counters, their_counters],
                max_attacks,
                &mut self.rng,
                &mut (),
            );
            // What each side sees of the other: the boards that went in, and
            // the counters at the first attack, after Start of Combat.
            let their_seen = Seen::at_fight(them, tier, turn, out.sides[1].seen);
            let my_seen = Seen::at_fight(me, my_tier, turn, out.sides[0].seen);
            match self.players[p].opponent.unwrap() {
                Opponent::Player(q) => {
                    let q = q as usize;
                    self.players[p].seen[q] = their_seen;
                    self.players[q].seen[p] = my_seen;
                    match out.winner {
                        Some(0) => {
                            damage[q] += capped(my_tier as i32 + out.survivor_tiers);
                            self.players[p].lost_last_combat = false;
                            self.players[p].won_last_combat = true;
                            self.players[q].lost_last_combat = true;
                            self.players[q].won_last_combat = false;
                        }
                        Some(_) => {
                            damage[p] += capped(tier as i32 + out.survivor_tiers);
                            self.players[p].lost_last_combat = true;
                            self.players[p].won_last_combat = false;
                            self.players[q].lost_last_combat = false;
                            self.players[q].won_last_combat = true;
                        }
                        None => {
                            self.players[p].lost_last_combat = false;
                            self.players[p].won_last_combat = false;
                            self.players[q].lost_last_combat = false;
                            self.players[q].won_last_combat = false;
                        }
                    }
                    self.apply_side_outcome(p, &out.sides[0], true);
                    self.apply_side_outcome(q, &out.sides[1], true);
                }
                Opponent::Ghost(g) => {
                    self.players[p].seen[g as usize] = their_seen;
                    match out.winner {
                        Some(0) => {
                            self.players[p].lost_last_combat = false;
                            self.players[p].won_last_combat = true;
                        }
                        Some(_) => {
                            damage[p] += capped(tier as i32 + out.survivor_tiers);
                            self.players[p].lost_last_combat = true;
                            self.players[p].won_last_combat = false;
                        }
                        None => {
                            self.players[p].lost_last_combat = false;
                            self.players[p].won_last_combat = false;
                        }
                    }
                    self.apply_side_outcome(p, &out.sides[0], true);
                }
            }
        }
        for (p, d) in damage.into_iter().enumerate() {
            // A lost fight means the board wasn't in play, so no minion
            // reacts: no rewind, no "after your hero takes damage", no
            // damage event count. The hero just loses the Health, and
            // `knockouts` settles any that reach 0.
            self.players[p].health -= d;
            let pl = &mut self.players[p];
            pl.last_opponent = pl.opponent.take();
        }
        self.knockouts(alive);
    }

    /// The sandbox fight: the player against a copy of their own board,
    /// hand and counters. Only the player's side outcome is applied, and
    /// the fight never costs Health.
    fn sandbox_combat(&mut self) {
        let max_attacks = self.data.rules.max_attacks;
        let cards = &self.data.cards;
        let pl = &self.players[0];
        let reads_board =
            |b: &[Minion]| b.iter().any(|m| m.parts().any(|(c, _)| cards.get(c).reads_hand()));
        let reads_hand = |h: &[Minion]| h.iter().any(|m| cards.get(m.card).reads_hand());
        let hand: &[Minion] =
            if reads_board(&pl.board) || reads_hand(&pl.hand) { &pl.hand } else { &[] };
        let me = pl.board.clone();
        let counters = pl.counters(cards, self.lobby);
        let out = fight_with_hands(
            cards,
            &me,
            &me,
            [hand, hand],
            [counters, counters],
            max_attacks,
            &mut self.rng,
            &mut (),
        );
        let fight = SandboxFight {
            turn: self.turn,
            tier: pl.tier,
            health: pl.health,
            board: me.clone(),
            hand: pl.hand.clone(),
            winner: out.winner,
            side: out.sides[0],
        };
        let pl = &mut self.players[0];
        pl.won_last_combat = out.winner == Some(0);
        pl.lost_last_combat = out.winner == Some(1);
        self.apply_side_outcome(0, &out.sides[0], true);
        if let Some(sb) = &mut self.sandbox {
            sb.last_fight = Some(fight);
        }
        if self.turn >= self.data.rules.max_turns {
            self.players[0].placement = 1;
            self.over = true;
        }
    }

    fn knockouts(&mut self, alive: usize) {
        let mut dead: ArrayVec<u8, PLAYERS> = (0..PLAYERS as u8)
            .filter(|&p| self.players[p as usize].alive && self.players[p as usize].health <= 0)
            .collect();
        // best first: more health left, ties at random
        self.rng.shuffle(&mut dead);
        dead.sort_by_key(|&p| std::cmp::Reverse(self.players[p as usize].health));
        let first_place = alive - dead.len() + 1;
        for (k, &p) in dead.iter().enumerate() {
            let Game { data, players, pool, ghost, lobby, .. } = self;
            let pl = &mut players[p as usize];
            pl.alive = false;
            pl.placement = (first_place + k) as u8;
            pl.final_tribes = pl.board_tribes(&data.cards);
            if k == 0 {
                *ghost = Some(Ghost {
                    player: p,
                    board: pl.board.clone(),
                    tier: pl.tier,
                    counters: pl.counters(&data.cards, *lobby),
                });
            }
            pl.shop_spell = None;
            for m in pl.shop.drain(..).chain(pl.hand.drain(..)).chain(pl.board.drain(..)) {
                give_back(pool, &data.cards, &m);
            }
            for c in pl.discover.drain(..) {
                if !data.cards.get(c).is_spell() {
                    let slot = data.cards.get(c).pool_slot as usize;
                    pool[slot] += 1;
                }
            }
            pl.pending_choose_one = None;
            pl.pending_target = None;
            pl.pending_shop_target = None;
            pl.pending_discard = None;
            pl.choose_queue.clear();
            pl.tavern_buffs.clear();
            pl.discover_queue.clear();
        }
        let mut left: ArrayVec<u8, PLAYERS> =
            (0..PLAYERS as u8).filter(|&p| self.players[p as usize].alive).collect();
        if left.len() <= 1 || self.turn >= self.data.rules.max_turns {
            self.rng.shuffle(&mut left);
            left.sort_by_key(|&p| std::cmp::Reverse(self.players[p as usize].health));
            for (k, &p) in left.iter().enumerate() {
                let pl = &mut self.players[p as usize];
                pl.placement = k as u8 + 1;
                pl.final_tribes = pl.board_tribes(&self.data.cards);
            }
            self.over = true;
        }
    }

    /// Every card copy is in the pool, held by someone, or consumed by a
    /// Magnetization; nothing is over its limit; and the dead hold nothing.
    /// Checked after every step in debug builds; the fuzzer calls it in
    /// release too.
    pub fn check_invariants(&self) -> Result<(), String> {
        let cards = &self.data.cards;
        let mut held = [0u32; MAX_POOL_CARDS];
        for (i, pl) in self.players.iter().enumerate() {
            for m in pl.shop.iter().chain(&pl.hand).chain(&pl.board) {
                let c = cards.get(m.card);
                if !c.token && !c.is_spell() {
                    let slot = c.pool_slot as usize;
                    held[slot] += m.pool_copies as u32;
                }
            }
            for &c in &pl.discover {
                if !cards.get(c).is_spell() {
                    let slot = cards.get(c).pool_slot as usize;
                    held[slot] += 1;
                }
            }
            // A Tavern minion may be above the player's tier: Robust
            // Evolution (which ignores the Tavern tier) can put one there.
            // Generated cards (Fishbait) may be any Tier too, and a Golden
            // Touch makes a Tavern minion golden.
            if let Some(sp) = pl.shop_spell
                && (!cards.get(sp).is_spell() || cards.get(sp).tier > pl.tier)
            {
                return Err(format!("player {i}: shop_spell invalid"));
            }
            // The minion slots plus the spell slot, which a Fodder can take
            // (and fill with a minion once it feeds).
            if pl.shop.len() > self.data.rules.shop_size(pl.tier) + 1 {
                return Err(format!("player {i}: shop too big"));
            }
            if pl.shop.len() == self.data.rules.shop_size(pl.tier) + 1
                && pl.shop_spell.is_some()
            {
                return Err(format!("player {i}: shop spell in a taken slot"));
            }
            if let Some(pt) = pl.pending_target
                && let Some(src) = pt.source
                && src as usize >= pl.board.len()
            {
                return Err(format!("player {i}: pending target source off the board"));
            }
            if let Some(pst) = pl.pending_shop_target
                && (pst.source as usize >= pl.board.len() || pl.shop.is_empty())
            {
                return Err(format!("player {i}: pending shop target has nothing to act on"));
            }
            if let Some(pd) = pl.pending_discard
                && pd.source as usize >= pl.board.len()
            {
                return Err(format!("player {i}: pending discard source off the board"));
            }
            if let Some(pco) = pl.pending_choose_one
                && let Some(src) = pco.source
                && src as usize >= pl.board.len()
            {
                return Err(format!("player {i}: pending choose_one source off the board"));
            }
            let holds_cards = !(pl.shop.is_empty()
                && pl.shop_spell.is_none()
                && pl.hand.is_empty()
                && pl.board.is_empty()
                && pl.discover.is_empty());
            if !pl.alive && holds_cards {
                return Err(format!("player {i}: dead but holds cards"));
            }
            if !(1..=MAX_TIER).contains(&pl.tier) {
                return Err(format!("player {i}: tier {}", pl.tier));
            }
        }
        for c in 0..cards.pool_cards {
            let card = &cards.cards[c];
            // A card outside the lobby holds no copies anywhere: it was never
            // in this game's pool.
            let want = if card.pool_slot as usize == c && card.lobby_ok(self.lobby) {
                self.data.rules.copies(card.tier) as u32
            } else {
                0
            };
            let have = self.pool[c] as u32 + held[c] + self.consumed[c];
            if have != want {
                return Err(format!("{}: {have} copies, should be {want}", card.name));
            }
        }
        if self.over {
            let mut places = self.placements();
            places.sort();
            if places != [1, 2, 3, 4, 5, 6, 7, 8] {
                return Err(format!("placements {places:?}"));
            }
        }
        Ok(())
    }

    /// Take a card straight into a player's hand. Pool minions come out of
    /// `self.pool`; spells and tokens are generated with `pool_copies = 0`.
    /// For tests and setting up positions, not part of the game.
    pub fn give(&mut self, p: usize, name: &str) -> Option<Minion> {
        let c = self.data.cards.by_name(name)?;
        if self.players[p].hand.is_full() {
            return None;
        }
        let card = self.data.cards.get(c);
        if card.is_spell() || card.token {
            let m = self.players[p].make_card(&self.data.cards, c, false, 0);
            self.add_to_hand(p, m);
            return Some(m);
        }
        let slot = card.pool_slot as usize;
        if self.pool.get(slot).copied().unwrap_or(0) == 0 {
            return None;
        }
        self.pool[slot] -= 1;
        let m = self.players[p].make_card(&self.data.cards, c, false, 1);
        self.add_to_hand(p, m);
        Some(m)
    }
}
