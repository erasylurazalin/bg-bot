//! The flat action space: every action has a fixed index, and the engine
//! gives a legality mask over all of them.

use crate::{BOARD, DISCOVER, HAND, SHOP};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    Buy(u8),
    Sell(u8),
    Play { hand: u8, pos: u8 },
    Target(u8),
    Choose(u8),
    Move { from: u8, to: u8 },
    Roll,
    Freeze,
    Level,
    EndTurn,
}

const BUY: usize = 0;
const SELL: usize = BUY + SHOP;
const PLAY: usize = SELL + BOARD;
const TARGET: usize = PLAY + HAND * (BOARD + 1);
const CHOOSE: usize = TARGET + BOARD;
const MOVE: usize = CHOOSE + DISCOVER;
const ROLL: usize = MOVE + BOARD * (BOARD - 1);
const FREEZE: usize = ROLL + 1;
const LEVEL: usize = FREEZE + 1;
const END_TURN: usize = LEVEL + 1;

pub const N_ACTIONS: usize = END_TURN + 1;
pub const N_KINDS: usize = 10;

impl Action {
    pub fn index(self) -> usize {
        match self {
            Action::Buy(s) => BUY + s as usize,
            Action::Sell(s) => SELL + s as usize,
            Action::Play { hand, pos } => PLAY + hand as usize * (BOARD + 1) + pos as usize,
            Action::Target(s) => TARGET + s as usize,
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
            _ if i < SELL => Action::Buy((i - BUY) as u8),
            _ if i < PLAY => Action::Sell((i - SELL) as u8),
            _ if i < TARGET => {
                let j = i - PLAY;
                Action::Play { hand: (j / (BOARD + 1)) as u8, pos: (j % (BOARD + 1)) as u8 }
            }
            _ if i < CHOOSE => Action::Target((i - TARGET) as u8),
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

    /// Which of the 10 kinds of action this is, in declaration order.
    pub fn kind(self) -> usize {
        match self {
            Action::Buy(_) => 0,
            Action::Sell(_) => 1,
            Action::Play { .. } => 2,
            Action::Target(_) => 3,
            Action::Choose(_) => 4,
            Action::Move { .. } => 5,
            Action::Roll => 6,
            Action::Freeze => 7,
            Action::Level => 8,
            Action::EndTurn => 9,
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
    }
}
