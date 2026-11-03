//! Following a log line by line: the lobby and the fights.
//!
//! A port of `Tracker` in the older tracker's `battlegrounds.py`. It reads through
//! [`PowerLog`] and hears about the four moments the reader reports (a new game,
//! a tag change, a block opening and closing), which is all it needs to spot a
//! fight being staged, snapped and played out.
//!
//! How the log stages a fight (the Python's notes, worked out from real logs):
//! your player and the stand-in player that plays your opponent both get
//! `BACON_CURRENT_COMBAT_PLAYER_ID` set to the PlayerIDs fighting, inside a setup
//! block that also creates copies of both warbands in PLAY. Usually that happens
//! before the game step goes to `MAIN_START`, and the snapshot is taken at
//! `MAIN_START`. Sometimes the step comes first, and then the snapshot is taken
//! when the setup block closes. Either way both boards are complete and nothing
//! has attacked yet. When `BACON_CURRENT_COMBAT_PLAYER_ID` drops back to 0 the
//! fight is over.
//!
//! Once the fight is staged the game logs every attack as an ATTACK block. At
//! the first one, start-of-combat effects have already resolved into the tags,
//! so a second snapshot taken there is what the simulator starts from. The block
//! also says who attacks first. The fight ends with the winning hero attacking
//! the loser's hero (another ATTACK block, from a HERO), or with no hero attack
//! at all for a draw.

use std::collections::BTreeMap;
use std::mem;

use crate::cards::Cards;
use crate::entity::{Entity, TagValue};
use crate::game::Game;
use crate::parser::{Block, Handler, PowerLog};
use crate::views::{
    Board, Deity, Fight, HeroPower, LOCK_AND_LOAD, Reward, Seat, Trinket, UPPER_HAND,
    WINGMEN, board, counts, deity, gem, hand, held_entity, hero_power, hidden_in_hand, one_minion,
    reanimates, rewards, secrets, side_numbers, trinkets, turn_of, waiting,
};

/// The fight's state at one moment: what a snapshot fills in, both before
/// start-of-combat effects (`MAIN_START`) and at the first attack.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub at_attack: (Board, Board),
    pub gems: ((i64, i64), (i64, i64)),
    pub hand: Board,
    pub their_hand: Board,
    pub hidden: (i64, i64),
    pub waits: (i64, i64),
    pub hero_powers: (Option<HeroPower>, Option<HeroPower>),
    pub trinkets: (Vec<Trinket>, Vec<Trinket>),
    pub rewards: (Vec<Reward>, Vec<Reward>),
    pub counters: (BTreeMap<&'static str, i64>, BTreeMap<&'static str, i64>),
    pub side_numbers: (BTreeMap<&'static str, i64>, BTreeMap<&'static str, i64>),
    pub deities: (Option<Deity>, Option<Deity>),
}

impl Snapshot {
    fn apply(self, fight: &mut Fight) {
        fight.at_attack = Some(self.at_attack);
        fight.gems = self.gems;
        fight.hand = Some(self.hand);
        fight.their_hand = Some(self.their_hand);
        fight.hidden = self.hidden;
        fight.waits = self.waits;
        fight.hero_powers = self.hero_powers;
        fight.trinkets = self.trinkets;
        fight.rewards = self.rewards;
        fight.counters = self.counters;
        fight.side_numbers = self.side_numbers;
        fight.deities = self.deities;
    }
}

/// Follows a stream of log lines and keeps the lobby and fights current.
///
/// `version` goes up every time something a window would show has changed.
#[derive(Clone, Debug)]
pub struct Tracker {
    log: PowerLog,
    cards: Cards,
    /// The lobby, by PlayerID. Sorted so a dump is stable.
    seats: BTreeMap<i32, Seat>,
    /// The latest fight, as an index into `fights`. It stays set until the
    /// next fight replaces it: the log stages a fight long before the client
    /// finishes animating it.
    fight: Option<usize>,
    fights: Vec<Fight>,
    /// The seats and fights of the earlier games in this log.
    past: Vec<(BTreeMap<i32, Seat>, Vec<Fight>)>,
    version: u64,
    /// Set while a fight is being staged: the block depth at which it started.
    setup_depth: Option<usize>,
    opponent_name: String,
    /// The fight the log is in the middle of, as an index into `fights`.
    live: Option<usize>,
    winner: String,
    /// The blocks open now: (type, entity).
    blocks: Vec<(String, Option<i32>)>,
    /// See [`Tracker::block_end`].
    after_wingmen: Option<Snapshot>,
    /// Upper Hands each side cast since the fight last started (see
    /// [`UPPER_HAND`]), mine and theirs.
    upper_hands: [i64; 2],
    /// A minion Lock and Load holds (see [`crate::views::held_entity`]) to the
    /// side that played it, while its copy is still to come.
    held: BTreeMap<i32, usize>,
}

impl Tracker {
    /// A tracker that names cards from `cards` (see [`Cards`]).
    pub fn new(cards: Cards) -> Tracker {
        Tracker {
            log: PowerLog::new(),
            cards,
            seats: BTreeMap::new(),
            fight: None,
            fights: Vec::new(),
            past: Vec::new(),
            version: 0,
            setup_depth: None,
            opponent_name: String::new(),
            live: None,
            winner: String::new(),
            blocks: Vec::new(),
            after_wingmen: None,
            upper_hands: [0, 0],
            held: BTreeMap::new(),
        }
    }

    /// Read one line.
    pub fn feed(&mut self, line: &str) {
        // The reader reports to this tracker, and the tracker owns the reader:
        // hand the reader over for the call and take it back after.
        let mut log = mem::take(&mut self.log);
        log.feed(line, self);
        self.log = log;
    }

    /// The game being read.
    pub fn game(&self) -> Option<&Game> {
        self.log.game()
    }

    /// Whether the game being read is a Battlegrounds game.
    pub fn is_battlegrounds(&self) -> bool {
        self.log
            .game()
            .map(|g| g.game_type.starts_with("GT_BATTLEGROUNDS"))
            .unwrap_or(false)
    }

    /// My PlayerID: the smallest with a real account behind it.
    pub fn me(&self) -> Option<i32> {
        self.log.game().and_then(me)
    }

    /// The lobby of the game being read, by PlayerID.
    pub fn seats(&self) -> &BTreeMap<i32, Seat> {
        &self.seats
    }

    /// Every fight of the game being read, in the order they were staged.
    pub fn fights(&self) -> &[Fight] {
        &self.fights
    }

    /// The latest fight of the game being read.
    pub fn fight(&self) -> Option<&Fight> {
        self.fight.and_then(|i| self.fights.get(i))
    }

    /// The seats and fights of the earlier games in the log.
    pub fn past(&self) -> &[(BTreeMap<i32, Seat>, Vec<Fight>)] {
        &self.past
    }

    /// Goes up every time something a window would show has changed.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// The blocks open now, the outermost first: each one's `BlockType` and
    /// the entity it acts on. A block the log opened but the reader could not
    /// parse keeps its place with an empty type, so a caller can count the
    /// nesting the log itself printed.
    pub fn blocks(&self) -> &[(String, Option<i32>)] {
        &self.blocks
    }

    /// Every seat and fight of the log, the finished games first, ending with
    /// the game being read.
    pub fn games(&self) -> Vec<(&BTreeMap<i32, Seat>, &[Fight])> {
        let mut games: Vec<(&BTreeMap<i32, Seat>, &[Fight])> = self
            .past
            .iter()
            .map(|(seats, fights)| (seats, fights.as_slice()))
            .collect();
        games.push((&self.seats, &self.fights));
        games
    }

    /// The player standing in for the opponent: the first player with no real
    /// account behind them.
    fn capture(&self, g: &Game) -> Snapshot {
        let me = me(g);
        let stand_pid = g.stand_in_player();
        let turn = self.live.map(|i| self.fights[i].turn).unwrap_or(0);
        Snapshot {
            at_attack: (
                board(g, &self.cards, me),
                board(g, &self.cards, stand_pid),
            ),
            gems: (gem(player(g, me)), gem(player(g, stand_pid))),
            hand: hand(g, &self.cards, me),
            their_hand: hand(g, &self.cards, stand_pid),
            hidden: (hidden_in_hand(g, me), hidden_in_hand(g, stand_pid)),
            waits: (waiting(g, player(g, me)), waiting(g, player(g, stand_pid))),
            hero_powers: (
                hero_power(g, &self.cards, me, turn),
                hero_power(g, &self.cards, stand_pid, turn),
            ),
            trinkets: (
                trinkets(g, &self.cards, me),
                trinkets(g, &self.cards, stand_pid),
            ),
            rewards: (
                rewards(g, &self.cards, me),
                rewards(g, &self.cards, stand_pid),
            ),
            counters: (counts(g, player(g, me)), counts(g, player(g, stand_pid))),
            side_numbers: (
                side_numbers(g, player(g, me)),
                side_numbers(g, player(g, stand_pid)),
            ),
            deities: (deity(g, &self.cards, me), deity(g, &self.cards, stand_pid)),
        }
    }

    /// Start the snapshot of a staged fight, at `MAIN_START` or when its setup
    /// block closed.
    ///
    /// This is the one moment a file's board is read from: the board before
    /// Start of Combat. So it is also where the state that only exists then is
    /// read: Rapid Reanimation's mark (by the first attack the minion it named
    /// is destroyed and the enchantment sits on the copy), each side's Secret
    /// zone, the Upper Hands cast this turn, and the minions Lock and Load
    /// still holds.
    fn start_fight(&mut self, g: &Game) {
        self.setup_depth = None; // one snapshot per fight
        self.held.clear();
        let Some(stand_pid) = g.stand_in_player() else {
            return;
        };
        let opponent = g
            .player_entity(stand_pid)
            .map(|e| e.int("BACON_CURRENT_COMBAT_PLAYER_ID"))
            .unwrap_or(0) as i32;
        let me = me(g);
        let mut fight = Fight::new(
            turn_of(g),
            opponent,
            board(g, &self.cards, me),
            board(g, &self.cards, Some(stand_pid)),
        );
        fight.secrets = (secrets(g, me), secrets(g, Some(stand_pid)));
        fight.reanimates = (reanimates(g, me), reanimates(g, Some(stand_pid)));
        fight.upper_hands = (self.upper_hands[0], self.upper_hands[1]);
        for (side, pid) in [(0, me), (1, Some(stand_pid))] {
            let held = held_entity(g, pid);
            if held != 0 {
                self.held.insert(held as i32, side);
            }
        }
        self.fights.push(fight);
        let index = self.fights.len() - 1;
        self.fight = Some(index);
        self.live = Some(index);
        self.winner.clear();
        self.after_wingmen = None;
        // A new turn's Upper Hands start from zero.
        self.upper_hands = [0, 0];
        if let Some(seat) = self.seats.get_mut(&opponent) {
            seat.last_fight = Some(index);
            if !self.opponent_name.is_empty() {
                seat.name = self.opponent_name.clone();
            }
        }
        self.version += 1;
    }

    /// Close the fight in progress with its result.
    fn end_fight(&mut self) {
        let Some(live) = self.live.take() else {
            return;
        };
        if self.fights[live].at_attack.is_none()
            && let Some(snapshot) = self.after_wingmen.take()
        {
            // Only Wingmen attacked: nothing came after it to snapshot.
            snapshot.apply(&mut self.fights[live]);
        }
        if self.fights[live].at_attack.is_none() {
            // Nobody could attack.
            let at_attack = (self.fights[live].mine.clone(), self.fights[live].theirs.clone());
            self.fights[live].at_attack = Some(at_attack);
        }
        let result = match self.winner.as_str() {
            "me" => "win",
            "them" => "lose",
            _ => "tie",
        };
        self.fights[live].result = result.to_string();
        self.version += 1;
    }

    /// The lobby seat of the leaderboard hero an entity is.
    fn update_seat(&mut self, g: &Game, hero: i32) {
        let Some(entity) = g.entities.get(&hero) else {
            return;
        };
        let player_id = entity.int("PLAYER_ID");
        if player_id == 0 {
            return;
        }
        let player_id = player_id as i32;
        let hero_card = entity.card_id.clone();
        let hero_name = self.cards.name(&hero_card).to_string();
        let health = entity.int("HEALTH") - entity.int("DAMAGE");
        let armor = entity.int("ARMOR");
        let tier = entity.int("PLAYER_TECH_LEVEL");
        let place = entity.int("PLAYER_LEADERBOARD_PLACE");
        let seat = self
            .seats
            .entry(player_id)
            .or_insert_with(|| Seat::new(player_id));
        seat.hero_card = hero_card;
        seat.hero = hero_name;
        seat.health = health;
        seat.armor = armor;
        seat.tier = tier;
        seat.place = place;
        self.version += 1;
    }

    /// True while the log is inside a block opened by Illidan's Wingmen (the
    /// innermost block, the one being looked at, is left out).
    fn in_wingmen(&self, g: &Game) -> bool {
        let open = &self.blocks[..self.blocks.len().saturating_sub(1)];
        open.iter().any(|(_, entity)| is_card(g, *entity, WINGMEN))
    }

    fn in_wingmen_open(&self, g: &Game) -> bool {
        self.blocks
            .iter()
            .any(|(_, entity)| is_card(g, *entity, WINGMEN))
    }

    /// Take down the minion a Lock and Load copy was made from.
    ///
    /// Lock and Load's held minion is not in the log when the opponent stole
    /// it (their Tavern is hidden), but the copy the power summons names it in
    /// `COPIED_FROM_ENTITY_ID`, with the stats it had when it was taken. It is
    /// read as it appears, before it fights.
    fn waiting_copy(&mut self, g: &Game, entity: i32, value: &TagValue) {
        let Some(&side) = self.held.get(&(value.as_int() as i32)) else {
            return;
        };
        let Some(live) = self.live else {
            return;
        };
        let Some(e) = g.entities.get(&entity) else {
            return;
        };
        if e.text("CARDTYPE") != "MINION" || e.card_id.is_empty() {
            return;
        }
        let waiting = &mut self.fights[live].waiting;
        let slot = if side == 0 { &mut waiting.0 } else { &mut waiting.1 };
        if slot.is_none() {
            *slot = Some(one_minion(e, g, &self.cards));
        }
    }
}

impl Handler for Tracker {
    /// The game a `CREATE_GAME` ended: its seats and fights are done, if it is
    /// a Battlegrounds game.
    fn end_game(&mut self, game: &Game) {
        if game.game_type.starts_with("GT_BATTLEGROUNDS") {
            self.past.push((
                mem::take(&mut self.seats),
                mem::take(&mut self.fights),
            ));
        }
    }

    fn new_game(&mut self, _game: &mut Game) {
        self.fight = None;
        self.setup_depth = None;
        self.live = None;
        self.blocks.clear();
        self.after_wingmen = None;
        self.version += 1;
    }

    fn tag_change(
        &mut self,
        g: &mut Game,
        depth: usize,
        entity: i32,
        tag: &str,
        value: &TagValue,
    ) {
        if matches!(
            tag,
            "PLAYER_LEADERBOARD_PLACE" | "DAMAGE" | "ARMOR" | "HEALTH" | "PLAYER_TECH_LEVEL"
        ) {
            let place = g
                .entities
                .get(&entity)
                .map(|e| e.int("PLAYER_LEADERBOARD_PLACE"))
                .unwrap_or(0);
            if place != 0 {
                self.update_seat(g, entity);
            }
        } else if tag == "STEP" && value.as_text() == Some("MAIN_START") && is_game_entity(g, entity)
        {
            if self.setup_depth.is_some() {
                self.start_fight(g);
            }
        } else if tag == "STATE"
            && value.as_text() == Some("COMPLETE")
            && is_game_entity(g, entity)
        {
            // The fight that ends a game never gets its marker reset.
            if self.live.is_some() {
                self.end_fight();
            }
        } else if tag == "BACON_CURRENT_COMBAT_PLAYER_ID" {
            let my_entity = me(g).and_then(|me| g.players.get(&me).copied());
            if my_entity == Some(entity) {
                self.setup_depth = if truthy(value) { Some(depth) } else { None };
                if !truthy(value) && self.live.is_some() {
                    self.end_fight();
                }
            } else if truthy(value) {
                // The name the log calls the stand-in player is only the
                // opponent's during a fight: outside one it hosts Bob's tavern.
                let name = g.last_name.get(&entity).cloned().unwrap_or_default();
                self.opponent_name = if name == "Bartender Bob" {
                    String::new()
                } else {
                    name
                };
            }
        }
        if tag == "COPIED_FROM_ENTITY_ID" {
            self.waiting_copy(g, entity, value);
        }
    }

    fn block_start(&mut self, g: &mut Game, depth: usize, block: Block<'_>) {
        // Keep the open blocks in step with the log's own depth, since a
        // BLOCK_START the reader could not parse still counts there.
        let depth = depth.max(1);
        self.blocks.truncate(depth - 1);
        while self.blocks.len() < depth - 1 {
            self.blocks.push((String::new(), None));
        }
        let entity = block.entity;
        let block_type = block.kind;
        self.blocks.push((block_type.to_string(), entity));

        // An Upper Hand cast this turn is a pending Start of Combat trigger,
        // and the spell's card goes back to SETASIDE, so the cast is all the
        // log says: count it for the fight it will fire in.
        if block_type == "PLAY"
            && let Some(e) = entity.and_then(|e| g.entities.get(&e))
            && e.card_id == UPPER_HAND
        {
            let side = if Some(e.int("CONTROLLER")) == me(g).map(|me| me as i64) {
                0
            } else {
                1
            };
            self.upper_hands[side] += 1;
        }

        let Some(live) = self.live else {
            return;
        };
        if block_type != "ATTACK" {
            return;
        }
        let Some(entity) = entity else {
            return;
        };
        let side = if Some(g.entities.get(&entity).map(|e| e.int("CONTROLLER")).unwrap_or(0))
            == me(g).map(|me| me as i64)
        {
            "me"
        } else {
            "them"
        };
        let snapped = self.fights[live].at_attack.is_some();
        // Illidan's Wingmen attacks before every other Start of Combat, so a
        // snapshot taken here would miss the trinkets and minions that come
        // next, and the attacker would not be the first of the normal order.
        // Its attacks sit inside a block of the hero power (as Lock and Load's
        // shot does), so while inside one the snapshot waits for the next
        // attack. UNTESTED on a real log: none of Era's has an Illidan.
        if !snapped && self.in_wingmen(g) {
            return;
        }
        if !snapped {
            let snapshot = self.capture(g);
            snapshot.apply(&mut self.fights[live]);
            self.fights[live].first = side.to_string();
            // An attack made by a minion a hero power created is the hero
            // power's own shot, not the start of the normal order.
            let maker = match g.entities.get(&entity).map(|e| e.int("CREATOR")).unwrap_or(0) {
                0 => None,
                creator => g.entities.get(&(creator as i32)),
            };
            let cardtype = g
                .entities
                .get(&entity)
                .map(|e| e.text("CARDTYPE"))
                .unwrap_or("");
            if maker.map(|m| m.card_id.as_str()) == Some(LOCK_AND_LOAD)
                && cardtype == "MINION"
            {
                let controller = g.entities.get(&entity).map(|e| e.int("CONTROLLER")).unwrap_or(0);
                let mut ids: Vec<&Entity> = g
                    .entities_in_order()
                    .filter(|x| {
                        x.text("CARDTYPE") == "MINION"
                            && x.text("ZONE") == "PLAY"
                            && x.int("CONTROLLER") == controller
                    })
                    .collect();
                ids.sort_by_key(|x| x.int("ZONE_POSITION"));
                let mine_side = if side == "me" { 0 } else { 1 };
                if let Some(index) = ids.iter().position(|x| x.id == entity) {
                    self.fights[live].opening = Some((mine_side, index));
                }
            }
        }
        if g.entities.get(&entity).map(|e| e.text("CARDTYPE")) == Some("HERO") {
            self.winner = side.to_string();
        }
    }

    fn block_end(&mut self, g: &mut Game, depth: usize) {
        let kind = self
            .blocks
            .get(depth)
            .map(|(kind, _)| kind.clone())
            .unwrap_or_default();
        self.blocks.truncate(depth);
        if let Some(live) = self.live
            && self.fights[live].at_attack.is_none()
            && kind == "ATTACK"
            && self.in_wingmen_open(g)
        {
            // A Wingmen attack just finished. If nothing attacks after it (it
            // killed the whole enemy board), this is the last state there is.
            self.after_wingmen = Some(self.capture(g));
        }
        if self.setup_depth.is_some_and(|setup| depth < setup)
            && g.game_text("STEP") == "MAIN_START"
        {
            self.start_fight(g);
        }
    }
}

/// Whether the entity with this id is that card (the Python's
/// `entity.card_id == card` for a block's entity). `None` is never a card.
fn is_card(g: &Game, entity: Option<i32>, card_id: &str) -> bool {
    entity
        .and_then(|e| g.entities.get(&e))
        .map(|e| e.card_id == card_id)
        .unwrap_or(false)
}

/// Whether a tag change is on the game entity (`e is g.game_entity`).
fn is_game_entity(g: &Game, entity: i32) -> bool {
    let game_entity = g.names.get("GameEntity").copied().unwrap_or(1);
    game_entity == entity
}

/// The Python's truthiness for a tag value: 0 and "" are false.
fn truthy(value: &TagValue) -> bool {
    match value {
        TagValue::Int(n) => *n != 0,
        TagValue::Text(text) => !text.is_empty(),
    }
}

/// My PlayerID: the smallest with a real account behind it.
fn me(g: &Game) -> Option<i32> {
    g.real_players.iter().copied().min()
}

/// A player's entity, if the log has one.
fn player(g: &Game, player_id: Option<i32>) -> Option<&Entity> {
    player_id.and_then(|player_id| g.player_entity(player_id))
}
