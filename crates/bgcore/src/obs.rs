//! The observation: what one player can see, written as flat f32 arrays.
//! Values are raw (gold 7 is 7.0); scaling belongs to the training side.
//!
//! Layout, in order:
//! - SCALARS own values, see `write`
//! - shop, hand, board, discover: SHOP + HAND + BOARD + DISCOVER minion slots
//! - 7 opponents, in seat order starting after me: OPP_SCALARS each, then
//!   their board as I last saw it, BOARD minion slots
//!
//! A minion slot is MINION_FEATURES values: present, card id + 1, attack,
//! health, golden, tier, then one flag per keyword (taunt, divine shield,
//! windfury, venomous, reborn). An empty slot is all zeros.

use crate::game::{Game, Opponent};
use crate::minion::{Keywords, Minion};
use crate::{BOARD, DISCOVER, HAND, PLAYERS, SHOP};

pub const MINION_FEATURES: usize = 11;
pub const SCALARS: usize = 12;
pub const OPP_SCALARS: usize = 6;
pub const OBS_SIZE: usize = SCALARS
    + (SHOP + HAND + BOARD + DISCOVER) * MINION_FEATURES
    + (PLAYERS - 1) * (OPP_SCALARS + BOARD * MINION_FEATURES);

fn minions<'a>(game: &Game, out: &mut [f32], slots: usize, ms: impl Iterator<Item = &'a Minion>) -> usize {
    let n = slots * MINION_FEATURES;
    out[..n].fill(0.0);
    for (k, m) in ms.take(slots).enumerate() {
        let o = &mut out[k * MINION_FEATURES..(k + 1) * MINION_FEATURES];
        o[0] = 1.0;
        o[1] = m.card as f32 + 1.0;
        o[2] = m.attack as f32;
        o[3] = m.health as f32;
        o[4] = m.golden as u8 as f32;
        o[5] = game.data.cards.get(m.card).tier as f32;
        for (j, (bit, _)) in Keywords::ALL.iter().take(5).enumerate() {
            o[6 + j] = m.keywords.has(*bit) as u8 as f32;
        }
    }
    n
}

/// Write player `p`'s view into `out`, which must hold OBS_SIZE values.
pub fn write(game: &Game, p: usize, out: &mut [f32]) {
    assert_eq!(out.len(), OBS_SIZE);
    let pl = &game.players[p];
    let r = &game.data.rules;
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
    ];
    out[..SCALARS].copy_from_slice(&s);
    let mut i = SCALARS;
    i += minions(game, &mut out[i..], SHOP, pl.shop.iter());
    i += minions(game, &mut out[i..], HAND, pl.hand.iter());
    i += minions(game, &mut out[i..], BOARD, pl.board.iter());
    let options: Vec<Minion> = pl.discover.iter().map(|&c| game.data.cards.make(c, false)).collect();
    i += minions(game, &mut out[i..], DISCOVER, options.iter());
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
        i += minions(game, &mut out[i..], BOARD, seen.board.iter());
    }
    debug_assert_eq!(i, OBS_SIZE);
}
