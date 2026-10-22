//! One 8-player game: shop phase, pairing, fights, knockouts.
//!
//! The shop phase is simultaneous in the real game. Here the players act one
//! action at a time, in a random order that changes every turn, until each
//! has ended their turn. `current_player` says whose action is next.

use std::sync::Arc;

use arrayvec::ArrayVec;

use crate::action::{Action, N_ACTIONS};
use crate::cards::{
    N_COUNTERS,
    Aura, Card, CardDb, ChooseOption, ConsumeBy, ConsumePick, DiscoverKind, Effect, EndOfTurn,
    EventKind, OnCastSpell, OnCastSpellOnMech, OnPlayMech, OnPlayOrMagnetizeMech, OnSell,
    OnSummonBeast, SpellEffect, SpellTarget, StartOfTurn, Tribe,
};
use crate::fight::{Counters, SideOutcome, fight_with_hands};
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
    Consume {
        tribe: Option<Tribe>,
        pick: ConsumePick,
        keywords: bool,
        count: u8,
    },
    Spell(SpellTarget),
    /// Trigger the Rally of the chosen minion in the shop (Sky-hatch
    /// Runaway), `runs` times.
    TriggerRally,
    /// Trigger the Battlecry of the chosen minion (Kelp Keeper), `runs`
    /// times.
    TriggerBattlecry,
}

/// A targeted Battlecry, Choose One option, or Tavern spell waiting for the
/// player to pick its target on the board.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PendingTarget {
    /// Board slot of the minion whose effect it is, or None for a spell.
    pub source: Option<u8>,
    pub kind: TargetKind,
    pub runs: u8,
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

/// One buff per (tribe filter, duration): 5 tribes and no filter, times 2.
pub const MAX_TAVERN_BUFFS: usize = 12;

/// A Choose One minion or spell waiting for the player to pick one of its
/// two options.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PendingChooseOne {
    pub source: Option<u8>,
    pub golden: bool,
    pub options: [ChooseOption; 2],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DiscoverRequest {
    MinionTier(u8),
    SpellTier(u8),
    DeathrattleMinion(u8),
    BattlecryMinion(u8),
    TribeMinion(Tribe),
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
    /// Health Refreshes paid so far this turn.
    pub health_refreshes_used: u8,
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
    pub spell_bonus: (i32, i32),
    pub spell_discount: u8,
    /// Magicfin Mycologist uses spent this turn. Resets every turn.
    pub mycologist_uses: u8,
    pub last_spell: Option<CardId>,
    pub shop_spell: Option<CardId>,
    pub shop: ArrayVec<Minion, SHOP>,
    pub hand: ArrayVec<Minion, HAND>,
    pub board: ArrayVec<Minion, BOARD>,
    pub pending_choose_one: Option<PendingChooseOne>,
    pub pending_target: Option<PendingTarget>,
    pub pending_shop_target: Option<PendingShopTarget>,
    /// Discover options waiting for a choice. Minion cards here are out of the pool.
    pub discover: ArrayVec<CardId, DISCOVER>,
    /// Queued Discover requests to offer once pending choices finish.
    pub discover_queue: ArrayVec<DiscoverRequest, 8>,
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
            health_refreshes_used: 0,
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
            volumizer_bonus: (0, 0),
            beetle_bonus: (0, 0),
            improve: [0; N_COUNTERS],
            beast_bonus: (0, 0),
            deathrattles_triggered: 0,
            spell_bonus: (0, 0),
            spell_discount: 0,
            mycologist_uses: 0,
            last_spell: None,
            shop_spell: None,
            shop: ArrayVec::new(),
            hand: ArrayVec::new(),
            board: ArrayVec::new(),
            pending_choose_one: None,
            pending_target: None,
            pending_shop_target: None,
            discover: ArrayVec::new(),
            discover_queue: ArrayVec::new(),
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

    /// Counts per `TRIBES` slot (Beast, Mech, neutral, Dragon, Murloc, Demon)
    /// of the minions currently on this player's board. A multi-tribe minion
    /// counts once in each of its tribes.
    pub fn board_tribes(&self, cards: &CardDb) -> [u8; TRIBES] {
        let mut out = [0u8; TRIBES];
        for m in &self.board {
            let t = cards.get(m.card).tribes;
            if t.is_neutral() {
                out[2] += 1;
            }
            for (slot, tribe) in [
                (0, Tribe::Beast),
                (1, Tribe::Mech),
                (3, Tribe::Dragon),
                (4, Tribe::Murloc),
                (5, Tribe::Demon),
            ] {
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

    /// Health Refreshes left this turn, from the minions on the board.
    pub fn health_refreshes_left(&self, cards: &CardDb) -> u8 {
        let mut total = 0u8;
        for m in &self.board {
            for (card, golden) in m.parts() {
                let n = cards.get(card).health_refreshes;
                total = total.saturating_add(n.saturating_mul(if golden { 2 } else { 1 }));
            }
        }
        total.saturating_sub(self.health_refreshes_used)
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
            if cards.get(m.card).is(b.tribe) {
                buff(m, b.attack, b.health);
            }
        }
    }

    /// Put every Tavern buff that applies on a minion that just showed up in
    /// the Tavern.
    pub fn apply_tavern_buffs(&self, cards: &CardDb, m: &mut Minion) {
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
                if cards.get(m.card).is(b.tribe) {
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

    pub fn counters(&self, cards: &CardDb) -> Counters {
        let (spell_attack, spell_health) = self.total_spell_bonus(cards);
        Counters {
            beetle_attack: self.beetle_bonus.0,
            beetle_health: self.beetle_bonus.1,
            improve: self.improve,
            beast_attack: self.beast_bonus.0,
            beast_health: self.beast_bonus.1,
            tier: self.tier,
            boon_of_beetles: self.boon_of_beetles,
            brood_of_nozdormu: self.brood_of_nozdormu,
            upper_hand: self.upper_hand,
            spell_attack,
            spell_health,
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
    pub turn: u32,
    pub players: [Player; PLAYERS],
    /// Copies left of each pool card, indexed by card `pool_slot`.
    pub pool: Vec<u8>,
    /// Pool copies permanently consumed into hosts via Magnetize, indexed by
    /// card `pool_slot`.
    pub consumed: Vec<u32>,
    pub ghost: Option<Ghost>,
    pub over: bool,
    order: ArrayVec<u8, PLAYERS>,
    cursor: usize,
}

/// Draw one card from the pool, weighted by copies left, among cards of
/// tier `lo..=hi` that satisfy `pred` and aren't in `exclude`. When the shared
/// Volumizer slot is drawn, picks a random Volumizer color not in `exclude`.
fn draw_filtered(
    pool: &mut [u8],
    cards: &CardDb,
    rng: &mut Rng,
    lo: u8,
    hi: u8,
    exclude: &[CardId],
    pred: impl Fn(&Card) -> bool,
) -> Option<CardId> {
    let ok = |c: usize| {
        let card = &cards.cards[c];
        if card.pool_slot as usize != c {
            return false;
        }
        let t = card.tier;
        if !(lo <= t && t <= hi && pred(card)) {
            return false;
        }
        if card.is_volumizer {
            cards.volumizers.iter().any(|v| !exclude.contains(v))
        } else {
            !exclude.contains(&(c as CardId))
        }
    };
    let total: u32 = (0..pool.len()).filter(|&c| ok(c)).map(|c| pool[c] as u32).sum();
    if total == 0 {
        return None;
    }
    let mut r = rng.below(total);
    for c in 0..pool.len() {
        if !ok(c) {
            continue;
        }
        let n = pool[c] as u32;
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

fn draw(
    pool: &mut [u8],
    cards: &CardDb,
    rng: &mut Rng,
    lo: u8,
    hi: u8,
    exclude: &[CardId],
) -> Option<CardId> {
    draw_filtered(pool, cards, rng, lo, hi, exclude, |_| true)
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

impl Game {
    pub fn new(data: Arc<Data>, seed: u64) -> Game {
        let pool = (0..data.cards.pool_cards)
            .map(|c| {
                let card = &data.cards.cards[c];
                if card.pool_slot as usize == c {
                    data.rules.copies(card.tier)
                } else {
                    0
                }
            })
            .collect();
        let consumed = vec![0u32; data.cards.pool_cards];
        let players = std::array::from_fn(|_| Player::new(&data));
        let mut g = Game {
            data,
            rng: Rng::new(seed),
            turn: 0,
            players,
            pool,
            consumed,
            ghost: None,
            over: false,
            order: ArrayVec::new(),
            cursor: 0,
        };
        g.start_turn();
        g
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
            return matches!(a, Action::Target(s) if self.valid_target(p, &pt, s as usize));
        }
        if pl.pending_shop_target.is_some() {
            return matches!(a, Action::TargetShop(s) if (s as usize) < pl.shop.len());
        }
        if let Some(pco) = pl.pending_choose_one {
            return matches!(a, Action::Choose(k) if k < 2 && self.can_choose_option(p, pco.options[k as usize]));
        }
        if !pl.discover.is_empty() {
            return matches!(a, Action::Choose(k) if (k as usize) < pl.discover.len());
        }
        match a {
            Action::Buy(s) => {
                (s as usize) < pl.shop.len() && pl.gold >= r.buy_cost && !pl.hand.is_full()
            }
            Action::BuySpell => {
                pl.spell_cost(&self.data.cards).is_some_and(|cost| pl.gold >= cost)
                    && !pl.hand.is_full()
            }
            Action::Sell(s) => (s as usize) < pl.board.len(),
            Action::Activate(s) => {
                (s as usize) < pl.board.len()
                    && self.activate_of(p, s as usize).is_some_and(|(act, _)| {
                        pl.gold >= act.cost
                            && (!act.effect.needs_shop_slot() || !pl.shop.is_empty())
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
                if self.data.cards.get(card_id).is_spell() {
                    pos == 0 && self.can_cast_spell(p, card_id)
                } else if pos > pl.board.len() {
                    false
                } else if pl.hand[h].keywords.has(Keywords::MAGNETIC)
                    && pos < pl.board.len()
                    && self.data.cards.get(pl.board[pos].card).tribes.has(Tribe::Mech)
                {
                    true
                } else {
                    !pl.board.is_full()
                }
            }
            Action::Target(_) | Action::TargetShop(_) | Action::Choose(_) => false,
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

    pub fn legal_mask(&self, mask: &mut [bool; N_ACTIONS]) {
        for (i, m) in mask.iter_mut().enumerate() {
            *m = self.is_legal(Action::from_index(i).unwrap());
        }
    }

    /// The Activate ability of the board minion in slot `s` (the first part
    /// of it that has one) and whether that part is golden.
    fn activate_of(&self, p: usize, s: usize) -> Option<(crate::cards::Activate, bool)> {
        self.players[p].board[s]
            .parts()
            .find_map(|(card, golden)| self.data.cards.get(card).activate.map(|a| (a, golden)))
    }

    fn valid_spell_target(&self, p: usize, st: SpellTarget, s: usize) -> bool {
        let b = &self.players[p].board;
        if s >= b.len() {
            return false;
        }
        let c = self.data.cards.get(b[s].card);
        match st {
            SpellTarget::Buff { .. }
            | SpellTarget::BuffPerBonusKeyword { .. }
            | SpellTarget::SetStats { .. } => true,
            SpellTarget::BuffSameTribe { .. } | SpellTarget::SameTribeMinion => {
                !c.tribes.is_neutral()
            }
            SpellTarget::MakeGolden { max_tier } => !b[s].golden && c.tier <= max_tier,
        }
    }

    fn can_choose_option(&self, p: usize, opt: ChooseOption) -> bool {
        match opt {
            ChooseOption::SpellTarget(st) => {
                (0..self.players[p].board.len()).any(|s| self.valid_spell_target(p, st, s))
            }
            ChooseOption::MinionTarget { .. }
            | ChooseOption::SpellBonus { .. }
            | ChooseOption::SpellEffect(_) => true,
        }
    }

    pub fn can_cast_spell(&self, p: usize, card_id: CardId) -> bool {
        let c = self.data.cards.get(card_id);
        if let Some(st) = c.spell_target {
            (0..self.players[p].board.len()).any(|s| self.valid_spell_target(p, st, s))
        } else if let Some(opts) = c.choose_one {
            opts.iter().any(|&opt| self.can_choose_option(p, opt))
        } else {
            c.spell_effect.is_some()
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
            TargetKind::Spell(st) => self.valid_spell_target(p, st, s),
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

    /// Take an action for the current player.
    pub fn step(&mut self, a: Action) -> Result<(), IllegalAction> {
        if !self.is_legal(a) {
            return Err(IllegalAction);
        }
        let p = self.current_player().unwrap();
        self.apply(p, a);
        self.check_attack_thresholds(p);
        let pl = &mut self.players[p];
        pl.actions += 1;
        if a == Action::EndTurn || pl.actions >= self.data.rules.max_actions_per_turn {
            self.finish_shop(p);
        }
        self.advance();
        #[cfg(debug_assertions)]
        self.check_invariants().unwrap();
        Ok(())
    }

    fn apply(&mut self, p: usize, a: Action) {
        match a {
            Action::Buy(s) => {
                let buy_cost = self.data.rules.buy_cost;
                let pl = &mut self.players[p];
                let m = pl.shop.remove(s as usize);
                pl.gold -= buy_cost;
                pl.hand.push(m);
                self.check_triple(p, m.card);
                self.fire_event(p, EventKind::GoldSpent, buy_cost as u32);
                self.fire_event(p, EventKind::CardsBought, 1);
            }
            Action::BuySpell => {
                let cost = self.players[p].spell_cost(&self.data.cards).unwrap();
                let id = self.players[p].shop_spell.take().unwrap();
                let pl = &mut self.players[p];
                pl.gold -= cost;
                pl.spell_discount = 0;
                let m = pl.make_card(&self.data.cards, id, false, 0);
                pl.hand.push(m);
                self.fire_event(p, EventKind::GoldSpent, cost as u32);
                self.fire_event(p, EventKind::CardsBought, 1);
                self.magicfin_teach(p, id);
            }
            Action::Sell(s) => {
                let m = self.players[p].board.remove(s as usize);
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
                        Some(OnSell::GoldIfLost(_)) | None => {}
                    }
                }
                self.fire_event(p, EventKind::MinionsSold, 1);
            }
            Action::Activate(s) => {
                let (act, golden) = self.activate_of(p, s as usize).unwrap();
                self.players[p].gold -= act.cost;
                let mut pos = s as usize;
                self.run_effect(p, &mut pos, act.effect, golden, 1);
                self.maybe_start_discover(p);
                self.fire_event(p, EventKind::GoldSpent, act.cost as u32);
            }
            Action::Play { hand, pos } => {
                let m = self.players[p].hand.remove(hand as usize);
                if self.data.cards.get(m.card).is_spell() {
                    self.cast_spell(p, m.card);
                    return;
                }
                let pos = pos as usize;
                let is_merge = m.keywords.has(Keywords::MAGNETIC)
                    && pos < self.players[p].board.len()
                    && self.data.cards.get(self.players[p].board[pos].card).tribes.has(Tribe::Mech);
                if is_merge {
                    self.magnetize_to(p, pos, m, true);
                } else {
                    let mut m = m;
                    if self.data.cards.get(m.card).tribes.has(Tribe::Beast) {
                        let (ba, bh) = self.players[p].beast_bonus;
                        m.attack += ba;
                        m.health += bh;
                    }
                    self.players[p].board.insert(pos, m);
                    if let Some((da, dh)) = self.data.cards.get(m.card).volumizer_buff {
                        let mul = if m.golden { 2 } else { 1 };
                        self.add_volumizer_bonus(p, da * mul, dh * mul);
                    }
                    let t = self.data.cards.get(m.card).tribes;
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
                    let tier = (self.players[p].tier + 1).min(MAX_TIER);
                    let _ = self.players[p]
                        .discover_queue
                        .try_push(DiscoverRequest::MinionTier(tier));
                    self.maybe_start_discover(p);
                }
            }
            Action::Target(s) => self.resolve_target(p, s as usize),
            Action::TargetShop(s) => self.resolve_shop_target(p, s as usize),
            Action::Choose(k) => self.resolve_choose(p, k as usize),
            Action::Move { from, to } => {
                let pl = &mut self.players[p];
                let m = pl.board.remove(from as usize);
                pl.board.insert(to as usize, m);
            }
            Action::Roll => {
                let Game { data, rng, players, pool, .. } = self;
                let (cards, rules) = (&data.cards, &data.rules);
                let pl = &mut players[p];
                let mut gold_spent = 0;
                let mut health_paid = 0;
                if pl.free_rolls > 0 {
                    pl.free_rolls -= 1;
                } else if pl.can_refresh_with_health(cards, rules) {
                    pl.health_refreshes_used += 1;
                    health_paid = rules.health_refresh_cost;
                } else {
                    pl.gold -= rules.roll_cost;
                    gold_spent = rules.roll_cost;
                }
                Self::refresh_shop(pl, pool, cards, rules, rng);
                if health_paid > 0 {
                    self.take_hero_damage(p, health_paid);
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

    /// Throw the Tavern back and draw a new one. Unfreezes it.
    fn refresh_shop(pl: &mut Player, pool: &mut [u8], cards: &CardDb, rules: &Rules, rng: &mut Rng) {
        pl.frozen = false;
        for m in pl.shop.drain(..) {
            give_back(pool, cards, &m);
        }
        pl.shop_spell = None;
        Self::fill_shop(pl, pool, cards, rules.shop_size(pl.tier), rng);
    }

    fn fill_shop(pl: &mut Player, pool: &mut [u8], cards: &CardDb, size: usize, rng: &mut Rng) {
        while pl.shop.len() < size {
            match draw(pool, cards, rng, 1, pl.tier, &[]) {
                Some(c) => {
                    let mut m = pl.make_card(cards, c, false, 1);
                    pl.apply_tavern_buffs(cards, &mut m);
                    pl.shop.push(m);
                }
                None => break,
            }
        }
        if pl.shop_spell.is_none() {
            pl.shop_spell = cards.random_spell(pl.tier, rng);
        }
    }

    /// Total spell stat bonus for player `p`, combining Intrepid Botanist's
    /// permanent bonus with board auras (Enchanted Sentinel, including when
    /// Magnetized into a host, and Humon'gozz).
    pub fn total_spell_bonus(&self, p: usize) -> (i32, i32) {
        self.players[p].total_spell_bonus(&self.data.cards)
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
        self.trigger_mech_played_or_magnetized(p, pos, from_play, false);
        // Drone Duplicator: the same Magnetization happens again, without
        // taking another copy from the pool.
        for _ in 0..extra {
            if pos >= self.players[p].board.len() {
                break;
            }
            self.players[p].board[pos].magnetize(&self.data.cards, m);
            self.trigger_mech_played_or_magnetized(p, pos, from_play, false);
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
                    && let Some(OnPlayMech::MagnetizeSatellite { card: sat_id }) = c.on_play_mech
                {
                    let imp = self.players[p].board[i].improvements;
                    self.players[p].board[i].improvements = imp.saturating_add(1);
                    let stacks = (1 + imp as i32) * (if golden { 2 } else { 1 });
                    let mut sat = self.data.cards.make(sat_id, false);
                    sat.attack *= stacks;
                    sat.health *= stacks;
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
    /// teaches a 1/1 Murloc token that spell, once per turn each.
    fn magicfin_teach(&mut self, p: usize, spell: CardId) {
        let Some(token) = self.data.cards.taught_token else { return };
        let cards = &self.data.cards;
        let mut limit = 0u8;
        for m in &self.players[p].board {
            for (card, golden) in m.parts() {
                if cards.get(card).teaches_spell {
                    limit = limit.saturating_add(if golden { 2 } else { 1 });
                }
            }
        }
        if self.players[p].mycologist_uses >= limit || self.players[p].hand.is_full() {
            return;
        }
        self.players[p].mycologist_uses += 1;
        let mut m = self.players[p].make_card(cards, token, false, 0);
        m.taught_spell = Some(spell);
        self.players[p].hand.push(m);
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

    fn brann_mult(&self, p: usize) -> u8 {
        let mut mult = 1;
        for m in &self.players[p].board {
            for (card, golden) in m.parts() {
                if self.data.cards.get(card).aura == Some(Aura::Brann) {
                    mult = mult.max(if golden { 3 } else { 2 });
                }
            }
        }
        mult
    }

    fn drakkari_mult(&self, p: usize) -> u8 {
        let mut mult = 1;
        for m in &self.players[p].board {
            for (card, golden) in m.parts() {
                if self.data.cards.get(card).aura == Some(Aura::Drakkari) {
                    mult = mult.max(if golden { 3 } else { 2 });
                }
            }
        }
        mult
    }

    fn balinda_mult(&self, p: usize) -> u8 {
        let mut mult = 1;
        for m in &self.players[p].board {
            for (card, golden) in m.parts() {
                if self.data.cards.get(card).aura == Some(Aura::Balinda) {
                    mult = mult.max(if golden { 3 } else { 2 });
                }
            }
        }
        mult
    }

    fn cast_spell(&mut self, p: usize, card_id: CardId) {
        self.cast_spell_inner(p, card_id);
        self.fire_event(p, EventKind::SpellsCast, 1);
    }

    fn cast_spell_inner(&mut self, p: usize, card_id: CardId) {
        self.players[p].last_spell = Some(card_id);
        let c = self.data.cards.get(card_id);
        if let Some(st) = c.spell_target {
            let runs = self.balinda_mult(p);
            self.players[p].pending_target = Some(PendingTarget {
                source: None,
                kind: TargetKind::Spell(st),
                runs,
            });
        } else if let Some(options) = c.choose_one {
            self.players[p].pending_choose_one = Some(PendingChooseOne {
                source: None,
                golden: false,
                options,
            });
        } else if let Some(se) = c.spell_effect {
            self.trigger_on_cast_spell(p);
            self.execute_spell_effect(p, se);
            self.maybe_start_discover(p);
        }
    }

    fn execute_spell_effect(&mut self, p: usize, se: SpellEffect) {
        let (da, dh) = self.total_spell_bonus(p);
        match se {
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
                for m in &mut self.players[p].shop {
                    buff(m, attack + da, health + dh);
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
                if let Some(c) = self.data.cards.random_tier_minion(tier, &mut self.rng) {
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
                };
                let _ = self.players[p].discover_queue.try_push(req);
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
                if let Some(c) = self.data.cards.random_pool_minion(Tribe::Murloc, &mut self.rng) {
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
            SpellEffect::UpperHand => {
                self.players[p].upper_hand = self.players[p].upper_hand.saturating_add(1);
            }
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
                other => other,
            });
            self.players[p].pending_choose_one = Some(PendingChooseOne {
                source: Some(pos as u8),
                golden: m.golden,
                options: scaled,
            });
            return;
        }
        let Some(bc) = card.battlecry else { return };
        let runs = self.brann_mult(p);
        self.run_effect(p, &mut pos, bc, m.golden, runs);
        self.fire_after_battlecry(p, runs);
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
                };
                self.offer_target(p, pt);
            }
            Effect::RandomKeyword => {
                for _ in 0..mul as u8 * runs {
                    self.give_random_keyword(p, *pos);
                }
            }
            Effect::Consume { by, pick, keywords, count } => {
                let count = count.saturating_mul(mul as u8).saturating_mul(runs);
                match by {
                    ConsumeBy::This => self.start_consume(p, *pos, pick, keywords, count),
                    ConsumeBy::Chosen(tribe) => {
                        let pt = PendingTarget {
                            source: Some(*pos as u8),
                            kind: TargetKind::Consume { tribe, pick, keywords, count },
                            runs: 1,
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
            Effect::DamageHero(n) => {
                self.take_hero_damage(p, n * mul * runs as i32);
            }
            Effect::RewindDamage => {
                let pl = &mut self.players[p];
                pl.health += pl.last_damage;
                pl.last_damage = 0;
            }
            Effect::MagnetizeVolumizerAndCopy => {
                for _ in 0..runs {
                    if let Some(v_id) = self.data.cards.random_volumizer(&mut self.rng) {
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
                    if let Some(v) = self.data.cards.random_volumizer(&mut self.rng) {
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
                for _ in 0..mul * runs as i32 {
                    if let Some(c) = self.data.cards.random_pool_minion(tribe, &mut self.rng) {
                        self.give_generated_tracking(p, c, pos);
                    }
                }
            }
            Effect::GetRandomSpell { cost, count } => {
                let total = count as i32 * mul * runs as i32;
                for _ in 0..total {
                    if let Some(c) = self.data.cards.random_spell_of_cost(cost, &mut self.rng) {
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
                    if let Some(c) = self.data.cards.random_chromadrake(&mut self.rng) {
                        self.give_generated_tracking(p, c, pos);
                    }
                }
            }
            Effect::CastSpell { card, times } => {
                let Some(se) = self.data.cards.get(card).spell_effect else { return };
                for _ in 0..times as u32 * mul as u32 * runs as u32 {
                    self.execute_spell_effect(p, se);
                }
            }
            Effect::TriggerRally => {
                let pt = PendingTarget {
                    source: Some(*pos as u8),
                    kind: TargetKind::TriggerRally,
                    runs: (mul as u8).saturating_mul(runs),
                };
                self.offer_target(p, pt);
            }
            Effect::TriggerBattlecry => {
                let pt = PendingTarget {
                    source: Some(*pos as u8),
                    kind: TargetKind::TriggerBattlecry,
                    runs: (mul as u8).saturating_mul(runs),
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
                    self.cast_spell(p, sp);
                    if self.players[p].pending_target.is_some()
                        || self.players[p].pending_choose_one.is_some()
                    {
                        break;
                    }
                }
                self.maybe_start_discover(p);
            }
            // These only happen in fights.
            Effect::BuffKiller { .. }
            | Effect::BuffAttacker { .. }
            | Effect::BuffRandomOther { .. }
            | Effect::SummonRandomDeathrattle { .. }
            | Effect::TriggerDeathrattle { .. }
            | Effect::SummonFromHand { .. }
            | Effect::BuffRandomHand { .. } => {}
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
    fn refresh_with_fishbait(&mut self, p: usize, card: CardId, golden: bool) {
        let Game { data, rng, players, pool, .. } = self;
        Self::refresh_shop(&mut players[p], pool, &data.cards, &data.rules, rng);
        if !players[p].shop.is_empty() {
            self.put_fishbait(p, 0, card, golden);
        }
    }

    /// The left-most Beast on the player's board attacks the Tavern minion in
    /// `slot`. It dies, or stays if the Beast can't hurt it.
    fn tavern_attack(&mut self, p: usize, slot: usize) {
        let Game { data, rng, players, .. } = self;
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
            pl.counters(&data.cards),
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

    /// The minion in board slot `pos` consumes `count` minions from the
    /// Tavern. A chosen one waits for the player to pick (or takes the only
    /// choice there is).
    fn start_consume(&mut self, p: usize, pos: usize, pick: ConsumePick, keywords: bool, count: u8) {
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
                        kind: ShopTargetKind::Consume { keywords, remaining: left },
                    });
                    return;
                }
            };
            self.consume(p, pos, slot, keywords);
        }
    }

    /// The minion in board slot `pos` takes the Tavern minion in `slot`: its
    /// stats, and its Bonus Keywords if `keywords`. The copy goes back to the
    /// pool.
    fn consume(&mut self, p: usize, pos: usize, slot: usize, keywords: bool) {
        let Game { data, players, pool, .. } = self;
        let pl = &mut players[p];
        let eaten = pl.shop.remove(slot);
        give_back(pool, &data.cards, &eaten);
        let m = &mut pl.board[pos];
        buff(m, eaten.attack, eaten.health);
        if keywords {
            m.keywords.0 |= eaten.keywords.0 & data.rules.bonus_mask;
        }
    }

    fn resolve_shop_target(&mut self, p: usize, slot: usize) {
        let pst = self.players[p].pending_shop_target.take().unwrap();
        match pst.kind {
            ShopTargetKind::Consume { keywords, remaining } => {
                self.consume(p, pst.source as usize, slot, keywords);
                if remaining > 1 {
                    self.start_consume(p, pst.source as usize, ConsumePick::Chosen, keywords, remaining - 1);
                }
            }
            ShopTargetKind::Fishbait { card, golden } => self.put_fishbait(p, slot, card, golden),
        }
        self.maybe_start_discover(p);
    }

    /// Run every event trigger of `kind` on the player's board. Each adds
    /// `amount` to its count and fires once for every full `every` in it,
    /// keeping the remainder.
    fn fire_event(&mut self, p: usize, kind: EventKind, amount: u32) {
        if amount == 0 {
            return;
        }
        let mut pos = 0;
        while pos < self.players[p].board.len() {
            let m = self.players[p].board[pos];
            for (card, golden) in m.parts() {
                let Some(t) = self.data.cards.get(card).on_event else { continue };
                if t.kind != kind || pos >= self.players[p].board.len() {
                    continue;
                }
                let imp = &mut self.players[p].board[pos].improvements;
                let mut count = *imp as u32 + amount;
                let mut fires = 0;
                while count >= t.every as u32 {
                    count -= t.every as u32;
                    fires += 1;
                }
                *imp = count as u16;
                for _ in 0..fires {
                    self.run_effect(p, &mut pos, t.effect, golden, 1);
                }
            }
            pos += 1;
        }
    }

    /// The hero takes `amount` damage, then the "after your hero takes
    /// damage" effects and the damage event triggers run. They fire even
    /// when a rewind among them gives the Health back. Damage from a card
    /// effect can kill (the hero is knocked out after the fights); Health
    /// costs are checked by `can_pay_health` before they're paid.
    fn take_hero_damage(&mut self, p: usize, amount: i32) {
        let pl = &mut self.players[p];
        if amount <= 0 {
            return;
        }
        pl.health -= amount;
        pl.last_damage = amount;
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

    fn offer_target(&mut self, p: usize, pt: PendingTarget) {
        let mut targets = (0..self.players[p].board.len()).filter(|&s| self.valid_target(p, &pt, s));
        match (targets.next(), targets.next()) {
            (None, _) => {}
            (Some(s), None) => self.apply_target(p, &pt, s),
            _ => self.players[p].pending_target = Some(pt),
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
            TargetKind::Consume { pick, keywords, count, .. } => {
                self.start_consume(p, s, pick, keywords, count);
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
                for _ in 0..pt.runs {
                    if target_pos >= self.players[p].board.len() {
                        break;
                    }
                    let (da, dh) = self.total_spell_bonus(p);
                    let mut generated: Option<CardId> = None;
                    match st {
                        SpellTarget::Buff {
                            attack,
                            health,
                            keyword,
                            times,
                            if_win_next_turn_buff,
                        } => {
                            let m = &mut self.players[p].board[target_pos];
                            for _ in 0..times {
                                buff(m, attack + da, health + dh);
                            }
                            if keyword != 0 {
                                m.keywords.set(keyword);
                            }
                            if let Some((wb_a, wb_h)) = if_win_next_turn_buff {
                                m.winners_bread = m.winners_bread.saturating_add(1);
                                self.players[p].winners_bread_buff = (wb_a, wb_h);
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
                            for o in &mut self.players[p].board {
                                if self.data.cards.get(o.card).tribes.overlaps(t) {
                                    buff(o, attack + da, health + dh);
                                }
                            }
                        }
                        SpellTarget::SameTribeMinion => {
                            let c_id = self.players[p].board[target_pos].card;
                            let t = self.data.cards.get(c_id).tribes;
                            generated =
                                self.data.cards.random_different_minion(t, c_id, &mut self.rng);
                        }
                        SpellTarget::SetStats { attack, health } => {
                            let m = &mut self.players[p].board[target_pos];
                            m.attack = attack;
                            m.health = health;
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
                    }
                    self.trigger_on_cast_spell(p);
                    self.trigger_on_cast_spell_on_target(p, target_pos);
                    let is_mech = self
                        .data
                        .cards
                        .get(self.players[p].board[target_pos].card)
                        .tribes
                        .has(Tribe::Mech);
                    if is_mech {
                        self.trigger_on_cast_spell_on_mech(p, target_pos);
                    }
                    if let Some(gen_c) = generated {
                        self.give_generated_tracking(p, gen_c, &mut target_pos);
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
        while self.players[p].pending_target.is_none()
            && self.players[p].pending_shop_target.is_none()
            && self.players[p].pending_choose_one.is_none()
            && self.players[p].discover.is_empty()
            && !self.players[p].discover_queue.is_empty()
        {
            let req = self.players[p].discover_queue.remove(0);
            let Game { data, rng, players, pool, .. } = self;
            let pl = &mut players[p];
            while !pl.discover.is_full() {
                let next = match req {
                    DiscoverRequest::MinionTier(t) => {
                        draw(pool, &data.cards, rng, t, t, &pl.discover)
                    }
                    DiscoverRequest::SpellTier(t) => {
                        data.cards.random_tier_spell(t, &pl.discover, rng)
                    }
                    DiscoverRequest::DeathrattleMinion(max_t) => draw_filtered(
                        pool,
                        &data.cards,
                        rng,
                        1,
                        max_t,
                        &pl.discover,
                        |c| c.deathrattle.is_some(),
                    ),
                    DiscoverRequest::BattlecryMinion(max_t) => draw_filtered(
                        pool,
                        &data.cards,
                        rng,
                        1,
                        max_t,
                        &pl.discover,
                        |c| c.battlecry.is_some(),
                    ),
                    DiscoverRequest::TribeMinion(t) => {
                        draw_filtered(pool, &data.cards, rng, 1, MAX_TIER, &pl.discover, |c| {
                            c.tribes.has(t)
                        })
                    }
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
            match pco.options[k] {
                ChooseOption::MinionTarget { tribe, attack, health, keyword } => {
                    let pt = PendingTarget {
                        source: pco.source,
                        kind: TargetKind::MinionBuff {
                            tribe,
                            attack,
                            health,
                            keyword,
                            random_keyword: false,
                        },
                        runs: 1,
                    };
                    self.offer_target(p, pt);
                }
                ChooseOption::SpellBonus { attack, health } => {
                    self.players[p].spell_bonus.0 += attack;
                    self.players[p].spell_bonus.1 += health;
                }
                ChooseOption::SpellTarget(st) => {
                    let runs = self.balinda_mult(p);
                    self.players[p].pending_target = Some(PendingTarget {
                        source: None,
                        kind: TargetKind::Spell(st),
                        runs,
                    });
                }
                ChooseOption::SpellEffect(se) => {
                    self.trigger_on_cast_spell(p);
                    self.execute_spell_effect(p, se);
                }
            }
            // A minion's Choose One is its Battlecry.
            if pco.source.is_some() {
                self.fire_after_battlecry(p, 1);
            }
            self.maybe_start_discover(p);
            return;
        }
        let Game { data, players, pool, .. } = self;
        let pl = &mut players[p];
        let c = pl.discover[k];
        for (i, &o) in pl.discover.iter().enumerate() {
            if i != k && !data.cards.get(o).is_spell() {
                let slot = data.cards.get(o).pool_slot as usize;
                pool[slot] += 1;
            }
        }
        pl.discover.clear();
        let is_spell = data.cards.get(c).is_spell();
        if !pl.hand.is_full() {
            let pool_copies = if is_spell { 0 } else { 1 };
            let m = pl.make_card(&data.cards, c, false, pool_copies);
            pl.hand.push(m);
            if !is_spell {
                self.check_triple(p, c);
            }
        } else if !is_spell {
            let slot = self.data.cards.get(c).pool_slot as usize;
            self.pool[slot] += 1;
        }
        self.maybe_start_discover(p);
    }

    fn give_generated(&mut self, p: usize, card: CardId) {
        let mut dummy = 0;
        self.give_generated_tracking(p, card, &mut dummy);
    }

    fn give_generated_tracking(&mut self, p: usize, card: CardId, tracked_pos: &mut usize) {
        if self.players[p].hand.is_full() {
            return;
        }
        let m = self.players[p].make_card(&self.data.cards, card, false, 0);
        self.players[p].hand.push(m);
        if !self.data.cards.get(card).is_spell() {
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
        let pl = &mut self.players[p];
        let is_copy = |m: &Minion| m.card == card && !m.golden;
        let n = pl.hand.iter().filter(|m| is_copy(m)).count()
            + pl.board.iter().filter(|m| is_copy(m)).count();
        if n < 3 {
            return;
        }
        let base = pl.make_card(cards, card, false, 0);
        let mut gold = pl.make_card(cards, card, true, 0);
        let mut left = 3;
        // a Beast on the board carries the until-next-turn Beast bonus; the
        // golden one in hand gets it when it is played, not twice
        let board_bonus =
            if cards.get(card).tribes.has(Tribe::Beast) { pl.beast_bonus } else { (0, 0) };
        let mut take = |m: Minion, bonus: (i32, i32)| {
            gold.attack += m.attack - bonus.0 - base.attack;
            gold.health += m.health - bonus.1 - base.health;
            gold.keywords.0 |= m.keywords.0;
            gold.pool_copies = gold.pool_copies.saturating_add(m.pool_copies);
            gold.winners_bread = gold.winners_bread.saturating_add(m.winners_bread);
            gold.magnetizations = gold.magnetizations.saturating_add(m.magnetizations);
            gold.volumizers =
                gold.volumizers.saturating_add(m.volumizers.saturating_sub(base.volumizers));
            gold.improvements = gold.improvements.saturating_add(m.improvements);
            for &a in m.attached_slice() {
                gold.push_attached(a);
            }
        };
        while left > 0 {
            if let Some(i) = pl.hand.iter().position(is_copy) {
                take(pl.hand.remove(i), (0, 0));
            } else {
                let i = pl.board.iter().position(is_copy).unwrap();
                if i < *tracked_pos {
                    *tracked_pos -= 1;
                }
                take(pl.board.remove(i), board_bonus);
            }
            left -= 1;
        }
        pl.hand.push(gold);
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
                                if let Some(v_id) = self.data.cards.random_volumizer(&mut self.rng)
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
    }

    /// The player is done for this turn. Anything still waiting on a choice
    /// takes the first option, then end-of-turn effects run.
    fn finish_shop(&mut self, p: usize) {
        loop {
            let pl = &self.players[p];
            if let Some(pt) = pl.pending_target {
                let s = (0..pl.board.len()).find(|&s| self.valid_target(p, &pt, s)).unwrap();
                self.resolve_target(p, s);
            } else if pl.pending_shop_target.is_some() {
                self.resolve_shop_target(p, 0);
            } else if let Some(pco) = pl.pending_choose_one {
                let k = (0..2).find(|&k| self.can_choose_option(p, pco.options[k])).unwrap();
                self.resolve_choose(p, k);
            } else if !pl.discover.is_empty() {
                self.resolve_choose(p, 0);
            } else {
                break;
            }
        }
        self.run_end_of_turn(p);
        for m in &mut self.players[p].board {
            m.extra_magnetize = 0;
        }
        self.players[p].end_turn_tavern_buffs(&self.data.cards);
        self.players[p].done = true;
    }

    fn advance(&mut self) {
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
            let Game { data, rng, players, pool, .. } = self;
            let pl = &mut players[p];
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
            pl.done = false;
            pl.actions = 0;
            pl.health_refreshes_used = 0;
            pl.mycologist_uses = 0;
            if pl.frozen {
                pl.frozen = false;
            } else {
                for m in pl.shop.drain(..) {
                    give_back(pool, &data.cards, &m);
                }
                pl.shop_spell = None;
            }
            let size = data.rules.shop_size(pl.tier);
            Self::fill_shop(pl, pool, &data.cards, size, rng);
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
        }
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
        if after_fight {
            self.players[p].boon_of_beetles =
                self.players[p].boon_of_beetles.saturating_sub(so.boon_of_beetles_used);
            self.players[p].brood_of_nozdormu = 0;
            self.players[p].upper_hand = 0;
        }
        for &c in so.cards_gained() {
            self.give_generated(p, c);
        }
    }

    fn combat(&mut self) {
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
                    (q.board.clone(), q.tier, q.counters(cards), hand)
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
            let my_counters = self.players[p].counters(cards);
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
            let turn = self.turn;
            match self.players[p].opponent.unwrap() {
                Opponent::Player(q) => {
                    let q = q as usize;
                    self.players[p].seen[q] = Seen { board: them, tier, turn };
                    let my_tier = self.players[p].tier;
                    self.players[q].seen[p] = Seen { board: me, tier: my_tier, turn };
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
                    self.players[p].seen[g as usize] = Seen { board: them, tier, turn };
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
            self.take_hero_damage(p, d);
            let pl = &mut self.players[p];
            pl.last_opponent = pl.opponent.take();
        }
        self.knockouts(alive);
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
            let Game { data, players, pool, ghost, .. } = self;
            let pl = &mut players[p as usize];
            pl.alive = false;
            pl.placement = (first_place + k) as u8;
            pl.final_tribes = pl.board_tribes(&data.cards);
            if k == 0 {
                *ghost = Some(Ghost {
                    player: p,
                    board: pl.board.clone(),
                    tier: pl.tier,
                    counters: pl.counters(&data.cards),
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
            // generated cards (Fishbait) may be any Tier, a Golden Touch makes a
            // minion in the Tavern golden
            if pl.shop.iter().any(|m| !cards.get(m.card).token && cards.get(m.card).tier > pl.tier) {
                return Err(format!("player {i}: shop has a card it shouldn't"));
            }
            if let Some(sp) = pl.shop_spell
                && (!cards.get(sp).is_spell() || cards.get(sp).tier > pl.tier)
            {
                return Err(format!("player {i}: shop_spell invalid"));
            }
            if pl.shop.len() > self.data.rules.shop_size(pl.tier) {
                return Err(format!("player {i}: shop too big"));
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
            let want = if card.pool_slot as usize == c {
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
            self.players[p].hand.push(m);
            return Some(m);
        }
        let slot = card.pool_slot as usize;
        if self.pool.get(slot).copied().unwrap_or(0) == 0 {
            return None;
        }
        self.pool[slot] -= 1;
        let m = self.players[p].make_card(&self.data.cards, c, false, 1);
        self.players[p].hand.push(m);
        Some(m)
    }
}
