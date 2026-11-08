//! The flat action space: every action has a fixed index, and the engine
//! gives a legality mask over all of them.

use crate::{BOARD, CHOICES, HAND, SHOP, TRINKET_CHOICES};

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
    /// Answer an open discard prompt: the hand slot of the card to throw
    /// away (Mangled Bandit's Activate).
    Discard(u8),
    Move { from: u8, to: u8 },
    Roll,
    Freeze,
    Level,
    EndTurn,
    /// Press the MAIN hero power button.
    HeroPower,
    /// Press the SECOND hero power button (Genn, the Reinvigorating Light
    /// trinket): the same action, its own uses and cost.
    HeroPower2,
    /// Pick a trinket from an open offer.
    ChooseTrinket(u8),
    /// Press the Dark Discovery button (every player has it).
    DarkGift,
    /// Buy the Tavern minion in this slot, play it at the end of the board,
    /// sell it, then play and sell what the line put in the hand.
    CycleShop(u8),
    /// Play the hand minion in this slot at the end of the board, sell it,
    /// then play and sell what the line put in the hand.
    CycleHand(u8),
}

const BUY: usize = 0;
const BUY_SPELL: usize = BUY + SHOP;
const SELL: usize = BUY_SPELL + 1;
const ACTIVATE: usize = SELL + BOARD;
const PLAY: usize = ACTIVATE + BOARD;
const TARGET: usize = PLAY + HAND * (BOARD + 1);
const TARGET_SHOP: usize = TARGET + BOARD;
const CHOOSE: usize = TARGET_SHOP + SHOP;
const DISCARD: usize = CHOOSE + CHOICES;
const MOVE: usize = DISCARD + HAND;
const ROLL: usize = MOVE + BOARD * (BOARD - 1);
const FREEZE: usize = ROLL + 1;
const LEVEL: usize = FREEZE + 1;
const END_TURN: usize = LEVEL + 1;
const HERO_POWER: usize = END_TURN + 1;
const HERO_POWER2: usize = HERO_POWER + 1;
const CHOOSE_TRINKET: usize = HERO_POWER2 + 1;
const DARK_GIFT: usize = CHOOSE_TRINKET + TRINKET_CHOICES;
const CYCLE_SHOP: usize = DARK_GIFT + 1;
const CYCLE_HAND: usize = CYCLE_SHOP + SHOP;

pub const N_ACTIONS: usize = CYCLE_HAND + HAND;
pub const N_KINDS: usize = 19;
pub const ACTION_KINDS: [(&str, usize); N_KINDS] = [
    ("buy", SHOP),
    ("buy_spell", 1),
    ("sell", BOARD),
    ("activate", BOARD),
    ("play", HAND * (BOARD + 1)),
    ("target", BOARD),
    ("target_shop", SHOP),
    ("choose", CHOICES),
    ("discard", HAND),
    ("move", BOARD * (BOARD - 1)),
    ("roll", 1),
    ("freeze", 1),
    ("level", 1),
    ("end", 1),
    ("hero_power", 1),
    ("hero_power2", 1),
    ("choose_trinket", TRINKET_CHOICES),
    ("dark_gift", 1),
    ("cycle", SHOP + HAND),
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
            Action::Discard(s) => DISCARD + s as usize,
            Action::Move { from, to } => {
                let to = if to > from { to - 1 } else { to };
                MOVE + from as usize * (BOARD - 1) + to as usize
            }
            Action::Roll => ROLL,
            Action::Freeze => FREEZE,
            Action::Level => LEVEL,
            Action::EndTurn => END_TURN,
            Action::HeroPower => HERO_POWER,
            Action::HeroPower2 => HERO_POWER2,
            Action::ChooseTrinket(k) => CHOOSE_TRINKET + k as usize,
            Action::DarkGift => DARK_GIFT,
            Action::CycleShop(s) => CYCLE_SHOP + s as usize,
            Action::CycleHand(h) => CYCLE_HAND + h as usize,
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
            _ if i < DISCARD => Action::Choose((i - CHOOSE) as u8),
            _ if i < MOVE => Action::Discard((i - DISCARD) as u8),
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
            HERO_POWER => Action::HeroPower,
            HERO_POWER2 => Action::HeroPower2,
            _ if i < DARK_GIFT => Action::ChooseTrinket((i - CHOOSE_TRINKET) as u8),
            DARK_GIFT => Action::DarkGift,
            _ if i < CYCLE_HAND => Action::CycleShop((i - CYCLE_SHOP) as u8),
            _ if i < N_ACTIONS => Action::CycleHand((i - CYCLE_HAND) as u8),
            _ => return None,
        })
    }

    /// Which of the 19 kinds of action this is, in declaration order.
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
            Action::Discard(_) => 8,
            Action::Move { .. } => 9,
            Action::Roll => 10,
            Action::Freeze => 11,
            Action::Level => 12,
            Action::EndTurn => 13,
            Action::HeroPower => 14,
            Action::HeroPower2 => 15,
            Action::ChooseTrinket(_) => 16,
            Action::DarkGift => 17,
            Action::CycleShop(_) | Action::CycleHand(_) => 18,
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
