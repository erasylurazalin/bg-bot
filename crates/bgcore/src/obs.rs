//! The observation: what one player can see, written as flat f32 arrays.
//! Values are raw (gold 7 is 7.0); scaling belongs to the training side.
//! Card ids are written as id + 1 (0 for none); the model embeds them.
//!
//! Layout, in order:
//! - SCALARS own values (103):
//!   0  gold
//!   1  tier
//!   2  upgrade_cost
//!   3  health
//!   4  turn
//!   5  frozen
//!   6  actions_left (remaining actions before the turn cap)
//!   7  alive (number of living players in the lobby)
//!   8  fighting_ghost (1 if paired against the ghost this turn)
//!   9  won_last_combat
//!   10 lost_last_combat
//!   11 gold_next_turn (extra gold queued for next turn)
//!   12 extra_max_gold (permanent increase to the turn gold from Strike Oil)
//!   13 free_rolls (free Refreshes left)
//!   14 next_roll_health (1 if the next Refresh is paid with Health, Malchezaar)
//!   15 health_refreshes_left (Health Refreshes left this turn)
//!   16 can_rewind (1 if a minion on the board gives hero damage back)
//!   17 spell_discount (cost reduction on the next Tavern spell bought)
//!   18 shop_spell_cost (cost of the spell slot after the discount, 0 if empty)
//!   19 shop_spell_pay_health (1 if the spell slot is bought with Health)
//!   20 spell_attack (total Tavern spell Attack bonus, permanent + board auras)
//!   21 spell_health (total Tavern spell Health bonus, permanent + board auras)
//!   22 volumizer_attack (permanent Volumizer Attack bonus)
//!   23 volumizer_health
//!   24 beetle_attack (permanent Beetle Attack bonus)
//!   25 beetle_health
//!   26 boon_of_beetles (Boon of Beetles charges)
//!   27 brood_of_nozdormu (pending Brood of Nozdormu Start of Combat triggers)
//!   28 upper_hand (pending Upper Hand Start of Combat triggers)
//!   29 patient_scout_tier (highest Discover tier of a Patient Scout on board, 0 if none)
//!   30 tasty_lobster (Lobster Deathrattles this game, the "improve" count)
//!   31 beast_bonus_attack (shop Goldrinn bonus on Beasts until next turn)
//!   32 beast_bonus_health
//!   33 deathrattles_triggered (this game, Falling Sky Golem)
//!   34 fodder_next (Fodders the next explicit Refresh brings)
//!   35 fodder_queued (Fodders queued over all coming Refreshes)
//!   36 fodder_refreshes (coming Refreshes in the queue, with or without Fodder)
//!   37 magicfin_left (Magicfin Mycologist teachings left this turn)
//!   38 overconfidence_win (gold next turn if the coming fight is won)
//!   39 overconfidence_tie (gold next turn if it is a tie)
//!   40 winners_bread_attack (what each Winner's Bread stack gives on a win)
//!   41 winners_bread_health
//!   42 next_turn_buff_attack (board buff queued for the start of next turn, summed)
//!   43 next_turn_buff_health
//!   44..68 Tavern buffs: 6 filters (any, Beast, Mech, Dragon, Murloc, Demon)
//!   x 4 values (this game attack, this game health, this turn attack, this
//!   turn health)
//!   68 last_spell (card id + 1 of the last Tavern spell cast, 0 if none)
//!   Pending choice (while one is open, only actions that answer it are legal):
//!   69 pending_target (a minion target on the board only)
//!   70 pending_target_shop (a minion target on the board or in the Tavern)
//!   71 pending_shop_slot (a Tavern slot target: a chosen consume or Fishbait)
//!   72 pending_choose_one
//!   73 pending_discover
//!   74 pending_source (board slot + 1 of the minion whose effect it is, 0 for a spell)
//!   75 pending_card (card id + 1 of that minion, or of the last spell cast
//!   for a spell target or Choose One spell; 0 for a Discover)
//!   76 pending_runs (times the target effect runs: Brann, Balinda)
//!   77 pending_attack (Attack the target effect gives or sets, 0 if none)
//!   78 pending_health
//!   79..86 target kind, one-hot: buff, consume, spell, trigger_rally,
//!   trigger_battlecry, any_buff, any_set_stats
//!   86 shop_consume_left (Tavern minions the chosen consume still takes)
//!   87 shop_fishbait (1 if the Tavern slot target puts a Fishbait there)
//!   88 discover_locked (1 if the Discovered minion is locked in hand a turn)
//!   89 discover_damage (1 if the hero takes damage equal to the chosen Tier)
//!   90 discover_queue (more Discovers waiting after this one)
//!   91..103 Choose One options, 2 x 6: kind one-hot (minion target, spell
//!   bonus, spell target, spell effect), attack, health
//!   (Its legality is in the action mask, so it isn't repeated here.)
//! - Own card slots (OWN_SLOTS = 28, each SLOT_FEATURES values):
//!   shop minions (SHOP = 7), shop spell (SHOP_SPELL = 1), hand (HAND = 10),
//!   board (BOARD = 7), Discover options (DISCOVER = 3). A Tavern spell that
//!   sits in a minion slot (Saloon's Finest) is written there as a spell.
//! - 7 opponents, in seat order starting after me (each OPP_BLOCK values):
//!   OPP_SCALARS (13): alive, health, tier, seen (1 if I ever fought them),
//!   turns_since (since that fight), next (my opponent this turn, or the ghost
//!   of this player), last (my opponent last turn), placement (0 while
//!   alive), then the tribe counts of their last seen board (Beast, Mech,
//!   Dragon, Murloc, Demon); followed by that board (BOARD = 7 card slots).
//!
//! A card slot is SLOT_FEATURES (32) values:
//!   0  present (1 if occupied)
//!   1  card id + 1 (one id space covering minions, tokens and spells)
//!   2  is_spell
//!   3  attack (0 for spells)
//!   4  health (0 for spells)
//!   5  fight_attack: the Attack it would fight with on this player's board
//!      now: plus Holy Vanguard's +30 (+60 golden) when that hero has 15
//!      Health or less, and for a Beast not on the board yet the shop
//!      Goldrinn bonus it gets when played. Equal to attack otherwise.
//!   6  fight_health
//!   7  golden
//!   8  tier (1 to 6)
//!   9..14 tribes, multi-hot: Beast, Mech, Dragon, Murloc, Demon (an all-type
//!         minion sets all five, a neutral none)
//!   14..21 keywords: taunt, divine_shield, windfury, venomous, reborn,
//!          magnetic, stealth
//!   21 magnetizations (total Magnetizations on this minion)
//!   22 volumizers (Volumizers this minion is or carries)
//!   23 improvements (the minion's own count: Leviathan, Spark Snapper, Patient
//!      Scout, Hopebringer, Sacrificial Wrathguard, event counters)
//!   24 events_left (the "(N left!)" of its event trigger, 0 if it has none)
//!   25 from_pool (1 if backed by pool copies, 0 if generated, token or spell)
//!   26 winners_bread (pending Winner's Bread stacks)
//!   27 taught_spell (card id + 1 of the spell a Magicfin token casts, 0 if none)
//!   28 locked_turns (turns it stays locked in hand, Search Through Time)
//!   29 activate_cost (Gold of its Activate, 0 if it has none or it was
//!      already used this turn: once per turn per minion)
//!   30 extra_magnetize (extra times the next Magnetization to it happens, Drone Duplicator)
//!   31 cost (a spell's printed cost; the buy cost for a minion)
//! An empty slot is all zeros.

use crate::cards::{Aura, CardDb, ChooseOption, SpellEffect, SpellTarget, Tribe};
use crate::game::{Game, Opponent, Player, ShopTargetKind, TargetKind};
use crate::minion::{Keywords, Minion};
use crate::{BOARD, DISCOVER, HAND, PLAYERS, SHOP};

pub const SHOP_SPELL: usize = 1;
pub const SLOT_FEATURES: usize = 32;
pub const MINION_FEATURES: usize = SLOT_FEATURES;
pub const SCALARS: usize = 103;
pub const OPP_SCALARS: usize = 13;

pub const OWN_SLOTS: usize = SHOP + SHOP_SPELL + HAND + BOARD + DISCOVER;
pub const OPP_SLOTS: usize = (PLAYERS - 1) * BOARD;
pub const TOTAL_SLOTS: usize = OWN_SLOTS + OPP_SLOTS;
pub const OWN_END: usize = SCALARS + OWN_SLOTS * SLOT_FEATURES;
pub const OPP_BLOCK: usize = OPP_SCALARS + BOARD * SLOT_FEATURES;
pub const OBS_SIZE: usize = OWN_END + (PLAYERS - 1) * OPP_BLOCK;

pub const SHOP_START: usize = SCALARS;
pub const SHOP_SPELL_START: usize = SHOP_START + SHOP * SLOT_FEATURES;
pub const HAND_START: usize = SHOP_SPELL_START + SHOP_SPELL * SLOT_FEATURES;
pub const BOARD_START: usize = HAND_START + HAND * SLOT_FEATURES;
pub const DISCOVER_START: usize = BOARD_START + BOARD * SLOT_FEATURES;

pub const SCALAR_GOLD: usize = 0;
pub const SCALAR_TIER: usize = 1;
pub const SCALAR_UPGRADE_COST: usize = 2;
pub const SCALAR_HEALTH: usize = 3;
pub const SCALAR_TURN: usize = 4;
pub const SCALAR_FROZEN: usize = 5;
pub const SCALAR_ACTIONS_LEFT: usize = 6;
pub const SCALAR_ALIVE: usize = 7;
pub const SCALAR_GHOST: usize = 8;
pub const SCALAR_WON_LAST_COMBAT: usize = 9;
pub const SCALAR_LOST_LAST_COMBAT: usize = 10;
pub const SCALAR_GOLD_NEXT_TURN: usize = 11;
pub const SCALAR_EXTRA_MAX_GOLD: usize = 12;
pub const SCALAR_FREE_ROLLS: usize = 13;
pub const SCALAR_NEXT_ROLL_HEALTH: usize = 14;
pub const SCALAR_HEALTH_REFRESHES_LEFT: usize = 15;
pub const SCALAR_CAN_REWIND: usize = 16;
pub const SCALAR_SPELL_DISCOUNT: usize = 17;
pub const SCALAR_SHOP_SPELL_COST: usize = 18;
pub const SCALAR_SHOP_SPELL_PAY_HEALTH: usize = 19;
pub const SCALAR_SPELL_ATTACK: usize = 20;
pub const SCALAR_SPELL_HEALTH: usize = 21;
pub const SCALAR_VOLUMIZER_ATTACK: usize = 22;
pub const SCALAR_VOLUMIZER_HEALTH: usize = 23;
pub const SCALAR_BEETLE_ATTACK: usize = 24;
pub const SCALAR_BEETLE_HEALTH: usize = 25;
pub const SCALAR_BOON_OF_BEETLES: usize = 26;
pub const SCALAR_BROOD_OF_NOZDORMU: usize = 27;
pub const SCALAR_UPPER_HAND: usize = 28;
pub const SCALAR_PATIENT_SCOUT_TIER: usize = 29;
pub const SCALAR_TASTY_LOBSTER: usize = 30;
pub const SCALAR_BEAST_BONUS_ATTACK: usize = 31;
pub const SCALAR_BEAST_BONUS_HEALTH: usize = 32;
pub const SCALAR_DEATHRATTLES: usize = 33;
pub const SCALAR_FODDER_NEXT: usize = 34;
pub const SCALAR_FODDER_QUEUED: usize = 35;
pub const SCALAR_FODDER_REFRESHES: usize = 36;
pub const SCALAR_MAGICFIN_LEFT: usize = 37;
pub const SCALAR_OVERCONFIDENCE_WIN: usize = 38;
pub const SCALAR_OVERCONFIDENCE_TIE: usize = 39;
pub const SCALAR_WINNERS_BREAD_ATTACK: usize = 40;
pub const SCALAR_WINNERS_BREAD_HEALTH: usize = 41;
pub const SCALAR_NEXT_TURN_BUFF_ATTACK: usize = 42;
pub const SCALAR_NEXT_TURN_BUFF_HEALTH: usize = 43;
/// 6 filters (any, then the five tribes in `Tribe::ALL` order) x 4 values.
pub const SCALAR_TAVERN_BUFFS: usize = 44;
pub const TAVERN_BUFF_FILTERS: usize = 6;
pub const SCALAR_LAST_SPELL: usize = 68;
pub const SCALAR_PENDING_TARGET: usize = 69;
pub const SCALAR_PENDING_TARGET_SHOP: usize = 70;
pub const SCALAR_PENDING_SHOP_SLOT: usize = 71;
pub const SCALAR_PENDING_CHOOSE_ONE: usize = 72;
pub const SCALAR_PENDING_DISCOVER: usize = 73;
pub const SCALAR_PENDING_SOURCE: usize = 74;
pub const SCALAR_PENDING_CARD: usize = 75;
pub const SCALAR_PENDING_RUNS: usize = 76;
pub const SCALAR_PENDING_ATTACK: usize = 77;
pub const SCALAR_PENDING_HEALTH: usize = 78;
/// One-hot over buff, consume, spell, trigger_rally, trigger_battlecry,
/// any_buff, any_set_stats.
pub const SCALAR_TARGET_KIND: usize = 79;
pub const TARGET_KINDS: usize = 7;
pub const SCALAR_SHOP_CONSUME_LEFT: usize = 86;
pub const SCALAR_SHOP_FISHBAIT: usize = 87;
pub const SCALAR_DISCOVER_LOCKED: usize = 88;
pub const SCALAR_DISCOVER_DAMAGE: usize = 89;
pub const SCALAR_DISCOVER_QUEUE: usize = 90;
/// 2 options x CHOOSE_FEATURES: kind one-hot (minion target, spell bonus,
/// spell target, spell effect), attack, health.
pub const SCALAR_CHOOSE_ONE: usize = 91;
pub const CHOOSE_FEATURES: usize = 6;

pub const SLOT_PRESENT: usize = 0;
pub const SLOT_CARD_ID: usize = 1;
pub const SLOT_IS_SPELL: usize = 2;
pub const SLOT_ATTACK: usize = 3;
pub const SLOT_HEALTH: usize = 4;
pub const SLOT_FIGHT_ATTACK: usize = 5;
pub const SLOT_FIGHT_HEALTH: usize = 6;
pub const SLOT_GOLDEN: usize = 7;
pub const SLOT_TIER: usize = 8;
/// Five values, `Tribe::ALL` order: Beast, Mech, Dragon, Murloc, Demon.
pub const SLOT_TRIBES: usize = 9;
pub const SLOT_BEAST: usize = SLOT_TRIBES;
pub const SLOT_MECH: usize = SLOT_TRIBES + 1;
pub const SLOT_DRAGON: usize = SLOT_TRIBES + 2;
pub const SLOT_MURLOC: usize = SLOT_TRIBES + 3;
pub const SLOT_DEMON: usize = SLOT_TRIBES + 4;
/// Seven values, `Keywords::ALL` order.
pub const SLOT_KEYWORDS: usize = 14;
pub const SLOT_STEALTH: usize = SLOT_KEYWORDS + 6;
pub const SLOT_MAGNETIZATIONS: usize = 21;
pub const SLOT_VOLUMIZERS: usize = 22;
pub const SLOT_IMPROVEMENTS: usize = 23;
pub const SLOT_EVENTS_LEFT: usize = 24;
pub const SLOT_FROM_POOL: usize = 25;
pub const SLOT_WINNERS_BREAD: usize = 26;
pub const SLOT_TAUGHT_SPELL: usize = 27;
pub const SLOT_LOCKED: usize = 28;
pub const SLOT_ACTIVATE_COST: usize = 29;
pub const SLOT_EXTRA_MAGNETIZE: usize = 30;
pub const SLOT_COST: usize = 31;

pub const OPP_ALIVE: usize = 0;
pub const OPP_HEALTH: usize = 1;
pub const OPP_TIER: usize = 2;
pub const OPP_SEEN: usize = 3;
pub const OPP_TURNS_SINCE: usize = 4;
pub const OPP_NEXT: usize = 5;
pub const OPP_LAST: usize = 6;
pub const OPP_PLACEMENT: usize = 7;
/// Five values, `Tribe::ALL` order.
pub const OPP_TRIBES: usize = 8;

const _: () = assert!(SCALAR_TAVERN_BUFFS + TAVERN_BUFF_FILTERS * 4 == SCALAR_LAST_SPELL);
const _: () = assert!(SCALAR_TARGET_KIND + TARGET_KINDS == SCALAR_SHOP_CONSUME_LEFT);
const _: () = assert!(SCALAR_CHOOSE_ONE + 2 * CHOOSE_FEATURES == SCALARS);
const _: () = assert!(SLOT_TRIBES + Tribe::ALL.len() == SLOT_KEYWORDS);
const _: () = assert!(SLOT_KEYWORDS + Keywords::ALL.len() == SLOT_MAGNETIZATIONS);
const _: () = assert!(OPP_TRIBES + Tribe::ALL.len() == OPP_SCALARS);

/// Holy Vanguard's bonus on this minion if its hero is at `health`: +30/+30
/// (+60 golden) at 15 Health or less, added at the start of a fight.
pub fn vanguard_bonus(db: &CardDb, m: &Minion, health: i32) -> i32 {
    if health > 15 {
        return 0;
    }
    m.parts()
        .filter(|&(c, _)| db.get(c).aura == Some(Aura::HolyVanguard))
        .map(|(_, golden)| if golden { 60 } else { 30 })
        .max()
        .unwrap_or(0)
}

/// How a slot's minion relates to the player whose view this is, for its
/// fight stats.
#[derive(Clone, Copy)]
struct Owner {
    /// Health of the hero whose board it is (or would be on).
    health: i32,
    /// Bonus a Beast gets when played (0 for a minion already on a board).
    beast_bonus: (i32, i32),
}

fn write_slot(game: &Game, o: &mut [f32], m: &Minion, owner: Owner) {
    let db = &game.data.cards;
    let c = db.get(m.card);
    o[SLOT_PRESENT] = 1.0;
    o[SLOT_CARD_ID] = m.card as f32 + 1.0;
    o[SLOT_IS_SPELL] = c.is_spell() as u8 as f32;
    o[SLOT_TIER] = c.tier as f32;
    if c.is_spell() {
        o[SLOT_COST] = c.cost as f32;
        return;
    }
    o[SLOT_COST] = game.data.rules.buy_cost as f32;
    o[SLOT_ATTACK] = m.attack as f32;
    o[SLOT_HEALTH] = m.health as f32;
    let v = vanguard_bonus(db, m, owner.health);
    let (ba, bh) = if c.tribes.has(Tribe::Beast) { owner.beast_bonus } else { (0, 0) };
    o[SLOT_FIGHT_ATTACK] = (m.attack + v + ba) as f32;
    o[SLOT_FIGHT_HEALTH] = (m.health + v + bh) as f32;
    o[SLOT_GOLDEN] = m.golden as u8 as f32;
    for (j, t) in Tribe::ALL.into_iter().enumerate() {
        o[SLOT_TRIBES + j] = c.tribes.has(t) as u8 as f32;
    }
    for (j, (bit, _)) in Keywords::ALL.iter().enumerate() {
        o[SLOT_KEYWORDS + j] = m.keywords.has(*bit) as u8 as f32;
    }
    o[SLOT_MAGNETIZATIONS] = m.magnetizations as f32;
    o[SLOT_VOLUMIZERS] = m.volumizers as f32;
    o[SLOT_IMPROVEMENTS] = m.improvements as f32;
    o[SLOT_EVENTS_LEFT] = m.events_left(db).map_or(0.0, |n| n as f32);
    o[SLOT_FROM_POOL] = (m.pool_copies > 0) as u8 as f32;
    o[SLOT_WINNERS_BREAD] = m.winners_bread as f32;
    o[SLOT_TAUGHT_SPELL] = m.taught_spell.map_or(0.0, |s| s as f32 + 1.0);
    o[SLOT_LOCKED] = m.locked_turns as f32;
    // zero once used this turn, so the slot reads as "nothing to activate"
    o[SLOT_ACTIVATE_COST] = if m.activated {
        0.0
    } else {
        m.parts().find_map(|(card, _)| db.get(card).activate).map_or(0.0, |a| a.cost as f32)
    };
    o[SLOT_EXTRA_MAGNETIZE] = m.extra_magnetize as f32;
}

fn write_slots<'a>(
    game: &Game,
    out: &mut [f32],
    slots: usize,
    ms: impl Iterator<Item = &'a Minion>,
    owner: Owner,
) -> usize {
    let n = slots * SLOT_FEATURES;
    out[..n].fill(0.0);
    for (k, m) in ms.take(slots).enumerate() {
        let o = &mut out[k * SLOT_FEATURES..(k + 1) * SLOT_FEATURES];
        write_slot(game, o, m, owner);
    }
    n
}

/// The Attack and Health a Tavern spell target gives or sets, if it has
/// numbers.
fn spell_target_stats(st: SpellTarget) -> (i32, i32) {
    match st {
        SpellTarget::Buff { attack, health, .. }
        | SpellTarget::BuffSameTribe { attack, health }
        | SpellTarget::SetStats { attack, health }
        | SpellTarget::BuffPerBonusKeyword { attack, health }
        | SpellTarget::TrickyTrousers { attack, health }
        | SpellTarget::TavernBuffType { attack, health } => (attack, health),
        _ => (0, 0),
    }
}

fn spell_effect_stats(se: SpellEffect) -> (i32, i32) {
    match se {
        SpellEffect::BuffShop { attack, health }
        | SpellEffect::BuffBoard { attack, health, .. }
        | SpellEffect::BuffRandomBoard { attack, health, .. }
        | SpellEffect::NextTurnBoardBuff { attack, health, .. }
        | SpellEffect::MightyDragonbreath { attack, health }
        | SpellEffect::BuffTavernPermanent { attack, health }
        | SpellEffect::MisplacedTeaSet { attack, health } => (attack, health),
        _ => (0, 0),
    }
}

/// Index of a target kind in the one-hot, and the stats it gives or sets.
fn target_kind(k: TargetKind) -> (usize, (i32, i32)) {
    match k {
        TargetKind::MinionBuff { attack, health, .. } => (0, (attack, health)),
        TargetKind::Consume { .. } => (1, (0, 0)),
        TargetKind::Spell(st) => (2, spell_target_stats(st)),
        TargetKind::TriggerRally => (3, (0, 0)),
        TargetKind::TriggerBattlecry => (4, (0, 0)),
        TargetKind::AnyBuff { attack, health } => (5, (attack, health)),
        TargetKind::AnySetStats { attack, health } => (6, (attack, health)),
        // Same slot as the Make Golden spell (the layout is frozen until the
        // observation run).
        TargetKind::MakeGolden { .. } => (2, (0, 0)),
    }
}

fn choose_option(opt: ChooseOption) -> (usize, (i32, i32)) {
    match opt {
        ChooseOption::MinionTarget { attack, health, .. } => (0, (attack, health)),
        ChooseOption::SpellBonus { attack, health } => (1, (attack, health)),
        ChooseOption::SpellTarget(st) => (2, spell_target_stats(st)),
        ChooseOption::SpellEffect(se) => (3, spell_effect_stats(se)),
    }
}

/// Magicfin Mycologist teachings left this turn: one per Mycologist on the
/// board (two for a golden one), minus the ones used.
pub fn magicfin_left(pl: &Player, db: &CardDb) -> u8 {
    let mut limit = 0u8;
    for m in &pl.board {
        for (card, golden) in m.parts() {
            if db.get(card).teaches_spell {
                limit = limit.saturating_add(if golden { 2 } else { 1 });
            }
        }
    }
    limit.saturating_sub(pl.mycologist_uses)
}

fn write_scalars(game: &Game, p: usize, s: &mut [f32]) {
    let pl = &game.players[p];
    let db = &game.data.cards;
    let r = &game.data.rules;
    let (spell_atk, spell_hp) = game.total_spell_bonus(p);
    s.fill(0.0);
    s[SCALAR_GOLD] = pl.gold as f32;
    s[SCALAR_TIER] = pl.tier as f32;
    s[SCALAR_UPGRADE_COST] = pl.upgrade_cost as f32;
    s[SCALAR_HEALTH] = pl.health as f32;
    s[SCALAR_TURN] = game.turn as f32;
    s[SCALAR_FROZEN] = pl.frozen as u8 as f32;
    s[SCALAR_ACTIONS_LEFT] = r.max_actions_per_turn.saturating_sub(pl.actions) as f32;
    s[SCALAR_ALIVE] = game.alive() as f32;
    s[SCALAR_GHOST] = matches!(pl.opponent, Some(Opponent::Ghost(_))) as u8 as f32;
    s[SCALAR_WON_LAST_COMBAT] = pl.won_last_combat as u8 as f32;
    s[SCALAR_LOST_LAST_COMBAT] = pl.lost_last_combat as u8 as f32;
    s[SCALAR_GOLD_NEXT_TURN] = pl.gold_next_turn as f32;
    s[SCALAR_EXTRA_MAX_GOLD] = pl.extra_max_gold as f32;
    s[SCALAR_FREE_ROLLS] = pl.free_rolls as f32;
    s[SCALAR_NEXT_ROLL_HEALTH] =
        (pl.free_rolls == 0 && pl.can_refresh_with_health(db, r)) as u8 as f32;
    s[SCALAR_HEALTH_REFRESHES_LEFT] = pl.health_refreshes_left(db) as f32;
    s[SCALAR_CAN_REWIND] = pl.can_rewind(db) as u8 as f32;
    s[SCALAR_SPELL_DISCOUNT] = pl.spell_discount as f32;
    if let Some(sp) = pl.shop_spell {
        let c = db.get(sp);
        s[SCALAR_SHOP_SPELL_PAY_HEALTH] = c.pay_health as u8 as f32;
        s[SCALAR_SHOP_SPELL_COST] =
            if c.pay_health { c.cost } else { c.cost.saturating_sub(pl.spell_discount) } as f32;
    }
    s[SCALAR_SPELL_ATTACK] = spell_atk as f32;
    s[SCALAR_SPELL_HEALTH] = spell_hp as f32;
    s[SCALAR_VOLUMIZER_ATTACK] = pl.volumizer_bonus.0 as f32;
    s[SCALAR_VOLUMIZER_HEALTH] = pl.volumizer_bonus.1 as f32;
    s[SCALAR_BEETLE_ATTACK] = pl.beetle_bonus.0 as f32;
    s[SCALAR_BEETLE_HEALTH] = pl.beetle_bonus.1 as f32;
    s[SCALAR_BOON_OF_BEETLES] = pl.boon_of_beetles as f32;
    s[SCALAR_BROOD_OF_NOZDORMU] = pl.brood_of_nozdormu as f32;
    s[SCALAR_UPPER_HAND] = pl.upper_hand as f32;
    s[SCALAR_PATIENT_SCOUT_TIER] = pl.patient_scout_tier(db) as f32;
    s[SCALAR_TASTY_LOBSTER] = pl.improve[crate::cards::Counter::TastyLobster.index()] as f32;
    s[SCALAR_BEAST_BONUS_ATTACK] = pl.beast_bonus.0 as f32;
    s[SCALAR_BEAST_BONUS_HEALTH] = pl.beast_bonus.1 as f32;
    s[SCALAR_DEATHRATTLES] = pl.deathrattles_triggered as f32;
    s[SCALAR_FODDER_NEXT] = pl.fodder_refreshes.first().copied().unwrap_or(0) as f32;
    s[SCALAR_FODDER_QUEUED] = pl.fodder_refreshes.iter().map(|&n| n as u32).sum::<u32>() as f32;
    s[SCALAR_FODDER_REFRESHES] = pl.fodder_refreshes.len() as f32;
    s[SCALAR_MAGICFIN_LEFT] = magicfin_left(pl, db) as f32;
    s[SCALAR_OVERCONFIDENCE_WIN] = pl.overconfidence_win as f32;
    s[SCALAR_OVERCONFIDENCE_TIE] = pl.overconfidence_tie as f32;
    s[SCALAR_WINNERS_BREAD_ATTACK] = pl.winners_bread_buff.0 as f32;
    s[SCALAR_WINNERS_BREAD_HEALTH] = pl.winners_bread_buff.1 as f32;
    let (mut na, mut nh) = (0, 0);
    for &(a, h, times) in &pl.next_turn_board_buffs {
        na += a * times as i32;
        nh += h * times as i32;
    }
    s[SCALAR_NEXT_TURN_BUFF_ATTACK] = na as f32;
    s[SCALAR_NEXT_TURN_BUFF_HEALTH] = nh as f32;
    for b in &pl.tavern_buffs {
        let filter = b.tribe.map_or(0, |t| t as usize + 1);
        let k = SCALAR_TAVERN_BUFFS + filter * 4 + if b.this_turn { 2 } else { 0 };
        s[k] += b.attack as f32;
        s[k + 1] += b.health as f32;
    }
    s[SCALAR_LAST_SPELL] = pl.last_spell.map_or(0.0, |c| c as f32 + 1.0);

    // The card a pending choice comes from: the source minion, or for a
    // spell (no source) the spell just cast.
    let card_at = |src: Option<u8>| match src {
        Some(s) => pl.board.get(s as usize).map_or(0.0, |m| m.card as f32 + 1.0),
        None => pl.last_spell.map_or(0.0, |c| c as f32 + 1.0),
    };
    if let Some(pt) = pl.pending_target {
        let k = if pt.kind.takes_shop() { SCALAR_PENDING_TARGET_SHOP } else { SCALAR_PENDING_TARGET };
        s[k] = 1.0;
        s[SCALAR_PENDING_SOURCE] = pt.source.map_or(0.0, |x| x as f32 + 1.0);
        s[SCALAR_PENDING_CARD] = card_at(pt.source);
        s[SCALAR_PENDING_RUNS] = pt.runs as f32;
        let (kind, (a, h)) = target_kind(pt.kind);
        s[SCALAR_TARGET_KIND + kind] = 1.0;
        s[SCALAR_PENDING_ATTACK] = a as f32;
        s[SCALAR_PENDING_HEALTH] = h as f32;
    } else if let Some(pst) = pl.pending_shop_target {
        s[SCALAR_PENDING_SHOP_SLOT] = 1.0;
        s[SCALAR_PENDING_SOURCE] = pst.source as f32 + 1.0;
        s[SCALAR_PENDING_CARD] = card_at(Some(pst.source));
        match pst.kind {
            ShopTargetKind::Consume { remaining, .. } => {
                s[SCALAR_SHOP_CONSUME_LEFT] = remaining as f32;
            }
            ShopTargetKind::Fishbait { .. } => s[SCALAR_SHOP_FISHBAIT] = 1.0,
        }
    } else if let Some(pco) = pl.pending_choose_one {
        s[SCALAR_PENDING_CHOOSE_ONE] = 1.0;
        s[SCALAR_PENDING_SOURCE] = pco.source.map_or(0.0, |x| x as f32 + 1.0);
        s[SCALAR_PENDING_CARD] = card_at(pco.source);
        for (j, &opt) in pco.options.iter().enumerate() {
            let (kind, (a, h)) = choose_option(opt);
            let base = SCALAR_CHOOSE_ONE + j * CHOOSE_FEATURES;
            s[base + kind] = 1.0;
            s[base + 4] = a as f32;
            s[base + 5] = h as f32;
        }
    } else if !pl.discover.is_empty() {
        s[SCALAR_PENDING_DISCOVER] = 1.0;
        s[SCALAR_DISCOVER_LOCKED] = pl.discover_locked as u8 as f32;
        s[SCALAR_DISCOVER_DAMAGE] = pl.discover_damage_from.is_some() as u8 as f32;
    }
    s[SCALAR_DISCOVER_QUEUE] = pl.discover_queue.len() as f32;
}

/// Write player `p`'s view into `out`, which must hold OBS_SIZE values.
pub fn write(game: &Game, p: usize, out: &mut [f32]) {
    assert_eq!(out.len(), OBS_SIZE);
    let pl = &game.players[p];
    let db = &game.data.cards;
    write_scalars(game, p, &mut out[..SCALARS]);
    let on_board = Owner { health: pl.health, beast_bonus: (0, 0) };
    let off_board = Owner { health: pl.health, beast_bonus: pl.beast_bonus };
    let mut i = SCALARS;
    i += write_slots(game, &mut out[i..], SHOP, pl.shop.iter(), off_board);
    out[i..i + SHOP_SPELL * SLOT_FEATURES].fill(0.0);
    if let Some(sp) = pl.shop_spell {
        let m = pl.make_card(db, sp, false, 0);
        write_slot(game, &mut out[i..i + SLOT_FEATURES], &m, off_board);
    }
    i += SHOP_SPELL * SLOT_FEATURES;
    i += write_slots(game, &mut out[i..], HAND, pl.hand.iter(), off_board);
    i += write_slots(game, &mut out[i..], BOARD, pl.board.iter(), on_board);
    out[i..i + DISCOVER * SLOT_FEATURES].fill(0.0);
    for (k, &c) in pl.discover.iter().take(DISCOVER).enumerate() {
        let pool_copies = u8::from(!db.get(c).is_spell());
        let m = pl.make_card(db, c, false, pool_copies);
        let o = &mut out[i + k * SLOT_FEATURES..i + (k + 1) * SLOT_FEATURES];
        write_slot(game, o, &m, off_board);
    }
    i += DISCOVER * SLOT_FEATURES;
    for k in 1..PLAYERS {
        let q = (p + k) % PLAYERS;
        let them = &game.players[q];
        let seen = &pl.seen[q];
        let is = |o: Option<Opponent>| match o {
            Some(Opponent::Player(x)) | Some(Opponent::Ghost(x)) => x as usize == q,
            None => false,
        };
        let o = &mut out[i..i + OPP_SCALARS];
        o.fill(0.0);
        o[OPP_ALIVE] = them.alive as u8 as f32;
        o[OPP_HEALTH] = them.health as f32;
        o[OPP_TIER] = them.tier as f32;
        o[OPP_SEEN] = (seen.turn > 0) as u8 as f32;
        o[OPP_TURNS_SINCE] = if seen.turn > 0 { (game.turn - seen.turn) as f32 } else { 0.0 };
        o[OPP_NEXT] = is(pl.opponent) as u8 as f32;
        o[OPP_LAST] = is(pl.last_opponent) as u8 as f32;
        o[OPP_PLACEMENT] = them.placement as f32;
        for m in &seen.board {
            let c = db.get(m.card);
            for (j, t) in Tribe::ALL.into_iter().enumerate() {
                o[OPP_TRIBES + j] += c.tribes.has(t) as u8 as f32;
            }
        }
        i += OPP_SCALARS;
        let owner = Owner { health: them.health, beast_bonus: (0, 0) };
        i += write_slots(game, &mut out[i..], BOARD, seen.board.iter(), owner);
    }
    debug_assert_eq!(i, OBS_SIZE);
}
