//! What the shop-turn search needs from the engine (`train/search.py`, the
//! "Search v1" brief in `docs/briefs/`): the dominance comparison of two
//! positions on copies of one game, the full pool a playout's Refresh draws
//! from, and the coming fight played against the boards of Era's real games.
//!
//! Nothing here changes a rule. The copies are the caller's (crates/bgpy's
//! `SearchBatch`): they are `Game` clones, they step their own actions and
//! they never touch the game they came from.

use crate::cards::Tribe;
use crate::fight::{Deity, fight_with_hands};
use crate::game::{Game, Ghost, Player};
use crate::minion::{Keywords, Minion};
use crate::real_boards::{RealBoard, RealBoards};
use crate::rng::Rng;
use crate::{Action, BOARD, Data, HAND, IllegalAction, PLAYERS};

// ---- dominance: Era's "same position, only better" ----

/// The verdict of one dominance comparison.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dominance {
    /// Nothing got worse and something got better.
    pub dominates: bool,
    /// Something got better, whether or not something got worse.
    pub better: bool,
    /// Everything that got worse, one line each: the field names, or a best
    /// effort reading of the state that changed outside them. Empty when the
    /// comparison found nothing worse.
    pub blocked: Vec<String>,
}

impl Dominance {
    pub fn of(before: &Position, after: &Game) -> Dominance {
        before.judge(after)
    }
}

/// A position to judge later positions against: the player's own state and
/// the other seven exactly as they stood, for one turn.
///
/// What it compares, field by field (Era's list in the brief):
///
/// - **At least as good** (`>=`, or `<=` for the two prices): Gold now, Gold
///   next turn, the extra maximum Gold, free Refreshes, Health, armor, the
///   overconfidence Gold, Winner's Bread, Boons of Beetles, Brood of
///   Nozdormu, Upper Hand, Sharing is Caring, the Volumizer, Beetle, Beast,
///   Blood Gem (all four numbers), Barrage and spell bonuses, the spell
///   discount, the hat, `enhance_times`, the ticket chance and Tickets, the
///   free minion buy, the `improve` counters, the hero counters (cards
///   bought, minions bought and sold, Gold spent, spells bought, Battlecries
///   bought and triggered, deathrattles, Discovers, Magnetizations, ...), the
///   Undead bonus, Eternal Knight deaths, `discarded`, `gold_spent_turn`,
///   `elementals_played_turn`, `spells_cast_turn`, `elementals_played`,
///   `refresh_buff`, Conviction, the Deity's stats, a hero power's uses left
///   and uses this game, and the board and hand minions (below). The Tavern
///   upgrade cost must be no higher.
/// - **The same**: everything else in `Player`, exactly: the tier, the hero
///   and both powers otherwise (which power, unlocked, its progress, its
///   price delta, armed), the trinkets and their state, the Prizes, the
///   Secrets, the two Health-Refresh numbers, `last_damage`, the two
///   "last combat" flags, the pending choice state, the Discover state, the
///   leaderboard and last-seen boards, the last three fights, the placement,
///   and the other seven players whole.
/// - **Ignored**: the Tavern's contents (`shop`, `shop_spell`,
///   `shop_spell_ticket`), the freeze, `recent_buys` (a buy appends to it by
///   design), the action count (`actions`: a cycle spends actions, that is
///   the point), and the game's pool, RNG, turn order and action transients.
///
/// A board or hand minion must be the same card in the same place with
/// Attack, Health, keywords and `improvements` at least as good and not
/// Golden turned plain; everything else about it must be identical.
#[derive(Clone, Debug)]
pub struct Position {
    /// The turn the position belongs to.
    pub turn: u32,
    /// The seat it belongs to.
    pub p: usize,
    me: Player,
    others: [Player; PLAYERS],
    over: bool,
    ghost: Option<Ghost>,
}

impl Position {
    pub fn of(game: &Game, p: usize) -> Position {
        Position {
            turn: game.turn,
            p,
            me: game.players[p].clone(),
            others: std::array::from_fn(|q| game.players[q].clone()),
            over: game.over,
            ghost: game.ghost.clone(),
        }
    }

    /// Judge `after` (another copy of the same game) against this position.
    pub fn judge(&self, after: &Game) -> Dominance {
        let mut v = Dominance::default();
        if after.turn != self.turn {
            v.blocked.push(format!("turn: {} -> {}", self.turn, after.turn));
            return v;
        }
        if after.over != self.over {
            v.blocked.push("game over".to_string());
        }
        if after.ghost != self.ghost {
            v.blocked.push("ghost".to_string());
        }
        for q in 0..PLAYERS {
            if q != self.p && after.players[q] != self.others[q] {
                v.blocked.push(format!("player {q}"));
            }
        }
        let mut b = after.players[self.p].clone();
        judge_player(&self.me, &mut b, &mut v);
        if self.me != b {
            v.blocked.push(first_difference(&self.me, &b));
        }
        v.dominates = v.blocked.is_empty() && v.better;
        v
    }
}

/// One thing that can be at least as good, equal or worse than another.
trait Part {
    /// (self is worse than other, self is better than other)
    fn part(&self, other: &Self) -> (bool, bool);
}

macro_rules! part_ordered {
    ($($t:ty),*) => {$(
        impl Part for $t {
            fn part(&self, other: &Self) -> (bool, bool) {
                (self < other, self > other)
            }
        }
    )*};
}
part_ordered!(u8, u16, u32, i32, u64);

/// A pair of numbers: any lower is worse, any higher is better.
impl Part for (i32, i32) {
    fn part(&self, other: &Self) -> (bool, bool) {
        (self.0 < other.0 || self.1 < other.1, self.0 > other.0 || self.1 > other.1)
    }
}

impl Part for (u8, i32, i32) {
    fn part(&self, other: &Self) -> (bool, bool) {
        (
            self.0 < other.0 || self.1 < other.1 || self.2 < other.2,
            self.0 > other.0 || self.1 > other.1 || self.2 > other.2,
        )
    }
}

impl<const N: usize> Part for [u16; N] {
    fn part(&self, other: &Self) -> (bool, bool) {
        (self.iter().zip(other).any(|(s, o)| s < o), self.iter().zip(other).any(|(s, o)| s > o))
    }
}

/// Every counter a hero power keeps: more is better.
impl Part for crate::game::HeroCounts {
    fn part(&self, other: &Self) -> (bool, bool) {
        let pairs = [
            (self.cards_bought, other.cards_bought),
            (self.minions_bought, other.minions_bought),
            (self.tiers_bought, other.tiers_bought),
            (self.minions_sold, other.minions_sold),
            (self.attacks, other.attacks),
            (self.gold_spent, other.gold_spent),
            (self.tavern_spells_bought, other.tavern_spells_bought),
            (self.battlecry_bought, other.battlecry_bought),
        ];
        (pairs.iter().any(|&(s, o)| s < o), pairs.iter().any(|&(s, o)| s > o))
    }
}

fn put<T: Clone>(dst: &mut T, src: &T) {
    *dst = src.clone();
}

/// Fields where more is better: `worse` when `b` has less.
macro_rules! judge_ge {
    ($v:expr, $a:expr, $b:expr, $($f:ident),* $(,)?) => {$(
        {
            let (worse, better) = $b.$f.part(&$a.$f);
            if worse {
                $v.blocked.push(format!("{}: {:?} -> {:?}", stringify!($f), $a.$f, $b.$f));
            }
            if better {
                $v.better = true;
            }
            put(&mut $b.$f, &$a.$f);
        }
    )*};
}

/// `worse` when `b` has more (the Tavern upgrade price).
macro_rules! judge_le {
    ($v:expr, $a:expr, $b:expr, $($f:ident),* $(,)?) => {$(
        {
            let (worse, better) = $a.$f.part(&$b.$f);
            if worse {
                $v.blocked.push(format!("{}: {:?} -> {:?}", stringify!($f), $a.$f, $b.$f));
            }
            if better {
                $v.better = true;
            }
            put(&mut $b.$f, &$a.$f);
        }
    )*};
}

/// Fields the comparison leaves out: they are copied over so the residual
/// equality below ignores them.
macro_rules! judge_ignore {
    ($a:expr, $b:expr, $($f:ident),* $(,)?) => {$(
        put(&mut $b.$f, &$a.$f);
    )*};
}

/// What the undead bonus and the deity carry: more is better for the stats.
impl Part for Deity {
    fn part(&self, other: &Self) -> (bool, bool) {
        if other.card != self.card || other.golden != self.golden {
            return (true, false);
        }
        (self.attack < other.attack || self.health < other.health,
         self.attack > other.attack || self.health > other.health)
    }
}

fn judge_player(a: &Player, b: &mut Player, v: &mut Dominance) {
    judge_ge!(
        v, a, b,
        health,
        armor,
        gold,
        gold_next_turn,
        extra_max_gold,
        free_rolls,
        overconfidence_win,
        overconfidence_tie,
        winners_bread_buff,
        boon_of_beetles,
        brood_of_nozdormu,
        upper_hand,
        sharing_is_caring,
        volumizer_bonus,
        beetle_bonus,
        improve,
        counts,
        hat,
        enhance_times,
        ticket_chance,
        tickets,
        beast_bonus,
        deathrattles_triggered,
        golden_played,
        gold_spent_turn,
        elementals_played_turn,
        spells_cast_turn,
        elementals_played,
        magnetizations,
        refresh_buff,
        spell_bonus,
        spell_discount,
        battlecries_triggered,
        undead_bonus,
        knight_deaths,
        gem_attack,
        gem_health,
        gem_hand_attack,
        gem_hand_health,
        barrage,
        free_minion_left,
        conviction,
        discarded,
        spells_cast,
    );
    judge_le!(v, a, b, upgrade_cost);
    // A hero power: the presses left and the presses this game can only go
    // up. Which power it is, its unlock, its progress, its price and whether
    // it is armed must stay as they were (a pressed power is a move of its
    // own, judged by the search like any other).
    for (x, y) in [(a.power.as_ref(), b.power.as_mut()), (a.power2.as_ref(), b.power2.as_mut())] {
        match (x, y) {
            (Some(x), Some(y)) => {
                if y.uses_left < x.uses_left {
                    v.blocked.push(format!("power uses_left: {} -> {}", x.uses_left, y.uses_left));
                }
                if y.used_game > x.used_game {
                    v.better = true;
                } else if y.used_game < x.used_game {
                    v.blocked.push(format!("power used_game: {} -> {}", x.used_game, y.used_game));
                }
                y.uses_left = x.uses_left;
                y.used_game = x.used_game;
            }
            (None, None) => {}
            _ => v.blocked.push("power".to_string()),
        }
    }
    // The Deity: the same card, its stats at least as good.
    match (a.deity.as_ref(), b.deity.as_mut()) {
        (Some(x), Some(y)) => {
            let (worse, better) = x.part(y);
            if worse {
                v.blocked.push(format!("deity: {} -> {}", x.health, y.health));
            }
            if better {
                v.better = true;
            }
            put(y, x);
        }
        (None, None) => {}
        _ => v.blocked.push("deity".to_string()),
    }
    // A held trinket's own counters only ever go up (its "every N" count, the
    // number its effect improves); which trinket it is and its uses stay as
    // they were.
    for (ta, tb) in a.trinkets.iter().zip(b.trinkets.iter_mut()) {
        if tb.of != ta.of {
            v.blocked.push("trinket slot".to_string());
            put(tb, ta);
            continue;
        }
        if tb.count < ta.count {
            v.blocked.push(format!("trinket count: {} -> {}", ta.count, tb.count));
        } else if tb.count > ta.count {
            v.better = true;
        }
        if tb.value < ta.value {
            v.blocked.push(format!("trinket value: {} -> {}", ta.value, tb.value));
        } else if tb.value > ta.value {
            v.better = true;
        }
        tb.count = ta.count;
        tb.value = ta.value;
        if *tb != *ta {
            v.blocked.push("trinket state".to_string());
        }
        put(tb, ta);
    }
    // The trinket block: a buy leaves its own marks on it (`bought_stats`,
    // `bought_card`, `bought_spell`, `shop_hit`), which say nothing about the
    // position; Battlecries triggered this game can only rise. Everything
    // else about a trinket must stay as it was.
    {
        let (ta, mut tb) = (a.tp, b.tp);
        let (worse, better) = tb.battlecries.part(&ta.battlecries);
        if worse {
            v.blocked.push(format!("trinket battlecries: {} -> {}", ta.battlecries, tb.battlecries));
        }
        if better {
            v.better = true;
        }
        tb.battlecries = ta.battlecries;
        tb.bought_stats = ta.bought_stats;
        tb.bought_card = ta.bought_card;
        tb.bought_spell = ta.bought_spell;
        tb.shop_hit = ta.shop_hit;
        if ta != tb {
            v.blocked.push("trinket powers".to_string());
        }
    }
    judge_zone(&a.board, &mut b.board, "board", v);
    judge_hand(&a.hand, &b.hand, v);
    put(&mut b.hand, &a.hand);
    judge_ignore!(
        a, b,
        shop,
        shop_spell,
        shop_spell_ticket,
        frozen,
        actions,
        recent_buys,
        tp,
    );
}

/// The hand: every card that was there must still be there (in any slot) with
/// stats at least as good, and any card the line gained is a gain, as long as
/// the hand had room for it (Era's free Dragon cycle gives Tavern spells to
/// hand: the engine's Blue Chromadrake's Battlecry, and a cycle that gains a
/// card at no cost is the "only better" the whole search is for). A card
/// that left the hand is worse unless it was played and is back on the board
/// the board comparison already passed.
fn judge_hand(a: &[Minion], b: &[Minion], v: &mut Dominance) {
    let mut used = [false; HAND];
    for (i, x) in a.iter().enumerate() {
        // The same slot first, then any slot holding the same card.
        let mut found = None;
        for (j, y) in b.iter().enumerate() {
            if used[j] || y.card != x.card {
                continue;
            }
            if j == i {
                found = Some(j);
                break;
            }
            if found.is_none() {
                found = Some(j);
            }
        }
        match found {
            Some(j) => {
                used[j] = true;
                let mut y = b[j];
                judge_minion(x, &mut y, "hand", i, v);
                if y.attack < x.attack || y.health < x.health {
                    v.blocked.push(format!(
                    "hand[{i}]: stats {}/{} -> {}/{}",
                    x.attack, x.health, y.attack, y.health
                ));
                }
                if y.attack > x.attack || y.health > x.health {
                    v.better = true;
                }
            }
            None => v.blocked.push(format!("hand[{i}]: the card is gone")),
        }
    }
    if used[..b.len()].iter().any(|u| !*u) {
        v.better = true;
    }
}

fn judge_zone(a: &[Minion], b: &mut [Minion], zone: &str, v: &mut Dominance) {
    if a.len() != b.len() {
        v.blocked.push(format!("{zone} length: {} -> {}", a.len(), b.len()));
        return;
    }
    for (i, (x, y)) in a.iter().zip(b.iter_mut()).enumerate() {
        judge_minion(x, y, zone, i, v);
    }
}

fn judge_minion(a: &Minion, b: &mut Minion, zone: &str, i: usize, v: &mut Dominance) {
    let mut worse = Vec::new();
    let mut better = false;
    if b.card != a.card {
        worse.push("card".to_string());
    }
    if b.golden && !a.golden {
        better = true;
    } else if a.golden && !b.golden {
        worse.push("golden".to_string());
    }
    if b.attack < a.attack || b.health < a.health {
        worse.push(format!("stats {}/{} -> {}/{}", a.attack, a.health, b.attack, b.health));
    }
    if b.attack > a.attack || b.health > a.health {
        better = true;
    }
    if b.keywords.0 & a.keywords.0 != a.keywords.0 {
        worse.push(format!("keywords {} -> {}", a.keywords.0, b.keywords.0));
    }
    if b.keywords.0 & !a.keywords.0 != 0 {
        better = true;
    }
    if b.improvements < a.improvements {
        worse.push(format!("improvements {} -> {}", a.improvements, b.improvements));
    }
    if b.improvements > a.improvements {
        better = true;
    }
    // Anything else about the minion has to be identical.
    // Copy the four that may differ so the rest can be compared whole.
    put(&mut b.attack, &a.attack);
    put(&mut b.health, &a.health);
    put(&mut b.keywords, &a.keywords);
    put(&mut b.improvements, &a.improvements);
    if b != a {
        worse.push("other state".to_string());
    }
    if !worse.is_empty() {
        v.blocked.push(format!("{zone}[{i}]: {}", worse.join(", ")));
    }
    if better {
        v.better = true;
    }
}

/// The first place two states differ, read off their `Debug` rendering (the
/// pieces are `field: value`, so the first piece that differs names the
/// field). Reporting only, and best effort: a nested value can hold ", ".
fn first_difference(a: &Player, b: &Player) -> String {
    let (sa, sb) = (format!("{a:?}"), format!("{b:?}"));
    for (x, y) in sa.split(", ").zip(sb.split(", ")) {
        if x != y {
            let name = x.split(':').next().unwrap_or("state");
            let clip = |s: &str| -> String { s.chars().take(60).collect::<String>() };
            return format!("state off the list at {name}: {} vs {}", clip(x), clip(y));
        }
    }
    "state off the list".to_string()
}

// ---- the pool a playout's Refresh draws from ----

/// Every pool card at the copies this game's lobby and rules give it, which
/// is what the game started with: what a playout's Refresh puts back, because
/// a real player only roughly knows which copies are gone (the brief, judge
/// 2).
pub fn full_pool(game: &Game) -> Vec<u8> {
    let cards = &game.data.cards;
    let mut pool: Vec<u8> = (0..cards.pool_cards)
        .map(|c| {
            let card = &cards.cards[c];
            if card.pool_slot as usize == c && card.lobby_ok(game.lobby) {
                game.data.rules.copies(card.tier)
            } else {
                0
            }
        })
        .collect();
    // Only the Dark Paradox version this game rolled is in its pool.
    for c in 0..cards.pool_cards {
        let card = &cards.cards[c];
        if card.paradox {
            pool[c] = if Some(c as crate::CardId) == game.paradox {
                game.data.rules.copies(card.tier)
            } else {
                0
            };
        }
    }
    pool
}

/// A Refresh of player `p`'s Tavern drawn from the full pool: the game's own
/// Refresh action, with every copy of every card back in first. The copy is
/// no longer a game the fuzzer's accounting holds (`Game::pool_untracked`),
/// which is why only search copies do this.
pub fn refresh_from_full_pool(game: &mut Game, p: usize) -> Result<(), IllegalAction> {
    if game.current_player() != Some(p) {
        return Err(IllegalAction);
    }
    game.pool = full_pool(game);
    game.pool_untracked = true;
    game.step(Action::Roll)
}

// ---- the coming fight, against the boards of Era's real games ----

/// The boards one decision's candidates fight, as indices into
/// `RealBoards::boards_at(turn)`: `n` of them, taken evenly so the list is
/// the same for every candidate of one decision.
///
/// The boards of the turn whose tier is the next opponent's tier and whose
/// top tribe is that opponent's leaderboard tribe come first; when fewer
/// than `MIN_MATCH` match, every board of the turn is in (a real player
/// guesses from the turn and the tribe, and falls back to the turn alone).
pub const MIN_MATCH: usize = 5;

pub fn fight_boards(
    data: &Data,
    boards: &RealBoards,
    turn: u32,
    opp_tier: Option<u8>,
    opp_tribe: Option<Tribe>,
    n: usize,
) -> Vec<usize> {
    let all = boards.boards_at(turn);
    if all.is_empty() || n == 0 {
        return Vec::new();
    }
    let matched: Vec<usize> = (0..all.len())
        .filter(|&i| {
            let b = &all[i];
            let tier_ok = opp_tier.is_none_or(|t| t == b.tier);
            let tribe_ok = opp_tribe.is_none_or(|t| {
                let line = crate::game::Leaderboard::of(&data.cards, &b.minions);
                !line.mixed && line.top == Some(t)
            });
            tier_ok && tribe_ok
        })
        .collect();
    let every: Vec<usize> = (0..all.len()).collect();
    let pool: &[usize] = if matched.len() >= MIN_MATCH { &matched } else { &every };
    // Evenly spread `n` indices, repeating the list when it is shorter.
    (0..n).map(|k| pool[k * pool.len() / n.max(1)]).collect()
}

/// How one fight against a real board went, from player `p`'s side: the
/// engine's own fight, with the same counters, hands and rules as
/// `fight_health_change`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FightOutcome {
    /// 1 when player `p`'s hero won, 0 for a tie, -1 when the real board won.
    pub outcome: i8,
    /// The Health player `p`'s hero loses: 0 when the player wins or ties.
    pub health_change: i32,
}

/// The Health player `p`'s hero loses in the coming fight against one real
/// board: 0 when the player wins or ties, else minus the damage the winner
/// deals, capped by the rules of the turn, exactly as `Game::combat` counts
/// it (the winner's tier plus its surviving minions' tiers). The fight is the
/// engine's own, with the player's hero power, trinkets, Deity and hand, and
/// the real board's counters and hand.
pub fn fight_health_change(
    data: &Data,
    game: &Game,
    p: usize,
    board: &RealBoard,
    rng: &mut Rng,
) -> i32 {
    fight_outcome(data, game, p, board, rng).health_change
}

/// One fight against a real board, with both numbers the judging of an
/// end-of-turn position reads: who won, and what the player's Health paid
/// (`fight_health_change`). A tie is a loss of 0.
pub fn fight_outcome(
    data: &Data,
    game: &Game,
    p: usize,
    board: &RealBoard,
    rng: &mut Rng,
) -> FightOutcome {
    let pl = &game.players[p];
    let them = Player::from_real_board(data, board);
    let counters = [pl.counters(data, game.lobby), them.counters(data, game.lobby)];
    let hands: [&[Minion]; 2] = [&pl.hand, &board.hand];
    let out = fight_with_hands(
        &data.cards,
        &pl.board,
        &board.minions,
        hands,
        counters,
        data.rules.max_attacks,
        rng,
        &mut (),
    );
    match out.winner {
        Some(0) => FightOutcome { outcome: 1, health_change: 0 },
        None => FightOutcome { outcome: 0, health_change: 0 },
        Some(_) => {
            let raw = board.tier as i32 + out.survivor_tiers;
            FightOutcome {
                outcome: -1,
                health_change: -data
                    .rules
                    .damage_cap(game.turn, game.alive())
                    .map_or(raw, |c| raw.min(c)),
            }
        }
    }
}

/// The keywords a minion has, for a one-line description (tests).
pub fn keyword_names(m: &Minion) -> Vec<&'static str> {
    Keywords::ALL
        .iter()
        .filter(|(k, _)| m.keywords.has(*k))
        .map(|(_, n)| *n)
        .collect()
}

/// The slots a zone can hold, re-exported so the bindings and tests agree.
pub const ZONES: (usize, usize) = (BOARD, HAND);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_larger_number_part_is_better_not_worse() {
        assert_eq!(7u8.part(&5), (false, true));
        assert_eq!(5u8.part(&7), (true, false));
        assert_eq!(5u8.part(&5), (false, false));
        assert_eq!((1, 5).part(&(2, 4)), (true, true));
    }
}
