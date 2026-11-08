//! The comp planner: a bot that plays a written comp from `data/comps`.
//!
//! The brief is `docs/briefs/comp-planner-1.md`; the plan is
//! `docs/notes/comp-planner.md`. The bot may name cards, as the scripted bots
//! do: the game rules still match effects, never names.
//!
//! One shop decision:
//! 1. Answer an open choice, preferring a comp card.
//! 2. Apply the standing rules (the Tortollan Blue Shell after a loss).
//! 3. Level when the comp's curve or the default curve says so, unless dying.
//! 4. Take a cycle that dominates, if there is one.
//! 5. Score legal moves by their fights against real boards of the turn plus
//!    `w(turn) * comp progress`, and play the best.
//!
//! The scoring may allocate (it is a bot): each candidate is a `Game` clone
//! stepped once. The fight sim itself does not allocate.

use std::sync::Arc;

use crate::comps::{Comp, Comps, Scales, SignalKind};
use crate::game::{Game, Opponent};
use crate::minion::{CardId, Keywords, Minion};
use crate::real_boards::RealBoards;
use crate::rng::Rng;
use crate::search::{Position, fight_boards, fight_outcome};
use crate::{Action, HAND, N_ACTIONS};

/// Fights per shop decision: the same 16 (or fewer, when a turn has fewer
/// boards) against the real boards of the turn.
pub const FIGHTS: usize = 16;
/// Candidates whose fights are played out. The rest are ranked by the cheap
/// score and dropped.
pub const CANDIDATES: usize = 20;
/// Turns the standard curve bot plays before the planner takes over. It gives
/// the early board a known-good tempo opening; the comp rules start on turn 4.
pub const STANDARD_OPENING_TURNS: u32 = 3;
/// How much stronger a card must be to replace a board minion.
pub const REPLACE_MARGIN: f32 = 6.0;
/// The signal score a comp needs before the planner commits inside its
/// commit turns. An early card alone is below it.
pub const COMMIT_MIN: f32 = 4.0;
/// Progress for a body of the comp's tribe that is not a named core card.
pub const TRIBE_PROGRESS: f32 = 0.8;
/// Spells Era holds rather than casting at once (`docs/notes/mining/sure-plays.md`,
/// "Spells that are always cast on buy"). Everything else is cast from hand
/// before the planner scores a move.
pub const HOLD_SPELLS: [&str; 5] = [
    "Them Apples",
    "Sludge Corrosion",
    "Methodical Madness",
    "Hasty Excavation",
    "Corrupted Coin",
];

/// The weight between "strong now" and comp progress, by turn. Low early
/// (survive), high late (build).
pub fn turn_weight(turn: u32) -> f32 {
    match turn {
        0..=3 => 0.10,
        4..=6 => 0.25,
        7..=9 => 0.50,
        10..=12 => 1.00,
        _ => 2.00,
    }
}

/// What the planner has chosen and done this turn or game.
#[derive(Clone, Debug, Default)]
pub struct PlannerState {
    /// The comp the planner committed to, if any.
    pub comp: Option<usize>,
    /// The comp the planner is leaning to before committing; used for progress
    /// and for final-board preference.
    pub lean: Option<usize>,
    /// The turn the current comp was committed to.
    pub committed_turn: Option<u32>,
    /// Turns on which the Shell standing rule applied (unique turns).
    pub shell_chance_turns: Vec<u32>,
    /// Turns on which the planner took a Shell cycle (unique turns).
    pub shell_cycle_turns: Vec<u32>,
    /// Turns on which the planner pressed the tier button and the target.
    pub level_turns: Vec<(u32, u8)>,
    /// The turn a give-up rule switched the comp.
    pub switched_turn: Option<u32>,
    /// Decisions made this game.
    pub decisions: u64,
}

/// The planner bot. One per seat (or per Era turn), deterministic for a seed.
pub struct Planner {
    pub comps: Comps,
    pub state: PlannerState,
    rng: Rng,
    boards: Option<Arc<RealBoards>>,
    fights: usize,
    shell: Option<CardId>,
    sure_buys: Vec<CardId>,
}

impl Planner {
    pub fn new(comps: Comps, seed: u64, data: &crate::Data) -> Planner {
        Planner {
            comps,
            state: PlannerState::default(),
            rng: Rng::new(seed),
            boards: None,
            fights: FIGHTS,
            shell: data.cards.by_name("Tortollan Blue Shell"),
            sure_buys: ["Leaf Through the Pages", "Wealthy Bounty", "Fandral's Fortune"]
                .iter()
                .filter_map(|n| data.cards.by_name(n))
                .collect(),
        }
    }

    pub fn with_boards(mut self, boards: Arc<RealBoards>) -> Planner {
        self.boards = Some(boards);
        self
    }

    pub fn set_boards(&mut self, boards: Option<Arc<RealBoards>>) {
        self.boards = boards;
    }

    /// Start a new game or turn with a fresh state and RNG.
    pub fn reset(&mut self, seed: u64) {
        self.state = PlannerState::default();
        self.rng = Rng::new(seed);
    }

    pub fn comp_name(&self) -> Option<&str> {
        self.state.comp.map(|i| self.comps.comps[i].name.as_str())
    }

    pub fn lean_name(&self) -> Option<&str> {
        self.state.lean.map(|i| self.comps.comps[i].name.as_str())
    }

    /// The share of the comp's total core weight that is on the player's
    /// board, and how many core cards are there. The `planner` command calls
    /// a final board the committed comp when the share is at least 0.5 and
    /// there are at least 2 core cards.
    pub fn core_share(&self, game: &Game, p: usize) -> (f32, usize) {
        let Some(ci) = self.state.comp else { return (0.0, 0) };
        let comp = &self.comps.comps[ci];
        let total: f32 = comp.core.iter().map(|c| c.weight).sum();
        let mut on_board = 0.0;
        let mut count = 0;
        for c in &comp.core {
            if game.players[p].board.iter().any(|m| m.card == c.id) {
                on_board += c.weight;
                count += 1;
            }
        }
        (if total > 0.0 { on_board / total } else { 0.0 }, count)
    }

    /// The next action for player `p` (the current player).
    pub fn action(&mut self, game: &Game, p: usize) -> Action {
        let a = self.decide(game, p);
        if game.is_legal(a) {
            return a;
        }
        // The scripted bots have the same guard: a planner bug must not hand
        // the engine an action it refuses. Print the state once so it can be
        // found, and play a legal action.
        let pl = &game.players[p];
        let names = |ms: &[Minion]| -> Vec<String> {
            ms.iter().map(|m| game.data.cards.get(m.card).name.clone()).collect()
        };
        eprintln!(
            "the planner picked an illegal {a:?}: turn {} player {p} comp {:?} lean {:?} gold {} tier {} health {}\n  \
             board {:?}\n  hand {:?}\n  shop {:?} spell {:?}\n  \
             discover {:?} choose_one {:?} target {:?} shop_target {:?} discard {:?}\n  \
             power_offer {:?} secret_offer {:?} trinket_offer {} hero_offer {:?}",
            game.turn,
            self.comp_name(),
            self.lean_name(),
            pl.gold,
            pl.tier,
            pl.health,
            names(&pl.board),
            names(&pl.hand),
            names(&pl.shop),
            pl.shop_spell.map(|c| game.data.cards.get(c).name.clone()),
            pl.discover.iter().map(|&c| game.data.cards.get(c).name.clone()).collect::<Vec<_>>(),
            pl.pending_choose_one,
            pl.pending_target,
            pl.pending_shop_target,
            pl.pending_discard,
            pl.power_offer,
            pl.secret_offer,
            pl.trinket_offer.len(),
            pl.hero_offer,
        );
        self.fallback_legal(game)
    }

    fn decide(&mut self, game: &Game, p: usize) -> Action {
        self.state.decisions += 1;
        self.update_comp(game, p);
        if let Some(a) = self.pending_action(game, p) {
            return a;
        }
        if let Some(a) = self.shell_action(game, p) {
            return a;
        }
        if game.turn <= STANDARD_OPENING_TURNS {
            return crate::bots::standard_action(game);
        }
        if let Some(a) = self.level_action(game, p) {
            return a;
        }
        if let Some(a) = self.sure_buy_action(game, p) {
            return a;
        }
        if let Some(a) = self.cast_spell_action(game, p) {
            return a;
        }
        if let Some(a) = self.make_room_action(game, p) {
            return a;
        }
        if let Some(a) = self.dominating_cycle(game, p) {
            return a;
        }
        self.score_action(game, p)
    }

    // ---- the comp -------------------------------------------------------

    fn update_comp(&mut self, game: &Game, p: usize) {
        if let Some(ci) = self.state.comp {
            let give = self.comps.comps[ci].give_up.clone();
            if let Some(g) = give {
                let progress = self.progress_for(game, p, Some(ci));
                let below = g.progress_below.is_none_or(|x| progress < x);
                if game.turn >= g.after_turn && below && g.to_index != ci {
                    self.state.comp = Some(g.to_index);
                    self.state.lean = Some(g.to_index);
                    self.state.committed_turn = Some(game.turn);
                    self.state.switched_turn = Some(game.turn);
                }
            }
            return;
        }
        let mut best: Option<usize> = None;
        let mut best_score = 0.0f32;
        for (i, comp) in self.comps.comps.iter().enumerate() {
            let s = self.signal_score(game, p, comp);
            if s > best_score {
                best_score = s;
                best = Some(i);
            }
        }
        if let Some(b) = best {
            let in_range =
                self.comps.comps[b].commit_turn.is_some_and(|(lo, hi)| game.turn >= lo && game.turn <= hi);
            if in_range && best_score >= COMMIT_MIN {
                self.state.comp = Some(b);
                self.state.lean = Some(b);
                self.state.committed_turn = Some(game.turn);
                return;
            }
            self.state.lean = Some(b);
            return;
        }
        // No signal at all: late in the game, scam is the honest fallback.
        if game.turn >= 10
            && let Some(f) = self.comps.fallback()
        {
            self.state.comp = Some(f);
            self.state.lean = Some(f);
            self.state.committed_turn = Some(game.turn);
            return;
        }
        self.state.lean = None;
    }

    fn signal_score(&self, game: &Game, p: usize, comp: &Comp) -> f32 {
        let pl = &game.players[p];
        let mut score = 0.0;
        for s in &comp.signals {
            match &s.kind {
                SignalKind::Cards(ids) => {
                    for id in ids {
                        if pl.board.iter().chain(&pl.hand).chain(&pl.shop).any(|m| m.card == *id) {
                            score += s.weight;
                        }
                    }
                }
                SignalKind::Tribe(t) => {
                    let n = pl.board
                        .iter()
                        .chain(&pl.hand)
                        .filter(|m| game.data.cards.tribes_of(m).has(*t))
                        .count() as u32;
                    if n >= s.min_count {
                        score += s.weight;
                    }
                }
                SignalKind::Hero(h) => {
                    if pl.hero == Some(*h) {
                        score += s.weight;
                    }
                }
                SignalKind::Trinket(t) => {
                    if pl.trinkets.iter().any(|held| held.of == t + 1) {
                        score += s.weight;
                    }
                }
                SignalKind::Keyword(k) => {
                    let n = pl.board
                        .iter()
                        .chain(&pl.hand)
                        .filter(|m| m.keywords.has(*k))
                        .count()
                        .min(5) as f32;
                    score += s.weight * n;
                }
            }
        }
        // Early cards are weak signals: seeing a Buzzing Vermin says Beast,
        // it does not say the comp is on.
        for id in &comp.early.cards {
            if pl.board.iter().chain(&pl.hand).chain(&pl.shop).any(|m| m.card == *id) {
                score += 0.5;
            }
        }
        score
    }

    // ---- choices, standing rules and leveling ---------------------------

    fn pending_action(&mut self, game: &Game, p: usize) -> Option<Action> {
        let pl = &game.players[p];
        let ci = self.state.comp.or(self.state.lean);
        let legal = |a: Action| game.is_legal(a);
        if game.turn == 0
            && pl.hero_offer.iter().any(|&h| h != u8::MAX)
            && let Some(ci) = ci
        {
            let comp = &self.comps.comps[ci];
            for (k, &h) in pl.hero_offer.iter().enumerate() {
                if h == u8::MAX {
                    continue;
                }
                let wanted = comp.signals.iter().any(|s| matches!(s.kind, SignalKind::Hero(x) if x == h));
                if wanted && legal(Action::Choose(k as u8)) {
                    return Some(Action::Choose(k as u8));
                }
            }
        }
        if !pl.trinket_offer.is_empty()
            && let Some(ci) = ci
        {
            let comp = &self.comps.comps[ci];
            for (k, &(of, _)) in pl.trinket_offer.iter().enumerate() {
                let wanted = comp
                    .signals
                    .iter()
                    .any(|s| matches!(s.kind, SignalKind::Trinket(x) if x == of));
                if wanted && legal(Action::ChooseTrinket(k as u8)) {
                    return Some(Action::ChooseTrinket(k as u8));
                }
            }
        }
        if !pl.discover.is_empty()
            && let Some(ci) = ci
        {
            let comp = &self.comps.comps[ci];
            let mut best: Option<(u8, f32)> = None;
            for k in 0..pl.discover.len() {
                let id = pl.discover[k];
                let mut w = comp.core_weight(id).unwrap_or(0.0);
                if comp.early_has(id) {
                    w += 1.0;
                }
                for s in &comp.signals {
                    if let SignalKind::Cards(ids) = &s.kind
                        && ids.contains(&id)
                    {
                        w += s.weight;
                    }
                }
                if w > 0.0
                    && legal(Action::Choose(k as u8))
                    && best.is_none_or(|(_, b)| w > b)
                {
                    best = Some((k as u8, w));
                }
            }
            if let Some((k, _)) = best {
                return Some(Action::Choose(k));
            }
        }
        let pref = ci.and_then(|c| self.comps.comps[c].tribes.first().copied());
        crate::bots::answer_pending(game, p, pref)
    }

    fn shell_action(&mut self, game: &Game, p: usize) -> Option<Action> {
        let shell = self.shell?;
        let pl = &game.players[p];
        if !pl.lost_last_combat {
            return None;
        }
        let slot = pl.shop.iter().position(|m| m.card == shell)?;
        let cost = game.data.cards.get(shell).cost;
        if pl.gold < cost || pl.hand.is_full() {
            return None;
        }
        if game.is_legal(Action::CycleShop(slot as u8)) {
            self.note_shell_chance(game.turn);
            self.note_shell_cycle(game.turn);
            return Some(Action::CycleShop(slot as u8));
        }
        // A full board needs a slot first. Sell the card with the lowest comp
        // weight (not the weakest), and only if it is not part of the comp.
        if pl.board.is_full() {
            let slot = self.sell_for_room(game, p)?;
            if game.is_legal(Action::Sell(slot as u8)) {
                self.note_shell_chance(game.turn);
                return Some(Action::Sell(slot as u8));
            }
        }
        None
    }

    /// The board slot with the lowest comp weight, when that weight is not
    /// positive. Core cards are never sold just to make room.
    fn sell_for_room(&self, game: &Game, p: usize) -> Option<usize> {
        let pl = &game.players[p];
        let mut best: Option<(usize, f32, i32)> = None;
        for (i, m) in pl.board.iter().enumerate() {
            let w = self.minion_weight(m);
            let stats = m.attack + m.health;
            let better = match best {
                None => true,
                Some((_, bw, bs)) => w < bw - 1e-6 || ((w - bw).abs() <= 1e-6 && stats < bs),
            };
            if better {
                best = Some((i, w, stats));
            }
        }
        match best {
            Some((i, w, _)) if w <= 0.0 => Some(i),
            _ => None,
        }
    }

    fn note_shell_chance(&mut self, turn: u32) {
        if self.state.shell_chance_turns.last() != Some(&turn) {
            self.state.shell_chance_turns.push(turn);
        }
    }

    fn note_shell_cycle(&mut self, turn: u32) {
        if self.state.shell_cycle_turns.last() != Some(&turn) {
            self.state.shell_cycle_turns.push(turn);
        }
    }

    fn level_action(&mut self, game: &Game, p: usize) -> Option<Action> {
        let pl = &game.players[p];
        if pl.health <= self.comps.defaults.dying_health {
            return None;
        }
        let default = &self.comps.defaults.level;
        let target = match self.state.comp.or(self.state.lean) {
            Some(ci) => self.comps.comps[ci].level_at(game.turn, default),
            None => {
                let index = (game.turn.saturating_sub(1) as usize).min(default.len().saturating_sub(1));
                default.get(index).copied()
            }
        }?;
        if pl.tier >= target || !game.is_legal(Action::Level) {
            return None;
        }
        if self.state.level_turns.last() != Some(&(game.turn, target)) {
            self.state.level_turns.push((game.turn, target));
        }
        Some(Action::Level)
    }

    /// Era's always-bought economy spells (docs/notes/mining/sure-plays.md):
    /// Leaf Through the Pages, Wealthy Bounty and Fandral's Fortune. Each is
    /// bought when it is offered and affordable, in that order. The cast rule
    /// then casts it the same turn.
    fn sure_buy_action(&self, game: &Game, p: usize) -> Option<Action> {
        let pl = &game.players[p];
        for &id in &self.sure_buys {
            if pl.shop_spell == Some(id) && game.is_legal(Action::BuySpell) {
                return Some(Action::BuySpell);
            }
            if let Some(slot) = pl.shop.iter().position(|m| m.card == id)
                && pl.gold >= game.data.cards.get(id).cost
            {
                let a = Action::Buy(slot as u8);
                if game.is_legal(a) {
                    return Some(a);
                }
            }
        }
        None
    }

    /// Cast a spell already in hand unless it is one Era holds. This is the
    /// standing "cast what you bought" rule: targeted and generated spells do
    /// not change the board in their own `Play` state, so the one-step score
    /// alone would leave them in hand forever.
    fn cast_spell_action(&self, game: &Game, p: usize) -> Option<Action> {
        let pl = &game.players[p];
        let mut best: Option<(u8, u8)> = None;
        for h in 0..pl.hand.len() {
            let c = game.data.cards.get(pl.hand[h].card);
            if !c.is_spell() || HOLD_SPELLS.contains(&c.name.as_str()) {
                continue;
            }
            if c.pay_health && pl.health - c.cost as i32 <= 5 {
                continue;
            }
            let a = Action::Play { hand: h as u8, pos: 0 };
            if !game.is_legal(a) {
                continue;
            }
            // Cast the cheapest first: a 0-cost generated spell clears hand
            // space for the rest, and a spell that opens a choice pauses here.
            let key = c.cost;
            if best.is_none_or(|(_, b)| key < b) {
                best = Some((h as u8, key));
            }
        }
        best.map(|(hand, _)| Action::Play { hand, pos: 0 })
    }

    /// With a full board, sell a card that is not in the comp to make room for
    /// a core card in hand or in the Tavern. This is the same Balinda rule as
    /// the Shell cycle: the card not in the comp goes, never a core one.
    fn make_room_action(&self, game: &Game, p: usize) -> Option<Action> {
        let pl = &game.players[p];
        if !pl.board.is_full() {
            return None;
        }
        let slot = self.sell_for_room(game, p)?;
        let comp = self.state.comp.or(self.state.lean);
        // A core card in hand or in the Tavern always gets a slot.
        if let Some(ci) = comp {
            let comp = &self.comps.comps[ci];
            let hand_core = pl.hand.iter().any(|m| {
                !game.data.cards.get(m.card).is_spell() && comp.core_weight(m.card).is_some()
            });
            let shop_core = pl.shop.iter().any(|m| {
                comp.core_weight(m.card).is_some()
                    && pl.gold >= game.data.cards.get(m.card).cost
                    && !pl.hand.is_full()
            });
            if hand_core || shop_core {
                return game.is_legal(Action::Sell(slot as u8)).then_some(Action::Sell(slot as u8));
            }
        }
        let worst = self.minion_value(game, &pl.board[slot]);
        // A much stronger body is how the board improves at all.
        let hand_better = pl.hand.iter().any(|m| {
            !game.data.cards.get(m.card).is_spell()
                && self.minion_value(game, m) > worst + REPLACE_MARGIN
        });
        let shop_better = pl.shop.iter().any(|m| {
            let c = game.data.cards.get(m.card);
            !c.is_spell()
                && pl.gold >= c.cost
                && !pl.hand.is_full()
                && self.minion_value(game, m) > worst + REPLACE_MARGIN
        });
        if !hand_better && !shop_better {
            return None;
        }
        game.is_legal(Action::Sell(slot as u8)).then_some(Action::Sell(slot as u8))
    }

    /// How much a minion is worth on a board or as a card to buy: stats,
    /// keywords, Tier, and the comp weight (a core card is worth more than
    /// its stats).
    fn minion_value(&self, game: &Game, m: &Minion) -> f32 {
        let c = game.data.cards.get(m.card);
        if c.is_spell() {
            return 0.0;
        }
        let mut v = (m.attack + m.health) as f32 + 1.5 * c.tier as f32;
        if m.keywords.has(Keywords::DIVINE_SHIELD) {
            v += 4.0;
        }
        if m.keywords.has(Keywords::VENOMOUS) || m.keywords.has(Keywords::POISONOUS) {
            v += 6.0;
        }
        if m.keywords.has(Keywords::REBORN) {
            v += 3.0;
        }
        if let Some(ci) = self.state.comp.or(self.state.lean) {
            let comp = &self.comps.comps[ci];
            if let Some(w) = comp.core_weight(m.card) {
                v += 10.0 * w;
            }
            if comp
                .tribes
                .iter()
                .any(|t| game.data.cards.tribes_of(m).has(*t))
            {
                v += 6.0;
            }
            for signal in &comp.signals {
                match &signal.kind {
                    SignalKind::Cards(ids) if ids.contains(&m.card) => v += 3.0 * signal.weight,
                    SignalKind::Keyword(k) if m.keywords.has(*k) => v += 3.0 * signal.weight,
                    _ => {}
                }
            }
        }
        v
    }

    /// A cycle that leaves the position no worse and something better (the
    /// search's dominance judge). The Shell rule already caught its own.
    fn dominating_cycle(&self, game: &Game, p: usize) -> Option<Action> {
        let before = Box::new(Position::of(game, p));
        let mut mask = [false; N_ACTIONS];
        game.legal_mask(&mut mask);
        for i in 0..N_ACTIONS {
            if !mask[i] {
                continue;
            }
            let Some(a) = Action::from_index(i) else { continue };
            if !matches!(a, Action::CycleShop(_) | Action::CycleHand(_)) {
                continue;
            }
            let mut copy = Box::new(game.clone());
            if copy.step(a).is_err() {
                continue;
            }
            if before.judge(&copy).dominates {
                return Some(a);
            }
        }
        None
    }

    // ---- scoring --------------------------------------------------------

    fn score_action(&mut self, game: &Game, p: usize) -> Action {
        let seed = self.rng.next_u64();
        let turn = game.turn;
        let comp = self.state.comp.or(self.state.lean);
        let base_progress = self.progress_for(game, p, comp);
        let base_strong = self.strong(game, p, seed);
        let end_score = base_strong + turn_weight(turn) * base_progress * 10.0;

        let pl = &game.players[p];
        let mut mask = [false; N_ACTIONS];
        game.legal_mask(&mut mask);
        // One play per hand card: attach a Magnetic, else append.
        let mut play: [Option<(u8, u8, i32)>; HAND] = [None; HAND];
        for i in 0..N_ACTIONS {
            if !mask[i] {
                continue;
            }
            let Some(Action::Play { hand, pos }) = Action::from_index(i) else { continue };
            let h = hand as usize;
            let score = play_pos_score(game, pl, &pl.hand[h], pos);
            if play[h].is_none_or(|(_, _, s)| score > s) {
                play[h] = Some((hand, pos, score));
            }
        }
        let mut actions: Vec<Action> = Vec::new();
        for i in 0..N_ACTIONS {
            if !mask[i] {
                continue;
            }
            let Some(a) = Action::from_index(i) else { continue };
            match a {
                Action::Buy(s) => {
                    let m = &pl.shop[s as usize];
                    let c = game.data.cards.get(m.card);
                    // A full board can only take a card it can play: a core
                    // card, a Magnetic merge, or the third copy of a triple.
                    // The make-room rule already sold a slot for a stronger
                    // card before this point.
                    let core = comp.is_some_and(|ci| {
                        self.comps.comps[ci].core_weight(m.card).is_some()
                    });
                    let held = pl
                        .board
                        .iter()
                        .chain(&pl.hand)
                        .filter(|x| x.card == m.card && !x.golden)
                        .count();
                    let magnetic = m.keywords.has(Keywords::MAGNETIC)
                        && pl.board.iter().any(|x| game.data.cards.tribes_of(x).has(crate::Tribe::Mech));
                    if pl.board.is_full() && !c.is_spell() && !core && held != 2 && !magnetic {
                        continue;
                    }
                    actions.push(a);
                }
                Action::BuySpell
                | Action::Sell(_)
                | Action::Activate(_)
                | Action::Level
                | Action::Roll
                | Action::HeroPower
                | Action::HeroPower2
                | Action::DarkGift => actions.push(a),
                _ => {}
            }
        }
        for p in play.iter().flatten() {
            actions.push(Action::Play { hand: p.0, pos: p.1 });
        }

        let mut cands: Vec<Cand> = Vec::with_capacity(actions.len());
        for a in actions {
            let mut after = Box::new(game.clone());
            if after.step(a).is_err() {
                continue;
            }
            let progress = self.progress_for(&after, p, comp);
            let stat = self.board_stat(&after, p);
            let mut extra = 0.0;
            match a {
                Action::Roll => extra += self.roll_value(game, p),
                Action::DarkGift => {
                    // Do not spend the last buy's gold on the button.
                    let r = &game.data.rules;
                    if pl.gold >= r.dark_gift_cost.saturating_add(r.buy_cost) {
                        extra += if (3..=5).contains(&turn) { 4.0 } else { 1.0 };
                    }
                }
                Action::HeroPower | Action::HeroPower2 => extra += 2.0,
                Action::Activate(_) => extra += 1.0,
                Action::Buy(s) => {
                    if let Some(m) = pl.shop.get(s as usize) {
                        let c = game.data.cards.get(m.card);
                        if !c.is_spell() {
                            extra += 0.1 * (m.attack + m.health) as f32 + 0.5 * c.tier as f32;
                            let held = pl
                                .board
                                .iter()
                                .chain(&pl.hand)
                                .filter(|x| x.card == m.card && !x.golden)
                                .count();
                            if held == 2 {
                                extra += 8.0;
                            }
                            if m.keywords.has(Keywords::MAGNETIC)
                                && pl.board.iter().any(|x| game.data.cards.tribes_of(x).has(crate::Tribe::Mech))
                            {
                                extra += 2.0;
                            }
                            let core = comp.is_some_and(|ci| {
                                self.comps.comps[ci].core_weight(m.card).is_some()
                            });
                            // A full board can only take a card it will sell
                            // for. Save the Gold for a core card, a level or
                            // a Refresh instead of filling the hand with junk.
                            if pl.board.is_full() && !core {
                                extra -= 2.0;
                            }
                        }
                    }
                    if self.is_stat_spell(game, pl, pl.shop.get(s as usize).map(|m| m.card)) {
                        extra += 2.0;
                    }
                }
                Action::BuySpell => {
                    if self.is_stat_spell(game, pl, pl.shop_spell) {
                        extra += 2.0;
                    }
                }
                _ => {}
            }
            let quick = progress * 10.0 + stat * 0.1 + extra;
            let same_board = after.players[p].board == game.players[p].board;
            cands.push(Cand { action: a, after, quick, progress, extra, same_board });
        }
        cands.sort_by(|a, b| b.quick.partial_cmp(&a.quick).unwrap_or(std::cmp::Ordering::Equal));
        cands.truncate(CANDIDATES);

        let mut best = (Action::EndTurn, end_score);
        for c in &cands {
            let strong = if c.same_board { base_strong } else { self.strong(&c.after, p, seed) };
            let score = strong + turn_weight(turn) * c.progress * 10.0 + c.extra;
            if score > best.1 + 1e-6 {
                best = (c.action, score);
            }
        }
        if game.is_legal(best.0) {
            best.0
        } else {
            self.fallback_legal(game)
        }
    }

    fn fallback_legal(&self, game: &Game) -> Action {
        if game.is_legal(Action::EndTurn) {
            return Action::EndTurn;
        }
        let mut mask = [false; N_ACTIONS];
        game.legal_mask(&mut mask);
        mask.iter()
            .position(|&m| m)
            .and_then(Action::from_index)
            .unwrap_or(Action::EndTurn)
    }

    fn is_stat_spell(&self, game: &Game, pl: &crate::game::Player, card: Option<CardId>) -> bool {
        card.is_some_and(|c| {
            let card = game.data.cards.get(c);
            card.is_spell() && card.gives_stats() && !pl.board.is_empty()
        })
    }

    /// A Refresh is worth taking when the Gold would otherwise be left over
    /// and no move scored better. The brief's "spend the gold": if the Tavern
    /// has nothing worth buying, look again.
    fn roll_value(&self, game: &Game, p: usize) -> f32 {
        let pl = &game.players[p];
        if pl.free_rolls > 0 || pl.gold >= game.data.rules.roll_cost { 1.0 } else { 0.0 }
    }

    fn board_stat(&self, game: &Game, p: usize) -> f32 {
        let mut s = 0.0;
        for m in &game.players[p].board {
            s += (m.attack + m.health) as f32;
            if m.keywords.has(Keywords::DIVINE_SHIELD) {
                s += 4.0;
            }
            if m.keywords.has(Keywords::VENOMOUS) || m.keywords.has(Keywords::POISONOUS) {
                s += 6.0;
            }
            if m.keywords.has(Keywords::REBORN) {
                s += 3.0;
            }
            if m.keywords.has(Keywords::TAUNT) {
                s += 1.0;
            }
            if m.golden {
                s += 2.0;
            }
        }
        s
    }

    /// Fights the board against the real boards of the turn, preferring the
    /// next opponent's tier and leaderboard tribe (search::fight_boards).
    /// Without real boards, a stat reading stands in.
    fn strong(&self, game: &Game, p: usize, seed: u64) -> f32 {
        if let Some(boards) = &self.boards {
            let (tier, tribe) = match game.players[p].opponent {
                Some(Opponent::Player(q)) => {
                    let q = &game.players[q as usize];
                    (Some(q.tier), if q.leaderboard.mixed { None } else { q.leaderboard.top })
                }
                _ => (None, None),
            };
            let indices = fight_boards(&game.data, boards, game.turn, tier, tribe, self.fights);
            if !indices.is_empty() {
                let all = boards.boards_at(game.turn);
                let mut rng = Rng::new(seed);
                let mut sum = 0.0;
                for &i in &indices {
                    let out = fight_outcome(&game.data, game, p, &all[i], &mut rng);
                    sum += out.outcome as f32 * 100.0 + out.health_change as f32 * 2.0;
                }
                return sum / indices.len() as f32;
            }
        }
        self.board_stat(game, p)
    }

    // ---- comp progress --------------------------------------------------

    fn progress_for(&self, game: &Game, p: usize, comp: Option<usize>) -> f32 {
        let Some(ci) = comp else { return self.tempo_progress(game, p) };
        let comp = &self.comps.comps[ci];
        let pl = &game.players[p];
        let mut s = 0.0;
        for m in &pl.board {
            s += self.minion_progress(game, comp, m, 1.0);
        }
        for m in &pl.hand {
            s += self.minion_progress(game, comp, m, 0.5);
        }
        for c in &comp.core {
            let n = pl.board
                .iter()
                .chain(&pl.hand)
                .filter(|m| m.card == c.id && !m.golden)
                .count();
            if n >= 2 {
                s += c.weight * 1.5;
            }
        }
        match comp.scales {
            Scales::Beetles => {
                s += 0.1 * (pl.beetle_bonus.0 + pl.beetle_bonus.1).max(0) as f32;
            }
            Scales::Stats => {
                for m in pl.board.iter().chain(&pl.hand) {
                    s += 0.05 * (m.improvements + m.magnetizations) as f32;
                }
            }
            Scales::None => {}
        }
        s
    }

    fn minion_progress(&self, game: &Game, comp: &Comp, m: &Minion, mult: f32) -> f32 {
        if game.data.cards.get(m.card).is_spell() {
            return 0.0;
        }
        let mut s = 0.0;
        if let Some(w) = comp.core_weight(m.card) {
            s += w;
            if comp.core.iter().any(|c| c.id == m.card && c.scale) {
                s += w * 0.25 * (m.improvements + m.magnetizations) as f32;
                if m.golden {
                    s += w * 0.5;
                }
            }
        }
        if comp.is_not_in_comp(m.card) {
            s -= 2.0;
        } else if comp.early_has(m.card) {
            s += 0.8;
        } else if game.turn <= 8 && comp.early.strong_stats && strong_body(game, m) {
            s += 0.3;
        }
        if comp
            .tribes
            .iter()
            .any(|t| game.data.cards.tribes_of(m).has(*t))
        {
            s += TRIBE_PROGRESS;
        }
        // A named signal card and a keyword signal are worth buying even when
        // the card is not on the end board: Headhunter Gryphon, a Venomous
        // body for scam.
        for signal in &comp.signals {
            match &signal.kind {
                SignalKind::Cards(ids) if ids.contains(&m.card) => s += 0.5 * signal.weight,
                SignalKind::Keyword(k) if m.keywords.has(*k) => s += signal.weight,
                _ => {}
            }
        }
        s * mult
    }

    fn tempo_progress(&self, game: &Game, p: usize) -> f32 {
        let pl = &game.players[p];
        let mut s = 0.0;
        for m in &pl.board {
            if strong_body(game, m) {
                s += 0.4;
            }
        }
        for m in &pl.hand {
            if strong_body(game, m) {
                s += 0.2;
            }
        }
        s
    }

    fn minion_weight(&self, m: &Minion) -> f32 {
        let Some(ci) = self.state.comp.or(self.state.lean) else { return 0.0 };
        let comp = &self.comps.comps[ci];
        if let Some(w) = comp.core_weight(m.card) {
            w
        } else if comp.is_not_in_comp(m.card) {
            -2.0
        } else {
            0.0
        }
    }
}

struct Cand {
    action: Action,
    after: Box<Game>,
    quick: f32,
    progress: f32,
    extra: f32,
    same_board: bool,
}

/// A minion with stats at least twice its Tier (not merely printed stats: an
/// early body).
fn strong_body(game: &Game, m: &Minion) -> bool {
    let c = game.data.cards.get(m.card);
    !c.is_spell() && m.attack + m.health > 2 * c.tier as i32
}

/// How good a play position is for one hand card: a Magnetic merge goes onto
/// the biggest Mech; everything else appends.
fn play_pos_score(game: &Game, pl: &crate::game::Player, m: &Minion, pos: u8) -> i32 {
    if m.keywords.has(Keywords::MAGNETIC) {
        let pos = pos as usize;
        if pos < pl.board.len() {
            let host = &pl.board[pos];
            if game.data.cards.tribes_of(host).has(crate::Tribe::Mech) {
                return 100 + host.attack + host.health;
            }
        }
        return 0;
    }
    if pos as usize == pl.board.len() { 10 } else { 0 }
}
