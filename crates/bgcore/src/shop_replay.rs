//! The shop replay: load the state Era had before each step of a real shop
//! phase from `boards/actions/*.json` (docs/real-boards.md "The action file"),
//! apply the step in the engine, and compare the result with the state the
//! game left (docs/real-boards.md "The shop replay").
//!
//! It is a bug finder like the fight check (`crate::fight_check`): every step
//! is replayed on its own, from the logged state before it, so one early
//! mismatch spoils nothing later. A step that draws no random number is
//! compared exactly; one that does is run with several seeds and passes when
//! the log's result is one of the engine's, or, for a large random space (a
//! Refresh), when it has the same shape.
//!
//! Nothing here changes a rule. What the engine does not build, or what the log
//! does not say, makes a step *incomplete* rather than flagged.

use std::collections::HashMap;
use std::sync::Arc;

use arrayvec::ArrayVec;
use serde::Deserialize;

use crate::cards::{Effect, EventKind, Tribe};
use crate::fight_check::{
    SeatMode, seat_power as fight_seat_power, seat_secrets, seat_trinkets,
};
use crate::game::{Opponent, Player};
use crate::minion::Minion;
use crate::real_boards::{
    CardIdMap, RealBoard, RealCounters, RealMinionSpec, RealPowerSpec, RealTrinketSpec,
};
use crate::{Action, BOARD, CardId, Data, Game, HAND, Keywords, MAX_TIER};

// ---- the action file (docs/real-boards.md "The action file") ----

/// A Dark Gift on a logged card: the gift's own card id and the name the file
/// shows (the name is for the report only).
#[derive(Clone, Debug, Default, Deserialize)]
pub struct ShopGiftSpec {
    #[serde(default)]
    pub card_id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    /// The countdown the gift's own enchantment shows
    /// (TAG_SCRIPT_DATA_NUM_1 in the log): Incubation, Replication and
    /// Affinity number their turns there, so the loader seats where they
    /// have got to instead of starting them over.
    #[serde(default)]
    pub state: Option<i32>,
    /// The turn this gift first shows on this card anywhere in the file, the
    /// turn it landed: the loader seats Steady Growth's growth from it, which
    /// the gift fixes at landing and which no single state can say
    /// (`seat_gift_landings`).
    #[serde(skip)]
    pub landed: Option<u32>,
}

/// One card of a logged state: my board (with `position`), my hand (`index`),
/// or a Tavern offer (`slot`, `kind`, `cost`, `frozen`). The stats, keywords,
/// gifts and numbers are the file's, the way `boards/real/` writes a minion.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct ShopMinionSpec {
    #[serde(default)]
    pub id: u64,
    #[serde(default)]
    pub card_id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub attack: i32,
    #[serde(default)]
    pub health: i32,
    #[serde(default)]
    pub golden: bool,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub numbers: Vec<i32>,
    #[serde(default)]
    pub dark_gifts: Vec<ShopGiftSpec>,
    #[serde(default)]
    pub reanimate: bool,
    /// The Attack and Health this minion's Blood Gems have given it, from the
    /// log's own pair of tags on the card (`Minion::gem_attack`/`gem_health`,
    /// docs/real-boards.md "The action file"). Gem Confiscation steals exactly
    /// this pair from the neighbours, a Gem adds to it and a triple sums it. A
    /// file written before the keys existed has it nowhere, and then the
    /// minion starts at 0, as it always did.
    #[serde(default)]
    pub gem_attack: i32,
    #[serde(default)]
    pub gem_health: i32,
    /// How many Magnetizations this minion carries (`Minion.magnetizations`,
    /// which Utility Drone's "+4/+5 for each Magnetization they have" reads).
    /// The collector counts the Magnetic enchantments the log leaves on the
    /// host, one per merge (docs/real-boards.md "The action file"). A file
    /// written before the key existed has it nowhere, and then the minion
    /// starts at 0, as it always did.
    #[serde(default)]
    pub magnetizations: u32,
    /// The extra times the next Magnetization onto this minion happens
    /// (`Minion.extra_magnetize`, a Drone Duplicator's Activate). The collector
    /// reads the game's own held-extra tag (docs/real-boards.md "The action
    /// file"). A file written before the key existed has it nowhere, and then
    /// the minion holds none, as it always did.
    #[serde(default)]
    pub extra_magnetize: u8,
    /// A Golden minion that gives no Triple Reward when played, the file's
    /// mark: only the sources docs/rules/design.md names say so (Silent
    /// Deliverer's goldens, Sinister Invitation's, Gilding, Aureate Laureate).
    /// The engine's `Minion.no_reward`, which the loader also sets for an
    /// `always_golden` card whatever the file says.
    #[serde(default)]
    pub no_reward: bool,
    #[serde(default)]
    pub position: Option<u8>,
    #[serde(default)]
    pub index: Option<u8>,
    #[serde(default)]
    pub slot: Option<u8>,
    /// `minion` or `spell` in the Tavern, `spell` in the hand.
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub frozen: bool,
    /// A Tavern spell's Gold price, the one the client charged.
    #[serde(default)]
    pub cost: Option<i32>,
    /// A Tavern card priced in Health (Bazaar Sticker, Hasty Excavation).
    #[serde(default)]
    pub costs_health: bool,
    /// The Gold price the log shows on a Tavern card when it is not the
    /// rule's: the override Chillmere Mosaic's Spellcraft puts on the minions
    /// it deals (BACON_OVERRIDE_BG_COST, the whole price).
    #[serde(default)]
    pub cost_override: Option<i32>,
    /// What a Tavern card's price is reduced by (BACON_REDUCE_BUY_COST): a
    /// free buy takes the whole buy cost off every card until it is used
    /// (Aranna's Spectral Sight).
    #[serde(default)]
    pub cost_reduce: Option<i32>,
    /// The spell a Magicfin token was taught, by the name cards.toml builds
    /// it under. The log keeps it as a dbf id in the token's first number and
    /// only `ref/bg-cards.json` can say which card that is, so
    /// `seat_taught_spells` fills this in before the replay runs; a file may
    /// also carry it.
    #[serde(default)]
    pub taught: Option<String>,
    /// The card this one came from (the `from` the collector writes on a card
    /// a source handed out): a linked pair is read off it (`seat_links`). No
    /// file written before the key existed has it.
    #[serde(default)]
    pub from: Option<ShopPlace>,
}

impl ShopMinionSpec {
    /// The card as the board loader builds one: name, attack, health, golden
    /// flag, keywords, gifts, numbers and the Rapid Reanimation mark.
    fn real_spec(&self) -> RealMinionSpec {
        let name = self.name.as_deref().unwrap_or_default();
        RealMinionSpec {
            // Dark Paradox's five versions share one name in the log, and the
            // file's card id says which one (crate::real_boards); every other
            // card keeps the file's name, which needs no ref data.
            name: match self.card_id.as_deref() {
                Some(id) => crate::real_boards::resolved_name(id, name),
                None => name.to_string(),
            },
            attack: self.attack,
            health: self.health,
            golden: self.golden,
            keywords: self.keywords.clone(),
            numbers: self.numbers.clone(),
            dark_gifts: self
                .dark_gifts
                .iter()
                .filter_map(|g| g.card_id.clone())
                .collect(),
            dark_gift_states: self.dark_gifts.iter().map(|g| g.state.unwrap_or(-1)).collect(),
            reanimate: self.reanimate,
            no_reward: self.no_reward,
        }
    }
}

/// A logged hero power: the card id, its numbers ({0} {1} {2} {3}), the price
/// on its button, the presses used this turn, whether it sits in the second
/// slot, and the minion it still owes the board (Lock and Load).
#[derive(Clone, Debug, Default, Deserialize)]
pub struct ShopPowerSpec {
    #[serde(default)]
    pub card_id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub numbers: Vec<i32>,
    /// The price the log shows on the button, when it shows one: the card's
    /// own cost plus whatever the power's discounts and surcharges have made
    /// of it (Pirate Parrrrty!'s Pirates, The Galaxy's Lens's turns).
    #[serde(default)]
    pub cost: Option<i32>,
    #[serde(default)]
    pub used_this_turn: u8,
    #[serde(default)]
    pub second: bool,
    #[serde(default)]
    pub waiting: Option<ShopMinionSpec>,
}

/// A logged trinket: the card id, whether it sits in the greater slot, and its
/// current numbers.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct ShopTrinketSpec {
    #[serde(default)]
    pub card_id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub greater: bool,
    #[serde(default)]
    pub numbers: Vec<i32>,
}

/// One state of a shop phase as the file writes it: a turn's `start`, the
/// `after` of any step, or the turn's `end`. One shape for all three, so the
/// replay loads any of them the same way.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct ShopStateSpec {
    #[serde(default)]
    pub board: Vec<ShopMinionSpec>,
    #[serde(default)]
    pub hand: Vec<ShopMinionSpec>,
    #[serde(default)]
    pub tavern: Vec<ShopMinionSpec>,
    #[serde(default)]
    pub hand_hidden: u32,
    #[serde(default)]
    pub gold: i32,
    #[serde(default)]
    pub gold_max: i32,
    #[serde(default)]
    pub tier: u8,
    /// The price on the tier up button, null at the top tier.
    #[serde(default)]
    pub upgrade_cost: Option<i32>,
    #[serde(default)]
    pub health: i32,
    #[serde(default)]
    pub armor: i32,
    #[serde(default)]
    pub free_refreshes: u8,
    /// The Demon Fodders queued for the coming Refreshes, one number per
    /// Refresh (`Player.fodder_refreshes`): the log's "Fodder Player Enchant"
    /// numbers. None in a file written before the collector read them, and
    /// then the step is judged without the queue.
    #[serde(default)]
    pub fodder_refreshes: Option<Vec<u8>>,
    #[serde(default)]
    pub frozen: bool,
    #[serde(default)]
    pub hero_powers: Vec<ShopPowerSpec>,
    #[serde(default)]
    pub trinkets: Vec<ShopTrinketSpec>,
    /// The player-level numbers, by key (`RealCounters::set`). A key is there
    /// only when the log showed a number.
    #[serde(default)]
    pub counters: HashMap<String, i32>,
    /// The Secret zone, by card id in the order the Secrets were got.
    #[serde(default)]
    pub secrets: Option<Vec<String>>,
    /// Whether Era won the last fight, which the cards that say "if you lost
    /// your last combat" (Tortollan Blue Shell) read. None on a first turn.
    #[serde(default)]
    pub won_last_combat: Option<bool>,
    /// The warband of the opponent I last fought, as the log last showed it
    /// in combat: the board that went into the fight, before start-of-combat
    /// effects, in the shape `board` has. It is the seen board of the last
    /// opponent (`Player.seen`, which Bob's Burgles and Time Thief copy from)
    /// and the closest board the file has to the next opponent's warband for
    /// I Spy (`Player.next_board`): the log writes the next opponent's board
    /// only after their own shop phase, so no state can hold it. None before
    /// the first fight, and in a file written before the collector read it;
    /// then the powers that copy a warband have nothing to copy.
    #[serde(default)]
    pub last_opponent_board: Option<Vec<ShopMinionSpec>>,
}

/// Where a step's card or target sits, as the file writes it.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct ShopPlace {
    #[serde(default)]
    pub id: u64,
    #[serde(default)]
    pub card_id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    /// `board`, `hand`, `slot`, `hero`, `offer` or a zone name.
    #[serde(default)]
    pub kind: Option<String>,
    /// The same for a card that went somewhere: `moved`'s `from` and `to` say
    /// it as `where` (`hand`, `board`, `tavern`, `offer`, `off`), while a
    /// step's `target` and `card` say it as `kind` (`zone()` reads both).
    #[serde(default, rename = "where")]
    pub place: Option<String>,
    /// The Dark Gifts the card carries (a Dark Discovery's offers).
    #[serde(default)]
    pub dark_gifts: Vec<ShopGiftSpec>,
    /// The card an `entered` entry came from, as the collector writes it (the
    /// card that handed it out): which option's cards arrived is what
    /// `logged_choose_one` reads.
    #[serde(default)]
    pub from: Option<Box<ShopPlace>>,
    /// The keywords and numbers of an offer's card, as it stood: a Discover's
    /// pick lands with them, not with the card's printed ones. None in a file
    /// written before they were collected, and then the card's own stand.
    #[serde(default)]
    pub keywords: Option<Vec<String>>,
    #[serde(default)]
    pub numbers: Option<Vec<i32>>,
    #[serde(default)]
    pub attack: i32,
    #[serde(default)]
    pub health: i32,
    #[serde(default)]
    pub golden: bool,
    #[serde(default)]
    pub position: Option<u8>,
    #[serde(default)]
    pub index: Option<u8>,
    #[serde(default)]
    pub slot: Option<u8>,
    /// The stats a changed card had and has: a step's `changed` list writes
    /// each card as `was` and `now`. The Magnetic host is the card the played
    /// minion's own stats went into (`magnetize_host`).
    #[serde(default)]
    pub was: Option<ShopStats>,
    #[serde(default)]
    pub now: Option<ShopStats>,
}

/// The stats of a card a step changed, before and after the step.
#[derive(Clone, Copy, Debug, Default, Deserialize)]
pub struct ShopStats {
    #[serde(default)]
    pub attack: i32,
    #[serde(default)]
    pub health: i32,
}

/// The Gold a step gained from a card's own trigger, as the collector writes
/// it: how much, and which card gave it.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct ShopGoldGain {
    #[serde(default)]
    pub gold: i32,
    #[serde(default)]
    pub by: Option<ShopPlace>,
}

impl ShopPlace {
    /// The zone this place names: a step's `target` and `card` say it as
    /// `kind`, a `moved` destination as `where`.
    pub fn zone(&self) -> Option<&str> {
        self.place.as_deref().or(self.kind.as_deref())
    }
}

/// One step of a shop phase, as the file names it, with the state it left.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct ShopStepSpec {
    /// `buy`, `sell`, `play`, `activate`, `move`, `roll`, `freeze`, `level`,
    /// `hero_power`, `dark_discovery`, `discover`, `trinket_pick`, `end_turn`.
    #[serde(default)]
    pub step: String,
    #[serde(default)]
    pub slot: Option<u8>,
    #[serde(default)]
    pub index: Option<u8>,
    #[serde(default)]
    pub position: Option<u8>,
    /// Where a dragged board minion landed (`move`).
    #[serde(default)]
    pub to: Option<u8>,
    /// `main` or `second` for a hero power press.
    #[serde(default)]
    pub which: Option<String>,
    /// Whether a `freeze` step froze or unfroze the Tavern.
    #[serde(default)]
    pub on: Option<bool>,
    /// The price the client put on the card or button.
    #[serde(default)]
    pub cost: Option<i32>,
    #[serde(default)]
    pub tier_after: Option<u8>,
    /// The client's own sub-option: which way a Choose One card was played.
    #[serde(default)]
    pub sub_option: Option<i32>,
    /// `lesser` or `greater` for a trinket pick.
    #[serde(default)]
    pub slot_kind: Option<String>,
    /// Where the card the step acted on went.
    #[serde(default)]
    pub moved: Option<ShopMoved>,
    /// The cards the step changed, by where they sit: a Magnetized minion does
    /// not go anywhere itself, it lands in the host it merged into.
    #[serde(default)]
    pub changed: Vec<ShopPlace>,
    #[serde(default)]
    pub spell: bool,
    /// Free Refreshes left when the Refresh button showed a price.
    #[serde(default)]
    pub free: Option<u8>,
    /// The Gold a card's own trigger gave this step, by the card that gave it
    /// (`gold_gain` in the file).
    #[serde(default)]
    pub gold_gain: Option<ShopGoldGain>,
    /// What a `discover` offered: `discover`, `choose_one`, `dark_gift` or
    /// `triple_reward`. A `trinket_pick` has none.
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub target: Option<ShopPlace>,
    #[serde(default)]
    pub card: Option<ShopPlace>,
    #[serde(default)]
    pub picked: Option<ShopPlace>,
    #[serde(default)]
    pub options: Vec<ShopPlace>,
    #[serde(default)]
    pub source: Option<ShopPlace>,
    /// The cards that came to the board or the hand this step, and the ones
    /// that left it. A card that left a slot and came back to it is the same
    /// card still standing there (a Reborn minion the step destroyed), which
    /// is part of where a played minion went (`play_slot`).
    #[serde(default)]
    pub entered: Vec<ShopPlace>,
    #[serde(default)]
    pub left: Vec<ShopPlace>,
    /// What the step took (net of what it gave back).
    #[serde(default)]
    pub paid: Option<ShopPaid>,
    /// The state the step left: what the next step loads and what this one is
    /// compared with.
    #[serde(default)]
    pub after: Option<ShopStateSpec>,
}

/// Where the card a step acted on went: a Tavern slot to the hand, the hand to
/// a board position, off the board.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct ShopMoved {
    #[serde(default)]
    pub id: u64,
    #[serde(default)]
    pub from: Option<ShopPlace>,
    #[serde(default)]
    pub to: Option<ShopPlace>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
pub struct ShopPaid {
    #[serde(default)]
    pub gold: i32,
    #[serde(default)]
    pub health: i32,
}

/// One shop turn: the state it started from, its steps, and the state it
/// ended in (None when the log ends inside it).
#[derive(Clone, Debug, Default, Deserialize)]
pub struct ShopTurnSpec {
    #[serde(default)]
    pub turn: u32,
    #[serde(default)]
    pub start: ShopStateSpec,
    #[serde(default)]
    pub actions: Vec<ShopStepSpec>,
    #[serde(default)]
    pub end: Option<ShopStateSpec>,
}

/// One game as `boards/actions/` writes it.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct ShopFileSpec {
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub game: u32,
    /// The hero Era played, by card id (a skin); None when the log shows none.
    #[serde(default)]
    pub hero: Option<HeroRef>,
    /// The tribes the log showed this game, in engine spelling (a lower bound
    /// on the lobby's: the lobby itself is not in the log).
    #[serde(default)]
    pub tribes: Vec<String>,
    #[serde(default)]
    pub turns: Vec<ShopTurnSpec>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct HeroRef {
    #[serde(default)]
    pub card_id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

impl ShopFileSpec {
    pub fn file(&self) -> String {
        format!("{}.json", self.source_name())
    }

    fn source_name(&self) -> String {
        format!("{}-g{}", self.source, self.game)
    }

    /// The lobby tribes the file lists, as the engine's bit set. A file that
    /// lists none (one written before the key existed) leaves the pool to the
    /// engine's roll of five.
    pub fn lobby(&self) -> Option<crate::cards::Tribes> {
        let mut lobby = crate::cards::Tribes::NONE;
        for name in &self.tribes {
            if let Some(t) = tribe_by_name(name) {
                lobby = lobby.with(t);
            }
        }
        (!lobby.is_neutral()).then_some(lobby)
    }

    /// The hero Era played, as a hero index, if the card set builds it.
    ///
    /// The log names a hero by the game's own shop id (`TB_BaconShop_HERO_201`,
    /// often a skin: `..._SKIN_G`), which the card set does not use, so the
    /// hero is found through the hero power the state shows instead: a power's
    /// card id is the same in both, and a chooser power's parents lead back to
    /// the power the hero starts with. The card id and the name are tried
    /// first, for a file that names a hero the card set knows.
    pub fn hero(&self, data: &Data, state: &ShopStateSpec) -> Option<u8> {
        if let Some(card_id) = self.hero.as_ref().and_then(|h| h.card_id.as_deref()) {
            let base = card_id.split("_SKIN_").next().unwrap_or(card_id);
            for id in [card_id, base] {
                if let Some(h) = data.heroes.by_card_id(id) {
                    return Some(h);
                }
            }
        }
        if let Some(name) = self.hero.as_ref().and_then(|h| h.name.as_deref())
            && let Some(h) = data.heroes.by_name(name)
        {
            return Some(h);
        }
        for power in &state.hero_powers {
            let Some(card_id) = power.card_id.as_deref() else { continue };
            let Some(i) = data.heroes.power_by_card_id(card_id) else { continue };
            // Walk back to the power the hero starts with (a chooser's picks
            // keep a parent), then find the hero that starts with it.
            let mut top = i;
            for _ in 0..8 {
                match data.heroes.power(top).parent {
                    Some(parent) => top = parent,
                    None => break,
                }
            }
            if let Some(h) = data.heroes.heroes.iter().position(|h| h.power == top) {
                return Some(h as u8);
            }
        }
        None
    }
}

/// The tribe a name stands for, in the spelling the action file uses (the
/// engine's own: `beast`, `mech`, ...).
pub fn tribe_by_name(name: &str) -> Option<Tribe> {
    Tribe::EVERY.iter().copied().find(|t| tribe_name(*t) == name)
}

pub fn tribe_name(t: Tribe) -> &'static str {
    match t {
        Tribe::Beast => "beast",
        Tribe::Mech => "mech",
        Tribe::Dragon => "dragon",
        Tribe::Murloc => "murloc",
        Tribe::Demon => "demon",
        Tribe::Pirate => "pirate",
        Tribe::Elemental => "elemental",
        Tribe::Undead => "undead",
        Tribe::Quilboar => "quilboar",
        Tribe::Aberration => "aberration",
    }
}

/// Parse every action file of a folder: `files` are the (name, text) pairs,
/// one per game. Returns the specs in the folder's order, so a run is
/// repeatable.
pub fn parse_action_files(
    files: &[(String, String)],
) -> Result<Vec<ShopFileSpec>, String> {
    let mut out = Vec::with_capacity(files.len());
    for (name, text) in files {
        let mut spec: ShopFileSpec =
            serde_json::from_str(text).map_err(|e| format!("{name}: {e}"))?;
        seat_gift_landings(&mut spec);
        out.push(spec);
    }
    Ok(out)
}

/// Seat the spell each Magicfin token was taught. "After you buy a Tavern
/// spell, get a 1/1 Murloc and teach it that spell" (Magicfin Sticker,
/// Magicfin Mycologist): the log keeps the taught spell as a dbf id in the
/// token's first number, and only `ref/bg-cards.json` can turn that into the
/// card. The loader cannot ask it (a file names its cards, and the replay may
/// run without ref data), so the caller that has the card id map fills the
/// token's `taught` in first. A number that names no card leaves the token
/// with nothing to cast, as before.
pub fn seat_taught_spells(specs: &mut [ShopFileSpec], ids: &CardIdMap) {
    fn note(state: &mut ShopStateSpec, ids: &CardIdMap) {
        for zone in [&mut state.board, &mut state.hand, &mut state.tavern] {
            for m in zone {
                let Some(first) = m.numbers.first().copied() else { continue };
                m.taught = ids.name_of_dbf(first).map(str::to_string);
            }
        }
    }
    for f in specs {
        for t in &mut f.turns {
            note(&mut t.start, ids);
            for a in &mut t.actions {
                if let Some(state) = a.after.as_mut() {
                    note(state, ids);
                }
            }
            if let Some(end) = t.end.as_mut() {
                note(end, ids);
            }
        }
    }
}

/// When a Dark Gift landed, from the file itself: a countdown gift's turns
/// left are in its own `state`, but Steady Growth's growth is fixed by the
/// turn it landed and no one state says that turn. What does say it is the
/// game's history: the card carries the gift from the turn it landed on, so
/// the first turn the file shows a card with that name, that golden flag and
/// that gift is the turn it landed. The log's own entity ids cannot follow a
/// card across turns (the game gives a minion a new id every turn, so the
/// board's `start` and the step before it never share one), and a name and a
/// gift together are the closest thing to a card's identity the file has: a
/// second copy of one card wearing the same gift, bought later than the
/// first, would be read as having landed with it.
fn seat_gift_landings(spec: &mut ShopFileSpec) {
    let mut landed: HashMap<(String, bool, String), u32> = HashMap::new();
    fn note(
        state: &mut ShopStateSpec,
        turn: u32,
        landed: &mut HashMap<(String, bool, String), u32>,
    ) {
        for zone in [&mut state.board, &mut state.hand, &mut state.tavern] {
            for m in zone {
                for g in &mut m.dark_gifts {
                    let Some(id) = g.card_id.clone() else { continue };
                    let key = (m.name.clone().unwrap_or_default(), m.golden, id);
                    let at = *landed.entry(key).or_insert(turn);
                    g.landed = Some(at);
                }
            }
        }
    }
    for turn in &mut spec.turns {
        let t = turn.turn;
        note(&mut turn.start, t, &mut landed);
        for step in &mut turn.actions {
            if let Some(after) = &mut step.after {
                note(after, t, &mut landed);
            }
        }
        if let Some(end) = &mut turn.end {
            note(end, t, &mut landed);
        }
    }
}

// ---- loading a logged state into a game ----

/// A game holding one logged shop state, with what the loader could not use.
pub struct LoadedShop {
    pub game: Game,
    /// What the engine does not build in this state, as lines for the report.
    /// A step replayed from a state with lines here is incomplete, not
    /// flagged.
    pub missing: Vec<String>,
    /// The game's entity ids of the cards in the Tavern, in engine slot order,
    /// so a step's own entity id (`target.id`) finds its slot. After a step
    /// that changes the Tavern the ids are the ones the file shows, and a
    /// Refresh leaves them unknown (0).
    pub tavern_ids: Vec<u64>,
    pub hand_ids: Vec<u64>,
    pub board_ids: Vec<u64>,
    /// The entity id of the Tavern's spell slot, which the engine keeps apart
    /// from the minion slots (`Player.shop_spell`) so that nothing that picks a
    /// Tavern minion can pick the spell.
    pub spell_id: Option<u64>,
}

/// The Tavern of a logged state, split the way the engine models it: the
/// minion slots in slot order (a spell the game put in a minion slot, Saloon's
/// Finest, stays there) and the spell slot, which is the last spell in slot
/// order.
fn tavern_split(state: &ShopStateSpec) -> (Vec<&ShopMinionSpec>, Option<&ShopMinionSpec>) {
    let mut sorted: Vec<&ShopMinionSpec> = state.tavern.iter().collect();
    sorted.sort_by_key(|m| (m.slot.unwrap_or(0), m.id));
    let spell = sorted
        .iter()
        .rposition(|m| m.kind.as_deref() == Some("spell"))
        .map(|i| sorted.remove(i));
    (sorted, spell)
}

/// The engine's shop index of a slot the file names: the file counts the
/// spell slot in its slot order and the engine keeps it apart
/// (`Player.shop_spell`), so only the minion slots before it count.
fn engine_shop_slot(state: &ShopStateSpec, slot: u8) -> usize {
    let (slots, _) = tavern_split(state);
    slots.iter().filter(|m| m.slot.unwrap_or(0) < slot).count()
}

/// The player index every replay loads into: a sandbox game with one player.
pub const ME: usize = 0;

/// The seat the loaded last opponent sits in: any slot but `ME`, whose others
/// are dead and empty in a sandbox game. Nothing plays them: the seat only
/// holds the warband a power copies.
const THEM: usize = 1;

/// Build the engine's game from a logged state: the board, the hand and the
/// Tavern, Gold, the Tavern tier and its upgrade price, the free Refreshes and
/// the freeze, Health and armor, the player-level numbers, the hero power, the
/// trinkets and the Secret zone. `seed` drives the game's randomness, which is
/// what makes a step deterministic or random. `previous` is the state the
/// logged state follows (the turn before, for a turn's start), which one
/// turn-start trigger needs: a chosen hero power that completes at the start
/// of the turn (Galewing's flightpaths).
pub fn load(
    data: &Arc<Data>,
    spec: &ShopStateSpec,
    hero: Option<u8>,
    lobby: Option<crate::cards::Tribes>,
    turn: u32,
    seed: u64,
    previous: Option<&ShopStateSpec>,
) -> LoadedShop {
    let mut missing = Vec::new();
    // A sandbox game has one player and no opponents, which is all a shop step
    // reads, and its lobby is the file's when the file lists tribes.
    let mut game = match hero {
        Some(h) => Game::new_sandbox_with_hero(
            data.clone(),
            seed,
            lobby,
            spec.free_refreshes,
            h,
        ),
        None => Game::new_sandbox(data.clone(), seed, lobby, spec.free_refreshes),
    };
    game.turn = turn;
    // The hero pick and its powers can leave a choice open (Finley's Discover);
    // the logged state is past all of them, and a step that opens a choice
    // brings its own options in.
    clear_choices(&mut game.pool, &game.data.cards, &mut game.players[ME]);

    let (board_ids, mut board) = build_zone(data, &mut missing, &spec.board, turn);
    let (hand_ids, mut hand) = build_zone(data, &mut missing, &spec.hand, turn);
    // The Tavern keeps the file's slot order, and the spell slot is the
    // engine's own (`shop_spell`), so nothing that picks a Tavern minion can
    // pick the spell: a spell in the minion slots (Saloon's Finest) stays
    // there, as the game has it.
    let (tavern_slots, spell_slot) = tavern_split(spec);
    let spell_id = spell_slot.map(|m| m.id);
    let mut tavern_ids = Vec::new();
    let mut tavern: Vec<Minion> = Vec::new();
    for m in &tavern_slots {
        let (minion, bad) = build_minion(data, m, turn);
        if let Some(name) = bad {
            missing.push(format!("minion not built: {name} (a Stand-in with its stats)"));
        }
        let mut minion = minion;
        minion.frozen = m.frozen;
        tavern_ids.push(m.id);
        tavern.push(minion);
    }
    let spell_card = spell_slot.map(|m| {
        let (minion, bad) = build_minion(data, m, turn);
        if let Some(name) = bad {
            missing.push(format!("card not built: {name} (a Stand-in with its stats)"));
        }
        minion.card
    });
    if spec.hand_hidden > 0 {
        missing.push(format!(
            "{} card(s) in hand the log does not name",
            spec.hand_hidden
        ));
    }

    // The sandbox's own turn 1 dealt a Tavern (and a power may have put a card
    // in the hand) and took those copies from the pool; the logged state
    // replaces all three zones, and those copies go back first so the pool
    // still adds up after every step (`Game::check_invariants`).
    let dealt: Vec<Minion> = game.players[ME]
        .shop
        .drain(..)
        .chain(game.players[ME].hand.drain(..))
        .chain(game.players[ME].board.drain(..))
        .collect();
    give_back_copies(&mut game.pool, &game.data.cards, &dealt);
    for m in board.iter_mut().chain(hand.iter_mut()).chain(tavern.iter_mut()) {
        take_pool_copy(&mut game.pool, &game.data.cards, m);
    }
    {
        let pl = &mut game.players[ME];
        pl.tier = spec.tier.clamp(1, MAX_TIER);
        pl.upgrade_cost = match spec.upgrade_cost {
            // The button's price is the player's own upgrade cost plus whatever
            // a power adds to it (Manastorm's 1 Gold): the file shows the
            // button, so the power's part comes back out.
            Some(c) => (c - power_upgrade_extra(data, &spec.hero_powers)).clamp(0, u8::MAX as i32) as u8,
            None => data.rules.upgrade_cost(pl.tier),
        };
        pl.gold = spec.gold.clamp(0, u8::MAX as i32) as u8;
        pl.health = if spec.health > 0 {
            spec.health
        } else {
            data.rules.starting_health
        };
        pl.armor = spec.armor;
        pl.free_rolls = spec.free_refreshes;
        // The Fodders queued for the coming Refreshes. The engine keeps the
        // queue in the player, and only the player's own Refresh takes from
        // it (`Shop::fill_shop`); the file's numbers are the same queue.
        pl.fodder_refreshes.clear();
        for n in spec.fodder_refreshes.iter().flatten() {
            if pl.fodder_refreshes.try_push(*n).is_err() {
                break;
            }
        }
        // The sandbox's own turn 1 may have marked the player skipped (a
        // Procrastinate hero skips his first turn); the logged state is a turn
        // Era acted in, or the file would have no step to replay.
        pl.skipped = false;
        // "If you lost your last combat" (Tortollan Blue Shell) and its win
        // twin: the file says whether the last fight was won.
        pl.won_last_combat = spec.won_last_combat.unwrap_or(false);
        pl.lost_last_combat = spec.won_last_combat == Some(false);
        pl.hero = hero;
        pl.board.clear();
        for m in &board {
            let _ = pl.board.try_push(*m);
        }
        pl.hand.clear();
        for m in &hand {
            let _ = pl.hand.try_push(*m);
        }
        // The linked pairs a source handed out (Faceless Operative, Kith'ix's
        // Dark Ritual): playing one discards the other.
        {
            let (hand_slots, specs) = (&mut pl.hand[..], &spec.hand[..]);
            seat_links(data, hand_slots, specs);
        }
        pl.shop.clear();
        for m in &tavern {
            let _ = pl.shop.try_push(*m);
        }
        pl.shop_spell = spell_card;
        // A Tavern spell priced in Health: the log marks the card itself
        // (`costs_health`, BACON_COSTS_HEALTH_TO_BUY), which is either the
        // card's own price (Hasty Excavation, which pays Health whenever it is
        // bought) or a charge of the turn that a trinket gives the next spell
        // (Bazaar Sticker's one a turn). The engine keeps the charge as
        // `spell_health_left`, and the state's cards say whether it is still
        // there: nothing marks a card once the charge is spent.
        let charged = |spec: &ShopMinionSpec, card: CardId| {
            let c = data.cards.get(card);
            spec.costs_health && c.is_spell() && !c.pay_health
        };
        let health_price = tavern_slots
            .iter()
            .zip(tavern.iter())
            .any(|(spec, m)| charged(spec, m.card))
            || spell_slot.zip(spell_card).is_some_and(|(spec, card)| charged(spec, card));
        if health_price {
            pl.tp.spell_health_left = 1;
        }
        // A Tavern minion priced in Health (Demonic Tapestry, Pilgrimp
        // Sticker, The Eye of Sargeras): one charge, taken by the next minion
        // bought, which is what `health_price_left` is.
        let health_minion = tavern_slots.iter().zip(tavern.iter()).any(|(spec, m)| {
            let card = data.cards.get(m.card);
            spec.costs_health && !card.is_spell()
        });
        if health_minion {
            pl.tp.health_price_left = 1;
            pl.tp.health_price_tribe = None;
        }
        // The Tavern's price when the log shows one that is not the rule's.
        // The game writes it on the cards: an override (Chillmere Mosaic's
        // Spellcraft deals Battlecry minions that cost (1), docs/rules/
        // trinkets.md) or a reduction (a free buy leaves the whole buy cost
        // off every Tavern minion until it is used: Aranna's Spectral Sight).
        let buy_cost = data.rules.buy_cost;
        let prices: Vec<u8> = tavern_slots
            .iter()
            .zip(tavern.iter())
            .filter(|(_, m)| !data.cards.get(m.card).is_spell())
            .map(|(spec, _)| match spec.cost_override {
                Some(p) => p.clamp(0, u8::MAX as i32) as u8,
                None => buy_cost.saturating_sub(
                    spec.cost_reduce
                        .unwrap_or(0)
                        .clamp(0, u8::MAX as i32) as u8,
                ),
            })
            .collect();
        // Every minion the same price is the Tavern's own price: the whole
        // shop costs (1) until the next Refresh, or the next buy is free.
        // A price that differs card by card is a rule this engine has no
        // state for, and is left to the step's own comparison.
        if let Some(&price) = prices.first()
            && prices.iter().all(|&p| p == price)
        {
            if price == 0 && buy_cost > 0 {
                pl.free_minion_left = pl.free_minion_left.max(1);
            } else if price != buy_cost {
                pl.tp.tavern_price = price;
            }
        }
        // The freeze is one state: the engine keeps it on the player, the game
        // on each card. It is loaded on the player and off the cards, so a
        // Freeze click (which toggles the player's flag) turns the whole
        // Tavern with it, as in the game.
        pl.frozen = spec.frozen || pl.shop.iter().any(|m| m.frozen);
        for m in pl.shop.iter_mut() {
            m.frozen = false;
        }
        // The numbers the file shows, in the shape the board loader uses.
        let mut counters = RealCounters::default();
        for (key, value) in &spec.counters {
            counters.set(key, *value);
        }
        // The Tasty Lobster count is the pair its Deathrattle gives now, read
        // off any Lobster on the board or in the hand (the board loader's
        // rule: every Lobster shares one count, and the best one is kept).
        let lobster = spec
            .board
            .iter()
            .chain(&spec.hand)
            .filter(|m| m.name.as_deref() == Some("Tasty Lobster"))
            .map(|m| lobster_count(&m.numbers, m.golden))
            .max()
            .unwrap_or(0);
        let slot = crate::cards::Counter::TastyLobster.index();
        counters.improve[slot] = counters.improve[slot].max(lobster);
        // Golden minions played this game: a file the collector wrote after
        // 2026-11-03 counts them (`counters`), and an older one shows the
        // count on a card that improves with it (see
        // `golden_played_shown`).
        let shown = golden_played_shown(data, &spec.board, &board_ids, &board)
            .max(golden_played_shown(data, &spec.hand, &hand_ids, &hand))
            .max(golden_played_shown(data, &spec.tavern, &tavern_ids, &tavern));
        pl.take_real_counters_public(&counters);
        // `take_real_counters` leaves it alone: only a shop step reads it.
        pl.golden_played = counters.golden_played.max(shown);
    }
    // The warband of the opponent I last fought (`last_opponent_board`): the
    // engine keeps it as the seen board of the last opponent, which Bob's
    // Burgles and Time Thief copy from, and as the next opponent's warband,
    // which is the best the file can do for I Spy (see the field's own note).
    // A card the pool cannot build is left out: a warband is copied by card,
    // and nothing else reads it.
    if let Some(specs) = spec.last_opponent_board.as_ref() {
        let mut theirs: ArrayVec<Minion, BOARD> = ArrayVec::new();
        for m in specs {
            let (minion, bad) = build_minion(data, m, turn);
            if bad.is_none() {
                let _ = theirs.try_push(minion);
            }
        }
        if !theirs.is_empty() {
            let pl = &mut game.players[ME];
            pl.last_opponent = Some(Opponent::Player(THEM as u8));
            pl.seen[THEM].board = theirs.clone();
            pl.next_board = theirs;
        }
    }

    // The hero power and the trinkets, through the loader the fight check uses.
    // The sandbox's own hero left a power on the player; the file's powers
    // replace it, the main one first and the second one in its own slot.
    game.players[ME].power = None;
    game.players[ME].power2 = None;
    // Which slot each logged power sits in: its own `second` mark when the
    // file has them (a state can list the second power first: Rafaam with
    // Patches' power in the second slot and a trinket's power in the main
    // one, `11_02_00_10_22-g5` turn 11), else the order the list is in.
    let marked = spec.hero_powers.iter().any(|p| p.second);
    for (i, power) in spec.hero_powers.iter().enumerate() {
        if i > 1 {
            // The engine has two power slots; a log with three is a game this
            // engine does not model.
            missing.push(format!(
                "a third hero power: {}",
                power.name.clone().unwrap_or_default()
            ));
            continue;
        }
        let real = RealPowerSpec {
            card_id: completed_power(data, previous, power)
                .unwrap_or_else(|| power.card_id.clone().unwrap_or_default()),
            name: power.name.clone().unwrap_or_default(),
            numbers: power.numbers.clone(),
            waiting: power.waiting.as_ref().map(|w| w.real_spec()),
            waiting_known: power.waiting.is_some(),
        };
        let uses_left = data
            .heroes
            .power_by_card_id(&real.card_id)
            .map(|idx| {
                data.heroes
                    .power(idx)
                    .uses_per_turn
                    .saturating_sub(power.used_this_turn)
            });
        let mode = SeatMode { turn, uses_left, armed: false };
        let main = game.players[ME].power.take();
        let hit = fight_seat_power(
            data,
            &mut game.players[ME],
            &real,
            spec.secrets.as_ref(),
            &mode,
        );
        missing.extend(hit);
        // `seat_power` fills the main slot; a power the file marks as the
        // second one goes to the other slot.
        let seated = game.players[ME].power.take();
        if if marked { power.second } else { i == 1 } {
            game.players[ME].power2 = seated;
            game.players[ME].power = main;
        } else {
            game.players[ME].power = seated;
        }
    }
    // The standing rules the seated power and trinkets set (a power's Tavern
    // shape, its freezes, its discounts) are recomputed the way a turn start
    // does, so a Refresh and a price see them.
    game.sync_seat_rules(ME);
    // A hero always has a power in the game (the engine's invariants say so),
    // and a state the log shows without one gets the hero's own starting
    // power: the power's own numbers are the only thing a step would miss.
    // A state with no power at all is a hero whose passive power has no
    // entity in play ("On Turn 8, choose a Greater Trinket to buy"), and the
    // log keeps that hero out of the state it writes: the player is seated
    // without a hero rather than with a power the state does not show.
    if spec.hero_powers.is_empty() {
        game.players[ME].hero = None;
    } else if let Some(h) = hero
        && game.players[ME].power.is_none()
    {
        let power = data.heroes.get(h).power;
        game.players[ME].hold_power(data, power, 0, turn);
        missing.push(
            "the hero power not in the state: the hero's own starting power is used".to_string(),
        );
    }
    let trinkets: Vec<RealTrinketSpec> = spec
        .trinkets
        .iter()
        .map(|t| RealTrinketSpec {
            card_id: t.card_id.clone().unwrap_or_default(),
            name: t.name.clone().unwrap_or_default(),
            greater: t.greater,
            numbers: t.numbers.clone(),
        })
        .collect();
    let hit = seat_trinkets(data, &mut game.players[ME], &trinkets);
    missing.extend(hit);
    // A trinket's pick never ran, so the rules it leaves standing (Lovely
    // Locket, a portrait's rewrite, Innkeeper's Stein's extra card, the
    // Tavern's shape) come from the pick's own code, before the counters the
    // file gives go on top.
    game.seat_trinket_pick_state(ME);
    seat_trinket_counters(data, &mut game.players[ME], &spec.trinkets);
    seat_power_counters(data, &mut game.players[ME], &spec.hero_powers);
    match &spec.secrets {
        Some(_) => {
            let hit = seat_secrets(&mut game.players[ME], spec.secrets.as_ref());
            missing.extend(hit);
        }
        None => missing.push("Secrets not in the file (the Secret zone)".to_string()),
    }
    // The log's Tavern spell bonus (`spell_attack`/`spell_health`) is the
    // total the game gives every spell, and it already holds what a minion on
    // the board adds (Humon'gozz, Enchanted Sentinel) and what a trinket's
    // standing rule adds (Honeycomb Ring's +1/+1, Multilayered Shield's per
    // type). The engine adds those itself, so the stored value takes the
    // engine's own part back out and the total stays the log's number. The
    // evidence: in `Hearthstone_2026_10_27_17_42_08-g1` turn 8 the counter
    // reads 4/5 with Humon'gozz on the board, the Tavern Dish Banana gives
    // +6/+7 (2/2 plus 4/5, no +1/+2 on top), and selling the Humon'gozz drops
    // the counter by exactly 1/2 (turn 9).
    {
        let cards = &game.data.cards;
        let pl = &mut game.players[ME];
        let (total_a, total_h) = pl.total_spell_bonus(cards);
        let (stored_a, stored_h) = pl.spell_bonus;
        pl.spell_bonus = (
            stored_a - (total_a - stored_a),
            stored_h - (total_h - stored_h),
        );
    }
    // A Tavern spell whose price the log shows on the card itself: the same
    // override and reduction a Tavern minion carries (Chillmere Mosaic's
    // Spellcraft, a free buy). The engine's price for a spell is its own rule's
    // minus the player's next-spell discount, so the log's price goes in as
    // that discount. What makes a spell cheaper without saying so on the card
    // (the buys of a printed 1 to 4 for 0 or 1 in the frozen games) is left to
    // the step's own `paid` (`seat_spell_paid`).
    if let (Some(spec), Some(card)) = (spell_slot, spell_card)
        && !spec.costs_health
        && let Some(price) = logged_spell_price(data, spec, card)
    {
        let pl = &mut game.players[ME];
        if let Some(engine) = pl.spell_cost(&data.cards)
            && price < engine
        {
            pl.spell_discount = pl.spell_discount.saturating_add(engine - price);
        }
    }
    LoadedShop { game, missing, tavern_ids, hand_ids, board_ids, spell_id }
}

/// Whether a step is the one the game's own triple check runs in: the state
/// the step starts from holds three plain copies of a card on the board, and
/// the state it leaves holds a Golden card of that name in the hand the first
/// one did not. The check waits while a Discover is open, so the copies sit
/// on the board for a step or two (log 2026_10_24_15_25_26-g6 turn 8 step 14
/// leaves three Skeletons, and step 16, the second pick of the Maw Caster's
/// Discover, is where the Golden one joins the hand).
fn board_triple_due(before: &ShopStateSpec, after: &ShopStateSpec) -> bool {
    // By name: a Golden card has a card_id of its own (`BG_ICC_026t_G`
    // against `BG_ICC_026t`), and it is the same card to the game.
    let plain_on_board = |s: &ShopStateSpec, name: &str| {
        s.board
            .iter()
            .filter(|m| m.name.as_deref() == Some(name) && !m.golden)
            .count()
    };
    let gold_in_hand = |s: &ShopStateSpec, name: &str| {
        s.hand
            .iter()
            .filter(|m| m.name.as_deref() == Some(name) && m.golden)
            .count()
    };
    before.board.iter().filter_map(|m| m.name.as_deref()).any(|name| {
        plain_on_board(before, name) >= 3
            && gold_in_hand(after, name) > gold_in_hand(before, name)
    })
}

/// The Gold price the log shows on a Tavern spell, when it shows one that is
/// not the card's printed cost: the card's own override (the whole new price)
/// or what a reduction takes off the printed one.
fn logged_spell_price(data: &Data, spec: &ShopMinionSpec, card: CardId) -> Option<u8> {
    let printed = data.cards.get(card).cost;
    match spec.cost_override {
        Some(p) => Some(p.clamp(0, u8::MAX as i32) as u8),
        None => {
            let reduce = spec.cost_reduce.unwrap_or(0).clamp(0, u8::MAX as i32) as u8;
            (reduce > 0).then(|| printed.saturating_sub(reduce))
        }
    }
}

/// A Tavern spell the step bought from a minion slot for less than the engine's
/// price: the same shortfall as `seat_spell_paid`, seated as the turn's
/// "Tavern spells cost less" (`PrizePowers::spell_discount_turn`), which is the
/// state the engine prices such a card from. Nothing is seated for a spell
/// paid in Health, or one the engine already prices at or under what the log
/// took.
fn seat_shop_spell_paid(game: &mut Game, data: &Data, card: CardId, step: &ShopStepSpec) {
    let Some(paid) = step.paid.filter(|p| p.health <= 0 && p.gold >= 0) else {
        return;
    };
    let c = data.cards.get(card);
    if c.pay_health {
        return;
    }
    let pl = &mut game.players[ME];
    if pl.tp.spell_health_left > 0 {
        return;
    }
    let engine = c.cost.saturating_sub(pl.prize.spell_discount_turn);
    let paid = paid.gold.clamp(0, u8::MAX as i32) as u8;
    if paid < engine {
        pl.prize.spell_discount_turn = pl.prize.spell_discount_turn.saturating_add(engine - paid);
    }
}

/// The log's Tavern spell bonus is the whole bonus the next spell gets, and
/// the game keeps what this turn's spells on a minion improved inside it
/// (Honeycomb Ring's "+1/+1 more this turn": `Hearthstone_2026_11_03_15_22_36-g4`
/// turn 10 starts at 2/3 and reaches 38/39 by its end, then drops back next
/// turn). `load` seats that whole number as the player's own bonus, so the
/// total is the log's; this moves the part the turn added into the bonus that
/// is only this turn's, which is what the engine's own improve-for-this-turn
/// writes and what the end of the turn drops. It needs the turn's start state
/// (the counter there is the standing part), which `load` does not take.
///
/// The same difference gives the Gold the turn has spent so far, which
/// "improved by each Gold you spent this turn" reads (Lovesick Balladist): the
/// log's `gold_spent` counter is the game's total, 19 and 18 more than the
/// turn's start by the two plays in `Hearthstone_2026_10_28_02_28_32-g4` turn
/// 11 and `Hearthstone_2026_11_03_00_29_06-g4` turn 11. Every step loads a
/// fresh state, so without it the count is nothing at every step.
pub fn seat_spell_bonus_turn(pl: &mut Player, state: &ShopStateSpec, turn_start: &ShopStateSpec) {
    let added = |key: &str| {
        let now = state.counters.get(key).copied().unwrap_or(0);
        let start = turn_start.counters.get(key).copied().unwrap_or(0);
        (now - start).max(0)
    };
    let (attack, health) = (added("spell_attack"), added("spell_health"));
    pl.spell_bonus.0 -= attack;
    pl.spell_bonus.1 -= health;
    pl.tp.spell_bonus_turn.0 += attack;
    pl.tp.spell_bonus_turn.1 += health;
    pl.gold_spent_turn = added("gold_spent").min(u16::MAX as i32) as u16;
}

/// Gold a card's own trigger gives at the start of the turn lands on the step
/// after it, because the client's tag moves with the turn's first board update
/// and the logged turn-start state is taken before that (Accord-o-Tron's "At
/// the start of your turn, gain 1 Gold": `10_27_04_30_04-g2` turn 6 step 1
/// gains its Gold there, and the file's `gold_gain` names the card). The replay
/// never runs a start of turn, so the step's own `gold_gain` pays it, and only
/// for a card in play whose start of turn is Gold: a played Tavern spell's Gold
/// is the step's own effect, which the engine gives itself.
fn seat_start_of_turn_gold(data: &Data, ids: &CardIdMap, pl: &mut Player, step: &ShopStepSpec) {
    let Some(gain) = step.gold_gain.as_ref().filter(|g| g.gold > 0) else {
        return;
    };
    let Some(card) = gain.by.as_ref().and_then(|b| resolve_card(data, ids, b)) else {
        return;
    };
    let Some(crate::cards::StartOfTurn::GainGold(n)) = data.cards.get(card).start_of_turn else {
        return;
    };
    if !pl.board.iter().any(|m| m.parts().any(|(c, _)| c == card)) {
        return;
    }
    pl.gold = pl
        .gold
        .saturating_add(gain.gold.min(n as i32).max(0) as u8);
}

/// Souvenir Stand becomes a copy of the Greater trinket the player picked. The
/// game writes that a step or two after the pick, not at it: the state the step
/// starts from still names the Stand in the lesser slot and the state it leaves
/// names the copy (`Hearthstone_2026_11_03_00_29_06-g4` turn 9 step 1, whose
/// greater Maldraxxus Dagger was picked at turn 6). The loader does the
/// transformation where the log shows it, which is what `Game::pick_trinket`
/// does at a pick, and seats the copy's own numbers from that state.
fn seat_souvenir_copy(data: &Data, game: &mut Game, before: &ShopStateSpec, after: &ShopStateSpec) {
    let souvenir = |card_id: Option<&str>| {
        card_id
            .and_then(|id| data.trinkets.by_card_id(id))
            .is_some_and(|i| {
                data.trinkets
                    .get(i)
                    .on_pick
                    .iter()
                    .any(|a| matches!(a, crate::trinkets::Act::Souvenir))
            })
    };
    for (slot, (was, now)) in before.trinkets.iter().zip(&after.trinkets).enumerate() {
        if !souvenir(was.card_id.as_deref()) || was.card_id == now.card_id {
            continue;
        }
        let Some(i) = now.card_id.as_deref().and_then(|id| data.trinkets.by_card_id(id)) else {
            continue;
        };
        game.set_trinket(ME, slot, Some(i));
        seat_trinket_counters(data, &mut game.players[ME], &after.trinkets);
    }
}

/// The cards the state's trinkets hand out at the start of the turn: a
/// Spellcraft spell, or a Lockbox. Only a trinket the turn's own start state
/// holds is due: one the player picked this turn has had its start of turn
/// already, and its card comes at the next one.
fn start_of_turn_cards(data: &Data, pl: &Player, start: &ShopStateSpec) -> Vec<CardId> {
    let mut out: Vec<CardId> = Vec::new();
    for st in pl.trinkets.iter() {
        if st.of == 0 {
            continue;
        }
        let trinket = data.trinkets.get(st.of - 1);
        if !start
            .trinkets
            .iter()
            .any(|t| t.card_id.as_deref() == Some(trinket.card_id.as_str()))
        {
            continue;
        }
        for act in trinket.start_of_turn.iter() {
            match *act {
                crate::trinkets::Act::Get { card, .. } => out.push(card),
                crate::trinkets::Act::GetLockbox(_) => out.extend(data.cards.lockbox),
                _ => {}
            }
        }
    }
    out
}

/// A card a trinket hands out at the start of the turn that the hand had no
/// room for: the game queues it and lets it join at the first step that frees
/// a slot (docs/rules/design.md "A full hand", the Lockbox Portrait and the
/// Token of the Old Gods in `Hearthstone_2026_10_28_22_11_50-g6` turn 8 and
/// `Hearthstone_2026_10_29_22_20_04-g3` turn 10). A queued card is not in the
/// logged state and cannot be, so a replay of that step would never hand it
/// over: it is queued again here, from the trinket the state does show.
/// Nothing is queued for a card the hand still holds, or one this turn already
/// shows arriving or being played: that is how a state behind an early cast
/// says the handout was delivered.
///
/// The turn's start state does not always hold the card either: the client
/// hands it out with the turn's first board update, so the logged start state
/// is taken before it and the turn's first step already shows it arriving
/// (`Hearthstone_2026_10_26_00_48_15-g2` turn 9 and
/// `Hearthstone_2026_10_30_19_45_15-g2` turn 9 hand a Floating Candle and a
/// Token of the Old Gods over beside the step's own pick). A queued card joins
/// at the end of the step's action, which is where the log shows it.
/// How many Elementals this turn has played before step `index`: no state in
/// the file carries the count (the counters hold Gold, the Gem value, spell
/// power and the spell counts), and the turn's own earlier steps do, a `play`
/// of a minion of the tribe. Conflagration's "improved by each Elemental you
/// played this turn" reads it (`Hearthstone_2026_11_03_00_29_06-g3` turn 9
/// step 36: one Firelands Fugitive played at step 34, and the log's
/// Conflagration gives +8/+10 a cast, the base +4/+4, the Elemental's +4/+4
/// and the turn's +0/+2 spell bonus).
fn seat_elementals_played_turn(
    data: &Data,
    ids: &CardIdMap,
    pl: &mut Player,
    file: &ShopFileSpec,
    turn: usize,
    index: usize,
) {
    let Some(turn_spec) = file.turns.get(turn) else { return };
    let mut played: u16 = 0;
    for step in turn_spec.actions.iter().take(index) {
        if step.step != "play" {
            continue;
        }
        let Some(card) = step.card.as_ref().and_then(|c| resolve_card(data, ids, c)) else {
            continue;
        };
        if data.cards.get(card).is_spell() {
            continue;
        }
        if data.cards.get(card).tribes.has(Tribe::Elemental) {
            played = played.saturating_add(1);
        }
    }
    pl.elementals_played_turn = played;
}

fn seat_queued_start_of_turn(
    data: &Data,
    pl: &mut Player,
    file: &ShopFileSpec,
    turn: usize,
    index: usize,
) {    let Some(turn_spec) = file.turns.get(turn) else { return };
    let due = start_of_turn_cards(data, pl, &turn_spec.start);
    // Whether a card of the state's is the one the trinket hands out, by the
    // name the loader builds it under (the log writes Floating Candle Set's
    // spell with the trinket's name, and the greater Jailer Sticker's with the
    // lesser one's).
    let named = |place_id: Option<&str>, place_name: Option<&str>, card: CardId| {
        let name = data.cards.get(card).name.as_str();
        match place_id {
            Some(id) => crate::real_boards::resolved_name(id, place_name.unwrap_or_default()) == name,
            None => place_name == Some(name),
        }
    };
    // The step this one replays, which says itself when the card arrives: the
    // collector writes every card that entered the hand and the card it came
    // from, so a start-of-turn card in `entered` is the handout this step.
    let arriving = |card: CardId| {
        turn_spec.actions.get(index).is_some_and(|a| {
            a.entered.iter().any(|e| {
                e.zone() == Some("hand") && named(e.card_id.as_deref(), e.name.as_deref(), card)
            })
        })
    };
    let due: Vec<CardId> = due
        .into_iter()
        .filter(|&card| {
            if pl.hand.iter().any(|m| m.card == card) || pl.hand_queue.iter().any(|m| m.card == card)
            {
                return false;
            }
            let held = |state: &ShopStateSpec| {
                state
                    .hand
                    .iter()
                    .any(|m| named(m.card_id.as_deref(), m.name.as_deref(), card))
            };
            if held(&turn_spec.start) {
                return false;
            }
            let earlier = turn_spec.actions[..index.min(turn_spec.actions.len())]
                .iter()
                .any(|a| {
                    let played = a
                        .card
                        .as_ref()
                        .is_some_and(|c| named(c.card_id.as_deref(), c.name.as_deref(), card));
                    played || a.after.as_ref().is_some_and(held)
                });
            !earlier && (pl.hand.is_full() || arriving(card))
        })
        .collect();
    for card in due {
        let m = pl.make_card(&data.cards, card, false, 0);
        if pl.hand_queue.is_full() {
            break;
        }
        let _ = pl.hand_queue.try_push(m);
    }
}

/// A Tavern spell the step bought for less than the engine's price: the file
/// says what the log charged (`paid.gold`) and nothing about why (a hero
/// power or a rule this engine does not model; docs/briefs/
/// replay-night-triage-report.md's rule question 3), so the shortfall goes in
/// as the player's next-spell discount, which is the engine's own state for
/// "the next Tavern spell costs less". Nothing is seated for a spell paid in
/// Health, or for one the engine already prices at or under what the log took.
fn seat_spell_paid(game: &mut Game, data: &Data, step: &ShopStepSpec) {
    let Some(paid) = step.paid.filter(|p| p.health <= 0 && p.gold >= 0) else {
        return;
    };
    let pl = &mut game.players[ME];
    let Some(card) = pl.shop_spell else { return };
    let c = data.cards.get(card);
    if c.pay_health || pl.tp.spell_health_left > 0 {
        return;
    }
    let Some(price) = pl.spell_cost(&data.cards) else { return };
    let paid = paid.gold.min(u8::MAX as i32) as u8;
    if paid < price {
        pl.spell_discount = pl.spell_discount.saturating_add(price - paid);
    }
}

/// What a logged hero power adds to the tier-up price (Manastorm's prices:
/// 1 Gold for him). The log's upgrade cost is the button, so it goes back out.
fn power_upgrade_extra(data: &Data, powers: &[ShopPowerSpec]) -> i32 {
    for power in powers {
        let Some(card_id) = power.card_id.as_deref() else { continue };
        let Some(i) = data.heroes.power_by_card_id(card_id) else { continue };
        if let crate::heroes::PowerEffect::Prices { upgrade_extra, .. } =
            data.heroes.power(i).effect
        {
            return upgrade_extra as i32;
        }
    }
    0
}

/// A minion of a logged state as the engine builds it, and the name of the
/// card the card set has no card for (a Stand-in takes its place).
fn build_minion(data: &Data, m: &ShopMinionSpec, turn: u32) -> (Minion, Option<String>) {
    let spec = m.real_spec();
    let (mut minion, bad) = RealBoard::build_minion(&spec, turn, &data.cards, &data.gifts);
    // The Blood Gem stats the card itself carries: the log keeps them per
    // minion (a copy takes them along, a triple sums them, Gem Confiscation
    // moves the neighbours' onto its target), and a fresh minion would start
    // at 0 and steal nothing. A file written before the keys existed leaves
    // them 0, which is what the loader always seated.
    minion.gem_attack = m.gem_attack;
    minion.gem_health = m.gem_health;
    // The Magnetizations inside it: the log leaves one enchantment per
    // Magnetic that merged (`BG34_170te` for a Red Volumizer, `BG31_171te`
    // for a Satellite, MAGNETIC 1, attached to the host), and the collector
    // counts them. A fresh minion would start at 0, and Utility Drone's end of
    // turn then gives it nothing. A file written before the key existed leaves
    // it 0, which is what the loader always seated.
    minion.magnetizations = m.magnetizations.min(u16::MAX as u32) as u16;
    // A Drone Duplicator's held extra Magnetization ("the next Magnetization to
    // this minion this turn happens an extra time"), which the step that
    // Magnetizes onto it spends: the log keeps it in a bare tag the collector
    // reads, and a fresh minion holds none.
    minion.extra_magnetize = m.extra_magnetize;
    // Magnetic is a card property the board loader drops on purpose (a fight
    // never reads it), but a shop step does: it is what lets a minion be
    // Magnetized onto the board the game built it from.
    let card = data.cards.get(minion.card);
    if card.keywords.has(Keywords::MAGNETIC) {
        minion.keywords.set(Keywords::MAGNETIC);
    }
    // A Magicfin token casts the spell it was taught (its Battlecry, and
    // Young Murk-Eye re-running that Battlecry at the end of the turn). The
    // log keeps the spell as the token's first number, a dbf id, and
    // `seat_taught_spells` has already turned that into a card name.
    if card.taught_token
        && let Some(name) = m.taught.as_deref()
    {
        minion.taught_spell = data.cards.by_name(name);
    }
    // A Magicfin Mycologist's teachings left this turn ("(1 left!)"), the
    // game's own count: the log's first number, and none at all when it has
    // none left. A fresh card would start the replay with its full use and
    // teach once more than the log did.
    if card.teaches_spell {
        let left = m.numbers.first().copied().unwrap_or(0).clamp(0, u8::MAX as i32);
        minion.uses_left = left as u8;
    }
    // A Malchezaar's Health Refreshes left this turn ("(2 left!)"), the same
    // per-turn use: the log's first number, and no number at all when they
    // are spent. A fresh card would start the replay with both back, which
    // gives the state a Health Refresh the log had already paid Gold for
    // (Hearthstone_2026_11_01_17_59_13-g1 turn 8 step 23).
    if card.health_refreshes > 0 {
        let left = m.numbers.first().copied().unwrap_or(0).clamp(0, u8::MAX as i32);
        minion.uses_left = left as u8;
    }
    // A Thorned Trailblazer's "One Choose One card each turn has both effects
    // combined. (@ left!)" and a Trailblazer Sticker's: the log keeps the uses
    // left in the first number, and shows no number at all once they are spent
    // (`Hearthstone_2026_11_03_15_22_36-g4` turn 8 step 30 has `[1, 0, 0, 0]`
    // and step 31 has none). The board loader reads a missing number as the
    // card's full uses, which gives a spent cover back and lets the replay
    // combine two options the log combined one of (`..._g4` turn 10 step 14).
    if card.choose_one_cover > 0 {
        let left = m.numbers.first().copied().unwrap_or(0).clamp(0, u8::MAX as i32);
        minion.uses_left = left as u8;
    }
    // A Stone Age Slab's "after you buy a minion" effect is once a turn too,
    // and the log keeps the use in the third number (`[20, 20, 1, 0]` while it
    // is there and `[20, 20, 0, 0]` after the buy that spent it,
    // `Hearthstone_2026_11_02_00_10_22-g8` turn 11 step 5 to 9). A card the
    // engine seats as spent would double the next bought minion's stats.
    if card.after_buy.is_some() {
        minion.activated = m.numbers.get(2).copied().unwrap_or(1) <= 0;
    }
    // A card whose trigger counts its events ("After you spend 9 Gold, get a
    // random Bounty. (6 Gold left!)"): the engine keeps how far the count has
    // come in `improvements` (`Minion::events_left`), and the logged card's
    // first number is what is left of it. The card data says which cards
    // count (`on_event`) and how many events the next firing takes, and the
    // cards whose numbers mean something else (Living Azerite's buff pair) or
    // that have no count (Doubloon Grifter, an "@ left!" the log never fills)
    // are left at 0, as they are today.
    if let Some(trigger) = card.on_event
        && trigger.every > 1
    {
        let left = m.numbers.first().copied().unwrap_or(trigger.every as i32);
        let done = (trigger.every as i32 - left).clamp(0, trigger.every as i32 - 1);
        minion.improvements = done as u16;
    }
    // Dark Gifts Steady Growth: "At the end of your turn, gain +X/+Y", and
    // the pair is fixed by the turn the gift landed (darkgifts.toml's table:
    // +2/+2 on turn 3, +2/+3 on turn 4, +3/+4 on turn 5). The board loader
    // seats a gift without a countdown at the board's own turn, which would
    // grow it a step further every turn the replay loads; the turn it landed
    // is the file's (`ShopGiftSpec::landed`).
    for g in &m.dark_gifts {
        let Some(id) = g.card_id.as_deref() else { continue };
        let Some(gift) = data.gifts.gifts.iter().find(|x| x.id == id) else { continue };
        if !matches!(
            data.cards.get(gift.card).end_of_turn,
            Some(crate::cards::EndOfTurn::GiftBuffSelf)
        ) {
            continue;
        }
        if let Some(turn) = g.landed {
            minion.set_gift_state(gift.card, turn.min(crate::minion::GIFT_STATE_MAX as u32) as u8);
        }
    }
    (minion, bad)
}

/// The counters a trinket's own numbers hold: what is left of a per-turn use
/// (Warcry Totem's free Battlecry minions, Transcribing Typewriter's copies,
/// Magicfin Sticker's uses) or how far an "every N" rule has come (Wand of
/// Divination's three spells on minions). Only the cards whose number the log
/// shows with that meaning are here: every other trinket's numbers are
/// something else (a stat, an improvement count, the card Portable Factory
/// remembers) and are left alone. Per-card mapping, as for the Tavern's
/// prices: the same number means a different thing on each trinket.
fn seat_trinket_counters(data: &Data, pl: &mut Player, specs: &[ShopTrinketSpec]) {
    for slot in 0..pl.trinkets.len() {
        let of = pl.trinkets[slot].of;
        if of == 0 {
            continue;
        }
        let card_id = data.trinkets.get(of - 1).card_id.clone();
        let Some(spec) = specs.iter().find(|t| t.card_id.as_deref() == Some(&card_id)) else {
            continue;
        };
        let n = |i: usize| spec.numbers.get(i).copied().unwrap_or(0).clamp(0, u8::MAX as i32) as u8;
        match card_id.as_str() {
            // "The first two Battlecry minions you buy each turn are free."
            "BG36_MagicItem_202" => pl.tp.free_buys = n(0),
            // "Get an extra copy of the next 2 (4, greater) minions you buy."
            "BG35_MagicItem_931" | "BG35_MagicItem_931t" => pl.tp.copy_buys = n(0),
            // "Your first two Refreshes each turn contain only Tier 6 cards."
            // The base is what the next turn starts over from.
            "BG32_MagicItem_366" => {
                pl.tp.refresh_tier6 = n(0);
                pl.tp.refresh_tier6_base = 2;
            }
            // "After you buy a Tavern spell, get a 1/1 Murloc and teach it that
            // spell. (2 times per turn.)"
            "BG35_MagicItem_750" => pl.trinkets[slot].uses_left = n(0),
            // "After you cast 3 spells on minions, gain 1 Gold." The number is
            // what is left of the three.
            "BG36_MagicItem_307" => pl.trinkets[slot].count = (3 - n(0)).min(3) as u16,
            // Snarky Portrait: "give your Beasts +4/+4 and improve this". The
            // log's first number is the buff the next Fishbait death gives: 4
            // at the pick, then 4 more per death (4, 8, 12, and 20 after four
            // of them in Hearthstone_2026_11_03_00_29_06-g5 turn 10).
            "BG36_MagicItem_450" => pl.trinkets[slot].value = n(0),
            // Goldenizer Supply: "every 3 turns, get The Goldenizer". The log's
            // first number is the turns left, and the count the engine keeps
            // is how many turns have come: 3 left is the turn after the pick
            // (1), 1 left is the turn it fires on (3). Hearthstone_2026_10_28
            // _22_11_50-g2 gives one at the end of turns 8, 11 and 14, with
            // the number at 1, 2 and 3 left.
            "BG30_MagicItem_435" => pl.trinkets[slot].count = (3 - n(0) + 1).clamp(0, 3) as u16,
            _ => {}
        }
    }
}

/// The count one hero power keeps, from the number its log entry shows:
/// Tae'thelan's Reliquary Research counts Tavern spells bought toward the free
/// third one, and the log's number is how many more it takes. The count lives
/// on the player (`HeroCounts`), which every card that reads it looks at.
///
/// Buried Treasure's number is the digs left ("(1 left!)" is the fourth dig),
/// which the engine keeps the other way round as `PowerState.progress`.
///
/// So is the price on the button: the engine adds the standing change a power
/// carries (`PowerState.cost_delta`, Lead Explorer's +1, Pirate Parrrrty!'s
/// -1 a Pirate bought, The Galaxy's Lens's -1 a turn waited) to the card's own
/// cost, and the log's price is the two together as the game shows them.
fn seat_power_counters(data: &Data, pl: &mut Player, powers: &[ShopPowerSpec]) {
    for power in powers {
        let Some(card_id) = power.card_id.as_deref() else { continue };
        let Some(i) = data.heroes.power_by_card_id(card_id) else { continue };
        let effect = data.heroes.power(i).effect;
        let left = power.numbers.first().copied().unwrap_or(0).clamp(0, 2);
        if card_id.starts_with("BG28_HERO_800p") {
            // "(3 left!)" is nothing bought yet, "(1 left!)" one bought.
            pl.counts.tavern_spells_bought = (2 - left) as u32;
        }
        // The slot the file's power sits in, by its card id.
        let slot = [false, true]
            .into_iter()
            .find(|&second| {
                pl.power_at(second)
                    .is_some_and(|st| data.heroes.power(st.of).card_id == card_id)
            });
        let Some(second) = slot else { continue };
        let Some(st) = pl.power_at_mut(second) else { continue };
        if let crate::heroes::PowerEffect::DigForGolden { digs } = effect {
            // The log counts the digs left, the engine the digs done.
            let done = power.numbers.first().copied().unwrap_or(0);
            st.progress = (digs as i32 - done).clamp(0, digs as i32) as u32;
        }
        if let crate::heroes::PowerEffect::RandomTribeAfterSells { every, .. } = effect {
            // Gone Fishing counts the sells left ("5" is none since the last
            // Murloc, then 4, 3, 2, 1 and a Murloc at 0, back to 5: the log
            // of 2026-10-28 22:11:50 game 4, turn 12), the engine the sells
            // done, so the two turn around. Every step loads a fresh state,
            // and without this the count would start again at 0 each step and
            // the power could never fire inside one.
            let left = power.numbers.first().copied().unwrap_or(0);
            let left = left.clamp(1, every as i32) as u32;
            st.progress = every as u32 - left;
        }
        if let crate::heroes::PowerEffect::CopyBuyAfterMinions { every } = effect {
            // Glaive Ricochet counts the minions still to buy ("(3 left!)" is
            // none bought yet, "(1 left!)" two), the engine the minions bought
            // this turn, so again the two turn around. Once the copy has fired
            // the log shows three left again with this turn's use spent
            // (`numbers[1]` 0, `Hearthstone_2026_10_27_04_30_04-g3` turn 10
            // step 15), and the engine's count stops at `every` so that the
            // power cannot fire twice in one turn.
            let left = power.numbers.first().copied().unwrap_or(0).clamp(0, every as i32) as u32;
            let spent = power.numbers.get(1).is_some_and(|n| *n == 0);
            st.progress = if spent { every as u32 } else { every as u32 - left };
        }
        if let Some(cost) = power.cost {
            let base = data.heroes.power(i).cost as i32;
            st.cost_delta = (cost - base).clamp(i8::MIN as i32, i8::MAX as i32) as i8;
        }
    }
}

/// The power a chosen one has completed into, when the state is the start of
/// the turn that completes it. A chooser's power (Galewing's flightpaths)
/// counts down one `PowerState.progress` at the start of every turn and
/// completes when the count runs out: the effect fires and the chooser comes
/// back. The turn-start state the log writes is taken before that trigger, so
/// the file still names the chosen power with its last number, and the state
/// before this one (the turn before) says the same number only when the count
/// had already reached it: the next count is the one that completes. The
/// parent's own effect is not run: a Discover it opens is the step the file
/// writes next, and every other one (Gold, a spell) is already in the logged
/// state.
fn completed_power(
    data: &Data,
    previous: Option<&ShopStateSpec>,
    power: &ShopPowerSpec,
) -> Option<String> {
    let card_id = power.card_id.as_deref()?;
    let i = data.heroes.power_by_card_id(card_id)?;
    let chosen = data.heroes.power(i);
    let parent = chosen.parent?;
    if chosen.complete_turns == 0 || power.numbers.first().copied().unwrap_or(0) > 1 {
        return None;
    }
    let before = previous?
        .hero_powers
        .iter()
        .find(|p| p.card_id.as_deref() == Some(card_id))?;
    (before.numbers.first().copied().unwrap_or(0) == 1)
        .then(|| data.heroes.power(parent).card_id.clone())
}

/// The state a turn's start follows: the turn before's `end` state, or the
/// last state it wrote when it has no `end`.
fn state_before_turn(file: &ShopFileSpec, turn_index: usize) -> Option<&ShopStateSpec> {
    let before = file.turns.get(turn_index.checked_sub(1)?)?;
    before
        .end
        .as_ref()
        .or_else(|| before.actions.iter().rev().find_map(|s| s.after.as_ref()))
}

/// "Golden minions you played this game", from what a card in this zone shows
/// now: a card whose "after you Discover" buff is a pair per Golden minion
/// played (Hooktusk, Master Marauder's +1/+1, improved by Golden minions you
/// played this game) keeps the pair it gives now in the card's own script
/// numbers, and that pair is the printed buff times (1 + the count), doubled
/// on a Golden copy. A file the collector wrote now carries the count itself
/// in the counters, which is what the loader uses; this is what an older file
/// still says.
fn golden_played_shown(
    data: &Data,
    zone: &[ShopMinionSpec],
    ids: &[u64],
    built: &[Minion],
) -> u32 {
    let mut best = 0;
    for (k, m) in built.iter().enumerate() {
        let Some(Effect::BuffOthersPerGolden { attack, health, .. }) =
            data.cards.get(m.card).after_discover
        else {
            continue;
        };
        let Some(id) = ids.get(k) else { continue };
        let Some(spec) = zone.iter().find(|s| s.id == *id) else {
            continue;
        };
        let n = |i: usize| spec.numbers.get(i).copied().unwrap_or(0);
        let (per, shown) = if attack != 0 {
            (attack * m.mul(), n(0))
        } else {
            (health * m.mul(), n(1))
        };
        if per > 0 && shown >= per && shown % per == 0 {
            best = best.max((shown / per - 1) as u32);
        }
    }
    best
}

/// The pair of cards one effect hands out: a sold Faceless Operative gives two
/// random Aberrations, a sold Wandering Willbreaker two random Tavern spells
/// and Kith'ix's Dark Ritual two random minions, and playing one discards the
/// other (docs/rules/design.md "Linked cards"). The engine keeps the pair as a
/// group number on the cards (`Minion::linked`), and the one thing a logged
/// state still says about it is the card each came from (`from`), so the loader
/// links the hand again: exactly two hand cards from a source that hands out a
/// pair are that pair.
///
/// A hero power hands its pairs out together and the hand keeps them side by
/// side, so four cards from it are two pairs in position order: Kith'ix's Dark
/// Ritual pressed twice leaves Brann Bronzebeard next to Runic Arcanist and
/// Hoarding Hyena next to Sin'dorei Straight Shot, and the log loses a card of
/// its own pair each time (`Hearthstone_2026_11_03_13_59_12-g2` turn 12). A
/// sold minion's four do not: two Faceless Operative sales gave four
/// Aberrations that the log plays without a discard
/// (`Hearthstone_2026_11_03_00_29_06-g5` turn 11 step 18, the Mindbending
/// Recruiter played and no other card lost), so they are left unlinked. An odd
/// one out (three from a source) is left unlinked too.
fn seat_links(data: &Data, hand: &mut [Minion], specs: &[ShopMinionSpec]) {
    let mut sorted: Vec<&ShopMinionSpec> = specs.iter().collect();
    sorted.sort_by_key(|m| (m.position.or(m.index).unwrap_or(0), m.id));
    // The source's key and whether its pairs come out side by side.
    let mut groups: Vec<(&str, bool, Vec<usize>)> = Vec::new();
    for (slot, spec) in sorted.iter().take(hand.len()).enumerate() {
        let Some(from) = spec.from.as_ref() else { continue };
        let Some(side_by_side) = hands_out_a_pair(data, from) else { continue };
        let key = from.card_id.as_deref().or(from.name.as_deref()).unwrap_or_default();
        match groups.iter_mut().find(|(k, _, _)| *k == key) {
            Some((_, _, slots)) => slots.push(slot),
            None => groups.push((key, side_by_side, vec![slot])),
        }
    }
    let mut next = 1u16;
    for (_, side_by_side, slots) in groups {
        if !side_by_side && slots.len() != 2 {
            continue;
        }
        for pair in slots.chunks(2) {
            if pair.len() != 2 {
                continue;
            }
            for &slot in pair {
                hand[slot].linked = next;
            }
            next += 1;
        }
    }
}

/// Whether a card hands out its cards as a linked pair, and whether two of its
/// pairs come out side by side: a minion whose sale is `linked_minions` or
/// `linked_spells` hands out one pair per sale and says nothing about two
/// sales, while the hero power `GetLinkedMinions` hands each pair out together
/// (`seat_links`).
fn hands_out_a_pair(data: &Data, from: &ShopPlace) -> Option<bool> {
    let name = from.name.as_deref()?;
    let linked_sale = data.cards.by_name(name).is_some_and(|c| {
        matches!(
            data.cards.get(c).on_sell,
            Some(crate::cards::OnSell::Effect(
                crate::cards::Effect::GetLinkedMinions { .. }
                    | crate::cards::Effect::GetLinkedSpells { .. }
            ))
        )
    });
    if linked_sale {
        return Some(false);
    }
    data.heroes
        .powers
        .iter()
        .any(|p| p.name == name && matches!(p.effect, crate::heroes::PowerEffect::GetLinkedMinions(_)))
        .then_some(true)
}

/// One zone (the board or the hand) of a logged state, in position order.
fn build_zone(
    data: &Data,
    missing: &mut Vec<String>,
    zone: &[ShopMinionSpec],
    turn: u32,
) -> (Vec<u64>, Vec<Minion>) {
    let mut ids = Vec::new();
    let mut out = Vec::new();
    let mut sorted: Vec<&ShopMinionSpec> = zone.iter().collect();
    sorted.sort_by_key(|m| (m.position.or(m.index).unwrap_or(0), m.id));
    for m in sorted {
        if out.len() >= BOARD.max(HAND) {
            break;
        }
        let (minion, bad) = build_minion(data, m, turn);
        if let Some(name) = bad {
            missing.push(format!("minion not built: {name} (a Stand-in with its stats)"));
        }
        ids.push(m.id);
        out.push(minion);
    }
    (ids, out)
}

/// The Tasty Lobster count a card's numbers stand for (the board loader's
/// read): the pair its Deathrattle gives now, 2/1 printed and doubled golden.
fn lobster_count(numbers: &[i32], golden: bool) -> u16 {
    let attack = numbers.first().copied().unwrap_or(0);
    let per_play = 2 * if golden { 2 } else { 1 };
    ((attack / per_play - 1).max(0)).min(u16::MAX as i32) as u16
}

/// Give the pool copies these cards hold back (the same rule as the engine's
/// `give_back`): a card that came from the pool returns what it holds, and a
/// token or a spell holds nothing.
fn give_back_copies(pool: &mut [u8], cards: &crate::cards::CardDb, minions: &[Minion]) {
    for m in minions {
        let c = cards.get(m.card);
        if m.pool_copies > 0 && !c.token && !c.is_spell() {
            let slot = c.pool_slot as usize;
            if slot < pool.len() {
                pool[slot] += m.pool_copies;
            }
        }
    }
}

/// Take the pool copies a loaded card holds out of the pool, so a Refresh draws
/// from what the game had left: one for a minion, three for a golden one (a
/// triple took three), and the card holds them so that selling it gives them
/// back. A token, a spell and a card with no pool slot take none.
fn take_pool_copy(pool: &mut [u8], cards: &crate::cards::CardDb, m: &mut Minion) {
    let c = cards.get(m.card);
    if c.token || c.is_spell() {
        return;
    }
    let slot = c.pool_slot as usize;
    if slot >= pool.len() {
        return;
    }
    let want = if m.golden { 3 } else { 1 };
    let take = want.min(pool[slot]);
    pool[slot] -= take;
    m.pool_copies = take;
}

/// Drop every choice an earlier turn (the hero pick) left open, and the
/// offers that came with it. An open Discover held a pool copy per option, so
/// those go back to the pool first (`Game::check_invariants` counts them).
fn clear_choices(pool: &mut [u8], cards: &crate::cards::CardDb, pl: &mut Player) {
    for (i, &card) in pl.discover.iter().enumerate() {
        let c = cards.get(card);
        if c.token || c.is_spell() || !pl.discover_minions.is_empty() {
            continue;
        }
        let n = if pl.dark_offer {
            pl.discover_copies.get(i).copied().unwrap_or(1)
        } else {
            1
        };
        let slot = c.pool_slot as usize;
        if slot < pool.len() {
            pool[slot] += n;
        }
    }
    pl.discover.clear();
    pl.discover_minions.clear();
    pl.discover_gifts.clear();
    pl.discover_copies.clear();
    pl.dark_offer = false;
    pl.power_offer.clear();
    pl.secret_offer.clear();
    pl.trinket_offer.clear();
    pl.trinket_offer_slot = 0;
    pl.pending_target = None;
    pl.pending_shop_target = None;
    pl.pending_discard = None;
    pl.pending_choose_one = None;
    pl.repeats.clear();
    pl.repeat_discover = 0;
    pl.discover_silent = false;
    pl.hero_offer = [u8::MAX; crate::DISCOVER];
}

// ---- a comparable state ----

/// One card of a state, as the comparison sees it: the card's name in the card
/// set (so the engine and the file name it the same way), whether it is
/// golden, its stats, keywords and Dark Gifts, and whether the Tavern has it
/// frozen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapMinion {
    pub name: String,
    pub golden: bool,
    pub attack: i32,
    pub health: i32,
    pub keywords: Vec<String>,
    pub gifts: Vec<String>,
    pub frozen: bool,
    /// Whether the card is a spell: a Tavern spell sits in the Tavern like a
    /// minion, and a random step has to deal the same kind.
    pub spell: bool,
}

impl SnapMinion {
    pub fn of(data: &Data, m: &Minion, frozen: bool) -> SnapMinion {
        let mut keywords: Vec<String> = Keywords::ALL
            .iter()
            // MAGNETIC is a card property, not a keyword a buff hands out: the
            // file loader drops it on purpose (`real_boards::set_keyword`)
            // and the engine sets it from the card.
            .filter(|(bit, _)| *bit != Keywords::MAGNETIC && m.keywords.has(*bit))
            .map(|(_, name)| name.to_string())
            .collect();
        keywords.sort();
        let mut gifts: Vec<String> = (0..m.n_gifts as usize)
            .filter_map(|i| {
                let card = m.gifts[i].card();
                data.gifts
                    .gifts
                    .iter()
                    .find(|g| g.card == card)
                    .map(|g| g.id.clone())
            })
            .collect();
        gifts.sort();
        SnapMinion {
            name: data.cards.get(m.card).name.clone(),
            golden: m.golden,
            attack: m.attack,
            health: m.health,
            keywords,
            gifts,
            frozen,
            spell: data.cards.get(m.card).is_spell(),
        }
    }

    /// Whether the card is a spell (the kind a random step has to keep right:
    /// a Tavern spell and a minion are different shapes).
    pub fn is_spell(&self) -> bool {
        self.spell
    }

    pub fn text(&self) -> String {
        let g = if self.golden { " (golden)" } else { "" };
        let mut out = format!("{}{g} {}/{}", self.name, self.attack, self.health);
        if self.frozen {
            out.push_str(" (frozen)");
        }
        if !self.keywords.is_empty() {
            out.push_str(&format!(" [{}]", self.keywords.join(" ")));
        }
        if !self.gifts.is_empty() {
            out.push_str(&format!(" gifts:{}", self.gifts.join(",")));
        }
        out
    }
}

/// The Fodder queue with its trailing empty Refreshes dropped: the length of
/// the queue is how far ahead the last Fodder is queued, so a `0` at the end
/// says what no entry at all says, and a file that writes four numbers holds
/// the same queue as one that writes the two that matter.
fn fodder_queue(queue: impl Iterator<Item = u8>) -> Vec<u8> {
    let mut out: Vec<u8> = queue.collect();
    while out.last() == Some(&0) {
        out.pop();
    }
    out
}

/// A hero power as the comparison sees it: the card id, the presses left and
/// the number the power carries (its `{0}` in the file's reading).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapPower {
    pub card_id: String,
    pub uses_left: u8,
    pub number: i32,
}

/// Everything a shop step is compared on: the client's numbers, the three
/// zones, the hero powers, the trinkets and the Secret zone.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShopSnapshot {
    pub gold: i32,
    pub tier: u8,
    pub upgrade_cost: Option<u8>,
    pub health: i32,
    pub armor: i32,
    pub free_refreshes: u8,
    /// The Fodders queued for the coming Refreshes (`fodder_queue`), when
    /// the file carries the queue at all.
    pub fodder_refreshes: Option<Vec<u8>>,
    pub frozen: bool,
    pub board: Vec<SnapMinion>,
    pub hand: Vec<SnapMinion>,
    pub tavern: Vec<SnapMinion>,
    pub powers: Vec<SnapPower>,
    pub trinkets: Vec<(String, bool)>,
    pub secrets: Vec<String>,
}

impl ShopSnapshot {
    /// The engine's state after a step.
    pub fn of_game(data: &Data, g: &Game, p: usize) -> ShopSnapshot {
        let pl = &g.players[p];
        let mut powers = Vec::new();
        for st in [pl.power, pl.power2].into_iter().flatten() {
            let power = data.heroes.power(st.of);
            powers.push(SnapPower {
                card_id: power.card_id.clone(),
                uses_left: st.uses_left,
                number: st.progress as i32,
            });
        }
        let mut trinkets = Vec::new();
        for (slot, st) in pl.trinkets.iter().enumerate() {
            if st.of == 0 {
                continue;
            }
            trinkets.push((data.trinkets.get(st.of - 1).card_id.clone(), slot == 1));
        }
        let secrets = pl
            .secrets
            .iter()
            .filter_map(|k| crate::secrets::CARD_IDS.get(k as usize).map(|s| s.to_string()))
            .collect();
        // The Tavern: the minion slots, then the spell slot when the engine
        // dealt one (a loaded Tavern keeps the spell among the slots). The
        // engine freezes the whole Tavern with one flag on the player, and a
        // card on its own only through Twice as Nice; the game freezes each
        // card, so a card is frozen when either says so.
        let mut tavern: Vec<SnapMinion> = pl
            .shop
            .iter()
            .map(|m| SnapMinion::of(data, m, m.frozen || pl.frozen))
            .collect();
        if let Some(spell) = pl.shop_spell {
            let m = data.cards.make(spell, false);
            // The spell slot freezes and unfreezes with the rest of the
            // Tavern, which the engine keeps on the player.
            tavern.push(SnapMinion::of(data, &m, pl.frozen));
        }
        ShopSnapshot {
            gold: pl.gold as i32,
            tier: pl.tier,
            // The button's price, which is what the file shows: the player's
            // own upgrade cost plus whatever a power adds (Manastorm's 1).
            upgrade_cost: (pl.tier < MAX_TIER).then_some(g.upgrade_price(p)),
            health: pl.health,
            armor: pl.armor,
            free_refreshes: pl.free_rolls,
            fodder_refreshes: Some(fodder_queue(pl.fodder_refreshes.iter().copied())),
            frozen: pl.frozen,
            board: pl.board.iter().map(|m| SnapMinion::of(data, m, false)).collect(),
            hand: pl.hand.iter().map(|m| SnapMinion::of(data, m, false)).collect(),
            tavern,
            powers,
            trinkets,
            secrets,
        }
    }

    /// The state the file holds, with every card built the way the loader
    /// builds it, so the two sides name cards the same way.
    pub fn of_state(
        data: &Data,
        state: &ShopStateSpec,
        turn: u32,
        missing: &mut Vec<String>,
    ) -> ShopSnapshot {
        // The Tavern is compared the way it is loaded: the minion slots in
        // slot order, then the spell slot (which the engine keeps apart).
        let (slots, spell) = tavern_split(state);
        let mut tavern = snap_zone(data, &slots, ZoneOrder::Tavern, turn, missing);
        if let Some(spell) = spell {
            tavern.push(snap_minion(data, spell, turn, missing));
        }
        let mut powers = Vec::new();
        // The two slots, in slot order: the file's `second` mark says which
        // power is the second one, and the list can name it first (the game's
        // own order is not the slot order). A file without the mark is in
        // slot order already.
        let mut listed: Vec<&ShopPowerSpec> = state.hero_powers.iter().collect();
        if listed.iter().any(|p| p.second) {
            listed.sort_by_key(|p| p.second);
        }
        for power in listed {
            let card_id = power.card_id.clone().unwrap_or_default();
            let uses_left = data
                .heroes
                .power_by_card_id(&card_id)
                .map(|i| {
                    data.heroes
                        .power(i)
                        .uses_per_turn
                        .saturating_sub(power.used_this_turn)
                })
                .unwrap_or(0);
            if data.heroes.power_by_card_id(&card_id).is_none() {
                missing.push(format!(
                    "hero power not built: {} ({card_id})",
                    power.name.clone().unwrap_or_default()
                ));
            }
            powers.push(SnapPower {
                card_id,
                uses_left,
                number: power.numbers.first().copied().unwrap_or(0),
            });
        }
        let mut trinkets = Vec::new();
        for t in &state.trinkets {
            let card_id = t.card_id.clone().unwrap_or_default();
            if data.trinkets.by_card_id(&card_id).is_none() {
                missing.push(format!(
                    "trinket not built: {} ({card_id})",
                    t.name.clone().unwrap_or_default()
                ));
            }
            trinkets.push((card_id, t.greater));
        }
        ShopSnapshot {
            gold: state.gold,
            tier: state.tier,
            upgrade_cost: state
                .upgrade_cost
                .map(|c| c.clamp(0, u8::MAX as i32) as u8),
            health: state.health,
            armor: state.armor,
            free_refreshes: state.free_refreshes,
            fodder_refreshes: state
                .fodder_refreshes
                .as_ref()
                .map(|queue| fodder_queue(queue.iter().copied())),
            frozen: state.frozen,
            board: zone_snapshot(data, &state.board, ZoneOrder::Board, turn, missing),
            hand: zone_snapshot(data, &state.hand, ZoneOrder::Hand, turn, missing),
            tavern,
            powers,
            trinkets,
            secrets: state.secrets.clone().unwrap_or_default(),
        }
    }
}

/// Which place field a zone's cards are ordered by: the board by position, the
/// hand by index, the Tavern by slot.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ZoneOrder {
    Board,
    Hand,
    Tavern,
}

/// One zone of a logged state as the comparison sees it, in place order.
fn zone_snapshot(
    data: &Data,
    zone: &[ShopMinionSpec],
    by: ZoneOrder,
    turn: u32,
    missing: &mut Vec<String>,
) -> Vec<SnapMinion> {
    let at = |m: &ShopMinionSpec| match by {
        ZoneOrder::Board => m.position,
        ZoneOrder::Hand => m.index,
        ZoneOrder::Tavern => m.slot,
    };
    let mut sorted: Vec<&ShopMinionSpec> = zone.iter().collect();
    sorted.sort_by_key(|m| (at(m).unwrap_or(0), m.id));
    snap_zone(data, &sorted, by, turn, missing)
}

/// `zone_snapshot` over cards that are already in place order.
fn snap_zone(
    data: &Data,
    sorted: &[&ShopMinionSpec],
    by: ZoneOrder,
    turn: u32,
    missing: &mut Vec<String>,
) -> Vec<SnapMinion> {
    let at = |m: &ShopMinionSpec| match by {
        ZoneOrder::Board => m.position,
        ZoneOrder::Hand => m.index,
        ZoneOrder::Tavern => m.slot,
    };
    let mut sorted = sorted.to_vec();
    sorted.sort_by_key(|m| (at(m).unwrap_or(0), m.id));
    sorted.iter().map(|m| snap_minion(data, m, turn, missing)).collect()
}

/// One logged card as the comparison sees it.
fn snap_minion(
    data: &Data,
    m: &ShopMinionSpec,
    turn: u32,
    missing: &mut Vec<String>,
) -> SnapMinion {
    let (minion, bad) = build_minion(data, m, turn);
    if let Some(name) = bad {
        missing.push(format!("minion not built: {name} (a Stand-in with its stats)"));
    }
    SnapMinion::of(data, &minion, m.frozen)
}

/// The zones a state holds, by the names the report uses.
const ZONES: [(&str, usize); 3] = [("board", 0), ("hand", 1), ("Tavern", 2)];

fn zone_of(s: &ShopSnapshot, i: usize) -> &Vec<SnapMinion> {
    match i {
        0 => &s.board,
        1 => &s.hand,
        _ => &s.tavern,
    }
}

/// Everything that differs between what the engine produced and what the log
/// holds, most useful first: the client's numbers, then the cards. Each
/// difference carries the cause the report groups by.
pub fn compare(ours: &ShopSnapshot, log: &ShopSnapshot) -> Vec<String> {
    compare_pressed(ours, log, None)
}

/// `compare`, leaving out the presses left of the power a step pressed: the
/// game does not count a press that opens a choice (Embrace the Elements and
/// the other chooser powers), so the log's own count stays where it was while
/// the engine spends the press.
pub fn compare_pressed(
    ours: &ShopSnapshot,
    log: &ShopSnapshot,
    pressed: Option<usize>,
) -> Vec<String> {
    let mut out = Vec::new();
    if ours.gold != log.gold {
        out.push(format!("Gold {} vs {}", ours.gold, log.gold));
    }
    if ours.health != log.health {
        out.push(format!("Health {} vs {}", ours.health, log.health));
    }
    if ours.armor != log.armor {
        out.push(format!("armor {} vs {}", ours.armor, log.armor));
    }
    if ours.tier != log.tier {
        out.push(format!("tier {} vs {}", ours.tier, log.tier));
    }
    if ours.upgrade_cost != log.upgrade_cost {
        out.push(format!(
            "upgrade cost {} vs {}",
            opt_text(ours.upgrade_cost),
            opt_text(log.upgrade_cost)
        ));
    }
    if ours.free_refreshes != log.free_refreshes {
        out.push(format!(
            "free Refreshes {} vs {}",
            ours.free_refreshes, log.free_refreshes
        ));
    }
    // The Fodder queue, which only a file the collector wrote for it holds:
    // an older file says nothing, so a step it was replayed from is not
    // judged on it.
    if let Some(log_queue) = &log.fodder_refreshes
        && ours.fodder_refreshes.as_ref() != Some(log_queue)
    {
        out.push(format!(
            "Fodders queued {:?} vs {:?}",
            ours.fodder_refreshes.as_ref().map_or(&[][..], |q| q.as_slice()),
            log_queue
        ));
    }
    for (zi, (zone, _)) in ZONES.iter().enumerate() {
        let a = zone_of(ours, zi);
        let b = zone_of(log, zi);
        // The Tavern and the hand are compared without their order. A Tavern
        // card's slot is the game's own display order, which the file's before
        // state does not always agree with: the turn-start state can hold one
        // order and the step's own Tavern another (a buy at
        // `Hearthstone_2026_10_28_19_47_59-g1` turn 1 puts the card the step
        // names at slot 2 while the turn's start state shows it at slot 1),
        // and the engine deals a Refresh in its own order. The hand's index is
        // the player's own arrangement. The board keeps its order: a position
        // is where a minion stands, and a summon that lands in the wrong place
        // is a difference.
        let (a, b) = if *zone == "board" {
            (a.iter().collect::<Vec<_>>(), b.iter().collect::<Vec<_>>())
        } else {
            fn by_name(z: &[SnapMinion]) -> Vec<&SnapMinion> {
                let mut v: Vec<&SnapMinion> = z.iter().collect();
                v.sort_by(|x, y| x.name.cmp(&y.name));
                v
            }
            (by_name(a), by_name(b))
        };
        // A card that is not there on one side and is on the other, or a card
        // whose stats, keywords, gifts or golden flag differ (the per-card
        // frozen flag is not compared: see `card_diff`).
        let n = a.len().max(b.len());
        for i in 0..n {
            match (a.get(i), b.get(i)) {
                (Some(x), Some(y)) => {
                    let Some(what) = card_diff(x, y) else { continue };
                    out.push(format!(
                        "{zone} {what} {}: {} vs {}",
                        i + 1,
                        x.text(),
                        y.text()
                    ));
                }
                (Some(x), None) => out.push(format!(
                    "{zone} count: {} at {} vs nothing",
                    x.text(),
                    i + 1
                )),
                (None, Some(y)) => out.push(format!(
                    "{zone} count: nothing at {} vs {}",
                    i + 1,
                    y.text()
                )),
                (None, None) => {}
            }
        }
    }
    let n = ours.powers.len().max(log.powers.len());
    for i in 0..n {
        // The number a power carries is not compared: the engine keeps its own
        // meaning for it per power (`PowerState.progress`), and the file's
        // numbers are the {0} {1} {2} {3} of the text, which is only the same
        // number for the few powers the engine models that way.
        // The presses left are the client's own counter, and it moves at
        // whichever of a press's clicks the client liked: a chooser's press is
        // not counted at all, a press whose pick lands in the Tavern is counted
        // at the pick. The power a press changed is caught by the state the
        // press left, so only the power itself is compared.
        let _ = pressed;
        let same = |x: &SnapPower, y: &SnapPower| x.card_id == y.card_id;
        match (ours.powers.get(i), log.powers.get(i)) {
            (Some(x), Some(y)) if same(x, y) => {}
            (Some(x), Some(y)) => out.push(format!(
                "hero power {}: {} ({} left) vs {} ({} left)",
                i + 1,
                x.card_id,
                x.uses_left,
                y.card_id,
                y.uses_left
            )),
            (Some(x), None) => out.push(format!("hero power {}: {} vs nothing", i + 1, x.card_id)),
            (None, Some(y)) => out.push(format!("hero power {}: nothing vs {}", i + 1, y.card_id)),
            (None, None) => {}
        }
    }
    if ours.trinkets != log.trinkets {
        out.push(format!(
            "trinkets {} vs {}",
            trinket_text(&ours.trinkets),
            trinket_text(&log.trinkets)
        ));
    }
    if ours.secrets != log.secrets {
        out.push(format!(
            "Secrets {:?} vs {:?}",
            ours.secrets, log.secrets
        ));
    }
    out
}

/// What differs between the two cards of one place, or `None` when they are the
/// same card. The per-card frozen flag is not compared, in either direction,
/// because the game's own flag is the log quirk design.md settles ("Moving a
/// minion on your board doesn't touch the freeze: the Tavern stays frozen",
/// Era 2026-11-04): the engine keeps one flag on the player and the game keeps
/// one on each card, and the log's cards read unfrozen after a move (triage
/// round 2's group Q1, `Hearthstone_2026_10_24_17_59_26-g1` turn 15 step 36 to
/// 41 and `Hearthstone_2026_11_02_00_10_22-g8` turn 15), after a Freeze press
/// the log's own state does not show (`..._10_28_02_28_32-g4` turn 8 step 27),
/// and on the Tavern's spell slot, which the game never marks (`..._10_30_19_45_15-g2`
/// turn 3 step 6). Nothing else in a step depends on the flag: every step loads
/// its own state, and a random step's shape does not hold the flag either, so a
/// freeze the step should not have made is not caught by the comparison.
fn card_diff(x: &SnapMinion, y: &SnapMinion) -> Option<&'static str> {
    if x.name != y.name {
        Some("a card in the wrong place")
    } else if x.golden != y.golden {
        Some("golden")
    } else if (x.attack, x.health) != (y.attack, y.health) {
        Some("stats")
    } else if x.keywords != y.keywords {
        Some("keywords")
    } else if x.gifts != y.gifts {
        Some("gifts")
    } else if x.spell != y.spell {
        Some("a spell")
    } else {
        None
    }
}

fn opt_text(v: Option<u8>) -> String {
    v.map_or_else(|| "none".to_string(), |v| v.to_string())
}

fn trinket_text(t: &[(String, bool)]) -> String {
    if t.is_empty() {
        return "none".to_string();
    }
    t.iter()
        .map(|(id, greater)| format!("{id}{}", if *greater { " (greater)" } else { "" }))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The shape of a state, which a random step is compared on: the client's
/// numbers, how many cards each zone holds, which of them are spells, and what
/// the step changed on the cards the zones held. A random step deals different
/// cards every time (a Refresh, a random minion from the pool) and hands its
/// buffs out to a card it picks, so the cards themselves cannot be compared;
/// what they did to the state can.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShopShape {
    pub gold: i32,
    pub health: i32,
    pub armor: i32,
    pub tier: u8,
    pub upgrade_cost: Option<u8>,
    pub free_refreshes: u8,
    /// The Fodders queued for the coming Refreshes: a Refresh that was given
    /// them takes the first number, so the queue's own shape says whether the
    /// Fodders reached the Tavern. None when the file does not carry it.
    pub fodder_refreshes: Option<Vec<u8>>,
    /// How many cards each zone holds, and how many of them are spells (the
    /// kind of a card a random step brings in is part of its shape).
    pub board: (usize, usize),
    pub hand: (usize, usize),
    pub tavern: (usize, usize),
    /// Every change the step made to a card of a zone, as a multiset (see
    /// [`ShopChange`]): a buff that landed, a keyword it gave or took, a card
    /// made golden. Which card of the zone got the change is not part of it,
    /// so the same set of changes on other cards has the same shape.
    pub changes: Vec<ShopChange>,
}

impl ShopShape {
    /// The shape of what a step produced (`after`), against the state it
    /// started from (`before`).
    pub fn of(after: &ShopSnapshot, before: &ShopSnapshot) -> ShopShape {
        let spells = |zone: &Vec<SnapMinion>| zone.iter().filter(|m| m.is_spell()).count();
        ShopShape {
            gold: after.gold,
            health: after.health,
            armor: after.armor,
            tier: after.tier,
            upgrade_cost: after.upgrade_cost,
            free_refreshes: after.free_refreshes,
            fodder_refreshes: after.fodder_refreshes.clone(),
            board: (after.board.len(), spells(&after.board)),
            hand: (after.hand.len(), spells(&after.hand)),
            tavern: (after.tavern.len(), spells(&after.tavern)),
            changes: zone_changes(after, before),
        }
    }

    /// How the two shapes differ, in the words the report groups by.
    pub fn diffs(&self, other: &ShopShape) -> Vec<String> {
        let mut out = Vec::new();
        if self.gold != other.gold {
            out.push(format!("Gold {} vs {} (shape)", self.gold, other.gold));
        }
        if self.health != other.health {
            out.push(format!("Health {} vs {} (shape)", self.health, other.health));
        }
        if self.armor != other.armor {
            out.push(format!("armor {} vs {} (shape)", self.armor, other.armor));
        }
        if self.tier != other.tier {
            out.push(format!("tier {} vs {} (shape)", self.tier, other.tier));
        }
        if self.upgrade_cost != other.upgrade_cost {
            out.push(format!(
                "upgrade cost {} vs {} (shape)",
                opt_text(self.upgrade_cost),
                opt_text(other.upgrade_cost)
            ));
        }
        if self.free_refreshes != other.free_refreshes {
            out.push(format!(
                "free Refreshes {} vs {} (shape)",
                self.free_refreshes, other.free_refreshes
            ));
        }
        if other.fodder_refreshes.is_some() && self.fodder_refreshes != other.fodder_refreshes {
            out.push(format!(
                "Fodders queued {:?} vs {:?} (shape)",
                self.fodder_refreshes, other.fodder_refreshes
            ));
        }
        for (name, a, b) in [
            ("board", self.board, other.board),
            ("hand", self.hand, other.hand),
            ("Tavern", self.tavern, other.tavern),
        ] {
            if a != b {
                out.push(format!(
                    "{name} count {} cards ({} spells) vs {} ({} spells) (shape)",
                    a.0, a.1, b.0, b.1
                ));
            }
        }
        out.extend(change_diffs(&self.changes, &other.changes));
        out
    }
}

/// One change a step made to a card that was in a zone before it and is still
/// there: the stats it gained or lost, the keywords it gained or lost, and
/// whether it is golden. A random step hands its buffs to a card it picks, so
/// which card of the zone got a change is the only thing the comparison is
/// allowed to differ on: the change itself (its zone, golden mark, stats and
/// keywords) is compared as a multiset, the card it landed on is only there
/// for the report, and a change with a different size, or one the other side
/// does not have, is a difference.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ShopChange {
    /// `board`, `hand` or `Tavern`.
    pub zone: &'static str,
    /// The card the change landed on, for the report.
    pub name: String,
    /// Whether that card is golden.
    pub golden: bool,
    pub attack: i32,
    pub health: i32,
    /// The keywords the card gained and lost, sorted.
    pub gained: Vec<String>,
    pub lost: Vec<String>,
}

/// What a change is, without the card it landed on: the two sides of a random
/// step may give the same change to different cards.
type ChangeKey<'a> = (&'static str, bool, i32, i32, &'a [String], &'a [String]);

fn change_key(c: &ShopChange) -> ChangeKey<'_> {
    (c.zone, c.golden, c.attack, c.health, &c.gained, &c.lost)
}

/// How two sets of changes differ, in both directions. A random step can give
/// its stats to a random card too (a random friendly minion, a random Tavern
/// minion), so which card got them is not compared: the changes are compared
/// as a multiset, and a change one side has and the other does not (or has at
/// another size, or on a card of another kind) is a difference.
fn change_diffs(a: &[ShopChange], b: &[ShopChange]) -> Vec<String> {
    let mut left: Vec<&ShopChange> = a.iter().collect();
    let mut right: Vec<&ShopChange> = b.iter().collect();
    left.sort_by_key(|c| change_key(c));
    right.sort_by_key(|c| change_key(c));
    let mut out = Vec::new();
    let mut rest: Vec<Option<&ShopChange>> = right.iter().map(|c| Some(*c)).collect();
    for c in &left {
        let found = rest
            .iter()
            .position(|o| o.is_some_and(|o| change_key(o) == change_key(c)));
        match found {
            Some(i) => rest[i] = None,
            None => out.push(format!(
                "a change {} on {} in the {} vs no such change (shape)",
                describe_change(c),
                c.name,
                c.zone
            )),
        }
    }
    for c in rest.into_iter().flatten() {
        out.push(format!(
            "no change {} vs one on {} in the {} (shape)",
            describe_change(c),
            c.name,
            c.zone
        ));
    }
    out
}

/// A change's numbers, as the report prints them: `+2/+2`, plus the keywords
/// it gave or took and the golden mark when it has them.
fn describe_change(c: &ShopChange) -> String {
    let mut out = format!("{:+}/{:+}", c.attack, c.health);
    if !c.gained.is_empty() {
        out.push_str(&format!(" gained [{}]", c.gained.join(" ")));
    }
    if !c.lost.is_empty() {
        out.push_str(&format!(" lost [{}]", c.lost.join(" ")));
    }
    if c.golden {
        out.push_str(" (golden)");
    }
    out
}

/// Every change a step made to a card of a zone: the three zones' own changes,
/// sorted so two runs of the same engine compare equal.
fn zone_changes(after: &ShopSnapshot, before: &ShopSnapshot) -> Vec<ShopChange> {
    let mut out = Vec::new();
    out.extend(name_changes("board", &after.board, &before.board));
    out.extend(name_changes("hand", &after.hand, &before.hand));
    out.extend(name_changes("Tavern", &after.tavern, &before.tavern));
    out.sort();
    out
}

/// The changes one zone's cards went through. A card is paired with the copy
/// of its own name on the other side, so a minion that moved, or that another
/// minion was sold from under, is still the same card (pairing by position
/// drops that pairing and reports a change the step did not make). When one
/// name has several copies on a side the pairings are by stats, keywords and
/// golden mark: the smallest change wins, so two copies that swapped their
/// stats are two no-changes and not one buff and one loss.
fn name_changes(zone: &'static str, now: &[SnapMinion], was: &[SnapMinion]) -> Vec<ShopChange> {
    let mut names: Vec<&str> = Vec::new();
    for m in was.iter().chain(now.iter()) {
        if !names.contains(&m.name.as_str()) {
            names.push(m.name.as_str());
        }
    }
    let key = |m: &&SnapMinion| (m.attack, m.health, m.golden, m.keywords.clone());
    let mut out = Vec::new();
    for name in names {
        let mut old: Vec<&SnapMinion> = was.iter().filter(|m| m.name == name).collect();
        let mut new: Vec<&SnapMinion> = now.iter().filter(|m| m.name == name).collect();
        old.sort_by_key(key);
        new.sort_by_key(key);
        for (a, b) in new.iter().zip(old.iter()) {
            let gained: Vec<String> = a
                .keywords
                .iter()
                .filter(|k| !b.keywords.contains(k))
                .cloned()
                .collect();
            let lost: Vec<String> = b
                .keywords
                .iter()
                .filter(|k| !a.keywords.contains(k))
                .cloned()
                .collect();
            let (da, dh) = (a.attack - b.attack, a.health - b.health);
            if da == 0 && dh == 0 && gained.is_empty() && lost.is_empty() && a.golden == b.golden {
                continue;
            }
            out.push(ShopChange {
                zone,
                name: name.to_string(),
                golden: a.golden,
                attack: da,
                health: dh,
                gained,
                lost,
            });
        }
    }
    out
}

/// How a step's result is judged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The step is deterministic and the engine's state is the log's.
    Ok,
    /// The step is random and one of the engine's outcomes is the log's.
    OkRandom,
    /// The step is random and the log's result has the engine's shape.
    OkShape,
    /// The engine's result differs from the log's.
    Flagged,
    /// Something in the state or the step is not built, or the log does not
    /// say enough.
    Incomplete,
    /// The engine does not allow the step.
    Illegal,
}

impl Verdict {
    pub fn name(self) -> &'static str {
        match self {
            Verdict::Ok => "ok",
            Verdict::OkRandom => "ok-random",
            Verdict::OkShape => "ok-shape",
            Verdict::Flagged => "flagged",
            Verdict::Incomplete => "incomplete",
            Verdict::Illegal => "illegal",
        }
    }
}

// ---- replaying one step ----

/// How many seeds a step is run with, and where the seeds start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReplayOptions {
    /// Seeds per step. Every seed that gives the same result makes the step a
    /// deterministic one; the first that differs makes it random.
    pub seeds: u32,
    pub seed: u64,
}

impl Default for ReplayOptions {
    fn default() -> ReplayOptions {
        ReplayOptions { seeds: 64, seed: 1 }
    }
}

/// One replayed step: what it was, what the engine did and how it compares.
#[derive(Clone, Debug)]
pub struct StepResult {
    pub game: String,
    pub turn: u32,
    /// The step's place in its turn, 0 first.
    pub index: usize,
    pub kind: String,
    /// The card the step acted on, as a card id (or the hero power's, or the
    /// trinket's), for the per-card tally, and the name the file gave it.
    pub card: Option<String>,
    pub card_name: Option<String>,
    /// More cards the step involves: a Discover's offer it picked, and every
    /// card whose stats the file says the step changed.
    pub cards: Vec<String>,
    pub verdict: Verdict,
    /// What the state or the step did not give (`incomplete` only).
    pub missing: Vec<String>,
    /// Every difference between the engine's result and the log's.
    pub diffs: Vec<String>,
    /// Whether every seed gave the same result.
    pub deterministic: bool,
    /// The engine's action, as text, for the report.
    pub action: String,
    /// The state before the step, the log's after, the engine's after: the
    /// report writes them out for a flagged step.
    pub before: ShopSnapshot,
    pub log_after: ShopSnapshot,
    pub ours: ShopSnapshot,
}

impl StepResult {
    /// The first difference, or what made the step incomplete.
    pub fn first(&self) -> String {
        if let Some(d) = self.diffs.first() {
            return d.clone();
        }
        self.missing.first().cloned().unwrap_or_default()
    }
}

/// The state a step starts from: the turn's `start`, else the state the step
/// before it left.
fn state_before(turn: &ShopTurnSpec, i: usize) -> Option<&ShopStateSpec> {
    if i == 0 {
        Some(&turn.start)
    } else {
        turn.actions.get(i - 1).and_then(|s| s.after.as_ref())
    }
}

/// The Tavern slot a card with this entity id sits in, by the ids the file
/// showed; the file's own `slot` when the ids are not known (a Refresh).
fn shop_index(loaded: &LoadedShop, id: u64, slot: Option<u8>) -> Option<u8> {
    if id != 0
        && let Some(i) = loaded.tavern_ids.iter().position(|&x| x == id)
    {
        return Some(i as u8);
    }
    slot.map(|s| s.saturating_sub(1))
}

/// The board (or hand) index of a card with this entity id, by the ids the
/// file showed, else by the position the file gives.
fn zone_index(ids: &[u64], id: u64, at: Option<u8>) -> Option<u8> {
    if id != 0
        && let Some(i) = ids.iter().position(|&x| x == id)
    {
        return Some(i as u8);
    }
    at.map(|p| p.saturating_sub(1))
}

/// The engine's action for a step's target, or an error when the target is
/// something the engine cannot point at (a card in the hand, a hero).
fn target_action(loaded: &LoadedShop, target: &ShopPlace) -> Result<Action, String> {
    match target.kind.as_deref() {
        Some("board") => zone_index(&loaded.board_ids, target.id, target.position)
            .map(Action::Target)
            .ok_or_else(|| "a board target that is not on the board".to_string()),
        // The action names a Tavern slot out of `SHOP` of them, the spell slot
        // included, so a full Tavern's spell slot (a card the game cannot show
        // anyway) is not one the replay can point at.
        Some("slot") => shop_index(loaded, target.id, target.slot)
            .filter(|s| (*s as usize) < crate::SHOP)
            .map(Action::TargetShop)
            .ok_or_else(|| "a Tavern target that is not in the Tavern".to_string()),
        other => Err(format!(
            "a target the engine cannot point at: {}",
            other.unwrap_or("unknown")
        )),
    }
}

/// The card in the hand a step throws away: an Activate or a power that
/// discards a card first opens the engine's discard prompt, and the step's
/// target names the card the log threw.
fn discard_action(loaded: &LoadedShop, target: &ShopPlace) -> Result<Action, String> {
    zone_index(&loaded.hand_ids, target.id, target.index)
        .map(Action::Discard)
        .ok_or_else(|| "a hand target that is not in the hand".to_string())
}

/// The card a place names, resolved against the card set through the card id
/// file: the card id the game used, its name in the card set, and the index.
fn resolve_card(data: &Data, ids: &CardIdMap, place: &ShopPlace) -> Option<CardId> {
    let card_id = place.card_id.as_deref()?;
    if let Some((name, _)) = ids.get(card_id)
        && let Some(card) = data.cards.by_name(name)
    {
        return Some(card);
    }
    place.name.as_deref().and_then(|name| data.cards.by_name(name))
}

/// What a `discover` step's pick still owes the Battlecry count. A Discover a
/// Battlecry opened counts for "for each Battlecry you've triggered" at the
/// pick, not on the play (docs/rules/design.md "Counting Battlecries", 17_59_26-g1 turn
/// 8: the Tavern's Dark Paradox grows at the pick). This replay plays one
/// step at a time from the log's own state, so the play that owed it is
/// behind us: the step names the card that opened the Discover (`source`),
/// and when that card has a Battlecry the pick pays the triggers it would
/// have had.
fn owe_battlecry_count(data: &Data, ids: &CardIdMap, game: &mut Game, step: &ShopStepSpec) {
    if step.kind.as_deref() != Some("discover") {
        // A Triple Reward, a Dark Gift offer or a Choose One spell's pick.
        return;
    }
    let Some(card) = step.source.as_ref().and_then(|s| resolve_card(data, ids, s)) else {
        return;
    };
    if data.cards.get(card).battlecry.is_none() {
        return;
    }
    let n = game.battlecry_triggers(ME, card);
    let pl = &mut game.players[ME];
    pl.pending_battlecries = pl.pending_battlecries.saturating_add(n);
}

/// What a `discover` step still owes the Battlecry that opened it, beyond the
/// count: Imposing Percussionist's "Discover a Demon. Deal damage to your hero
/// equal to its Tier" deals that damage at the pick. The engine takes it from
/// `discover_damage_from`, which its own Discover sets from the request
/// (`game/discover.rs`), and the replay seats the offer instead, so the step
/// names the Demon that dealt it: its `source`, found on the board. The board's
/// "after your hero takes damage" cards (Ashen Corruptor, Devout Hellcaller)
/// then react as the log's did (triage round 3's group A,
/// `Hearthstone_2026_11_02_00_10_22-g5` turn 10 step 11: the Tavern gains
/// +2/+2 from Ashen Corruptor on every Percussionist pick).
fn owe_pick_damage(loaded: &mut LoadedShop, data: &Data, ids: &CardIdMap, step: &ShopStepSpec) {
    if step.kind.as_deref() != Some("discover") {
        return;
    }
    let Some(source) = step.source.as_ref() else { return };
    let Some(card) = resolve_card(data, ids, source) else { return };
    if !matches!(
        data.cards.get(card).battlecry,
        Some(crate::cards::Effect::DiscoverDemonTierDamage(_))
    ) {
        return;
    }
    // The step's own entity id is the card that dealt it. A source the state
    // before does not hold by id falls back to the card of that name on the
    // board.
    let board = &loaded.game.players[ME].board;
    let pos = (source.id != 0)
        .then(|| loaded.board_ids.iter().position(|&id| id == source.id))
        .flatten()
        .or_else(|| board.iter().position(|m| m.card == card && m.golden == source.golden))
        .or_else(|| board.iter().position(|m| m.card == card));
    let Some(pos) = pos else {
        loaded.missing.push("a pick's damage source is not on the board".to_string());
        return;
    };
    loaded.game.players[ME].discover_damage_from = Some(pos as u8);
}

/// The card a pick step's play put in the log, when that card is also the
/// step's own `source` and the step is the last pick the play opened.
///
/// A Discover a Battlecry opened and a Choose One's option both come as a pick
/// step of their own right behind the play, and what that play still owed
/// waits for the pick (`Player::pending_after_play`, `hogrider_pending`, the
/// cast an `Action::Choose` ends). No logged state carries those flags, so the
/// pick step has to seed them, and the file's own history says which pick is
/// the one they land on. Two things rule a pick out:
///
/// - The card has to be one the player played from the hand. A Choose One card
///   a Magicfin token casts opens the same pick and owes none of it
///   (`Hearthstone_2026_10_26_00_48_15-g2` turn 8 step 33, where the log gives
///   no Turbo Hogrider Gems), so the card the player played last before the
///   pick has to be the card the pick names.
/// - The step has to be that play's *last* pick. A Thorned Trailblazer's both
///   options give two picks in a row and what the play owed lands at the
///   second (`Hearthstone_2026_10_24_17_59_26-g1` turn 12 steps 10 and 11).
fn pick_play_card(
    data: &Data,
    ids: &CardIdMap,
    file: &ShopFileSpec,
    turn: usize,
    index: usize,
) -> Option<CardId> {
    let turn_spec = file.turns.get(turn)?;
    let step = turn_spec.actions.get(index)?;
    if step.step != "discover" || !matches!(step.kind.as_deref(), Some("discover" | "choose_one")) {
        return None;
    }
    let card = step.source.as_ref().and_then(|s| resolve_card(data, ids, s))?;
    let played = turn_spec.actions[..index]
        .iter()
        .rev()
        .find(|a| a.step == "play")
        .and_then(|a| a.card.as_ref())
        .and_then(|c| resolve_card(data, ids, c))?;
    if played != card {
        return None;
    }
    // Every pick the play opens runs before what it owed: a Brann Bronzebeard
    // triples an Imposing Percussionist's Battlecry, and its three picks are
    // three steps in a row (`Hearthstone_2026_11_01_17_59_13-g1` turn 10 steps
    // 3 to 5: the Wrath Weavers gain at the third, not at the first).
    let more = turn_spec.actions.get(index + 1).is_some_and(|next| {
        next.kind == step.kind
            && next.source.as_ref().and_then(|s| resolve_card(data, ids, s)) == Some(card)
    });
    if more {
        return None;
    }
    Some(card)
}

/// What a minion's pick still owes the play that opened it: the "after you
/// play an X" triggers, which wait for the Battlecry to resolve and for
/// whatever it asks. The game keeps them in `pending_after_play` and fires
/// them once nothing is open (`Game::resolve_after_play`), and the replay's
/// play step still has the pick open, so the pick step carries them
/// (triage round 3's group A: at an Imposing Percussionist's pick the hero
/// damage comes first and then the Wrath Weavers' blocks,
/// `Hearthstone_2026_10_24_15_25_26-g4` turn 6 step 8; `game/play.rs` says the
/// same of the same game).
fn owe_after_play(
    data: &Data,
    ids: &CardIdMap,
    game: &mut Game,
    file: &ShopFileSpec,
    turn: usize,
    index: usize,
) {
    let Some(card) = pick_play_card(data, ids, file, turn, index) else { return };
    let c = data.cards.get(card);
    if c.is_spell() {
        // A spell's own aftermath is `seat_cast_after_pick`'s.
        return;
    }
    let pl = &mut game.players[ME];
    // The played minion never counts for its own trigger
    // (`Minion::just_played`), and the state the pick starts from holds it on
    // the board without that mark.
    let tribes = match pl.board.iter().position(|m| m.card == card) {
        Some(pos) => {
            pl.board[pos].just_played = true;
            data.cards.tribes_of(&pl.board[pos])
        }
        None => c.tribes,
    };
    pl.pending_after_play = Some(tribes);
}

/// What a Choose One play's pick still owes: Turbo Hogrider's "After you play a
/// Choose One card, this plays 2 Blood Gems on all your Quilboar". The game
/// sets `hogrider_pending` as the card is played from the hand and spends it
/// once nothing that play opened is still waiting for a pick
/// (`Game::resolve_hogrider`), so the play's last pick carries it (triage round
/// 3's group C, `Hearthstone_2026_10_26_00_48_15-g2` turn 8: a Boundless
/// Potential pick gives the only Quilboar on the board, the Hogrider itself,
/// +4/+4; `pick_play_card` says which pick that is). The golden doubling is
/// the Hogrider's own (`after_choose_one_played`), not the play's.
fn owe_hogrider_gems(
    data: &Data,
    ids: &CardIdMap,
    game: &mut Game,
    file: &ShopFileSpec,
    turn: usize,
    index: usize,
) {
    let Some(card) = pick_play_card(data, ids, file, turn, index) else { return };
    if data.cards.get(card).choose_one.is_none() {
        return;
    }
    let pl = &mut game.players[ME];
    pl.hogrider_pending = pl.hogrider_pending.saturating_add(1);
}

/// What a pick step owes the "after you spend Gold" cards of the Activate it
/// answers. The Gold leaves the purse on the click, but the cards that watch
/// it wait for the choice the Activate opened (see `Action::Activate`), and
/// the file logs the two as separate steps: the Activate step's hand is
/// unchanged and the pick step carries the Bounty
/// (`Hearthstone_2026_10_26_00_48_15-g3` turn 7 steps 29 and 30, Sky Admiral
/// Rogers) or the Lockbox the Gold opened (`Hearthstone_2026_11_02_00_10_22-g5`
/// turn 10 steps 78 to 80, an Enterprising Escapee). A golden Clever Castaway
/// opens two Discovers as two pick steps and the Activate ends with the
/// second, so only that one seats the Gold.
fn seat_gold_spent_at_pick(data: &Data, ids: &CardIdMap, loaded: &mut LoadedShop, one: &OneStep) {
    let OneStep { file, step, turn_index, index, .. } = *one;
    if step.step != "discover" {
        return;
    }
    let Some(source) = step.source.as_ref() else { return };
    let Some(turn) = file.turns.get(turn_index) else { return };
    let same_source = |s: &ShopStepSpec| {
        s.step == "discover" && s.source.as_ref().is_some_and(|src| src.id == source.id)
    };
    // Another pick of the same Activate follows: that one ends it.
    if turn.actions.get(index + 1).is_some_and(same_source) {
        return;
    }
    // What comes before this pick is the Activate's own earlier picks, and
    // before them the Activate.
    let mut at = index;
    while at > 0 && turn.actions.get(at - 1).is_some_and(same_source) {
        at -= 1;
    }
    let from_activate = at > 0
        && turn.actions.get(at - 1).is_some_and(|prev| {
            prev.step == "activate" && prev.card.as_ref().is_some_and(|c| c.id == source.id)
        });
    if !from_activate {
        return;
    }
    let Some(card) = resolve_card(data, ids, source) else { return };
    let Some(act) = data.cards.get(card).activate else { return };
    // The counts this state shows are already the click's: the log moves them
    // when the Gold is spent and only the trigger's own card comes at the pick
    // (Sky Admiral Rogers goes `[2, 0, 9, 0]` to `[9, 1, 9, 0]` at the
    // Activate and its Bounty lands at the pick). Seat them as the Activate's
    // own state had them, so the Gold counted now fires exactly the watchers
    // the log fired.
    let before = match at {
        0 | 1 => Some(&turn.start),
        _ => turn.actions.get(at - 2).and_then(|s| s.after.as_ref()),
    };
    if let Some(before) = before {
        for (slot, &id) in loaded.board_ids.iter().enumerate() {
            let Some(&m) = loaded.game.players[ME].board.get(slot) else { continue };
            let Some(trigger) = data.cards.get(m.card).on_event else { continue };
            if trigger.kind != EventKind::GoldSpent || trigger.every <= 1 {
                continue;
            }
            let Some(left) = before
                .board
                .iter()
                .find(|b| b.id == id)
                .and_then(|b| b.numbers.first().copied())
            else {
                continue;
            };
            let done = (trigger.every as i32 - left).clamp(0, trigger.every as i32 - 1);
            loaded.game.players[ME].board[slot].improvements = done as u16;
        }
    }
    let pl = &mut loaded.game.players[ME];
    pl.pending_gold_spent = pl.pending_gold_spent.saturating_add(act.cost as u32);
}

/// A pick that ends a *spell* the player cast (a Discover it opened, or the
/// Choose One option it asked for) was logged as its own step, with the play in
/// the step before, so the game's own action is split in two here: the pick
/// step has to carry what the cast still owed, or those triggers never fire at
/// all. What waits for the spell to resolve fires at the pick,
/// inside the cast: the "after you cast a spell" triggers and the Dark Gift
/// count (docs/rules/design.md "Targets in the shop"; log
/// Hearthstone_2026_10_27_04_30_04 turn 7: Planar Telescope is cast at
/// Power.log 68792, its Discover is picked at 69231 and the Spell Siphon Dark
/// Gift's count only grows at 69283, and 2026_10_24_17_59_26 turn 10 step 43
/// to 44, Fandral's Fortune's pick and its Felboar's consume). The "whenever"
/// half and the counters were taken on the play, which is why this seats
/// `at_play: true` and no counter of its own.
///
/// The number of casts is one: a spell the player doubled with Balinda is not
/// in a file this reads (`cast_spell_as` folds the extra casts into the play).
pub fn seat_cast_after_pick(data: &Data, ids: &CardIdMap, game: &mut Game, step: &ShopStepSpec) {
    if !matches!(step.kind.as_deref(), Some("discover" | "choose_one")) {
        // A Triple Reward or a Dark Gift offer.
        return;
    }
    let Some(card) = step.source.as_ref().and_then(|s| resolve_card(data, ids, s)) else {
        return;
    };
    let c = data.cards.get(card);
    if !c.is_spell() || matches!(c.spell_effect, Some(crate::cards::SpellEffect::TripleReward)) {
        return;
    }
    // The board's "after you cast a Tavern spell" cards as this step shows
    // them: the play that owed them is behind us in the file.
    let after = game.gather_on_cast_spell(ME, true);
    game.players[ME].cast_after_choice = Some(crate::game::CastAfterChoice {
        card,
        casts: 1,
        tavern: c.tavern_spell,
        at_play: true,
        count_owed: c.tavern_spell,
        after,
    });
}

/// Where a Discover's pick went when it did not join the hand, as a zone and
/// a place in it: `board` and the 1-based position, or `tavern` and the file's
/// 1-based slot. The file says it in the pick's `moved.to` when it writes one;
/// when it does not, the state the step left still shows the pick standing
/// where another card stood (Token of the Old Gods turns a minion on the board
/// or in the Tavern into the discovered one, and the turned card keeps its
/// entity id, so the card id is what says it).
fn pick_landing<'a>(
    before: &'a ShopStateSpec,
    step: &'a ShopStepSpec,
) -> Option<(&'static str, u8)> {
    let picked = step.picked.as_ref()?;
    let card_id = picked.card_id.as_deref()?;
    if let Some(to) = step.moved.as_ref().and_then(|m| m.to.as_ref()) {
        match to.zone() {
            Some("board") => return to.position.map(|p| ("board", p)),
            Some("tavern") => return to.slot.map(|s| ("tavern", s)),
            _ => {}
        }
    }
    let after = step.after.as_ref()?;
    // A pick that joins the hand is not a transform, even when the hand held
    // another copy of it: the copies it holds go up by one.
    let copies = |zone: &[ShopMinionSpec]| {
        zone.iter()
            .filter(|m| m.card_id.as_deref() == Some(card_id))
            .count()
    };
    if copies(&after.hand) > copies(&before.hand) {
        return None;
    }
    let empty_before = |m: &ShopMinionSpec| {
        before
            .board
            .iter()
            .find(|b| b.position == m.position)
            .is_none_or(|b| b.card_id.as_deref() != Some(card_id))
    };
    if let Some(m) = after.board.iter().find(|m| {
        m.card_id.as_deref() == Some(card_id) && m.position.is_some() && empty_before(m)
    }) {
        return Some(("board", m.position.unwrap_or(0)));
    }
    let slot_changed = |m: &ShopMinionSpec| {
        before
            .tavern
            .iter()
            .find(|b| b.slot == m.slot)
            .is_none_or(|b| b.card_id.as_deref() != Some(card_id))
    };
    if let Some(m) = after.tavern.iter().find(|m| {
        m.card_id.as_deref() == Some(card_id) && m.slot.is_some() && slot_changed(m)
    }) {
        return Some(("tavern", m.slot.unwrap_or(0)));
    }
    None
}

/// The Detective for Hire offer a step answers: its two cards and which of them
/// counted as the right guess. The pick itself joins nothing: a right guess
/// hands a Tavern Coin over (docs/rules/heroes.md, `resolve_choose`'s own
/// path), which is why the log's hand holds the Coin and not the card it
/// picked (`Hearthstone_2026_11_02_00_10_22-g6` turn 7 step 2). The log's step
/// says whether the Coin came (`entered`), so it says which index the game
/// counted right.
fn holmes_offer_answer(
    data: &Data,
    ids: &CardIdMap,
    step: &ShopStepSpec,
) -> Option<([CardId; 2], u8)> {
    let source = step.source.as_ref()?.card_id.as_deref()?;
    let i = data.heroes.power_by_card_id(source)?;
    let coin = match data.heroes.power(i).effect {
        crate::heroes::PowerEffect::DetectiveForHire { coin } => coin,
        _ => return None,
    };
    if step.options.len() != 2 || step.kind.as_deref() != Some("discover") {
        return None;
    }
    let picked = step.picked.as_ref()?.id;
    let at = step.options.iter().position(|o| o.id == picked)?;
    let cards = [
        resolve_card(data, ids, &step.options[0])?,
        resolve_card(data, ids, &step.options[1])?,
    ];
    let got = step
        .entered
        .iter()
        .any(|e| resolve_card(data, ids, e) == Some(coin));
    let correct = if got { at } else { 1 - at } as u8;
    Some((cards, correct))
}

/// Put the log's offer on the player before answering it: a Discover's
/// options, a Dark Discovery's offers with their gifts, or a trinket offer.
/// Returns the index of the option the log picked.
fn force_offer(
    data: &Data,
    ids: &CardIdMap,
    game: &mut Game,
    step: &ShopStepSpec,
    turn: u32,
    turned: bool,
    missing: &mut Vec<String>,
) -> Result<usize, String> {
    let picked = step
        .picked
        .as_ref()
        .ok_or_else(|| "the step does not say what was picked".to_string())?;
    let at = step
        .options
        .iter()
        .position(|o| o.id == picked.id)
        .ok_or_else(|| "the pick is not one of the options".to_string())?;
    let mut cards = Vec::new();
    for option in &step.options {
        match resolve_card(data, ids, option) {
            Some(card) => cards.push(card),
            None => return Err(format!(
                "an option the card set does not build: {}",
                option
                    .name
                    .clone()
                    .or_else(|| option.card_id.clone())
                    .unwrap_or_default()
            )),
        }
    }
    let pl = &mut game.players[ME];
    if step.step == "trinket_pick" {
        // The engine's offer holds trinket table indices and their prices. The
        // cost the pick was sold at is not in the options, only what the step
        // paid: the picked one carries that, the others nothing.
        let paid = step.paid.map_or(0, |p| p.gold);
        let mut offers = Vec::new();
        for (k, option) in step.options.iter().enumerate() {
            let index = option
                .card_id
                .as_deref()
                .and_then(|id| data.trinkets.by_card_id(id))
                .ok_or_else(|| {
                    format!(
                        "a trinket the trinket table does not build: {}",
                        option.name.clone().unwrap_or_default()
                    )
                })?;
            let price = if k == at { paid.clamp(0, u8::MAX as i32) as u8 } else { 0 };
            offers.push((index, price));
        }
        for (index, price) in offers {
            let _ = pl.trinket_offer.try_push((index, price));
        }
        pl.trinket_offer_slot = match step.slot_kind.as_deref() {
            Some("greater") => 2,
            Some("lesser") => 1,
            other => {
                if let Some(k) = other {
                    missing.push(format!("a trinket slot the engine has no kind for: {k}"));
                }
                1
            }
        };
        return Ok(at);
    }
    // Detective for Hire's two cards are a guess, not a Discover: the engine
    // keeps them in its own offer and the pick lands nothing but the Coin.
    if let Some((cards, correct)) = holmes_offer_answer(data, ids, step) {
        pl.holmes_offer = Some((cards, correct));
        return Ok(at);
    }
    let dark = step.kind.as_deref() == Some("dark_gift");
    // The options as they stood: each becomes the minion the log shows (its
    // stats, golden flag, keywords and numbers), which the engine lands as it
    // is (`discover_minions`), so a stat-set offer or a Golden one comes out
    // right and no pool copy moves.
    let mut ready: Vec<Minion> = step
        .options
        .iter()
        .map(|o| {
            let spec = ShopMinionSpec {
                card_id: o.card_id.clone(),
                name: o.name.clone(),
                attack: o.attack,
                health: o.health,
                golden: o.golden,
                keywords: o.keywords.clone().unwrap_or_default(),
                numbers: o.numbers.clone().unwrap_or_default(),
                dark_gifts: o.dark_gifts.clone(),
                ..ShopMinionSpec::default()
            };
            let (mut m, _) = build_minion(data, &spec, turn);
            // The card's own keywords stand whatever the file shows (a card
            // with Taunt printed has Taunt).
            m.keywords.0 |= data.cards.get(m.card).keywords.0;
            m
        })
        .collect();
    // Innkeeper's Hearth: "Discover two Tier 6 minions. Set their stats to
    // 30/30". The engine's own set (`Player::tp.discover_stats`) runs only on
    // a Discover it deals itself, and a state loaded mid-turn has lost it, so
    // the loader sets the pick from the trinket the step names
    // (`Hearthstone_2026_10_28_22_11_50-g5` turn 9 steps 2 and 3, both
    // 30/30). It adds the bonuses that apply wherever the card is, as the
    // engine's own set does.
    let set_stats = step
        .source
        .as_ref()
        .and_then(|s| s.card_id.as_deref())
        .and_then(|id| data.trinkets.by_card_id(id))
        .and_then(|i| {
            data.trinkets.get(i).on_pick.iter().find_map(|a| match a {
                crate::trinkets::Act::DiscoverTierStats { attack, health, .. } => {
                    Some((*attack, *health))
                }
                _ => None,
            })
        });
    if step.step == "discover"
        && let (Some((a, h)), Some(m)) = (set_stats, ready.get_mut(at))
    {
        let (wa, wh) = pl.wherever_stats(&data.cards, m);
        m.attack = a + wa;
        m.health = h + wh;
    }
    // A pick that lands on the board or in a Tavern slot (a transform, or
    // Galakrond's Greed taking the slot) goes through the engine's transform
    // paths, which need the offer's card ids, not ready-made minions. The step
    // that reads it says so (`pick_landing`), whether the file writes the
    // pick's `moved.to` or only shows the card it turned in the state it left.
    let placed_elsewhere = turned;
    if !placed_elsewhere && ready.len() == step.options.len() && !ready.is_empty() {
        for m in &ready {
            let _ = pl.discover_minions.try_push(*m);
        }
        for card in &cards {
            let _ = pl.discover.try_push(*card);
        }
        if dark {
            pl.dark_offer = true;
            pl.dark_offer_turn = turn;
            pl.dark_offer_golden = false;
        }
        return Ok(at);
    }
    // The offer holds one pool copy per minion option (three for a Golden Dark
    // Gift offer, as the engine's own offers do), so the pick and the copies
    // the unpicked options give back balance.
    let mut copies_taken = Vec::new();
    for (k, card) in cards.iter().enumerate() {
        let c = data.cards.get(*card);
        let mut take = 0;
        if !c.is_spell() && !c.token {
            let slot = c.pool_slot as usize;
            let want = if dark && step.options[k].golden { 3 } else { 1 };
            if slot < game.pool.len() {
                take = want.min(game.pool[slot]);
                game.pool[slot] -= take;
            }
        }
        copies_taken.push(take);
    }
    let pl = &mut game.players[ME];
    for (k, card) in cards.iter().enumerate() {
        let _ = pl.discover.try_push(*card);
        if dark {
            let gift = step.options[k]
                .dark_gifts
                .first()
                .and_then(|g| g.card_id.as_deref())
                .and_then(|id| data.gifts.gifts.iter().position(|g| g.id == id))
                .map(|i| data.gifts.gifts[i].card);
            match gift {
                Some(card) => {
                    let state = turn.min(crate::minion::GIFT_STATE_MAX as u32) as u8;
                    let _ = pl.discover_gifts.try_push(crate::minion::GiftOn::new(card, state));
                }
                None => {
                    let _ = pl.discover_gifts.try_push(crate::minion::GiftOn::new(0, 0));
                }
            }
            let _ = pl.discover_copies.try_push(copies_taken[k]);
        }
    }
    if dark {
        pl.dark_offer = true;
        pl.dark_offer_turn = turn;
        pl.dark_offer_golden = false;
    }
    if cards.is_empty() {
        return Err("the step offers nothing".to_string());
    }
    Ok(at)
}

/// The board slot a played minion went into, from the state the step left.
/// The file's `moved.to` is the position the log showed when the minion
/// landed, which is not where it was played: a Battlecry's summons come after
/// it (and a minion the Battlecry kills leaves a gap before it), so the logged
/// position can be past the board (an illegal play) or in front of the card
/// the log puts before it (triage group D3, and I5 with a full board). Where
/// the minion stands among the cards the loader already knows is the slot: the
/// one just past the last of them it stands behind.
fn play_slot(loaded: &LoadedShop, step: &ShopStepSpec) -> Option<u8> {
    let after = step.after.as_ref()?;
    let played = step.moved.as_ref().map(|m| m.id)?;
    if played == 0 {
        return None;
    }
    let mut board: Vec<&ShopMinionSpec> = after.board.iter().collect();
    board.sort_by_key(|m| (m.position.unwrap_or(0), m.id));
    // The game can give the played minion a new entity id as it lands (Scarlet
    // Skull: hand 18629, board 18667), so a card of the same name the state
    // before held no copy of by id is the one that landed (log
    // 2026_10_24_15_25_26-g6 turn 12 step 29).
    let same_card = step.card.as_ref().and_then(|c| c.card_id.as_deref());
    let at = board.iter().position(|m| m.id == played).or_else(|| {
        let card_id = same_card?;
        board
            .iter()
            .position(|m| m.card_id.as_deref() == Some(card_id) && !loaded.board_ids.contains(&m.id))
    })?;
    // The slot a card stands in: its entity id in the board the play started
    // from, or, for a card the game destroyed and brought back to the slot it
    // stood in (a Reborn minion: Dead Bellringer's Reborn copy has a new
    // entity id), the slot the card that left had.
    let slot = |m: &ShopMinionSpec| -> Option<u8> {
        if let Some(i) = loaded.board_ids.iter().position(|&x| x == m.id) {
            return Some(i as u8);
        }
        let gone = step
            .left
            .iter()
            .find(|l| l.position == m.position && l.card_id.is_some() && l.card_id == m.card_id)?;
        loaded.board_ids.iter().position(|&x| x == gone.id).map(|i| i as u8)
    };
    for m in board[..at].iter().rev() {
        if let Some(i) = slot(m) {
            return Some(i + 1);
        }
    }
    Some(0)
}

/// The board minion a Magnetized minion merged into. The file lists every
/// card the step changed, in no order of its own: a stat static the play set
/// off comes before the host (triage group M1: Timecap'n Hooktail's +3 Attack
/// was the first entry and the Volumizer merged into Relentless Deflector).
/// The host is the one the played card's own stats went into: a minion it can
/// Magnetize onto whose stats grew by at least the card's, the smallest growth
/// when a static grew it too.
fn magnetize_host(data: &Data, loaded: &LoadedShop, step: &ShopStepSpec) -> Option<u8> {
    let at = zone_index(&loaded.hand_ids, step.card.as_ref()?.id, step.index)?;
    let played = *loaded.game.players[ME].hand.get(at as usize)?;
    let mut best: Option<(i32, u8)> = None;
    for c in &step.changed {
        let (Some(pos), Some(was), Some(now)) = (c.position, c.was, c.now) else {
            continue;
        };
        let Some(host) = loaded.game.players[ME].board.get(pos.saturating_sub(1) as usize) else {
            continue;
        };
        if !data.cards.get(played.card).magnetize_tribes.overlaps(data.cards.tribes_of(host)) {
            continue;
        }
        let (da, dh) = (now.attack - was.attack, now.health - was.health);
        if da < played.attack || dh < played.health {
            continue;
        }
        if best.is_none_or(|(grown, _)| da + dh < grown) {
            best = Some((da + dh, pos.saturating_sub(1)));
        }
    }
    // A step the file lists no changed entry for: the first one it does list
    // is all the loader has to go on, as before.
    best.map(|(_, pos)| pos)
        .or_else(|| step.changed.iter().find_map(|c| c.position).map(|p| p.saturating_sub(1)))
}

/// The engine's actions for one logged step, in order. A step the engine has
/// no action for comes back as an error and the step is incomplete.
fn actions_for(
    data: &Data,
    ids: &CardIdMap,
    loaded: &mut LoadedShop,
    step: &ShopStepSpec,
    turn: u32,
    landing: bool,
    missing: &mut Vec<String>,
) -> Result<Vec<Action>, String> {
    let id = step.card.as_ref().map_or(0, |c| c.id);
    let board_len = loaded.game.players[ME].board.len();
    let mut out = Vec::new();
    match step.step.as_str() {
        "buy" => {
            let target_id = step.target.as_ref().map_or(0, |t| t.id);
            // The spell slot is the engine's own (`shop_spell`), bought with
            // its own action; a minion slot (a spell the game put there
            // included) is a slot.
            if target_id != 0 && Some(target_id) == loaded.spell_id {
                if loaded.game.players[ME].shop_spell.is_none() {
                    return Err("the Tavern has no spell slot to buy".to_string());
                }
                // The log's own price for the spell, when the card does not
                // say what made it cheaper (`seat_spell_paid`).
                seat_spell_paid(&mut loaded.game, data, step);
                out.push(Action::BuySpell);
            } else {
                let at = shop_index(loaded, target_id, step.slot).ok_or_else(|| {
                    "the Tavern slot the step bought is not in the Tavern".to_string()
                })?;
                if at as usize >= loaded.game.players[ME].shop.len() {
                    return Err("the Tavern slot the step bought is not in the Tavern".to_string());
                }
                // A Tavern spell in a minion slot (a spell the game dealt
                // there, a Saloon's Finest slot) is priced by the game the
                // same way as the spell slot's, and the file's own price is
                // what it took: a Tavern Coin bought at 0 Gold is legal in the
                // log (`Hearthstone_2026_11_02_00_10_22-g6` turn 1 step 4)
                // and the engine would refuse it (`seat_shop_spell_paid`).
                let bought = loaded.game.players[ME].shop[at as usize].card;
                if data.cards.get(bought).is_spell() {
                    seat_shop_spell_paid(&mut loaded.game, data, bought, step);
                }
                out.push(Action::Buy(at));
            }
        }
        "sell" => {
            let at = zone_index(&loaded.board_ids, id, step.position)
                .ok_or_else(|| "the card the step sold is not on the board".to_string())?;
            if at as usize >= board_len {
                return Err("the card the step sold is not on the board".to_string());
            }
            out.push(Action::Sell(at));
        }
        "play" => {
            let at = zone_index(&loaded.hand_ids, id, step.index)
                .ok_or_else(|| "the card the step played is not in the hand".to_string())?;
            let pl = &loaded.game.players[ME];
            if at as usize >= pl.hand.len() {
                return Err("the card the step played is not in the hand".to_string());
            }
            // A spell is played into no slot. A minion is played into the
            // slot it stands behind in the state the step left, which is not
            // the position the log showed when it landed: a Battlecry's
            // summons push that one right and a minion it killed leaves a
            // gap (triage group D3). A Magnetized minion goes nowhere at all
            // (`moved.to` is null): it merged into a host.
            let spell = data.cards.get(pl.hand[at as usize].card).is_spell();
            let landed = step
                .moved
                .as_ref()
                .and_then(|m| m.to.as_ref())
                .and_then(|to| to.position);
            let pos = if spell {
                0
            } else {
                play_slot(loaded, step)
                    .or_else(|| match landed {
                        Some(_) => landed.map(|p| p.saturating_sub(1)),
                        None => magnetize_host(data, loaded, step),
                    })
                    .unwrap_or(board_len as u8)
            };
            out.push(Action::Play { hand: at, pos });
            if let Some(target) = &step.target {
                let mut a = target_action(loaded, target)?;
                // The file's board target is the board before the play; a
                // minion that lands at or before it pushes it one slot right,
                // or the Battlecry hits the card just played (a Sprightly
                // Scarab's Battlecry; Hearthstone_2026_10_28_02_28_32-g1 turn
                // 10 step 2, a Magicfin token's Tricky Trousers on the
                // Drakkari Enchanter). A Magnetic merge lands nothing, so it
                // shifts nothing.
                let lands = !spell && landed.is_some();
                if lands
                    && let Action::Target(t) = a
                    && t as usize >= pos as usize
                {
                    a = Action::Target(t + 1);
                }
                out.push(a);
            }
        }
        "activate" => {
            let at = zone_index(&loaded.board_ids, id, step.position)
                .ok_or_else(|| "the card the step activated is not on the board".to_string())?;
            if at as usize >= board_len {
                return Err("the card the step activated is not on the board".to_string());
            }
            out.push(Action::Activate(at));
            if let Some(target) = &step.target {
                // Mangled Bandit and its kin: "Discard a card to get ...".
                let card = loaded.game.players[ME].board[at as usize].card;
                if target.kind.as_deref() == Some("hand")
                    && data.cards.get(card).activate.is_some_and(|a| a.discard)
                {
                    out.push(discard_action(loaded, target)?);
                } else {
                    out.push(target_action(loaded, target)?);
                }
            }
        }
        "move" => {
            let from = zone_index(&loaded.board_ids, id, step.position)
                .ok_or_else(|| "the card the step moved is not on the board".to_string())?;
            let to = step
                .to
                .ok_or_else(|| "the step does not say where the card went".to_string())?;
            out.push(Action::Move { from, to: to.saturating_sub(1) });
        }
        "roll" => out.push(Action::Roll),
        "freeze" => out.push(Action::Freeze),
        "level" => out.push(Action::Level),
        "hero_power" => {
            out.push(if step.which.as_deref() == Some("second") {
                Action::HeroPower2
            } else {
                Action::HeroPower
            });
            if let Some(target) = &step.target {
                // Drest'agath's Incubate: "Discard a card, then Discover".
                let slot = if step.which.as_deref() == Some("second") {
                    loaded.game.players[ME].power2
                } else {
                    loaded.game.players[ME].power
                };
                let discards = slot.is_some_and(|st| {
                    matches!(
                        data.heroes.power(st.of).effect,
                        crate::heroes::PowerEffect::DiscardGetRandomMinion { .. }
                    )
                });
                if target.kind.as_deref() == Some("hand") && discards {
                    out.push(discard_action(loaded, target)?);
                } else {
                    out.push(target_action(loaded, target)?);
                }
            }
        }
        "dark_discovery" => out.push(Action::DarkGift),
        "discover" | "trinket_pick" => {
            if step.step == "discover" {
                owe_battlecry_count(data, ids, &mut loaded.game, step);
                owe_pick_damage(loaded, data, ids, step);
            }
            seat_cast_after_pick(data, ids, &mut loaded.game, step);
            let at = force_offer(data, ids, &mut loaded.game, step, turn, landing, missing)?;
            out.push(Action::Choose(at as u8));
        }
        other => {
            return Err(format!("a step the engine has no action for: {other}"));
        }
    }
    Ok(out)
}

/// What one run of a step produced: the state, what the loader could not use,
/// and the engine's action as text.
type StepRun = (ShopSnapshot, Vec<String>, String);
/// Why a step could not be played, and what the loader could not use.
type StepError = (String, Vec<String>);

/// Apply one step to a freshly loaded game and snapshot the result. The error
/// is what made the step impossible, which makes it incomplete or illegal.
/// One step of one turn, and the state it starts from, as `run_once` needs
/// them: the file for the hero and the lobby, the state, the step, the turn
/// and whether a one-shot trinket effect is armed.
struct OneStep<'a> {
    file: &'a ShopFileSpec,
    before: &'a ShopStateSpec,
    step: &'a ShopStepSpec,
    turn: u32,
    /// The turn's place in the file, and the step's place in its turn.
    turn_index: usize,
    index: usize,
    armed: bool,
    /// The last Tavern spell the player had cast when this step began, from
    /// the file's own history: Cataclysmic Harbinger and the Galaxy's Lens
    /// copy it, and no single logged state carries it.
    last_spell: Option<CardId>,
}

/// A Choose One's answer: which option the log took, and whether it gave both.
#[derive(Clone, Copy, Debug)]
struct ChooseAnswer {
    option: u8,
    both: bool,
}

/// Whether a Choose One option's own outcome shows in the step: the free
/// Refreshes the option gives (`free_refreshes` before and after), or the cards
/// it puts in the hand, each of them from the card that played (`entered`).
/// `false` for an option whose outcome no state carries (maximum Gold, a Blood
/// Gem's bonus "this game", a target), which says nothing either way.
fn choose_option_shows(
    data: &Data,
    ids: &CardIdMap,
    opt: crate::cards::ChooseOption,
    step: &ShopStepSpec,
    before: &ShopStateSpec,
    after: &ShopStateSpec,
) -> bool {
    use crate::cards::{ChooseOption as Opt, SpellEffect};
    let source = step.card.as_ref().and_then(|c| c.card_id.as_deref());
    let entered: Vec<&ShopPlace> = step
        .entered
        .iter()
        .filter(|e| e.from.as_ref().and_then(|f| f.card_id.as_deref()) == source)
        .collect();
    let is = |e: &ShopPlace, card: CardId| resolve_card(data, ids, e) == Some(card);
    let of_tribe = |e: &ShopPlace, tribe: Tribe| {
        resolve_card(data, ids, e).is_some_and(|c| data.cards.get(c).tribes.has(tribe))
    };
    match opt {
        Opt::SpellEffect(SpellEffect::FreeRolls(n)) => {
            after.free_refreshes.saturating_sub(before.free_refreshes) >= n
        }
        Opt::SpellEffect(SpellEffect::GetGems(n)) => {
            n > 0
                && data.cards.blood_gem.is_some_and(|gem| {
                    entered.iter().any(|e| is(e, gem))
                })
        }
        Opt::SpellEffect(SpellEffect::GetCard { card, .. }) => {
            entered.iter().any(|e| is(e, card))
        }
        Opt::SpellEffect(SpellEffect::RandomTribeMinion { tribe, .. }) => {
            entered.iter().any(|e| of_tribe(e, tribe))
        }
        _ => false,
    }
}

/// How a Choose One the file does not mark was answered. The game writes the
/// client's own `sub_option` when the player picks a way, and nothing when it
/// picks the way itself (an auto-pick, triage round 2's group L2); the log's
/// own step still shows what ran (the free Refreshes, the cards that arrived),
/// which is what this reads. Both options show when a Thorned Trailblazer or a
/// Trailblazer Sticker combined them, and the step then says so in the same
/// two ways. `None` when the file marks no option and the log's outcome cannot
/// say which one ran (an option that changes no number the file holds).
fn logged_choose_one(
    data: &Data,
    ids: &CardIdMap,
    step: &ShopStepSpec,
    before: &ShopStateSpec,
    after: &ShopStateSpec,
) -> Option<ChooseAnswer> {
    if let Some(k) = step.sub_option
        && (0..2).contains(&k)
    {
        return Some(ChooseAnswer { option: k as u8, both: false });
    }
    if step.step != "play" {
        return None;
    }
    let card = resolve_card(data, ids, step.card.as_ref()?)?;
    let [a, b] = data.cards.get(card).choose_one?;
    let (a, b) = (
        choose_option_shows(data, ids, a, step, before, after),
        choose_option_shows(data, ids, b, step, before, after),
    );
    match (a, b) {
        (true, true) => Some(ChooseAnswer { option: 0, both: true }),
        (false, true) => Some(ChooseAnswer { option: 1, both: false }),
        (true, false) => Some(ChooseAnswer { option: 0, both: false }),
        (false, false) => None,
    }
}

fn run_once(data: &Arc<Data>, ids: &CardIdMap, one: &OneStep, seed: u64) -> Result<StepRun, StepError> {
    let OneStep { file, before, step, turn, turn_index, index, armed, last_spell } = *one;
    let hero = file.hero(data, before);
    // The start of a turn follows the turn before's last state, which one
    // start-of-turn trigger needs (a chosen power's countdown).
    let previous = (index == 0)
        .then(|| state_before_turn(file, turn_index))
        .flatten();
    let mut loaded = load(data, before, hero, file.lobby(), turn, seed, previous);
    // The state may hold three copies a summon left on the board before the
    // game's own triple check ran: the check runs as this step's action ends
    // when the step's own states say it did (`board_triple_due`).
    if let Some(after) = step.after.as_ref()
        && board_triple_due(before, after)
    {
        loaded.game.note_triples_due();
    }
    loaded.game.players[ME].tp.whistle = armed;
    loaded.game.players[ME].last_spell = last_spell;
    // The minions this turn has already bought, for Glaive Ricochet's plain
    // copy of one of them.
    loaded.game.players[ME].recent_buys.clear();
    for card in recent_buys_of_turn(data, ids, file, turn_index, index) {
        let _ = loaded.game.players[ME].recent_buys.try_push(card);
    }
    // A Gold-plated Compass's type is spent once the file has shown the Golden
    // minion it named; the state the loader seats says nothing about it
    // (`compass_spent`).
    if compass_spent(file, turn_index, index) {
        forget_compass_types(&mut loaded.game, data);
    }
    // What the turn's own spells improved the Tavern spell bonus by: the log's
    // counter now against the one the turn started with.
    if let Some(start) = file.turns.get(turn_index).map(|t| &t.start) {
        seat_spell_bonus_turn(&mut loaded.game.players[ME], before, start);
    }
    // A start-of-turn card the hand had no room for: queued again, so this
    // step can hand it over when it frees a slot.
    seat_queued_start_of_turn(data, &mut loaded.game.players[ME], file, turn_index, index);
    // How many Elementals this turn has played so far: no state in the file
    // carries it, and the turn's own earlier steps do (Conflagration's
    // "improved by each Elemental you played this turn").
    seat_elementals_played_turn(data, ids, &mut loaded.game.players[ME], file, turn_index, index);
    // What the play behind this pick still owed it, and no state carries: the
    // played minion's "after you play an X" triggers (an Imposing
    // Percussionist's Wrath Weavers) and a Turbo Hogrider's Gems for a Choose
    // One card.
    owe_after_play(data, ids, &mut loaded.game, file, turn_index, index);
    owe_hogrider_gems(data, ids, &mut loaded.game, file, turn_index, index);
    let mut missing = std::mem::take(&mut loaded.missing);
    // The way the log answered a Choose One this step plays. A card that gives
    // both options (a Thorned Trailblazer, a Trailblazer Sticker) has to know
    // it before the play, because that is the play's own path.
    let answer = step.after.as_ref().and_then(|after| {
        logged_choose_one(data, ids, step, before, after)
    });
    if answer.is_some_and(|a| a.both) {
        loaded.game.players[ME].tp.choose_one_all = true;
    }
    seat_start_of_turn_gold(data, ids, &mut loaded.game.players[ME], step);
    // The Gold the Activate before this pick spent, and what watches Gold
    // spent: the game answers the choice first (`Action::Activate`).
    seat_gold_spent_at_pick(data, ids, &mut loaded, one);
    // A Souvenir Stand the log turns into a copy of the Greater trinket in
    // this step rather than at its pick.
    if let Some(after) = step.after.as_ref() {
        seat_souvenir_copy(data, &mut loaded.game, before, after);
    }
    if step.step == "end_turn" {
        // The end of turn effects, without the fight that follows them: the
        // logged turn ends before the fight.
        loaded.game.shop_end_of_turn(ME);
        // And the picks those effects opened are in the same logged step (the
        // file folds them in), so a Battlecry they triggered has resolved and
        // owes its count here.
        loaded.game.pay_pending_battlecries(ME);
        let snap = ShopSnapshot::of_game(data, &loaded.game, ME);
        return Ok((snap, missing, "end of turn effects".to_string()));
    }
    // Where a Discover's pick lands, when it is not the hand: the engine only
    // does that from a state the press left (Galakrond's Greed's Tavern slot, a
    // transform on the board).
    let landing = (step.step == "discover")
        .then(|| pick_landing(before, step))
        .flatten();
    if let Some((zone, at)) = landing {
        match zone {
            "board" => loaded.game.players[ME].tp.transform_slot = at,
            _ => {
                // The pick takes the Tavern slot the old card held, which is
                // still where the logged state shows it (the game keeps it on
                // show while the Discover is open) and which the engine's own
                // pick clears. The slot is the file's, the spell slot counted
                // in it; the engine's shop holds the minion slots alone.
                let slot = engine_shop_slot(before, at);
                loaded.game.players[ME].discover_into_shop =
                    (slot.min(crate::SHOP - 1) + 1) as u8;
            }
        }
    }
    let actions = match actions_for(
        data,
        ids,
        &mut loaded,
        step,
        turn,
        landing.is_some(),
        &mut missing,
    ) {
        Ok(a) => a,
        Err(e) => return Err((e, missing)),
    };
    // The game's power button for a press is one the engine does not build:
    // the log then shows a press the engine has no state for. Spirit Swap's
    // second click is one (the game counts each click of a two-click power as
    // a use and swaps the button to the power's second half, while the engine
    // presses once and remembers the first pick itself), and so is a power no
    // card set has. Such a step is not a finding: the state cannot carry it.
    let press_not_built = step.step == "hero_power"
        && before
            .hero_powers
            .iter()
            .find(|p| p.second == (step.which.as_deref() == Some("second")))
            .and_then(|p| p.card_id.as_deref())
            .is_some_and(|id| data.heroes.power_by_card_id(id).is_none());
    let mut text = Vec::new();
    // The card this step plays is a Choose One card, so the step answers its
    // choice whether or not the file says which option ran (`logged_choose_one`
    // reads that when it can).
    let plays_choose_one = step.step == "play"
        && step
            .card
            .as_ref()
            .and_then(|c| resolve_card(data, ids, c))
            .is_some_and(|c| data.cards.get(c).choose_one.is_some());
    // A Choose One with a target asks the option first, then the target
    // (docs/rules/design.md "Targets in the shop"): the play's own sub-option is answered
    // before the step's target, not after it.
    let choose_first = plays_choose_one
        && !answer.is_some_and(|a| a.both)
        && actions.len() > 1;
    let mut chosen = false;
    for a in &actions {
        if choose_first
            && !chosen
            && matches!(a, Action::Target(_) | Action::TargetShop(_))
            && loaded.game.players[ME].pending_choose_one.is_some()
        {
            let k = answer.map_or(0, |a| a.option);
            if loaded.game.step(Action::Choose(k)).is_err() {
                return Err((format!("the engine does not allow {k:?} of the Choose One"), missing));
            }
            text.push(format!("Choose({k})"));
            chosen = true;
        }
        // A prompt with one answer is no prompt: the engine applies the only
        // target itself (`offer_target`), so the step's own target click has
        // nothing left to answer in the replay. The state the game left still
        // says whether the engine picked the same target.
        //
        // A Discover is not a target prompt: an effect can apply its one
        // target and open a Discover from it in the same click (Kelp Keeper's
        // trigger of a Battlecry that Discovers, Trash for Treasure), and the
        // step's target click is still one the engine has already answered.
        let pl = &loaded.game.players[ME];
        let nothing_open = pl.pending_target.is_none()
            && pl.pending_shop_target.is_none()
            && pl.pending_choose_one.is_none();
        if matches!(a, Action::Target(_) | Action::TargetShop(_)) && nothing_open {
            text.push(format!("{a:?} (one answer, so the engine took it)"));
            continue;
        }
        if loaded.game.step(*a).is_err() {
            // A pick of a two-step effect answers the prompt the *game* opened
            // in the step before, which one state cannot carry: the replay can
            // answer only one of the two picks.
            let pl = &loaded.game.players[ME];
            if matches!(a, Action::Target(_)) && pl.pending_shop_target.is_some() {
                return Err((
                    "a board pick where the engine is still on the Tavern pick of a \
                     two-step effect, whose first pick the file does not carry"
                        .to_string(),
                    missing,
                ));
            }
            if matches!(a, Action::TargetShop(_)) && pl.pending_target.is_some() {
                return Err((
                    "a Tavern pick where the engine is still on the board pick of a \
                     two-step effect, whose first pick the file does not carry"
                        .to_string(),
                    missing,
                ));
            }
            if press_not_built && matches!(a, Action::HeroPower | Action::HeroPower2) {
                return Err((
                    "the power the game shows is one the engine does not build: its \
                     second click of a two-click power, or a power no card set has"
                        .to_string(),
                    missing,
                ));
            }
            return Err((format!("the engine does not allow {a:?}"), missing));
        }
        text.push(format!("{a:?}"));
    }
    // A Choose One card played from the hand asks which way in the same click
    // in the game, and the client's own sub-option says which one it was (or,
    // when the game picked the way itself, the log's own step does: see
    // `logged_choose_one`): the engine opens the two options after the play.
    //
    // The game always answers it, so the replay does too: a Choose One whose
    // options change nothing the file carries (Intrepid Botanist's "+1 Attack
    // or +1 Health to your Tavern spells this game") still resolved, and what
    // it set off landed in the same step (a Turbo Hogrider's Gems on every
    // Quilboar, `Hearthstone_2026_10_28_02_28_32-g4` turn 13 step 27). Its
    // first option answers it, since the file gives neither one's outcome.
    if plays_choose_one
        && !chosen
        && !answer.is_some_and(|a| a.both)
        && loaded.game.players[ME].pending_choose_one.is_some()
    {
        let option = answer.map_or(0, |a| a.option);
        if loaded.game.step(Action::Choose(option)).is_err() {
            return Err((
                format!("the engine does not allow {option:?} of the Choose One"),
                missing,
            ));
        }
        text.push(format!("Choose({option})"));
    }
    let snap = ShopSnapshot::of_game(data, &loaded.game, ME);
    Ok((snap, missing, text.join(", ")))
}

/// The Warband Whistle: its pick makes the next Refresh deal plain copies of
/// the warband, once.
const WHISTLE: &str = "BG35_MagicItem_930";

/// Whether the Warband Whistle's "your next Refresh holds copies of your
/// warband" is still armed at a step. No state in the file says so, but the
/// steps before it do: the pick of the trinket armed it and the first Refresh
/// after that spent it.
fn whistle_armed(file: &ShopFileSpec, turn: usize, step: usize) -> bool {
    for t in (0..=turn).rev() {
        let Some(turn_spec) = file.turns.get(t) else { continue };
        let last = if t == turn { step } else { turn_spec.actions.len() };
        for i in (0..last).rev() {
            let a = &turn_spec.actions[i];
            if a.step == "roll" {
                return false;
            }
            let picked = a.picked.as_ref().and_then(|p| p.card_id.as_deref());
            if a.step == "trinket_pick" && picked == Some(WHISTLE) {
                return true;
            }
        }
    }
    false
}

/// Whether the Gold-plated Compass has spent its type by step `step` of turn
/// `turn`. It makes the next minion of its type the player buys Golden where it
/// stands in the Tavern, once, and no logged state says whether that has
/// happened: the steps before this one do. The payout is a `buy` whose Tavern
/// card was plain and whose hand holds that same entity Golden, unless the two
/// plain copies a triple needs were already in the hand or on the board: a buy
/// that completes a triple keeps the Compass armed
/// (docs/notes/UNSURE.md, "Gold-plated Compass and triples"). A Tavern card
/// Golden Touch made Golden is plain at no buy, so it reads as no payout.
fn compass_spent(file: &ShopFileSpec, turn: usize, step: usize) -> bool {
    for t in 0..=turn {
        let Some(turn_spec) = file.turns.get(t) else { continue };
        let last = if t == turn {
            step.min(turn_spec.actions.len())
        } else {
            turn_spec.actions.len()
        };
        let mut before: &ShopStateSpec = &turn_spec.start;
        for a in &turn_spec.actions[..last] {
            if golden_bought(before, a) {
                return true;
            }
            if let Some(aft) = &a.after {
                before = aft;
            }
        }
    }
    false
}

/// Whether the buy step `a`, taken from the state `before` it, left the Tavern
/// card it bought Golden in hand, with no triple behind it: the copies the
/// triple would have taken were not owned before the buy.
fn golden_bought(before: &ShopStateSpec, a: &ShopStepSpec) -> bool {
    if a.step != "buy" {
        return false;
    }
    let Some(card) = a.card.as_ref().filter(|c| !c.golden) else { return false };
    let Some(after) = a.after.as_ref() else { return false };
    if !after.hand.iter().any(|m| m.id == card.id && m.golden) {
        return false;
    }
    let owned = before
        .hand
        .iter()
        .chain(&before.board)
        .filter(|m| m.card_id == card.card_id && !m.golden)
        .count();
    owned < 2
}

/// Empty the type a Compass holds, the way its own buy does once the Golden
/// minion it named is bought.
fn forget_compass_types(game: &mut Game, data: &Data) {
    for slot in 0..crate::TRINKET_SLOTS {
        let Some(i) = game.trinket_of(ME, slot) else { continue };
        let makes_golden = data
            .trinkets
            .get(i)
            .on_buy
            .iter()
            .any(|a| matches!(a, crate::trinkets::Act::GoldenBoughtType));
        if makes_golden {
            game.players[ME].trinkets[slot].tribe = None;
        }
    }
}

/// The last Tavern spell the player had cast when step `step` of turn `turn`
/// began, from the steps before it in the same file. The engine keeps it
/// (`Player::last_spell`) and two cards read it (Cataclysmic Harbinger's
/// end of turn, the Galaxy's Lens), but no logged state carries it: only the
/// history does. A spell the player played from hand is a `play` step whose
/// card is a Tavern spell; a spell a minion cast for them never is.
fn last_tavern_spell(
    data: &Data,
    ids: &CardIdMap,
    file: &ShopFileSpec,
    turn: usize,
    step: usize,
) -> Option<CardId> {
    for t in (0..=turn).rev() {
        let Some(turn_spec) = file.turns.get(t) else { continue };
        let last = if t == turn { step } else { turn_spec.actions.len() };
        for a in turn_spec.actions[..last.min(turn_spec.actions.len())].iter().rev() {
            if a.step != "play" {
                continue;
            }
            let Some(card) = a.card.as_ref().and_then(|c| resolve_card(data, ids, c)) else {
                continue;
            };
            if data.cards.get(card).tavern_spell {
                return Some(card);
            }
        }
    }
    None
}

/// The minions bought earlier in the turn step `step` of turn `turn` began,
/// for Glaive Ricochet's "get a plain copy of one of them": the engine keeps
/// them (`Player::recent_buys`) and the turn's own steps are the only place
/// they show, since every step loads a fresh state
/// (`Hearthstone_2026_10_27_04_30_04-g3` turn 10 buys an Imposing
/// Percussionist, a Planar Telescope and a Wrath Weaver, and the third gives
/// the copy). A Tavern spell is not a minion and never counts; the list holds
/// the last three.
fn recent_buys_of_turn(
    data: &Data,
    ids: &CardIdMap,
    file: &ShopFileSpec,
    turn: usize,
    step: usize,
) -> Vec<CardId> {
    let Some(turn_spec) = file.turns.get(turn) else { return Vec::new() };
    let mut out: Vec<CardId> = Vec::new();
    for a in turn_spec.actions[..step.min(turn_spec.actions.len())].iter() {
        if a.step != "buy" {
            continue;
        }
        let Some(card) = a.card.as_ref().and_then(|c| resolve_card(data, ids, c)) else {
            continue;
        };
        let c = data.cards.get(card);
        if c.is_spell() || c.token {
            continue;
        }
        if out.len() == 3 {
            out.remove(0);
        }
        out.push(card);
    }
    out
}

/// Replay one step of one logged turn and judge it.
pub fn replay_step(
    data: &Arc<Data>,
    ids: &CardIdMap,
    file: &ShopFileSpec,
    turn: &ShopTurnSpec,
    i: usize,
    opts: &ReplayOptions,
) -> StepResult {
    let step = &turn.actions[i];
    let mut missing = Vec::new();
    let mut result = StepResult {
        game: file.file(),
        turn: turn.turn,
        index: i,
        kind: step.step.clone(),
        card: step.card.as_ref().and_then(|c| c.card_id.clone()),
        card_name: step.card.as_ref().and_then(|c| c.name.clone()),
        cards: Vec::new(),
        verdict: Verdict::Incomplete,
        missing: Vec::new(),
        diffs: Vec::new(),
        deterministic: true,
        action: String::new(),
        before: ShopSnapshot::default(),
        log_after: ShopSnapshot::default(),
        ours: ShopSnapshot::default(),
    };
    // Every card the step involves, for the per-card tally: the one it acted
    // on, the one it picked (and where that came from), and the hero power or
    // trinket whose button it pressed.
    let mut cards: Vec<String> = result.card.iter().cloned().collect();
    if let Some(p) = step.picked.as_ref().and_then(|p| p.card_id.clone()) {
        cards.push(p);
    }
    if let Some(s) = step.source.as_ref().and_then(|p| p.card_id.clone()) {
        cards.push(s);
    }
    if let Some(t) = step.target.as_ref().and_then(|p| p.card_id.clone()) {
        cards.push(t);
    }
    result.cards = cards;
    let Some(before) = state_before(turn, i) else {
        result.missing.push("no state before the step (the file has no step before it to load)".to_string());
        return result;
    };
    result.before = ShopSnapshot::of_state(data, before, turn.turn, &mut missing);
    let Some(after) = step.after.as_ref() else {
        result.missing.push("no state after the step (a file written before it had one)".to_string());
        return result;
    };
    result.log_after = ShopSnapshot::of_state(data, after, turn.turn, &mut missing);
    // The first run, then the ones that tell a deterministic step from a
    // random one: every seed that gives the same result as the first adds
    // nothing, and the first that differs stops the loop.
    // A one-shot trinket effect the state cannot show: the Warband Whistle's
    // next Refresh.
    let turn_index = file.turns.iter().position(|t| t.turn == turn.turn).unwrap_or(0);
    let armed = whistle_armed(file, turn_index, i);
    let last_spell = last_tavern_spell(data, ids, file, turn_index, i);
    let one = OneStep { file, before, step, turn: turn.turn, turn_index, index: i, armed, last_spell };
    let first = run_once(data, ids, &one, opts.seed);
    let (ours, load_missing, action) = match first {
        Ok(v) => v,
        Err((why, miss)) => {
            missing.extend(miss);
            result.missing.extend(missing);
            result.action = why.clone();
            // A step the engine does not allow is a finding; one whose state
            // or step the loader cannot build is incomplete.
            result.verdict = if why.starts_with("the engine does not allow") {
                Verdict::Illegal
            } else {
                Verdict::Incomplete
            };
            result.diffs.push(why);
            return result;
        }
    };
    missing.extend(load_missing);
    result.ours = ours.clone();
    result.action = action;
    // A hero power press is the one step whose own presses left the game does
    // not count when the press opens a choice.
    let pressed = (step.step == "hero_power").then(|| {
        usize::from(step.which.as_deref() == Some("second"))
    });
    // Every seed, not two: a step whose randomness is a pick (which of this
    // type the Tea Set buffs, which card a Refresh deals) is one of the
    // engine's outcomes when some seed lands on the log's, and the first
    // seed's own outcome says nothing about the others. The shape is the
    // fallback for a space too large to hit (a whole Refresh's cards).
    let log_shape = ShopShape::of(&result.log_after, &result.before);
    let mut deterministic = true;
    let mut one_seed_matches = false;
    let mut one_seed_has_the_shape = compare_pressed(&ours, &result.log_after, pressed).is_empty()
        || ShopShape::of(&ours, &result.before).diffs(&log_shape).is_empty();
    for k in 1..opts.seeds.max(1) {
        match run_once(data, ids, &one, opts.seed.wrapping_add(k as u64)) {
            Ok((snap, _, _)) => {
                if snap != ours {
                    deterministic = false;
                }
                if !one_seed_matches && compare_pressed(&snap, &result.log_after, pressed).is_empty()
                {
                    one_seed_matches = true;
                }
                if !one_seed_has_the_shape
                    && ShopShape::of(&snap, &result.before).diffs(&log_shape).is_empty()
                {
                    one_seed_has_the_shape = true;
                }
            }
            Err(_) => {
                // A seed that cannot even play the step (an illegal action)
                // says nothing about randomness.
                deterministic = false;
            }
        }
    }
    result.deterministic = deterministic;
    if !missing.is_empty() {
        // Whatever the engine does not build in this state: the step is not
        // judged, and the reason is what was missing.
        result.missing.extend(missing);
        result.verdict = Verdict::Incomplete;
        return result;
    }
    let diffs = compare_pressed(&ours, &result.log_after, pressed);
    result.diffs = diffs.clone();
    if deterministic {
        result.verdict = if diffs.is_empty() { Verdict::Ok } else { Verdict::Flagged };
        return result;
    }
    if diffs.is_empty() || one_seed_matches {
        result.verdict = Verdict::OkRandom;
        return result;
    }
    // A random step with a large random space (a Refresh, a random minion):
    // the log's result can only be compared by shape.
    result.diffs = ShopShape::of(&ours, &result.before).diffs(&log_shape);
    result.verdict = if one_seed_has_the_shape { Verdict::OkShape } else { Verdict::Flagged };
    result
}

// ---- the run over a folder, and what it writes ----

/// One run over every step of every game in a folder.
#[derive(Clone, Debug, Default)]
pub struct ShopReplayReport {
    pub entries: Vec<StepResult>,
    pub files: usize,
    pub turns: usize,
    /// Games whose log could not tell something (the file's `unread`).
    pub unread: Vec<String>,
    /// The cards no source knows, by name.
    pub unknown: Vec<String>,
}

impl ShopReplayReport {
    pub fn steps(&self) -> usize {
        self.entries.len()
    }

    pub fn count(&self, v: Verdict) -> usize {
        self.entries.iter().filter(|e| e.verdict == v).count()
    }

    pub fn deterministic(&self) -> usize {
        self.entries.iter().filter(|e| e.deterministic).count()
    }

    /// The verdicts, most common first, for the summary line.
    pub fn verdict_counts(&self) -> Vec<(&'static str, usize)> {
        let mut out = Vec::new();
        for v in [
            Verdict::Ok,
            Verdict::OkRandom,
            Verdict::OkShape,
            Verdict::Incomplete,
            Verdict::Flagged,
            Verdict::Illegal,
        ] {
            out.push((v.name(), self.count(v)));
        }
        out
    }

    /// The steps of each kind, with how many were flagged and incomplete.
    fn by_kind(&self) -> Vec<(String, usize, usize, usize)> {
        let mut out: Vec<(String, usize, usize, usize)> = Vec::new();
        for e in &self.entries {
            let at = match out.iter_mut().find(|(k, ..)| *k == e.kind) {
                Some(row) => row,
                None => {
                    out.push((e.kind.clone(), 0, 0, 0));
                    out.last_mut().expect("just pushed")
                }
            };
            at.1 += 1;
            if e.verdict == Verdict::Flagged || e.verdict == Verdict::Illegal {
                at.2 += 1;
            }
            if e.verdict == Verdict::Incomplete {
                at.3 += 1;
            }
        }
        out.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        out
    }

    /// The causes differences are grouped by, most common first: the number in
    /// front of a difference names what it is about.
    fn diff_causes(&self) -> Vec<(String, usize, String)> {
        let mut out: Vec<(String, usize, String)> = Vec::new();
        for e in &self.entries {
            if e.verdict != Verdict::Flagged && e.verdict != Verdict::Illegal {
                continue;
            }
            // One count per step and cause: a step can differ in the same way
            // many times over (a Tavern of ten cards), and the family is what
            // the report is about.
            let mut seen: Vec<String> = Vec::new();
            for d in &e.diffs {
                let cause = diff_cause(d);
                if seen.contains(&cause) {
                    continue;
                }
                seen.push(cause.clone());
                match out.iter_mut().find(|(c, ..)| *c == cause) {
                    Some(row) => row.1 += 1,
                    None => out.push((cause, 1, d.clone())),
                }
            }
        }
        out.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        out
    }

    /// The why of every incomplete step, most common first.
    fn missing_causes(&self) -> Vec<(String, usize)> {
        let mut out: Vec<(String, usize)> = Vec::new();
        for e in &self.entries {
            if e.verdict != Verdict::Incomplete {
                continue;
            }
            let mut seen: Vec<String> = Vec::new();
            for line in &e.missing {
                let head = missing_kind(line);
                if seen.contains(&head) {
                    continue;
                }
                seen.push(head.clone());
                match out.iter_mut().find(|(c, _)| *c == head) {
                    Some(row) => row.1 += 1,
                    None => out.push((head, 1)),
                }
            }
        }
        out.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        out
    }

    /// The cards, hero powers, trinkets and Dark Gifts the flagged steps share,
    /// most suspicious first.
    pub fn pieces(&self, data: &Data) -> Vec<PieceTally> {
        // The names the file gave the cards a step acted on, so a card the
        // engine's tables do not name is still named in the report.
        let mut names: HashMap<&str, &str> = HashMap::new();
        for e in &self.entries {
            if let (Some(id), Some(name)) = (e.card.as_deref(), e.card_name.as_deref()) {
                names.entry(id).or_insert(name);
            }
        }
        let mut out: Vec<PieceTally> = Vec::new();
        for e in &self.entries {
            let mut seen: Vec<&String> = Vec::new();
            for card in &e.cards {
                if seen.contains(&card) {
                    continue;
                }
                seen.push(card);
                let kind = piece_kind(data, card);
                let at = match out.iter_mut().find(|t| &t.id == card && t.kind == kind) {
                    Some(t) => t,
                    None => {
                        let name = names
                            .get(card.as_str())
                            .map(|n| n.to_string())
                            .filter(|n| !n.is_empty())
                            .unwrap_or_else(|| piece_name(data, card));
                        out.push(PieceTally { id: card.clone(), name, kind, steps: 0, flagged: 0 });
                        out.last_mut().expect("just pushed")
                    }
                };
                at.steps += 1;
                if e.verdict == Verdict::Flagged || e.verdict == Verdict::Illegal {
                    at.flagged += 1;
                }
            }
        }
        out.sort_by(|a, b| {
            b.ratio()
                .partial_cmp(&a.ratio())
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(b.flagged.cmp(&a.flagged))
                .then(b.steps.cmp(&a.steps))
                .then(a.name.cmp(&b.name))
        });
        out
    }

    /// One row per step: the game, turn and step, what it was, the card, the
    /// verdict and the first difference.
    pub fn steps_tsv(&self) -> String {
        let mut out = String::from("game\tturn\tstep\tkind\tcard\tverdict\tdeterministic\tfirst difference\n");
        for e in &self.entries {
            out.push_str(&format!(
                "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                e.game,
                e.turn,
                e.index,
                e.kind,
                e.card.clone().unwrap_or_default(),
                e.verdict.name(),
                if e.deterministic { "yes" } else { "no" },
                clean(&e.first())
            ));
        }
        out
    }

    /// Per card, hero power, trinket and Dark Gift: the steps it was in, how
    /// many were flagged, and the ratio.
    pub fn cards_tsv(&self, data: &Data) -> String {
        let mut out = String::from("card\tname\tkind\tsteps\tflagged\tratio\n");
        for t in self.pieces(data) {
            out.push_str(&format!(
                "{}\t{}\t{}\t{}\t{}\t{:.3}\n",
                t.id,
                t.name,
                t.kind,
                t.steps,
                t.flagged,
                t.ratio()
            ));
        }
        out
    }

    /// The summary: what was checked, how it went, why steps were incomplete,
    /// the differences grouped by cause, and examples.
    pub fn report_md(&self, data: &Data) -> String {
        let mut out = String::new();
        out.push_str("# The shop replay\n\n");
        out.push_str(&format!(
            "{} games, {} turns, {} steps replayed from the logged state before \
             each one.\n\n",
            self.files,
            self.turns,
            self.steps()
        ));
        out.push_str("| verdict | steps |\n|---|---|\n");
        for (name, n) in self.verdict_counts() {
            out.push_str(&format!("| {name} | {n} |\n"));
        }
        out.push_str(&format!(
            "\n{} steps were deterministic (every seed gave the same state) and {} \
             random (the seeds differ).\n\n",
            self.deterministic(),
            self.steps() - self.deterministic()
        ));
        let flagged = self.entries.iter().filter(|e| e.verdict == Verdict::Flagged).count();
        let illegal = self.entries.iter().filter(|e| e.verdict == Verdict::Illegal).count();
        out.push_str(&format!(
            "**{flagged} steps flagged** and {illegal} the engine does not allow, of \
             {} judged.\n\n",
            self.steps() - self.count(Verdict::Incomplete)
        ));

        out.push_str("## By step kind\n\n| step | steps | flagged | incomplete |\n|---|---|---|---|\n");
        for (kind, steps, flagged, incomplete) in self.by_kind() {
            out.push_str(&format!("| {kind} | {steps} | {flagged} | {incomplete} |\n"));
        }

        out.push_str("\n## Why steps are incomplete\n\n");
        let causes = self.missing_causes();
        if causes.is_empty() {
            out.push_str("Nothing was missing.\n");
        } else {
            out.push_str("| missing | steps |\n|---|---|\n");
            for (cause, n) in causes.iter().take(30) {
                out.push_str(&format!("| {cause} | {n} |\n"));
            }
        }

        out.push_str("\n## The most common differences\n\n");
        let diffs = self.diff_causes();
        if diffs.is_empty() {
            out.push_str("No step differed.\n");
        } else {
            out.push_str("| cause | steps | an example |\n|---|---|---|\n");
            for (cause, n, example) in diffs.iter().take(40) {
                out.push_str(&format!("| {cause} | {n} | {} |\n", clean(example)));
            }
        }

        out.push_str("\n## The 30 most suspicious cards\n\n");
        let pieces = self.pieces(data);
        if pieces.is_empty() {
            out.push_str("No card was in a flagged step.\n");
        } else {
            out.push_str("| card | kind | steps | flagged | ratio |\n|---|---|---|---|---|\n");
            for t in pieces.iter().take(30) {
                out.push_str(&format!(
                    "| {} | {} | {} | {} | {:.1}% |\n",
                    t.name,
                    t.kind,
                    t.steps,
                    t.flagged,
                    t.ratio() * 100.0
                ));
            }
        }

        out.push_str("\n## Flagged steps\n\n");
        out.push_str(
            "One example per kind of difference first, then the rest, so every family is \
             visible.\n\n",
        );
        let flagged: Vec<&StepResult> = self
            .entries
            .iter()
            .filter(|e| e.verdict == Verdict::Flagged || e.verdict == Verdict::Illegal)
            .collect();
        let mut examples: Vec<&StepResult> = Vec::new();
        for (cause, ..) in &diffs {
            if let Some(e) = flagged
                .iter()
                .find(|e| e.diffs.first().is_some_and(|d| &diff_cause(d) == cause))
            {
                examples.push(e);
            }
        }
        for e in &flagged {
            if examples.len() >= 30 {
                break;
            }
            if !examples.iter().any(|x| std::ptr::eq(*x, *e)) {
                examples.push(e);
            }
        }
        if examples.is_empty() {
            out.push_str("None.\n");
        }
        for e in examples {
            out.push_str(&format!(
                "### {} turn {} step {}: {} {}\n\n",
                e.game,
                e.turn,
                e.index,
                e.kind,
                e.card.clone().unwrap_or_default()
            ));
            out.push_str(&format!("The engine did: `{}`\n\n", e.action));
            out.push_str(&format!(
                "- {}\n",
                e.diffs.iter().take(6).cloned().collect::<Vec<_>>().join("\n- ")
            ));
            out.push_str("\nBefore:\n\n");
            out.push_str(&state_md(&e.before));
            out.push_str("\nThe log's result:\n\n");
            out.push_str(&state_md(&e.log_after));
            out.push_str("\nOurs:\n\n");
            out.push_str(&state_md(&e.ours));
            out.push('\n');
        }
        if !self.unread.is_empty() {
            out.push_str("\n## What the logs could not tell\n\n");
            for line in &self.unread {
                out.push_str(&format!("- {line}\n"));
            }
        }
        out
    }
}

/// A card, hero power, trinket or Dark Gift a step involved, and how many of
/// the steps it was in were flagged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PieceTally {
    pub id: String,
    pub name: String,
    pub kind: &'static str,
    pub steps: usize,
    pub flagged: usize,
}

impl PieceTally {
    pub fn ratio(&self) -> f64 {
        if self.steps == 0 { 0.0 } else { self.flagged as f64 / self.steps as f64 }
    }
}

fn piece_kind(data: &Data, card_id: &str) -> &'static str {
    if data.heroes.power_by_card_id(card_id).is_some() {
        "hero power"
    } else if data.trinkets.by_card_id(card_id).is_some() {
        "trinket"
    } else if data.gifts.gifts.iter().any(|g| g.id == card_id) {
        "dark gift"
    } else {
        "card"
    }
}

fn piece_name(data: &Data, card_id: &str) -> String {
    if let Some(i) = data.heroes.power_by_card_id(card_id) {
        return data.heroes.power(i).name.clone();
    }
    if let Some(i) = data.trinkets.by_card_id(card_id) {
        return data.trinkets.get(i).name.clone();
    }
    if let Some(g) = data.gifts.gifts.iter().find(|g| g.id == card_id) {
        return g.name.clone();
    }
    card_id.to_string()
}

/// What a difference is about, for grouping: the two words every line of
/// `compare` and of the shape comparison starts with (`board stats`,
/// `Tavern keywords`, `Gold`, `hero power`). A line of the shape comparison
/// (a random step) is grouped by what it is about instead, since it starts
/// with a card's name.
fn diff_cause(diff: &str) -> String {
    if diff.ends_with("(shape)") {
        let head = diff.split_whitespace().next().unwrap_or("");
        return match head {
            "Gold" | "Health" | "armor" | "tier" | "upgrade" | "free" | "Tavern" => {
                format!("a random step's {head}")
            }
            "board" | "hand" => format!("a random step's {head} count"),
            _ => "a random step's change".to_string(),
        };
    }
    let first = diff.split_whitespace().next().unwrap_or("");
    if matches!(
        first,
        "Gold" | "Health" | "armor" | "tier" | "trinkets" | "Secrets" | "free" | "upgrade"
    ) {
        return first.to_string();
    }
    let words: Vec<&str> = diff.split_whitespace().take(2).collect();
    words.join(" ").trim_end_matches(':').to_string()
}

/// The kind of a "what was missing" line, as the summary counts it: the part
/// before the colon.
pub fn missing_kind(line: &str) -> String {
    match line.split_once(':') {
        Some((head, _)) => head.to_string(),
        None => line.split_whitespace().take(4).collect::<Vec<_>>().join(" "),
    }
}

/// A state as three lines of cards, for the report's examples.
fn state_md(s: &ShopSnapshot) -> String {
    let zone = |name: &str, cards: &Vec<SnapMinion>| {
        if cards.is_empty() {
            format!("- {name}: nothing\n")
        } else {
            format!(
                "- {name}: {}\n",
                cards.iter().map(|c| c.text()).collect::<Vec<_>>().join(", ")
            )
        }
    };
    let mut out = String::new();
    out.push_str(&format!(
        "- Gold {}, Health {} (armor {}), tier {} (upgrade {}), {} free Refreshes{}\n",
        s.gold,
        s.health,
        s.armor,
        s.tier,
        opt_text(s.upgrade_cost),
        s.free_refreshes,
        if s.frozen { ", frozen" } else { "" }
    ));
    out.push_str(&zone("board", &s.board));
    out.push_str(&zone("hand", &s.hand));
    out.push_str(&zone("Tavern", &s.tavern));
    if !s.powers.is_empty() {
        out.push_str(&format!(
            "- hero power: {}\n",
            s.powers
                .iter()
                .map(|p| format!("{} ({} left)", p.card_id, p.uses_left))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if !s.trinkets.is_empty() {
        out.push_str(&format!("- trinkets: {}\n", trinket_text(&s.trinkets)));
    }
    out
}

/// One line of a cell: the tabs and newlines a difference could carry would
/// break the tables.
fn clean(text: &str) -> String {
    text.replace(['\t', '\n'], " ").replace('|', "/")
}

/// Replay every step of one logged turn (the driver in `bgbench shopreplay`
/// runs a turn per core).
pub fn replay_turn(
    data: &Arc<Data>,
    ids: &CardIdMap,
    file: &ShopFileSpec,
    turn: &ShopTurnSpec,
    opts: &ReplayOptions,
    card_filter: Option<&str>,
) -> Vec<StepResult> {
    let mut out = Vec::new();
    for i in 0..turn.actions.len() {
        let step = &turn.actions[i];
        if let Some(name) = card_filter
            && !step_involves(data, ids, step, name)
        {
            continue;
        }
        out.push(replay_step(data, ids, file, turn, i, opts));
    }
    out
}

/// Whether a step involves one card, by name or card id: the card it acted on,
/// the one it picked, its source, its target, or a card the file says it
/// changed (`--card NAME`).
fn step_involves(data: &Data, ids: &CardIdMap, step: &ShopStepSpec, name: &str) -> bool {
    let mut places: Vec<&ShopPlace> = Vec::new();
    places.extend(step.card.as_ref());
    places.extend(step.picked.as_ref());
    places.extend(step.source.as_ref());
    places.extend(step.target.as_ref());
    for place in places {
        if place.card_id.as_deref() == Some(name) || place.name.as_deref() == Some(name) {
            return true;
        }
        if let Some(card) = resolve_card(data, ids, place)
            && data.cards.get(card).name == name
        {
            return true;
        }
    }
    false
}

/// The name of every minion no card matches, from a state (the report says
/// which cards the pool could not build).
pub fn unknown_minions(data: &Data, spec: &ShopStateSpec) -> Vec<String> {
    let mut out = Vec::new();
    for m in spec.board.iter().chain(&spec.hand).chain(&spec.tavern) {
        let (_, bad) = build_minion(data, m, 1);
        if let Some(name) = bad
            && !out.contains(&name)
        {
            out.push(name);
        }
    }
    out
}
