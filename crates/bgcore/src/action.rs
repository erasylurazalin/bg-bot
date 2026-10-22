//! The flat action space: every action has a fixed index, and the engine
//! gives a legality mask over all of them.

use crate::{BOARD, DISCOVER, HAND, SHOP};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    Buy(u8),
    BuySpell,
    Sell(u8),
    /// Pay the Activate cost of the board minion in this slot.
    Activate(u8),
    Play { hand: u8, pos: u8 },
    Target(u8),
    /// Pick a minion in the Tavern (shop slot), for a chosen consume.
    TargetShop(u8),
    Choose(u8),
    Move { from: u8, to: u8 },
    Roll,
    Freeze,
    Level,
    EndTurn,
}

const BUY: usize = 0;
const BUY_SPELL: usize = BUY + SHOP;
const SELL: usize = BUY_SPELL + 1;
const ACTIVATE: usize = SELL + BOARD;
const PLAY: usize = ACTIVATE + BOARD;
const TARGET: usize = PLAY + HAND * (BOARD + 1);
const TARGET_SHOP: usize = TARGET + BOARD;
const CHOOSE: usize = TARGET_SHOP + SHOP;
const MOVE: usize = CHOOSE + DISCOVER;
const ROLL: usize = MOVE + BOARD * (BOARD - 1);
const FREEZE: usize = ROLL + 1;
const LEVEL: usize = FREEZE + 1;
const END_TURN: usize = LEVEL + 1;

pub const N_ACTIONS: usize = END_TURN + 1;
pub const N_KINDS: usize = 13;
pub const ACTION_KINDS: [(&str, usize); N_KINDS] = [
    ("buy", SHOP),
    ("buy_spell", 1),
    ("sell", BOARD),
    ("activate", BOARD),
    ("play", HAND * (BOARD + 1)),
    ("target", BOARD),
    ("target_shop", SHOP),
    ("choose", DISCOVER),
    ("move", BOARD * (BOARD - 1)),
    ("roll", 1),
    ("freeze", 1),
    ("level", 1),
    ("end", 1),
];

impl Action {
    pub fn index(self) -> usize {
        match self {
            Action::Buy(s) => BUY + s as usize,
            Action::BuySpell => BUY_SPELL,
            Action::Sell(s) => SELL + s as usize,
            Action::Activate(s) => ACTIVATE + s as usize,
            Action::Play { hand, pos } => PLAY + hand as usize * (BOARD + 1) + pos as usize,
            Action::Target(s) => TARGET + s as usize,
            Action::TargetShop(s) => TARGET_SHOP + s as usize,
            Action::Choose(k) => CHOOSE + k as usize,
            Action::Move { from, to } => {
                let to = if to > from { to - 1 } else { to };
                MOVE + from as usize * (BOARD - 1) + to as usize
            }
            Action::Roll => ROLL,
            Action::Freeze => FREEZE,
            Action::Level => LEVEL,
            Action::EndTurn => END_TURN,
        }
    }

    pub fn from_index(i: usize) -> Option<Action> {
        Some(match i {
            _ if i < BUY_SPELL => Action::Buy((i - BUY) as u8),
            BUY_SPELL => Action::BuySpell,
            _ if i < ACTIVATE => Action::Sell((i - SELL) as u8),
            _ if i < PLAY => Action::Activate((i - ACTIVATE) as u8),
            _ if i < TARGET => {
                let j = i - PLAY;
                Action::Play { hand: (j / (BOARD + 1)) as u8, pos: (j % (BOARD + 1)) as u8 }
            }
            _ if i < TARGET_SHOP => Action::Target((i - TARGET) as u8),
            _ if i < CHOOSE => Action::TargetShop((i - TARGET_SHOP) as u8),
            _ if i < MOVE => Action::Choose((i - CHOOSE) as u8),
            _ if i < ROLL => {
                let j = i - MOVE;
                let from = j / (BOARD - 1);
                let to = j % (BOARD - 1);
                let to = if to >= from { to + 1 } else { to };
                Action::Move { from: from as u8, to: to as u8 }
            }
            ROLL => Action::Roll,
            FREEZE => Action::Freeze,
            LEVEL => Action::Level,
            END_TURN => Action::EndTurn,
            _ => return None,
        })
    }

    /// Which of the 13 kinds of action this is, in declaration order.
    pub fn kind(self) -> usize {
        match self {
            Action::Buy(_) => 0,
            Action::BuySpell => 1,
            Action::Sell(_) => 2,
            Action::Activate(_) => 3,
            Action::Play { .. } => 4,
            Action::Target(_) => 5,
            Action::TargetShop(_) => 6,
            Action::Choose(_) => 7,
            Action::Move { .. } => 8,
            Action::Roll => 9,
            Action::Freeze => 10,
            Action::Level => 11,
            Action::EndTurn => 12,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_round_trips() {
        for i in 0..N_ACTIONS {
            let a = Action::from_index(i).unwrap();
            assert_eq!(a.index(), i, "{a:?}");
        }
        assert_eq!(Action::from_index(N_ACTIONS), None);
        let mut i = 0;
        for (kind, &(_, size)) in ACTION_KINDS.iter().enumerate() {
            for _ in 0..size {
                assert_eq!(Action::from_index(i).unwrap().kind(), kind);
                i += 1;
            }
        }
        assert_eq!(i, N_ACTIONS);
    }
}
