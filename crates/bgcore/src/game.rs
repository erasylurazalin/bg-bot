//! One 8-player game: shop phase, pairing, fights, knockouts.
//!
//! The shop phase is simultaneous in the real game. Here the players act one
//! action at a time, in a random order that changes every turn, until each
//! has ended their turn. `current_player` says whose action is next.

use std::sync::Arc;

use arrayvec::ArrayVec;

use crate::action::{Action, N_ACTIONS};
use crate::cards::{CardDb, Effect, Tribe};
use crate::fight::fight;
use crate::minion::{CardId, Minion};
use crate::rng::Rng;
use crate::{BOARD, DISCOVER, Data, HAND, MAX_TIER, PLAYERS, SHOP};

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

/// A targeted Battlecry waiting for the player to pick its target.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PendingTarget {
    /// Board slot of the minion whose Battlecry it is.
    pub source: u8,
    pub tribe: Option<Tribe>,
    pub attack: i32,
    pub health: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Player {
    pub health: i32,
    pub alive: bool,
    /// 1 to 8 once decided, 0 while still playing.
    pub placement: u8,
    pub gold: u8,
    pub tier: u8,
    pub upgrade_cost: u8,
    pub frozen: bool,
    pub shop: ArrayVec<Minion, SHOP>,
    pub hand: ArrayVec<Minion, HAND>,
    pub board: ArrayVec<Minion, BOARD>,
    pub pending_target: Option<PendingTarget>,
    /// Discover options waiting for a choice. The cards are out of the pool.
    pub discover: ArrayVec<CardId, DISCOVER>,
    /// A Discover to offer once the pending target is picked.
    pub discover_queued: bool,
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
            gold: 0,
            tier: 1,
            upgrade_cost: data.rules.upgrade_cost(1),
            frozen: false,
            shop: ArrayVec::new(),
            hand: ArrayVec::new(),
            board: ArrayVec::new(),
            pending_target: None,
            discover: ArrayVec::new(),
            discover_queued: false,
            actions: 0,
            done: false,
            opponent: None,
            last_opponent: None,
            seen: Default::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Ghost {
    pub player: u8,
    pub board: ArrayVec<Minion, BOARD>,
    pub tier: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IllegalAction;

#[derive(Clone, Debug)]
pub struct Game {
    pub data: Arc<Data>,
    pub rng: Rng,
    pub turn: u32,
    pub players: [Player; PLAYERS],
    /// Copies left of each pool card, indexed by card id.
    pub pool: Vec<u8>,
    pub ghost: Option<Ghost>,
    pub over: bool,
    order: ArrayVec<u8, PLAYERS>,
    cursor: usize,
}

/// Draw one card from the pool, weighted by copies left, among cards of
/// tier `lo..=hi` that aren't in `exclude`.
fn draw(pool: &mut [u8], cards: &CardDb, rng: &mut Rng, lo: u8, hi: u8, exclude: &[CardId]) -> Option<CardId> {
    let ok = |c: usize| {
        let t = cards.cards[c].tier;
        lo <= t && t <= hi && !exclude.contains(&(c as CardId))
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
            return Some(c as CardId);
        }
        r -= n;
    }
    unreachable!()
}

fn give_back(pool: &mut [u8], cards: &CardDb, m: &Minion) {
    if !cards.get(m.card).token {
        pool[m.card as usize] += if m.golden { 3 } else { 1 };
    }
}

fn buff(m: &mut Minion, attack: i32, health: i32) {
    m.attack += attack;
    m.health += health;
}

impl Game {
    pub fn new(data: Arc<Data>, seed: u64) -> Game {
        let pool = (0..data.cards.pool_cards)
            .map(|c| data.rules.copies(data.cards.cards[c].tier))
            .collect();
        let players = std::array::from_fn(|_| Player::new(&data));
        let mut g = Game {
            data,
            rng: Rng::new(seed),
            turn: 0,
            players,
            pool,
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
        if !pl.discover.is_empty() {
            return matches!(a, Action::Choose(k) if (k as usize) < pl.discover.len());
        }
        match a {
            Action::Buy(s) => (s as usize) < pl.shop.len() && pl.gold >= r.buy_cost && !pl.hand.is_full(),
            Action::Sell(s) => (s as usize) < pl.board.len(),
            Action::Play { hand, pos } => {
                (hand as usize) < pl.hand.len() && !pl.board.is_full() && pos as usize <= pl.board.len()
            }
            Action::Target(_) | Action::Choose(_) => false,
            Action::Move { from, to } => {
                (from as usize) < pl.board.len() && (to as usize) < pl.board.len() && from != to
            }
            Action::Roll => pl.gold >= r.roll_cost,
            Action::Freeze | Action::EndTurn => true,
            Action::Level => pl.tier < MAX_TIER && pl.gold >= pl.upgrade_cost,
        }
    }

    pub fn legal_mask(&self, mask: &mut [bool; N_ACTIONS]) {
        for (i, m) in mask.iter_mut().enumerate() {
            *m = self.is_legal(Action::from_index(i).unwrap());
        }
    }

    fn valid_target(&self, p: usize, pt: &PendingTarget, s: usize) -> bool {
        let b = &self.players[p].board;
        s < b.len() && s != pt.source as usize && self.data.cards.get(b[s].card).is(pt.tribe)
    }

    /// Take an action for the current player.
    pub fn step(&mut self, a: Action) -> Result<(), IllegalAction> {
        if !self.is_legal(a) {
            return Err(IllegalAction);
        }
        let p = self.current_player().unwrap();
        self.apply(p, a);
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
        let Game { data, rng, players, pool, .. } = self;
        let (cards, rules) = (&data.cards, &data.rules);
        let pl = &mut players[p];
        match a {
            Action::Buy(s) => {
                let m = pl.shop.remove(s as usize);
                pl.gold -= rules.buy_cost;
                pl.hand.push(m);
                self.check_triple(p, m.card);
            }
            Action::Sell(s) => {
                let m = pl.board.remove(s as usize);
                pl.gold = pl.gold.saturating_add(rules.sell_gold);
                give_back(pool, cards, &m);
            }
            Action::Play { hand, pos } => {
                let m = pl.hand.remove(hand as usize);
                pl.board.insert(pos as usize, m);
                self.battlecry(p, pos as usize);
                if m.golden {
                    let pl = &mut self.players[p];
                    pl.discover_queued = true;
                    if pl.pending_target.is_none() {
                        self.start_discover(p);
                    }
                }
            }
            Action::Target(s) => self.resolve_target(p, s as usize),
            Action::Choose(k) => self.resolve_choose(p, k as usize),
            Action::Move { from, to } => {
                let m = pl.board.remove(from as usize);
                pl.board.insert(to as usize, m);
            }
            Action::Roll => {
                pl.gold -= rules.roll_cost;
                pl.frozen = false;
                let tier = pl.tier;
                for m in pl.shop.drain(..) {
                    give_back(pool, cards, &m);
                }
                Self::fill_shop(pl, pool, cards, rules.shop_size(tier), rng);
            }
            Action::Freeze => pl.frozen = !pl.frozen,
            Action::Level => {
                pl.gold -= pl.upgrade_cost;
                pl.tier += 1;
                pl.upgrade_cost = rules.upgrade_cost(pl.tier);
            }
            Action::EndTurn => {}
        }
    }

    fn fill_shop(pl: &mut Player, pool: &mut [u8], cards: &CardDb, size: usize, rng: &mut Rng) {
        while pl.shop.len() < size {
            match draw(pool, cards, rng, 1, pl.tier, &[]) {
                Some(c) => pl.shop.push(cards.make(c, false)),
                None => break,
            }
        }
    }

    fn battlecry(&mut self, p: usize, pos: usize) {
        let cards = &self.data.cards;
        let pl = &mut self.players[p];
        let m = pl.board[pos];
        let mul = if m.golden { 2 } else { 1 };
        match cards.get(m.card).battlecry {
            Some(Effect::BuffOthers { tribe, attack, health }) => {
                for (i, o) in pl.board.iter_mut().enumerate() {
                    if i != pos && cards.get(o.card).is(tribe) {
                        buff(o, attack * mul, health * mul);
                    }
                }
            }
            Some(Effect::BuffChosen { tribe, attack, health }) => {
                let pt = PendingTarget { source: pos as u8, tribe, attack: attack * mul, health: health * mul };
                let mut targets = (0..pl.board.len()).filter(|&s| self.valid_target(p, &pt, s));
                match (targets.next(), targets.next()) {
                    (None, _) => {}
                    (Some(s), None) => buff(&mut self.players[p].board[s], pt.attack, pt.health),
                    _ => self.players[p].pending_target = Some(pt),
                }
            }
            _ => {}
        }
    }

    fn resolve_target(&mut self, p: usize, s: usize) {
        let pl = &mut self.players[p];
        let pt = pl.pending_target.take().unwrap();
        buff(&mut pl.board[s], pt.attack, pt.health);
        if pl.discover_queued {
            self.start_discover(p);
        }
    }

    /// The Triple Reward: 3 different minions from one tier up, taken out
    /// of the pool until the player picks one.
    fn start_discover(&mut self, p: usize) {
        let Game { data, rng, players, pool, .. } = self;
        let pl = &mut players[p];
        pl.discover_queued = false;
        let tier = (pl.tier + 1).min(MAX_TIER);
        while !pl.discover.is_full() {
            match draw(pool, &data.cards, rng, tier, tier, &pl.discover) {
                Some(c) => pl.discover.push(c),
                None => break,
            }
        }
    }

    fn resolve_choose(&mut self, p: usize, k: usize) {
        let Game { data, players, pool, .. } = self;
        let pl = &mut players[p];
        let c = pl.discover[k];
        for (i, &o) in pl.discover.iter().enumerate() {
            if i != k {
                pool[o as usize] += 1;
            }
        }
        pl.discover.clear();
        pl.hand.push(data.cards.make(c, false));
        self.check_triple(p, c);
    }

    /// Three copies of a card in hand and on board become a golden one in
    /// hand, with double printed stats plus the buffs of all three.
    fn check_triple(&mut self, p: usize, card: CardId) {
        let cards = &self.data.cards;
        let pl = &mut self.players[p];
        let is_copy = |m: &Minion| m.card == card && !m.golden;
        let n = pl.hand.iter().filter(|m| is_copy(m)).count() + pl.board.iter().filter(|m| is_copy(m)).count();
        if n < 3 {
            return;
        }
        let base = cards.make(card, false);
        let mut gold = cards.make(card, true);
        let mut left = 3;
        let mut take = |m: Minion| {
            gold.attack += m.attack - base.attack;
            gold.health += m.health - base.health;
        };
        while left > 0 {
            if let Some(i) = pl.hand.iter().position(is_copy) {
                take(pl.hand.remove(i));
            } else {
                let i = pl.board.iter().position(is_copy).unwrap();
                take(pl.board.remove(i));
            }
            left -= 1;
        }
        pl.hand.push(gold);
    }

    /// The player is done for this turn. Anything still waiting on a choice
    /// takes the first option.
    fn finish_shop(&mut self, p: usize) {
        loop {
            let pl = &self.players[p];
            if let Some(pt) = pl.pending_target {
                let s = (0..pl.board.len()).find(|&s| self.valid_target(p, &pt, s)).unwrap();
                self.resolve_target(p, s);
            } else if !pl.discover.is_empty() {
                self.resolve_choose(p, 0);
            } else {
                break;
            }
        }
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
        let Game { data, rng, players, pool, turn, .. } = self;
        let gold = data.rules.gold(*turn);
        for pl in players.iter_mut().filter(|p| p.alive) {
            if *turn > 1 {
                pl.upgrade_cost = pl.upgrade_cost.saturating_sub(1);
            }
            pl.gold = gold;
            pl.done = false;
            pl.actions = 0;
            if pl.frozen {
                pl.frozen = false;
            } else {
                for m in pl.shop.drain(..) {
                    give_back(pool, &data.cards, &m);
                }
            }
            let size = data.rules.shop_size(pl.tier);
            Self::fill_shop(pl, pool, &data.cards, size, rng);
        }
        self.pair();
        self.order = (0..PLAYERS as u8).filter(|&p| self.players[p as usize].alive).collect();
        self.rng.shuffle(&mut self.order);
        self.cursor = 0;
    }

    /// Pair the living players at random, avoiding last turn's opponent
    /// when a few shuffles can. With an odd count, one fights the ghost.
    fn pair(&mut self) {
        let mut alive: ArrayVec<u8, PLAYERS> = (0..PLAYERS as u8).filter(|&p| self.players[p as usize].alive).collect();
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

    fn combat(&mut self) {
        let alive = self.alive();
        let rules = &self.data.rules;
        let cap = rules.damage_cap(self.turn, alive);
        let capped = |d: i32| cap.map_or(d, |c| d.min(c));
        let mut damage = [0i32; PLAYERS];
        for p in 0..PLAYERS {
            if !self.players[p].alive {
                continue;
            }
            let (them, tier) = match self.players[p].opponent {
                Some(Opponent::Player(q)) if p < q as usize => {
                    let q = &self.players[q as usize];
                    (q.board.clone(), q.tier)
                }
                Some(Opponent::Ghost(_)) => {
                    let g = self.ghost.as_ref().unwrap();
                    (g.board.clone(), g.tier)
                }
                _ => continue,
            };
            let me = self.players[p].board.clone();
            let out = fight(&self.data.cards, &me, &them, rules.max_attacks, &mut self.rng);
            let turn = self.turn;
            match self.players[p].opponent.unwrap() {
                Opponent::Player(q) => {
                    let q = q as usize;
                    self.players[p].seen[q] = Seen { board: them, tier, turn };
                    let my_tier = self.players[p].tier;
                    self.players[q].seen[p] = Seen { board: me, tier: my_tier, turn };
                    match out.winner {
                        Some(0) => damage[q] += capped(my_tier as i32 + out.survivor_tiers),
                        Some(_) => damage[p] += capped(tier as i32 + out.survivor_tiers),
                        None => {}
                    }
                }
                Opponent::Ghost(g) => {
                    self.players[p].seen[g as usize] = Seen { board: them, tier, turn };
                    if out.winner == Some(1) {
                        damage[p] += capped(tier as i32 + out.survivor_tiers);
                    }
                }
            }
        }
        for (pl, d) in self.players.iter_mut().zip(damage) {
            pl.health -= d;
            pl.last_opponent = pl.opponent.take();
        }
        self.knockouts(alive);
    }

    fn knockouts(&mut self, alive: usize) {
        let mut dead: ArrayVec<u8, PLAYERS> =
            (0..PLAYERS as u8).filter(|&p| self.players[p as usize].alive && self.players[p as usize].health <= 0).collect();
        // best first: more health left, ties at random
        self.rng.shuffle(&mut dead);
        dead.sort_by_key(|&p| std::cmp::Reverse(self.players[p as usize].health));
        let first_place = alive - dead.len() + 1;
        for (k, &p) in dead.iter().enumerate() {
            let Game { data, players, pool, ghost, .. } = self;
            let pl = &mut players[p as usize];
            pl.alive = false;
            pl.placement = (first_place + k) as u8;
            if k == 0 {
                *ghost = Some(Ghost { player: p, board: pl.board.clone(), tier: pl.tier });
            }
            for m in pl.shop.drain(..).chain(pl.hand.drain(..)).chain(pl.board.drain(..)) {
                give_back(pool, &data.cards, &m);
            }
            for c in pl.discover.drain(..) {
                pool[c as usize] += 1;
            }
            pl.pending_target = None;
            pl.discover_queued = false;
        }
        let mut left: ArrayVec<u8, PLAYERS> = (0..PLAYERS as u8).filter(|&p| self.players[p as usize].alive).collect();
        if left.len() <= 1 || self.turn >= self.data.rules.max_turns {
            self.rng.shuffle(&mut left);
            left.sort_by_key(|&p| std::cmp::Reverse(self.players[p as usize].health));
            for (k, &p) in left.iter().enumerate() {
                self.players[p as usize].placement = k as u8 + 1;
            }
            self.over = true;
        }
    }

    /// Every card copy is in the pool or held by someone, nothing is over
    /// its limit, and the dead hold nothing. Checked after every step in
    /// debug builds; the fuzzer calls it in release too.
    pub fn check_invariants(&self) -> Result<(), String> {
        let cards = &self.data.cards;
        let mut held = vec![0u32; cards.pool_cards];
        for (i, pl) in self.players.iter().enumerate() {
            for m in pl.shop.iter().chain(&pl.hand).chain(&pl.board) {
                if !cards.get(m.card).token {
                    held[m.card as usize] += if m.golden { 3 } else { 1 };
                }
            }
            for &c in &pl.discover {
                held[c as usize] += 1;
            }
            if pl.shop.iter().any(|m| m.golden || cards.get(m.card).tier > pl.tier) {
                return Err(format!("player {i}: shop has a card it shouldn't"));
            }
            if pl.shop.len() > self.data.rules.shop_size(pl.tier) {
                return Err(format!("player {i}: shop too big"));
            }
            if let Some(pt) = pl.pending_target
                && pt.source as usize >= pl.board.len()
            {
                return Err(format!("player {i}: pending target source off the board"));
            }
            let holds_cards = !(pl.shop.is_empty() && pl.hand.is_empty() && pl.board.is_empty() && pl.discover.is_empty());
            if !pl.alive && holds_cards {
                return Err(format!("player {i}: dead but holds cards"));
            }
            if pl.alive && pl.health <= 0 && !self.over {
                return Err(format!("player {i}: alive at {} health", pl.health));
            }
            if !(1..=MAX_TIER).contains(&pl.tier) {
                return Err(format!("player {i}: tier {}", pl.tier));
            }
        }
        for c in 0..cards.pool_cards {
            let want = self.data.rules.copies(cards.cards[c].tier) as u32;
            let have = self.pool[c] as u32 + held[c];
            if have != want {
                return Err(format!("{}: {have} copies, should be {want}", cards.cards[c].name));
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

    /// Take a pool card straight into a player's hand. For tests and
    /// setting up positions, not part of the game.
    pub fn give(&mut self, p: usize, name: &str) -> Option<Minion> {
        let c = self.data.cards.by_name(name)?;
        if self.pool.get(c as usize).copied().unwrap_or(0) == 0 || self.players[p].hand.is_full() {
            return None;
        }
        self.pool[c as usize] -= 1;
        let m = self.data.cards.make(c, false);
        self.players[p].hand.push(m);
        Some(m)
    }
}
