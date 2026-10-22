//! The observation: what one player can see, written as flat f32 arrays.
//! Values are raw (gold 7 is 7.0); scaling belongs to the training side.
//!
//! Layout, in order:
//! - SCALARS own values (30):
//!   0  gold
//!   1  tier
//!   2  upgrade_cost
//!   3  health
//!   4  turn
//!   5  frozen
//!   6  pending_target (1 if a Battlecry, Choose One or spell target is waiting)
//!   7  pending_target_source (board slot + 1 of the minion, or 0 for a spell/none)
//!   8  pending_discover (1 if a Discover choice is waiting)
//!   9  actions_left (remaining actions before the turn cap)
//!   10 alive (number of living players in the lobby)
//!   11 fighting_ghost (1 if paired against the ghost this turn)
//!   12 volumizer_attack (player's permanent Volumizer Attack bonus)
//!   13 volumizer_health (player's permanent Volumizer Health bonus)
//!   14 beetle_attack (player's permanent Beetle Attack bonus)
//!   15 beetle_health (player's permanent Beetle Health bonus)
//!   16 patient_scout_tier (highest discover tier of any Patient Scout on board, 0 if none)
//!   17 gold_next_turn (extra gold queued for next turn)
//!   18 extra_max_gold (permanent increase to the base turn gold cap from Strike Oil)
//!   19 free_rolls (free shop refreshes remaining)
//!   20 spell_discount (cost reduction on the next Tavern spell bought)
//!   21 spell_attack (total Tavern spell Attack bonus, permanent + board auras)
//!   22 spell_health (total Tavern spell Health bonus, permanent + board auras)
//!   23 boon_of_beetles (remaining Boon of Beetles charges)
//!   24 brood_of_nozdormu (pending Brood of Nozdormu Start of Combat triggers)
//!   25 upper_hand (pending Upper Hand Start of Combat triggers)
//!   26 last_spell (card id + 1 of the last Tavern spell cast, 0 if none)
//!   27 won_last_combat (1 if won last fight)
//!   28 lost_last_combat (1 if lost last fight)
//!   29 pending_choose_one (1 if a Choose One choice is waiting)
//! - Own card slots (OWN_SLOTS = 27, each SLOT_FEATURES = 21 values):
//!   shop minions (SHOP = 6), shop spell (SHOP_SPELL = 1), hand (HAND = 10),
//!   board (BOARD = 7), Discover/Choose options (DISCOVER = 3).
//! - 7 opponents, in seat order starting after me (each OPP_BLOCK values):
//!   OPP_SCALARS (6) values (alive, health, tier, seen, turns_since, next),
//!   followed by their board as I last saw it (BOARD = 7 card slots).
//!
//! A card slot is SLOT_FEATURES (21) values:
//!   0  present (1 if occupied, 0 if empty)
//!   1  card id + 1 (one id space covering minions, tokens and spells; 0 if empty)
//!   2  is_spell (1 for a Tavern spell, 0 for a minion)
//!   3  attack (0 for spells)
//!   4  health (0 for spells)
//!   5  golden (0 for spells)
//!   6  tier (1 to 6)
//!   7  tribe_beast (1 for Beast minions, 0 otherwise)
//!   8  tribe_mech (1 for Mech minions, 0 otherwise)
//!   9  tribe_neutral (1 for neutral minions, 0 otherwise)
//!   10..16 keyword flags: taunt, divine_shield, windfury, venomous, reborn, magnetic
//!   16 magnetizations (total Magnetizations on this minion)
//!   17 volumizers (number of Volumizers this minion is or carries inside)
//!   18 improvements (Leviathan, Spark Snapper, Patient Scout, Auto Reveille counter)
//!   19 from_pool (1 if backed by pool copies, 0 if generated/token/spell)
//!   20 winners_bread (pending Winner's Bread stacks on this minion)
//! An empty slot is all zeros.

use crate::cards::Tribe;
use crate::game::{Game, Opponent};
use crate::minion::{Keywords, Minion};
use crate::{BOARD, DISCOVER, HAND, PLAYERS, SHOP};

pub const SHOP_SPELL: usize = 1;
pub const SLOT_FEATURES: usize = 21;
pub const MINION_FEATURES: usize = SLOT_FEATURES;
pub const SCALARS: usize = 30;
pub const OPP_SCALARS: usize = 6;

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
pub const SCALAR_PENDING_TARGET: usize = 6;
pub const SCALAR_PENDING_TARGET_SOURCE: usize = 7;
pub const SCALAR_PENDING_DISCOVER: usize = 8;
pub const SCALAR_ACTIONS_LEFT: usize = 9;
pub const SCALAR_ALIVE: usize = 10;
pub const SCALAR_GHOST: usize = 11;
pub const SCALAR_VOLUMIZER_ATTACK: usize = 12;
pub const SCALAR_VOLUMIZER_HEALTH: usize = 13;
pub const SCALAR_BEETLE_ATTACK: usize = 14;
pub const SCALAR_BEETLE_HEALTH: usize = 15;
pub const SCALAR_PATIENT_SCOUT_TIER: usize = 16;
pub const SCALAR_GOLD_NEXT_TURN: usize = 17;
pub const SCALAR_EXTRA_MAX_GOLD: usize = 18;
pub const SCALAR_FREE_ROLLS: usize = 19;
pub const SCALAR_SPELL_DISCOUNT: usize = 20;
pub const SCALAR_SPELL_ATTACK: usize = 21;
pub const SCALAR_SPELL_HEALTH: usize = 22;
pub const SCALAR_BOON_OF_BEETLES: usize = 23;
pub const SCALAR_BROOD_OF_NOZDORMU: usize = 24;
pub const SCALAR_UPPER_HAND: usize = 25;
pub const SCALAR_LAST_SPELL: usize = 26;
pub const SCALAR_WON_LAST_COMBAT: usize = 27;
pub const SCALAR_LOST_LAST_COMBAT: usize = 28;
pub const SCALAR_PENDING_CHOOSE_ONE: usize = 29;

pub const SLOT_PRESENT: usize = 0;
pub const SLOT_CARD_ID: usize = 1;
pub const SLOT_IS_SPELL: usize = 2;
pub const SLOT_ATTACK: usize = 3;
pub const SLOT_HEALTH: usize = 4;
pub const SLOT_GOLDEN: usize = 5;
pub const SLOT_TIER: usize = 6;
pub const SLOT_BEAST: usize = 7;
pub const SLOT_MECH: usize = 8;
pub const SLOT_NEUTRAL: usize = 9;
pub const SLOT_KEYWORDS: usize = 10;
pub const SLOT_MAGNETIZATIONS: usize = 16;
pub const SLOT_VOLUMIZERS: usize = 17;
pub const SLOT_IMPROVEMENTS: usize = 18;
pub const SLOT_FROM_POOL: usize = 19;
pub const SLOT_WINNERS_BREAD: usize = 20;

pub const OPP_ALIVE: usize = 0;
pub const OPP_HEALTH: usize = 1;
pub const OPP_TIER: usize = 2;
pub const OPP_SEEN: usize = 3;
pub const OPP_TURNS_SINCE: usize = 4;
pub const OPP_NEXT: usize = 5;

fn write_slot(game: &Game, o: &mut [f32], m: &Minion) {
    let c = game.data.cards.get(m.card);
    o[SLOT_PRESENT] = 1.0;
    o[SLOT_CARD_ID] = m.card as f32 + 1.0;
    o[SLOT_IS_SPELL] = c.is_spell() as u8 as f32;
    o[SLOT_TIER] = c.tier as f32;
    if c.is_spell() {
        return;
    }
    o[SLOT_ATTACK] = m.attack as f32;
    o[SLOT_HEALTH] = m.health as f32;
    o[SLOT_GOLDEN] = m.golden as u8 as f32;
    o[SLOT_BEAST] = c.tribes.has(Tribe::Beast) as u8 as f32;
    o[SLOT_MECH] = c.tribes.has(Tribe::Mech) as u8 as f32;
    o[SLOT_NEUTRAL] = c.tribes.is_neutral() as u8 as f32;
    // The layout has room for the first six keywords. Stealth, Dragon, Murloc
    // and Demon are not in the observation yet, it is redone later.
    for (j, (bit, _)) in Keywords::ALL.iter().take(6).enumerate() {
        o[SLOT_KEYWORDS + j] = m.keywords.has(*bit) as u8 as f32;
    }
    o[SLOT_MAGNETIZATIONS] = m.magnetizations as f32;
    o[SLOT_VOLUMIZERS] = m.volumizers as f32;
    o[SLOT_IMPROVEMENTS] = m.improvements as f32;
    o[SLOT_FROM_POOL] = (m.pool_copies > 0) as u8 as f32;
    o[SLOT_WINNERS_BREAD] = m.winners_bread as f32;
}

fn write_slots<'a>(
    game: &Game,
    out: &mut [f32],
    slots: usize,
    ms: impl Iterator<Item = &'a Minion>,
) -> usize {
    let n = slots * SLOT_FEATURES;
    out[..n].fill(0.0);
    for (k, m) in ms.take(slots).enumerate() {
        let o = &mut out[k * SLOT_FEATURES..(k + 1) * SLOT_FEATURES];
        write_slot(game, o, m);
    }
    n
}

/// Write player `p`'s view into `out`, which must hold OBS_SIZE values.
pub fn write(game: &Game, p: usize, out: &mut [f32]) {
    assert_eq!(out.len(), OBS_SIZE);
    let pl = &game.players[p];
    let r = &game.data.rules;
    let (spell_atk, spell_hp) = game.total_spell_bonus(p);
    let s = [
        pl.gold as f32,
        pl.tier as f32,
        pl.upgrade_cost as f32,
        pl.health as f32,
        game.turn as f32,
        pl.frozen as u8 as f32,
        pl.pending_target.is_some() as u8 as f32,
        pl.pending_target.and_then(|t| t.source).map_or(0.0, |s| s as f32 + 1.0),
        !pl.discover.is_empty() as u8 as f32,
        r.max_actions_per_turn.saturating_sub(pl.actions) as f32,
        game.alive() as f32,
        matches!(pl.opponent, Some(Opponent::Ghost(_))) as u8 as f32,
        pl.volumizer_bonus.0 as f32,
        pl.volumizer_bonus.1 as f32,
        pl.beetle_bonus.0 as f32,
        pl.beetle_bonus.1 as f32,
        pl.patient_scout_tier(&game.data.cards) as f32,
        pl.gold_next_turn as f32,
        pl.extra_max_gold as f32,
        pl.free_rolls as f32,
        pl.spell_discount as f32,
        spell_atk as f32,
        spell_hp as f32,
        pl.boon_of_beetles as f32,
        pl.brood_of_nozdormu as f32,
        pl.upper_hand as f32,
        pl.last_spell.map_or(0.0, |c| c as f32 + 1.0),
        pl.won_last_combat as u8 as f32,
        pl.lost_last_combat as u8 as f32,
        pl.pending_choose_one.is_some() as u8 as f32,
    ];
    out[..SCALARS].copy_from_slice(&s);
    let mut i = SCALARS;
    i += write_slots(game, &mut out[i..], SHOP, pl.shop.iter());
    out[i..i + SHOP_SPELL * SLOT_FEATURES].fill(0.0);
    if let Some(sp) = pl.shop_spell {
        let m = pl.make_card(&game.data.cards, sp, false, 0);
        write_slot(game, &mut out[i..i + SLOT_FEATURES], &m);
    }
    i += SHOP_SPELL * SLOT_FEATURES;
    i += write_slots(game, &mut out[i..], HAND, pl.hand.iter());
    i += write_slots(game, &mut out[i..], BOARD, pl.board.iter());
    out[i..i + DISCOVER * SLOT_FEATURES].fill(0.0);
    for (k, &c) in pl.discover.iter().take(DISCOVER).enumerate() {
        let pool_copies = u8::from(!game.data.cards.get(c).is_spell());
        let m = pl.make_card(&game.data.cards, c, false, pool_copies);
        write_slot(game, &mut out[i + k * SLOT_FEATURES..i + (k + 1) * SLOT_FEATURES], &m);
    }
    i += DISCOVER * SLOT_FEATURES;
    for k in 1..PLAYERS {
        let q = (p + k) % PLAYERS;
        let them = &game.players[q];
        let seen = &pl.seen[q];
        let next = match pl.opponent {
            Some(Opponent::Player(x)) | Some(Opponent::Ghost(x)) => x as usize == q,
            None => false,
        };
        let o = [
            them.alive as u8 as f32,
            them.health as f32,
            them.tier as f32,
            (seen.turn > 0) as u8 as f32,
            if seen.turn > 0 { (game.turn - seen.turn) as f32 } else { 0.0 },
            next as u8 as f32,
        ];
        out[i..i + OPP_SCALARS].copy_from_slice(&o);
        i += OPP_SCALARS;
        i += write_slots(game, &mut out[i..], BOARD, seen.board.iter());
    }
    debug_assert_eq!(i, OBS_SIZE);
}
