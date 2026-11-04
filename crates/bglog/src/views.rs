//! What a Battlegrounds game looks like from the log: the lobby, and the
//! boards at the start of every fight.
//!
//! A port of the older tracker's `battlegrounds.py` (its types and its free
//! functions; the reader that follows the lines is [`crate::Tracker`]). Every
//! function here is a pure view of one [`Game`] at one moment.
//!
//! The one fix over the Python: **Dark Gifts are counted per copy.** A minion
//! can carry two of the same gift (a golden that kept two Toughened Shields),
//! and the Python kept one id per gift, so it lost the second. See
//! [`gifts_by_minion`].

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::cards::Cards;
use crate::entity::{Entity, TagValue};
use crate::game::Game;

/// The combat keywords a minion can have. The game's tags are off when 0.
pub const KEYWORDS: [&str; 8] = [
    "TAUNT",
    "DIVINE_SHIELD",
    "REBORN",
    "VENOMOUS",
    "POISONOUS",
    "WINDFURY",
    "STEALTH",
    "CLEAVE",
];

/// One minion on a board or in a hand, with the numbers the log had at the
/// moment it was read.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Minion {
    pub card_id: String,
    pub name: String,
    pub attack: i64,
    pub health: i64,
    pub tier: i64,
    pub golden: bool,
    pub keywords: Vec<&'static str>,
    /// Dark Gift card ids on this minion, one entry per gift it carries (see
    /// [`gifts_by_minion`]), the pointed gift first when the log points at one.
    pub dark_gifts: Vec<String>,
    /// Hero power enchantments on it (see [`hero_enchants_by_minion`]).
    pub hp_enchants: Vec<String>,
    /// Every tag of the entity, as it stood.
    pub tags: HashMap<String, TagValue>,
}

/// The minions on a board or in a hand, in position order.
pub type Board = Vec<Minion>;

/// A player's hero power at the first attack, see [`hero_power`].
#[derive(Clone, Debug, PartialEq)]
pub struct HeroPower {
    pub card_id: String,
    pub name: String,
    pub turn: i64,
    /// Its current numbers, `TAG_SCRIPT_DATA_NUM_1..4` (the `{0} {1} {2} {3}`
    /// of its text).
    pub numbers: [i64; 4],
    /// A minion the power still owes the board: set by Lock and Load and Rapid
    /// Reanimation when they were used this turn and have not summoned it yet.
    pub waiting: Option<Minion>,
}

/// A trinket a player holds, see [`trinkets`].
#[derive(Clone, Debug, PartialEq)]
pub struct Trinket {
    pub card_id: String,
    pub name: String,
    /// The Greater slot (`TAG_SCRIPT_DATA_NUM_6` is 2), else the Lesser one.
    pub greater: bool,
    /// The `{0} {1} {2} {3}` of its text: `TAG_SCRIPT_DATA_NUM_1..4`.
    pub numbers: [i64; 4],
}

/// A quest reward a player holds, see [`rewards`].
#[derive(Clone, Debug, PartialEq)]
pub struct Reward {
    pub card_id: String,
    pub name: String,
    pub numbers: [i64; 4],
}

/// A player's Deity at the first attack, see [`deity`].
#[derive(Clone, Debug, PartialEq)]
pub struct Deity {
    /// The minion it awakens (`BGFYM_000` C'Thun, `BGFYM_011` Y'Shaarj, `_G`
    /// for golden).
    pub card_id: String,
    pub name: String,
    /// The stats it awakens with.
    pub attack: i64,
    pub health: i64,
    /// The friendly Aberration deaths it still needs this combat (0 once spent).
    pub left: i64,
}

/// One fight of one game: the boards the window shows, then the boards at the
/// first attack, which is where the simulator starts from.
#[derive(Clone, Debug, PartialEq)]
pub struct Fight {
    pub turn: i64,
    /// Their PlayerID.
    pub opponent: i32,
    /// Both boards before start-of-combat effects: what the window shows.
    pub mine: Board,
    pub theirs: Board,
    /// The Secret card ids in each side's Secret zone as the fight was staged,
    /// mine and theirs (see [`secrets`]).
    pub secrets: (Vec<String>, Vec<String>),
    /// Which of each side's minions carried Rapid Reanimation's mark as the
    /// fight was staged, in board order, mine and theirs (see [`reanimates`]).
    pub reanimates: (Vec<bool>, Vec<bool>),
    /// Both boards at the first attack, after start-of-combat effects, and who
    /// swings first.
    pub at_attack: Option<(Board, Board)>,
    /// "me" or "them", empty until the first attack.
    pub first: String,
    /// One Blood Gem, mine and theirs.
    pub gems: ((i64, i64), (i64, i64)),
    /// Both hands at the first attack: cards like Diremuck Forager summon out of
    /// them. `None` when not captured. `hidden` counts hand cards the log shows
    /// without a card id; a hand with any is not fully known.
    pub hand: Option<Board>,
    pub their_hand: Option<Board>,
    pub hidden: (i64, i64),
    /// Diremuck summons still waiting for space, mine and theirs.
    pub waits: (i64, i64),
    /// The minion Lock and Load still owes each side's board, read off the copy
    /// the power summoned as it came in (see [`crate::Tracker`]), mine and
    /// theirs. `None` for a side whose power is not Lock and Load or that owes
    /// nothing.
    pub waiting: (Option<Minion>, Option<Minion>),
    /// Both hero powers at the first attack, mine and theirs.
    pub hero_powers: (Option<HeroPower>, Option<HeroPower>),
    /// The minion that makes the first attack when a hero power sent it in
    /// (Lock and Load's shot): (side, index on the board at the first attack),
    /// side 0 for me and 1 for them. That attack comes before the normal order.
    pub opening: Option<(u8, usize)>,
    /// The trinkets each player has at the first attack, mine and theirs.
    pub trinkets: (Vec<Trinket>, Vec<Trinket>),
    /// The quest rewards each player has at the first attack.
    pub rewards: (Vec<Reward>, Vec<Reward>),
    /// Game-long counts per side at the first attack, see [`counts`].
    pub counters: (BTreeMap<&'static str, i64>, BTreeMap<&'static str, i64>),
    /// The Upper Hands each side cast before this fight, mine and theirs (see
    /// [`UPPER_HAND`]).
    pub upper_hands: (i64, i64),
    /// What the log shows of each side's player at the first attack, mine and
    /// theirs (see [`side_numbers`]).
    pub side_numbers: (BTreeMap<&'static str, i64>, BTreeMap<&'static str, i64>),
    /// Both Deities at the first attack.
    pub deities: (Option<Deity>, Option<Deity>),
    /// "win", "lose" or "tie" once the fight is over, empty before that.
    pub result: String,
    /// Win, tie and lose from the simulator. Not from the log: the Python's
    /// collector fills it in, and so will ours.
    pub odds: Option<(f64, f64, f64)>,
    /// Cards in the fight whose effect the simulator lacks. Not from the log.
    pub gaps: Vec<String>,
}

impl Fight {
    pub fn new(turn: i64, opponent: i32, mine: Board, theirs: Board) -> Fight {
        Fight {
            turn,
            opponent,
            mine,
            theirs,
            secrets: (Vec::new(), Vec::new()),
            reanimates: (Vec::new(), Vec::new()),
            at_attack: None,
            first: String::new(),
            gems: ((1, 1), (1, 1)),
            hand: None,
            their_hand: None,
            hidden: (0, 0),
            waits: (0, 0),
            waiting: (None, None),
            hero_powers: (None, None),
            opening: None,
            trinkets: (Vec::new(), Vec::new()),
            rewards: (Vec::new(), Vec::new()),
            counters: (BTreeMap::new(), BTreeMap::new()),
            upper_hands: (0, 0),
            side_numbers: (BTreeMap::new(), BTreeMap::new()),
            deities: (None, None),
            result: String::new(),
            odds: None,
            gaps: Vec::new(),
        }
    }
}

/// One lobby seat: the leaderboard hero of one player.
#[derive(Clone, Debug, PartialEq)]
pub struct Seat {
    pub player_id: i32,
    pub hero: String,
    pub hero_card: String,
    pub health: i64,
    pub armor: i64,
    pub tier: i64,
    pub place: i64,
    pub name: String,
    /// The fight this seat was in, as an index into the game's fights (the
    /// Python holds the Fight itself; indexing keeps one copy of it).
    pub last_fight: Option<usize>,
}

impl Seat {
    pub fn new(player_id: i32) -> Seat {
        Seat {
            player_id,
            hero: String::new(),
            hero_card: String::new(),
            health: 0,
            armor: 0,
            tier: 0,
            place: 0,
            name: String::new(),
            last_fight: None,
        }
    }

    pub fn dead(&self) -> bool {
        self.health <= 0
    }
}

/// The Battlegrounds round. The game counts recruit and combat as separate
/// turns.
pub fn turn_of(g: &Game) -> i64 {
    (g.game_tag("TURN") + 1) / 2
}

/// The gift a Dark Gift enchantment's card id names: `BG36_MidGameEffect_000t75e`
/// and its numbered copies are Toughened Shield. `None` for any other card.
fn gift_enchantment(card_id: &str) -> Option<&str> {
    let rest = card_id.strip_prefix("BG36_MidGameEffect_000t")?;
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    let tail = &rest[digits..];
    let tail = tail.strip_prefix('t').unwrap_or(tail);
    let tail = tail.strip_prefix('e')?;
    if !tail.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    // Group 1 stops at the "e" the numbers hang off.
    Some(&card_id[..card_id.len() - tail.len() - 1])
}

/// Dark Gift card ids per minion entity id, one entry per gift copy the minion
/// carries.
///
/// A gift reaches a minion two ways. `DARK_GIFT_ENTITY` points at the gift, but
/// the copies made for a fight mostly lack it. The gift's enchantment (a
/// `BG36_MidGameEffect_000t...` card) is attached to the minion in both cases,
/// and its id names the gift. A minion can carry the same gift twice: a golden
/// made from two copies that each had the same gift carries two, and this
/// counts both. (The Python kept one id per gift, `if m[1] not in ids`.)
///
/// Of the gift family only the **carriers** are gifts; the rest are the buffs a
/// gift leaves behind. A carrier is an enchantment that carries the gift tag
/// the game puts on a Dark Gift (`IS_NIGHTMARE_BONUS`, tag 3567); a gift whose
/// enchantments carry no tag at all while it is in play (Double Vision's
/// `BG36_MidGameEffect_000t11e2` never does) falls back to the ones the gift
/// card itself created (`CREATOR` with the gift's card id). Buffs are made by
/// the minion or by another minion's effect and carry no tag, and the ones the
/// gift card makes for itself carry it on a second enchantment (Offensive
/// Sacrifice's `...te2` carrier and `...te3` +0/+0 buff both name the gift card
/// as creator), so they are not counted.
///
/// One gift application can leave two carrier enchantments: Incubation puts
/// both `...t4e` and `...t4e2` ("In 2 turns, double this minion's stats") on
/// the minion. Carriers that share a `CREATOR` and were made fresh came from
/// one gift card and are one copy. Checked on all 20 of Era's logs, this
/// is the only rule tried that counts no buff, loses no carrier, and gives the
/// golden Clever Castaway of `Hearthstone_2026_11_02_00_10_22` game 5 (entity
/// 10380) its two Toughened Shields (carriers 10381 and 10384, created by the
/// two gift cards 1114 and 1742) rather than the four enchantments attached to
/// it.
///
/// A carrier the game **copied** is its own copy, even when the copy and its
/// original name the same creator. A triple's golden takes its copies' carriers
/// along: the golden Cadaver Caretaker of `Hearthstone_2026_10_26_00_48_15-g2`
/// turn 7 carries two Replication carriers (4025 and 4026, both created by the
/// gift card 1720, copied from 3770 and 3771), and counting creators alone made
/// them one. So the copy is keyed by creator **and** `COPIED_FROM_ENTITY_ID`,
/// which is 0 on a carrier the game made fresh (two carriers of one
/// application share it and stay one copy) and the id of the carrier it came
/// from on a copy.
pub fn gifts_by_minion(g: &Game) -> HashMap<i32, Vec<String>> {
    // The gift families with a tagged enchantment in play: for those the tag
    // alone picks the carriers.
    let mut tagged: HashSet<String> = HashSet::new();
    for x in g.entities_in_order() {
        if let Some(gift) = gift_enchantment(&x.card_id)
            && x.text("ZONE") == "PLAY"
            && x.int(GIFT_TAG) != 0
        {
            tagged.insert(gift.to_string());
        }
    }
    // (minion, gift) -> where its carriers came from, one entry per gift copy,
    // in the order the first carrier of each came.
    let mut origins: HashMap<(i32, String), HashSet<(i32, i32)>> = HashMap::new();
    let mut order: Vec<(i32, String)> = Vec::new();
    for x in g.entities_in_order() {
        let Some(gift) = gift_enchantment(&x.card_id) else {
            continue;
        };
        if x.text("ZONE") != "PLAY" {
            continue;
        }
        let attached = x.int("ATTACHED");
        if attached == 0 {
            continue;
        }
        let carrier = x.int(GIFT_TAG) != 0
            || (!tagged.contains(gift) && created_by_gift_card(g, x, gift));
        if !carrier {
            continue;
        }
        // An enchantment with no creator stands on its own (ids are positive).
        let creator = match x.int("CREATOR") {
            0 => -x.id,
            creator => creator as i32,
        };
        let copied_from = x.int("COPIED_FROM_ENTITY_ID") as i32;
        let key = (attached as i32, gift.to_string());
        let seen = origins.entry(key.clone()).or_insert_with(|| {
            order.push(key);
            HashSet::new()
        });
        seen.insert((creator, copied_from));
    }
    let mut out: HashMap<i32, Vec<String>> = HashMap::new();
    for (minion, gift) in order {
        let copies = origins[&(minion, gift.clone())].len();
        let gifts = out.entry(minion).or_default();
        for _ in 0..copies {
            gifts.push(gift.clone());
        }
    }
    out
}

/// The tag the game puts on a Dark Gift's own enchantment.
pub const GIFT_TAG: &str = "IS_NIGHTMARE_BONUS";

/// Whether the gift card itself created this enchantment (a `CREATOR` whose
/// card id is the gift's, which the fight copies may point at an entity the log
/// never made).
fn created_by_gift_card(g: &Game, x: &Entity, gift: &str) -> bool {
    let creator = x.int("CREATOR");
    creator != 0
        && g.entities
            .get(&(creator as i32))
            .map(|creator| creator.card_id == gift)
            .unwrap_or(false)
}

/// Enchantments a hero power leaves on minions to give them a Deathrattle:
/// Murloc King, Earth Invocation, Fragrant Phylactery. Their ids come from the
/// card data; none of the three is in Era's logs yet, so the ids are unchecked.
pub const HERO_ENCHANTMENTS: [&str; 3] =
    ["TB_BaconShop_HP_017e", "BG22_HERO_001p_t1e", "BG20_HERO_282e2"];

/// Hero power enchantment card ids per minion entity id.
pub fn hero_enchants_by_minion(g: &Game) -> HashMap<i32, Vec<String>> {
    let mut out: HashMap<i32, Vec<String>> = HashMap::new();
    for x in g.entities_in_order() {
        if !HERO_ENCHANTMENTS.contains(&x.card_id.as_str()) {
            continue;
        }
        if x.text("ZONE") != "PLAY" {
            continue;
        }
        let attached = x.int("ATTACHED");
        if attached == 0 {
            continue;
        }
        out.entry(attached as i32)
            .or_default()
            .push(x.card_id.clone());
    }
    out
}

/// One minion, with the Dark Gifts and hero power enchantments the log attached
/// to it. The two maps save recomputing them for every minion of a board.
pub fn minion(
    e: &Entity,
    g: &Game,
    cards: &Cards,
    gifts: &HashMap<i32, Vec<String>>,
    enchants: &HashMap<i32, Vec<String>>,
) -> Minion {
    let mut own: Vec<String> = gifts.get(&e.id).cloned().unwrap_or_default();
    let pointed = e.int("DARK_GIFT_ENTITY");
    if pointed != 0
        && let Some(pointed) = g.entities.get(&(pointed as i32))
        && !pointed.card_id.is_empty()
        && !own.contains(&pointed.card_id)
    {
        own.insert(0, pointed.card_id.clone());
    }
    Minion {
        card_id: e.card_id.clone(),
        name: cards.name(&e.card_id).to_string(),
        attack: e.int("ATK"),
        health: e.int("HEALTH") - e.int("DAMAGE"),
        tier: e.int("TECH_LEVEL"),
        golden: e.int("PREMIUM") != 0 || e.card_id.ends_with("_G"),
        keywords: KEYWORDS.iter().copied().filter(|k| e.int(k) != 0).collect(),
        dark_gifts: own,
        hp_enchants: enchants.get(&e.id).cloned().unwrap_or_default(),
        tags: e.tags.clone(),
    }
}

/// One minion, computed on its own (the maps are built here). The Python's
/// `minion(e, g)` with no maps given.
pub fn one_minion(e: &Entity, g: &Game, cards: &Cards) -> Minion {
    minion(e, g, cards, &gifts_by_minion(g), &hero_enchants_by_minion(g))
}

/// What one Blood Gem gives this player's minions. 0 means the tag was never
/// set: a plain +1/+1.
pub fn gem(player: Option<&Entity>) -> (i64, i64) {
    let Some(player) = player else {
        return (1, 1);
    };
    (
        or_one(player.int("BACON_BLOODGEMBUFFATKVALUE")),
        or_one(player.int("BACON_BLOODGEMBUFFHEALTHVALUE")),
    )
}

fn or_one(n: i64) -> i64 {
    if n == 0 { 1 } else { n }
}

/// Whether an entity is under a controller. `None` is the Python's `true` for
/// no player at all, which no entity has.
fn controlled_by(e: &Entity, controller: Option<i32>) -> bool {
    match controller {
        Some(controller) => e.int("CONTROLLER") == controller as i64,
        None => false,
    }
}

/// The minions in a player's hand that a fight can summon.
///
/// The opponent's hand is in the log too: during a fight the stand-in player
/// holds copies of it. Cards the game marks `LITERALLY_UNPLAYABLE` are left
/// out: they cannot be played or summoned yet (Ambassador Faelin's hero power
/// holds three such minions until he reaches Tiers 2, 4 and 6).
pub fn hand(g: &Game, cards: &Cards, controller: Option<i32>) -> Board {
    let gifts = gifts_by_minion(g);
    let enchants = hero_enchants_by_minion(g);
    let mut ms: Vec<&Entity> = g
        .entities_in_order()
        .filter(|x| {
            x.text("CARDTYPE") == "MINION"
                && x.text("ZONE") == "HAND"
                && controlled_by(x, controller)
                && x.int("LITERALLY_UNPLAYABLE") == 0
        })
        .collect();
    ms.sort_by_key(|x| x.int("ZONE_POSITION"));
    ms.iter()
        .map(|x| minion(x, g, cards, &gifts, &enchants))
        .collect()
}

/// Hand cards whose identity the log does not show (no card id).
pub fn hidden_in_hand(g: &Game, controller: Option<i32>) -> i64 {
    g.entities_in_order()
        .filter(|x| {
            x.text("ZONE") == "HAND" && controlled_by(x, controller) && x.card_id.is_empty()
        })
        .count() as i64
}

/// The Secret cards a player can hold in the Secret zone: Venomstrike Trap,
/// Autodefense Matrix, Redemption, Pack Tactics and the Ice Block a Safety
/// Patch gives. The zone also holds the Deity and quests, which are not
/// Secrets.
pub const SECRET_CARDS: [&str; 5] = [
    "TB_Bacon_Secrets_01",
    "TB_Bacon_Secrets_07",
    "TB_Bacon_Secrets_10",
    "TB_Bacon_Secrets_15",
    "TB_Bacon_Secrets_12",
];

/// The Secret card ids in a player's Secret zone, in entity id order (which is
/// the order they fire in).
pub fn secrets(g: &Game, controller: Option<i32>) -> Vec<String> {
    let mut xs: Vec<&Entity> = g
        .entities_in_order()
        .filter(|x| {
            x.text("ZONE") == "SECRET"
                && controlled_by(x, controller)
                && SECRET_CARDS.contains(&x.card_id.as_str())
        })
        .collect();
    xs.sort_by_key(|x| x.id);
    xs.iter().map(|x| x.card_id.clone()).collect()
}

/// Diremuck Forager summons this player still has waiting for a free slot.
///
/// Its Start of Combat puts an enchantment (`BG27_556pe`) on the player, which
/// stays in PLAY until the summon happens. The golden one summons two; its
/// enchantment id is a guess (`BG27_556_Gpe`).
pub fn waiting(g: &Game, player: Option<&Entity>) -> i64 {
    let Some(player) = player else {
        return 0;
    };
    let mut n = 0;
    for x in g.entities_in_order() {
        if x.card_id.starts_with("BG27_556")
            && x.card_id.ends_with("pe")
            && x.text("ZONE") == "PLAY"
            && x.int("ATTACHED") == player.id as i64
        {
            n += if x.card_id.contains("_G") { 2 } else { 1 };
        }
    }
    n
}

/// Counts a player carries for the whole game: Eternal Knight deaths and
/// Ancestral Automatons summoned sit in a player enchantment
/// (`TAG_SCRIPT_DATA_NUM_1`), spells cast is a tag on the player.
pub const PLAYER_COUNTS: [(&str, &str); 2] =
    [("BG25_008pe", "knights"), ("BG_TTN_401pe", "automatons")];

/// Game-long counters of one player at the first attack, for the engine:
/// spells (mine only: the stand-in player carries none for the opponent),
/// knights, automatons. Missing ones are left out.
pub fn counts(g: &Game, player: Option<&Entity>) -> BTreeMap<&'static str, i64> {
    let mut out = BTreeMap::new();
    let Some(player) = player else {
        return out;
    };
    let spells = player.int("NUM_SPELLS_PLAYED_THIS_GAME");
    if spells != 0 {
        out.insert("spells", spells);
    }
    for x in g.entities_in_order() {
        for (card_id, key) in PLAYER_COUNTS {
            if x.card_id == card_id
                && x.text("ZONE") == "PLAY"
                && x.int("ATTACHED") == player.id as i64
            {
                out.insert(key, x.int("TAG_SCRIPT_DATA_NUM_1"));
            }
        }
    }
    out
}

/// Lock and Load: its shot is a copy of a minion, and the shot's attack opens
/// the fight.
pub const LOCK_AND_LOAD: &str = "BG22_HERO_000p_Alt";

/// The entity id of the minion Lock and Load still holds
/// (`TAG_SCRIPT_DATA_ENT_1` of the power in play), 0 for none. The minion is
/// not created in the log when the opponent stole it, so its stats are read
/// off the copy the power summons (see [`crate::Tracker`]).
pub fn held_entity(g: &Game, controller: Option<i32>) -> i64 {
    g.entities_in_order()
        .find(|x| {
            x.card_id == LOCK_AND_LOAD
                && x.text("ZONE") == "PLAY"
                && controlled_by(x, controller)
        })
        .map(|x| x.int("TAG_SCRIPT_DATA_ENT_1"))
        .unwrap_or(0)
}

/// Upper Hand: "Start of Combat: Set a random enemy minion's Health to 1". The
/// spell hides its pending trigger (the card goes back to SETASIDE), so the
/// pending count is the Upper Hands each side cast since the turn began.
pub const UPPER_HAND: &str = "BG28_573";

/// Player enchantments that carry a side-wide number, and what their script
/// numbers are: the name the file gives the number, and the
/// `TAG_SCRIPT_DATA_NUM_1..4` slot holding it (`None` for a number the
/// enchantment does not have, attack first and health second). The log attaches
/// them to the player of both seats (the opponent's to the stand-in player) and
/// only while the number is nonzero, so a missing one means the base value.
pub const PLAYER_ENCHANTS: [(&str, Option<&str>, Option<&str>); 3] = [
    // Beetle Army: "Your Beetles have +{0}/+{1} this game" (Forest Rover,
    // Runed Progenitor, Turquoise Skitterer).
    ("BG31_808pe", Some("beetle_attack"), Some("beetle_health")),
    // Undead Bonus Attack: "Your Undead have +{0} Attack this game, wherever
    // they are" (Nerubian Deathswarmer, Plaguerunner, Forsaken Weaver,
    // Butchering). There is no health one on the player: the health half rides
    // on each Undead's own `Undead Army` enchant (`BG25_011e2`, numbers 1 and
    // 2), which the shop collector reads.
    ("BG25_011pe", Some("undead_attack"), None),
    // Baller: how many Ballers were sold, the "improve your future Ballers"
    // count.
    ("BG31_816pe", Some("baller"), None),
];

/// The script numbers of one player enchantment on a player, or `None` when it
/// is not there.
pub fn enchant_numbers(g: &Game, player: &Entity, card_id: &str) -> Option<[i64; 4]> {
    let x = g
        .entities_in_order()
        .filter(|x| {
            x.card_id == card_id
                && x.text("ZONE") == "PLAY"
                && x.int("ATTACHED") == player.id as i64
        })
        .max_by_key(|x| x.id)?;
    Some(numbers(x))
}

/// What the log shows of one side's player at the first attack besides its
/// board: the Tavern spell bonus and the [`PLAYER_ENCHANTS`] numbers, under the
/// names the board file uses. A number the log does not show is left out.
pub fn side_numbers(g: &Game, player: Option<&Entity>) -> BTreeMap<&'static str, i64> {
    let mut out = BTreeMap::new();
    let Some(player) = player else {
        return out;
    };
    let spell_attack = player.int("TAVERN_SPELL_ATTACK_INCREASE");
    if spell_attack != 0 {
        out.insert("spell_attack", spell_attack);
    }
    let spell_health = player.int("TAVERN_SPELL_HEALTH_INCREASE");
    if spell_health != 0 {
        out.insert("spell_health", spell_health);
    }
    for (card_id, attack_key, health_key) in PLAYER_ENCHANTS {
        let Some(sd) = enchant_numbers(g, player, card_id) else {
            continue;
        };
        if let Some(key) = attack_key
            && sd[0] != 0
        {
            out.insert(key, sd[0]);
        }
        if let Some(key) = health_key
            && sd[1] != 0
        {
            out.insert(key, sd[1]);
        }
    }
    out
}


/// Lock and Load and Rapid Reanimation: hero powers whose minion may not have
/// been summoned yet.
pub const OWES_A_SUMMON: [&str; 2] = [LOCK_AND_LOAD, "BG25_HERO_103p"];

/// Illidan's hero power, see [`crate::Tracker`].
pub const WINGMEN: &str = "TB_BaconShop_HP_069";

/// The `{0} {1} {2} {3}` of an entity's text.
fn numbers(e: &Entity) -> [i64; 4] {
    [
        e.int("TAG_SCRIPT_DATA_NUM_1"),
        e.int("TAG_SCRIPT_DATA_NUM_2"),
        e.int("TAG_SCRIPT_DATA_NUM_3"),
        e.int("TAG_SCRIPT_DATA_NUM_4"),
    ]
}

/// The hero power a player has in play at the first attack.
///
/// It is an entity of CARDTYPE HERO_POWER in zone PLAY under the player's
/// CONTROLLER, and that holds for the opponent too: the stand-in player has the
/// opponent's hero power, copied in for the fight. Its current numbers are in
/// `TAG_SCRIPT_DATA_NUM_1..4`.
///
/// Lock and Load and Rapid Reanimation keep the minion they act on in
/// `TAG_SCRIPT_DATA_ENT_1`, and `HEROPOWER_ACTIVATIONS_THIS_TURN` says whether
/// they were used this turn (ENT_1 goes stale after that). Lock and Load's shot
/// is a copy of that minion (`COPIED_FROM_ENTITY_ID`, `CREATOR` the hero
/// power), and Rapid Reanimation's ENT_1 is the copy itself, in PLAY. While
/// neither exists the summon is still waiting for a free slot.
pub fn hero_power(g: &Game, cards: &Cards, controller: Option<i32>, turn: i64) -> Option<HeroPower> {
    let hp = g
        .entities_in_order()
        .filter(|x| {
            x.text("CARDTYPE") == "HERO_POWER"
                && x.text("ZONE") == "PLAY"
                && controlled_by(x, controller)
        })
        .min_by_key(|x| x.id)?;
    let mut waiting = None;
    let held = match hp.int("TAG_SCRIPT_DATA_ENT_1") {
        0 => None,
        id => g.entities.get(&(id as i32)),
    };
    if OWES_A_SUMMON.contains(&hp.card_id.as_str())
        && hp.int("HEROPOWER_ACTIVATIONS_THIS_TURN") != 0
        && let Some(held) = held.filter(|h| h.text("CARDTYPE") == "MINION")
    {
        let done = held.text("ZONE") == "PLAY"
            || g.entities_in_order().any(|x| {
                x.int("COPIED_FROM_ENTITY_ID") == held.id as i64
                    && x.int("CREATOR") == hp.id as i64
            });
        if !done {
            waiting = Some(one_minion(held, g, cards));
        }
    }
    Some(HeroPower {
        card_id: hp.card_id.clone(),
        name: cards.name(&hp.card_id).to_string(),
        turn,
        numbers: numbers(hp),
        waiting,
    })
}

/// The trinkets a player holds, Lesser slot first.
///
/// They are BATTLEGROUND_TRINKET entities in PLAY. The opponent's are in the log
/// too: the stand-in player holds copies of them during a fight. An empty slot
/// is a placeholder card, "Lesser Trinket" / "Greater Trinket"
/// (`BG30_Trinket_1st` and `_2nd`), which is left out.
/// `TAG_SCRIPT_DATA_NUM_6` is the slot (1 Lesser, 2 Greater), and NUM_4 and
/// NUM_6 are not card numbers.
pub fn trinkets(g: &Game, cards: &Cards, controller: Option<i32>) -> Vec<Trinket> {
    let mut ts: Vec<&Entity> = g
        .entities_in_order()
        .filter(|x| {
            x.text("CARDTYPE") == "BATTLEGROUND_TRINKET"
                && x.text("ZONE") == "PLAY"
                && controlled_by(x, controller)
                && !x.card_id.is_empty()
                && !x.card_id.starts_with("BG30_Trinket_")
        })
        .collect();
    ts.sort_by_key(|x| (x.int("TAG_SCRIPT_DATA_NUM_6"), x.id));
    ts.iter()
        .map(|x| Trinket {
            card_id: x.card_id.clone(),
            name: cards.name(&x.card_id).to_string(),
            greater: x.int("TAG_SCRIPT_DATA_NUM_6") == 2,
            numbers: numbers(x),
        })
        .collect()
}

/// The quest rewards a player holds, in the order they were earned.
///
/// They are BATTLEGROUND_QUEST_REWARD entities. Where the log puts a held reward
/// is only half known: the only rewards in Era's 2026-10-18 logs are the
/// stand-in player's Ghastly Mask copies, made when the fight is staged and in
/// SETASIDE, which is the one case read here beside PLAY. One that was offered
/// and not picked is removed from the game, so it is not read.
pub fn rewards(g: &Game, cards: &Cards, controller: Option<i32>) -> Vec<Reward> {
    let mut rs: Vec<&Entity> = g
        .entities_in_order()
        .filter(|x| {
            x.text("CARDTYPE") == "BATTLEGROUND_QUEST_REWARD"
                && matches!(x.text("ZONE"), "PLAY" | "SETASIDE")
                && controlled_by(x, controller)
                && !x.card_id.is_empty()
        })
        .collect();
    rs.sort_by_key(|x| x.id);
    rs.iter()
        .map(|x| Reward {
            card_id: x.card_id.clone(),
            name: cards.name(&x.card_id).to_string(),
            numbers: numbers(x),
        })
        .collect()
}

/// The minion a Deity awakens, by the dbf id in its `TAG_SCRIPT_DATA_NUM_6`.
/// The golden ones are the cards' `BACON_TRIPLE_UPGRADE_MINION_ID`.
pub const DEITY_MINIONS: [(i64, &str); 4] = [
    (130610, "BGFYM_000"),
    (134633, "BGFYM_000_G"),
    (132532, "BGFYM_011"),
    (134634, "BGFYM_011_G"),
];

/// A player's Deity at the first attack.
///
/// It is a `BG_OldGod` spell in zone SECRET. Mine is created once at game start;
/// the opponent's is a fresh copy made for each fight under the stand-in player
/// (the entity names print a stale zone and player, the tags are right). Its
/// numbers are the `{0} {1} {2} {5}` of its text: `TAG_SCRIPT_DATA_NUM_1` counts
/// down the Aberration deaths still needed (4 each combat), NUM_2 and NUM_3 are
/// the stats it awakens with, and NUM_6 is the dbf id of the minion.
/// QUEST_PROGRESS reaches 4 when it awakens.
pub fn deity(g: &Game, cards: &Cards, controller: Option<i32>) -> Option<Deity> {
    let x = g
        .entities_in_order()
        .filter(|x| {
            x.card_id == "BG_OldGod"
                && x.text("ZONE") == "SECRET"
                && controlled_by(x, controller)
        })
        .max_by_key(|x| x.id)?;
    let dbf = x.int("TAG_SCRIPT_DATA_NUM_6");
    let card_id = DEITY_MINIONS
        .iter()
        .find(|(id, _)| *id == dbf)
        .map(|(_, card_id)| *card_id)?;
    let left = if x.int("QUEST_PROGRESS") >= 4 {
        0
    } else {
        x.int("TAG_SCRIPT_DATA_NUM_1")
    };
    Some(Deity {
        card_id: card_id.to_string(),
        name: cards.name(card_id).to_string(),
        attack: x.int("TAG_SCRIPT_DATA_NUM_2"),
        health: x.int("TAG_SCRIPT_DATA_NUM_3"),
        left,
    })
}

/// The minions on a player's board, in position order.
pub fn board(g: &Game, cards: &Cards, controller: Option<i32>) -> Board {
    let gifts = gifts_by_minion(g);
    let enchants = hero_enchants_by_minion(g);
    let mut ms: Vec<&Entity> = g
        .entities_in_order()
        .filter(|x| {
            x.text("CARDTYPE") == "MINION"
                && x.text("ZONE") == "PLAY"
                && controlled_by(x, controller)
        })
        .collect();
    ms.sort_by_key(|x| x.int("ZONE_POSITION"));
    ms.iter()
        .map(|x| minion(x, g, cards, &gifts, &enchants))
        .collect()
}

/// Rapid Reanimation (Teron Gorefiend) marks the minion it destroys at Start of
/// Combat with this enchantment, attached to it while both sit in PLAY.
pub const REANIMATE_MARK: &str = "BG25_HERO_103pe";

/// Which minions on a player's board carry [`REANIMATE_MARK`], in board order
/// (the order [`board`] uses). The mark is read while the fight is staged: by
/// the first attack the minion it named is destroyed and the enchantment sits
/// on its copy, which is not on the board.
pub fn reanimates(g: &Game, controller: Option<i32>) -> Vec<bool> {
    let marked: HashSet<i64> = g
        .entities_in_order()
        .filter(|x| {
            x.card_id == REANIMATE_MARK && x.text("ZONE") == "PLAY" && x.int("ATTACHED") != 0
        })
        .map(|x| x.int("ATTACHED"))
        .collect();
    let mut ms: Vec<&Entity> = g
        .entities_in_order()
        .filter(|x| {
            x.text("CARDTYPE") == "MINION"
                && x.text("ZONE") == "PLAY"
                && controlled_by(x, controller)
        })
        .collect();
    ms.sort_by_key(|x| x.int("ZONE_POSITION"));
    ms.iter().map(|x| marked.contains(&(x.id as i64))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One tag of a test entity.
    type Tag<'a> = (&'a str, TagValue);
    /// One entity of a test game: id, card id and its tags.
    type Given<'a> = (i32, &'a str, &'a [Tag<'a>]);

    /// A game with just the entities a view reads.
    fn game(entities: &[Given]) -> Game {
        let mut g = Game::new();
        for (id, card_id, tags) in entities {
            let e = g.entity(*id);
            e.card_id = card_id.to_string();
            for (tag, value) in *tags {
                e.set(tag, value.clone());
            }
        }
        g
    }

    fn text(value: &str) -> TagValue {
        TagValue::Text(value.to_string())
    }

    fn number(value: i64) -> TagValue {
        TagValue::Int(value)
    }

    #[test]
    fn the_secret_zone_holds_the_secret_cards_in_entity_order() {
        // The Deity's zone is SECRET too, a Secret still in hand is not in the
        // zone, and another player's Secrets are not mine.
        let g = game(&[
            (40, "BG_OldGod", &[("ZONE", text("SECRET")), ("CONTROLLER", number(6))]),
            (30, "TB_Bacon_Secrets_07", &[("ZONE", text("SECRET")), ("CONTROLLER", number(6))]),
            (20, "TB_Bacon_Secrets_01", &[("ZONE", text("SECRET")), ("CONTROLLER", number(6))]),
            (21, "TB_Bacon_Secrets_01", &[("ZONE", text("HAND")), ("CONTROLLER", number(6))]),
            (22, "TB_Bacon_Secrets_01", &[("ZONE", text("SECRET")), ("CONTROLLER", number(7))]),
        ]);
        assert_eq!(
            secrets(&g, Some(6)),
            vec!["TB_Bacon_Secrets_01".to_string(), "TB_Bacon_Secrets_07".to_string()]
        );
        assert!(secrets(&g, Some(7)).len() == 1);
        assert!(secrets(&g, None).is_empty());
    }

    #[test]
    fn a_reanimation_mark_marks_the_minion_it_is_attached_to() {
        let g = game(&[
            (
                10,
                "BGS_004",
                &[
                    ("CARDTYPE", text("MINION")),
                    ("ZONE", text("PLAY")),
                    ("CONTROLLER", number(6)),
                    ("ZONE_POSITION", number(2)),
                ],
            ),
            (
                11,
                "BGS_004",
                &[
                    ("CARDTYPE", text("MINION")),
                    ("ZONE", text("PLAY")),
                    ("CONTROLLER", number(6)),
                    ("ZONE_POSITION", number(1)),
                ],
            ),
            // A mark in play marks minion 10; one that is not in play marks
            // nothing.
            (12, REANIMATE_MARK, &[("ZONE", text("PLAY")), ("ATTACHED", number(10))]),
            (13, REANIMATE_MARK, &[("ZONE", text("SETASIDE")), ("ATTACHED", number(11))]),
        ]);
        // Board order, not entity order.
        assert_eq!(reanimates(&g, Some(6)), vec![false, true]);
    }

    #[test]
    fn side_numbers_are_the_tavern_spell_bonus_and_the_player_enchantments() {
        let g = game(&[
            (
                10,
                "",
                &[("CARDTYPE", text("PLAYER")), ("TAVERN_SPELL_ATTACK_INCREASE", number(2))],
            ),
            (
                20,
                "BG31_808pe",
                &[
                    ("ZONE", text("PLAY")),
                    ("ATTACHED", number(10)),
                    ("TAG_SCRIPT_DATA_NUM_1", number(3)),
                    ("TAG_SCRIPT_DATA_NUM_2", number(4)),
                ],
            ),
            // Two Undead Bonus Attack enchantments: the newest (highest id) is
            // the current one.
            (
                21,
                "BG25_011pe",
                &[
                    ("ZONE", text("PLAY")),
                    ("ATTACHED", number(10)),
                    ("TAG_SCRIPT_DATA_NUM_1", number(1)),
                ],
            ),
            (
                22,
                "BG25_011pe",
                &[
                    ("ZONE", text("PLAY")),
                    ("ATTACHED", number(10)),
                    ("TAG_SCRIPT_DATA_NUM_1", number(5)),
                ],
            ),
            // A zero number is left out (the log only shows the nonzero ones),
            // and another player's enchantment is not mine.
            (
                23,
                "BG31_816pe",
                &[
                    ("ZONE", text("PLAY")),
                    ("ATTACHED", number(10)),
                    ("TAG_SCRIPT_DATA_NUM_1", number(0)),
                ],
            ),
            (
                24,
                "BG31_808pe",
                &[
                    ("ZONE", text("PLAY")),
                    ("ATTACHED", number(11)),
                    ("TAG_SCRIPT_DATA_NUM_1", number(9)),
                ],
            ),
        ]);
        assert_eq!(
            side_numbers(&g, g.entities.get(&10)),
            BTreeMap::from([
                ("beetle_attack", 3),
                ("beetle_health", 4),
                ("spell_attack", 2),
                ("undead_attack", 5),
            ])
        );
        assert!(side_numbers(&g, None).is_empty());
    }

    #[test]
    fn lock_and_load_holds_the_minion_it_stole() {
        let g = game(&[
            (
                10,
                LOCK_AND_LOAD,
                &[
                    ("ZONE", text("PLAY")),
                    ("CONTROLLER", number(1)),
                    ("TAG_SCRIPT_DATA_ENT_1", number(44)),
                ],
            ),
            // Another power's held entity is not a Lock and Load minion.
            (
                11,
                "BG21_HERO_010p",
                &[
                    ("ZONE", text("PLAY")),
                    ("CONTROLLER", number(1)),
                    ("TAG_SCRIPT_DATA_ENT_1", number(45)),
                ],
            ),
        ]);
        assert_eq!(held_entity(&g, Some(1)), 44);
        assert_eq!(held_entity(&g, Some(2)), 0);
    }

    #[test]
    fn gift_enchantments_name_their_gift_and_numbers_do_not() {
        assert_eq!(
            gift_enchantment("BG36_MidGameEffect_000t75e"),
            Some("BG36_MidGameEffect_000t75")
        );
        assert_eq!(
            gift_enchantment("BG36_MidGameEffect_000t75e1"),
            Some("BG36_MidGameEffect_000t75")
        );
        assert_eq!(gift_enchantment("BG36_MidGameEffect_000t52e"), Some("BG36_MidGameEffect_000t52"));
        // `\d*t?` before the e: no digits at all is fine.
        assert_eq!(gift_enchantment("BG36_MidGameEffect_000te"), Some("BG36_MidGameEffect_000t"));
        assert_eq!(gift_enchantment("BG36_MidGameEffect_000t"), None);
        assert_eq!(gift_enchantment("BG36_MidGameEffect_000t75"), None);
        assert_eq!(gift_enchantment("BG36_MidGameEffect_000t75x"), None);
        // A second "t" belongs to the group, as the Python's greedy `t?` does.
        assert_eq!(
            gift_enchantment("BG36_MidGameEffect_000t75te"),
            Some("BG36_MidGameEffect_000t75t")
        );
        assert_eq!(gift_enchantment("BG36_MidGameEffect_000t7t5e"), None);
        assert_eq!(gift_enchantment("BG28_966"), None);
    }
}
