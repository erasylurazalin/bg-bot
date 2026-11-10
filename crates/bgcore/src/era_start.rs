//! Era starts (`docs/briefs/era-starts.md`): a training game can begin on a
//! turn Era really played, with Era's own board, hand, Gold, tier, Health,
//! hero, trinkets and counters at the start of that turn. The position comes
//! from `boards/actions/`, seated exactly as the shop replay seats the turn's
//! first step (`shop_replay::turn_start_game`). This is a training mode:
//! normal games never read anything here.
//!
//! Positions are split by source game into train and holdout, so an eval can
//! start from games training never saw. The split is a stable hash of the
//! file name and game number: adding a file does not move an old one.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::game::Player;
use crate::real_boards::CardIdMap;
use crate::shop_replay::{self, ShopFileSpec};
use crate::{Data, Game, Rng, Tribes, PLAYERS};

/// Which side of the source-game split a caller wants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EraSplit {
    Train,
    Holdout,
    All,
}

impl EraSplit {
    pub fn parse(name: &str) -> Option<EraSplit> {
        match name {
            "train" => Some(EraSplit::Train),
            "holdout" => Some(EraSplit::Holdout),
            "all" => Some(EraSplit::All),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            EraSplit::Train => "train",
            EraSplit::Holdout => "holdout",
            EraSplit::All => "all",
        }
    }
}

/// One logged position ready to seat: the turn it starts on, the file's
/// lobby, the player exactly as `turn_start_game` seats it, and whether its
/// source game is held out of training.
#[derive(Clone, Debug)]
pub struct EraPosition {
    pub turn: u32,
    pub lobby: Option<Tribes>,
    pub source: String,
    pub game: u32,
    pub holdout: bool,
    pub player: Player,
}

/// Every usable Era position of a turn range, split by source game. Built
/// once and shared (`Arc`) like `RealBoards`.
#[derive(Clone, Debug, Default)]
pub struct EraStarts {
    positions: Vec<EraPosition>,
    all: Vec<usize>,
    train: Vec<usize>,
    holdout: Vec<usize>,
    /// Usable positions per turn in the range, ascending.
    per_turn: Vec<(u32, usize)>,
    /// Total positions left out, and why (one line per missing item, counted
    /// once per position).
    dropped: usize,
    reasons: Vec<(String, usize)>,
    /// Usable positions whose loaded state had an open offer or choice. The
    /// action files do not carry offers, so those are left out.
    offers_dropped: usize,
}

impl EraStarts {
    /// Seat every turn of `specs` in `turn_min..=turn_max` with the shop
    /// replay and keep the ones whose seating reports nothing missing. The
    /// returned strings are loader misses, for the caller's report.
    pub fn build(
        specs: &[ShopFileSpec],
        data: &Arc<Data>,
        ids: &CardIdMap,
        turn_min: u32,
        turn_max: u32,
    ) -> (EraStarts, Vec<String>) {
        let mut starts = EraStarts::default();
        let mut reasons: BTreeMap<String, usize> = BTreeMap::new();
        let mut missing_lines = Vec::new();
        let mut per_turn: BTreeMap<u32, usize> = BTreeMap::new();
        for turn in turn_min..=turn_max {
            per_turn.insert(turn, 0);
        }
        for file in specs {
            let lobby = file.lobby();
            for turn in &file.turns {
                if turn.turn < turn_min || turn.turn > turn_max {
                    continue;
                }
                let Some((game, missing)) =
                    shop_replay::turn_start_game(data, ids, file, turn, 0x4552_4153_5441_5254)
                else {
                    starts.dropped += 1;
                    *reasons.entry("the turn's first state does not seat".to_string()).or_default() += 1;
                    continue;
                };
                if !missing.is_empty() {
                    starts.dropped += 1;
                    missing_lines.extend(missing.iter().cloned());
                    for line in &missing {
                        *reasons.entry(line.clone()).or_default() += 1;
                    }
                    continue;
                }
                let mut player = game.players[shop_replay::ME].clone();
                // The body and hand are the log's, not pool cards: they hold
                // no copies, exactly like a scenario body. The logged Tavern
                // is not kept; the engine rolls the start of the turn.
                strip_pool_copies(&mut player);
                if has_offers(&player) {
                    starts.offers_dropped += 1;
                }
                clear_offers(&mut player);
                player.shop.clear();
                player.shop_spell = None;
                player.frozen = false;
                player.done = false;
                player.actions = 0;
                player.placement = 0;
                player.alive = true;
                player.real = false;
                player.skipped = false;
                player.opponent = None;
                let holdout = is_holdout(&file.source, file.game);
                starts.positions.push(EraPosition {
                    turn: turn.turn,
                    lobby,
                    source: file.source.clone(),
                    game: file.game,
                    holdout,
                    player,
                });
                *per_turn.entry(turn.turn).or_default() += 1;
            }
        }
        for (i, pos) in starts.positions.iter().enumerate() {
            starts.all.push(i);
            if pos.holdout {
                starts.holdout.push(i);
            } else {
                starts.train.push(i);
            }
        }
        starts.per_turn = per_turn.into_iter().collect();
        starts.reasons = reasons.into_iter().collect();
        (starts, missing_lines)
    }

    pub fn len(&self) -> usize {
        self.positions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.positions.is_empty()
    }

    pub fn position(&self, i: usize) -> &EraPosition {
        &self.positions[i]
    }

    /// Positions of one side of the split, in the files' order.
    pub fn indices(&self, split: EraSplit) -> &[usize] {
        match split {
            EraSplit::Train => &self.train,
            EraSplit::Holdout => &self.holdout,
            EraSplit::All => &self.all,
        }
    }

    /// The positions per turn and the dropped ones, as lines for a report.
    pub fn report(&self) -> String {
        let mut out = String::new();
        out.push_str("era starts: ");
        out.push_str(
            &self
                .per_turn
                .iter()
                .map(|(t, n)| format!("t{t}: {n}"))
                .collect::<Vec<_>>()
                .join(", "),
        );
        out.push_str(&format!(
            " ({} positions, {} train, {} holdout; {} dropped, {} with an open offer left out)",
            self.positions.len(),
            self.train.len(),
            self.holdout.len(),
            self.dropped,
            self.offers_dropped
        ));
        for (reason, n) in &self.reasons {
            out.push_str(&format!("\n  {n} x {reason}"));
        }
        out
    }
}

/// Whether a seated position has an offer or choice open. The action files
/// carry none of this, so an Era start drops it.
fn has_offers(pl: &Player) -> bool {
    !pl.discover.is_empty()
        || !pl.power_offer.is_empty()
        || !pl.secret_offer.is_empty()
        || !pl.trinket_offer.is_empty()
        || pl.pending_target.is_some()
        || pl.pending_shop_target.is_some()
        || pl.pending_choose_one.is_some()
        || pl.pending_discard.is_some()
        || pl.wager_offer.is_some()
        || pl.holmes_offer.is_some()
}

/// Drop any offer or choice the seated trinkets and powers opened while the
/// position was built. The action files do not carry offers, so none of these
/// cards came from the new game's pool; keeping them would make the pool
/// accounting disagree with the held cards.
fn clear_offers(pl: &mut Player) {
    pl.discover.clear();
    pl.discover_minions.clear();
    pl.discover_gifts.clear();
    pl.discover_copies.clear();
    pl.discover_queue.clear();
    pl.dark_offer = false;
    pl.power_offer.clear();
    pl.secret_offer.clear();
    pl.hero_offer = [u8::MAX; crate::DISCOVER];
    pl.trinket_offer.clear();
    pl.trinket_offer_slot = 0;
    pl.pending_target = None;
    pl.pending_shop_target = None;
    pl.pending_choose_one = None;
    pl.pending_discard = None;
    pl.repeats.clear();
    pl.repeat_discover = 0;
    pl.discover_silent = false;
    pl.wager_offer = None;
    pl.holmes_offer = None;
}

/// Zero a player's pool copies in every zone a position can carry. Board and
/// hand minions are log cards, not pool cards; a queued start-of-turn card is
/// the same. The logged Tavern is dropped by the caller.
fn strip_pool_copies(pl: &mut Player) {
    for m in pl
        .board
        .iter_mut()
        .chain(pl.hand.iter_mut())
        .chain(pl.hand_queue.iter_mut())
        .chain(pl.discover_minions.iter_mut())
        .chain(pl.next_board.iter_mut())
        .chain(pl.locked_and_loaded.iter_mut())
        .chain(pl.last_discarded.iter_mut())
    {
        m.pool_copies = 0;
    }
    for seen in &mut pl.seen {
        for m in &mut seen.board {
            m.pool_copies = 0;
        }
    }
}

/// Whether a source game is held out: a stable FNV-1a hash of the file name
/// and game number, 15 in 100. It does not depend on the set of files.
pub fn is_holdout(source: &str, game: u32) -> bool {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for b in source.as_bytes().iter().chain(&game.to_le_bytes()) {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h % 100 < 15
}

/// What one Era-start game starts with: which position and which seat gets
/// it. The other seats are the game's own (real boards, real seats).
#[derive(Clone, Copy, Debug)]
pub struct EraStartPlan {
    pub position: usize,
    pub seat: usize,
}

/// The Era-start settings: the shared position list, the positions this
/// caller may draw (turn range and split), and the share of new games that
/// start from one.
#[derive(Clone, Debug)]
pub struct EraStartConfig {
    pub starts: Arc<EraStarts>,
    pub choices: Vec<usize>,
    pub share: f64,
}

impl EraStartConfig {
    pub fn new(
        starts: Arc<EraStarts>,
        share: f64,
        split: EraSplit,
    ) -> Result<EraStartConfig, String> {
        if !(0.0..=1.0).contains(&share) {
            return Err(format!("era starts: share {share} is not between 0 and 1"));
        }
        let choices = starts.indices(split).to_vec();
        if choices.is_empty() {
            return Err(format!(
                "era starts: no positions on the {} split",
                split.name()
            ));
        }
        Ok(EraStartConfig { starts, choices, share })
    }

    /// An Era-start plan for one game, or None when this game is normal.
    /// `seats` are the network seats that may take Era's position.
    pub fn plan(&self, seed: u64, seats: &[usize]) -> Option<EraStartPlan> {
        if seats.is_empty() || self.share <= 0.0 {
            return None;
        }
        let mut rng = Rng::new(seed ^ 0x4552_4153_5441_5254);
        if self.share < 1.0 {
            let take = (self.share * 1_000_000.0) as u32;
            if rng.below(1_000_000) >= take {
                return None;
            }
        }
        let position = self.choices[rng.below(self.choices.len() as u32) as usize];
        let seat = seats[rng.below(seats.len() as u32) as usize];
        Some(EraStartPlan { position, seat })
    }
}

impl Game {
    /// An Era-start game: turn `position.turn` begins with one network seat
    /// (`plan.seat`) holding Era's logged position, the other network seats
    /// holding a real board of that turn as a body, and the real seats playing
    /// as usual. The lobby is the file's, the Tavern is the engine's roll.
    /// The rest of the game runs normally, Trinkets and Dark Gifts included.
    pub fn new_era_start(
        data: Arc<Data>,
        seed: u64,
        real: &[usize],
        boards: Arc<crate::real_boards::RealBoards>,
        starts: &EraStarts,
        plan: &EraStartPlan,
    ) -> Game {
        let pos = starts.position(plan.position);
        assert!(pos.turn >= 1, "an Era start begins at turn 1 or later");
        let mut g = Game::unstarted(data, seed, pos.lobby);
        g.era_start_seat = Some(plan.seat);
        g.era_start_turn = Some(pos.turn);
        // The Era seat takes no dealt hero (and so none of a hero's starting
        // cards); its own logged hero and power go in after the deal.
        g.players[plan.seat].real = true;
        g.seat_real(real);
        g.real_boards = Some(boards.clone());
        g.turn = pos.turn - 1;
        let era_hero: Vec<u8> = pos.player.hero.into_iter().collect();
        g.deal_heroes_excluding(&era_hero);
        for p in 0..PLAYERS {
            if Self::choice_open(&g.players[p]) {
                g.resolve_open_choices(p);
            }
        }
        let lobby = g.lobby;
        // Real seats get the recorded Health and armor of a random real board
        // of the turn, not the flat 30 and 10 a scenario game gives them.
        for &p in real {
            let board = boards.draw_filtered(pos.turn, lobby, None, &mut g.rng);
            if board.health > 0 {
                g.players[p].health = board.health;
                g.players[p].armor = board.armor;
                g.players[p].vanguard_on = g.players[p].vanguard_health_on();
            }
        }
        // The other network seats: a real board of the turn through the lobby
        // filter, an empty hand, the board's tier and recorded Health.
        let mut health: Vec<(usize, i32, i32)> = Vec::new();
        for p in 0..PLAYERS {
            if p == plan.seat || real.contains(&p) {
                continue;
            }
            clear_network_zones(&mut g, p);
            let board = boards.draw_filtered(pos.turn, lobby, None, &mut g.rng);
            {
                let pl = &mut g.players[p];
                pl.board.clear();
                for m in &board.minions {
                    let _ = pl.board.try_push(*m);
                }
                pl.hand.clear();
                pl.tier = board.tier;
                pl.take_real_counters(&board.counters, board.deity);
            }
            let (h, a) = if board.health > 0 {
                (board.health, board.armor)
            } else {
                // The scenario Health rule; the hero's own armor stays.
                (15 + g.rng.below(21) as i32, g.players[p].armor)
            };
            health.push((p, h, a));
        }
        // The one Era seat: the exact position, over the hero the deal gave.
        clear_network_zones(&mut g, plan.seat);
        g.players[plan.seat] = pos.player.clone();
        g.start_turn();
        for (p, h, a) in health {
            g.players[p].health = h;
            g.players[p].armor = a;
            g.sync_vanguard(p);
        }
        g
    }

    /// Whether `p` is the Era seat at the turn the position starts on, whose
    /// start-of-turn effects already ran in the logged game.
    pub fn era_start_arrival(&self, p: usize) -> bool {
        self.era_start_seat == Some(p) && Some(self.turn) == self.era_start_turn
    }
}

/// Give back what the hero deal put in a network seat's zones, keeping pool
/// accounting exact, then empty them.
fn clear_network_zones(g: &mut Game, p: usize) {
    let old: Vec<_> = g.players[p]
        .hand
        .iter()
        .chain(g.players[p].board.iter())
        .chain(g.players[p].shop.iter())
        .chain(g.players[p].hand_queue.iter())
        .copied()
        .collect();
    for m in &old {
        let card = g.data.cards.get(m.card);
        if m.pool_copies > 0 && !card.token && !card.is_spell() {
            g.pool[card.pool_slot as usize] += m.pool_copies;
        }
    }
    let pl = &mut g.players[p];
    pl.hand.clear();
    pl.board.clear();
    pl.shop.clear();
    pl.hand_queue.clear();
    pl.shop_spell = None;
}
