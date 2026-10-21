pub type CardId = u8;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Keywords(pub u8);

impl Keywords {
    pub const TAUNT: u8 = 1;
    pub const DIVINE_SHIELD: u8 = 2;
    pub const WINDFURY: u8 = 4;
    pub const VENOMOUS: u8 = 8;
    pub const REBORN: u8 = 16;

    pub const ALL: [(u8, &'static str); 5] = [
        (Self::TAUNT, "taunt"),
        (Self::DIVINE_SHIELD, "divine_shield"),
        (Self::WINDFURY, "windfury"),
        (Self::VENOMOUS, "venomous"),
        (Self::REBORN, "reborn"),
    ];

    pub fn has(self, k: u8) -> bool {
        self.0 & k != 0
    }

    pub fn set(&mut self, k: u8) {
        self.0 |= k;
    }

    pub fn clear(&mut self, k: u8) {
        self.0 &= !k;
    }
}

/// A minion in a shop, hand, board or fight. Health is current health; in
/// the shop phase it is also the maximum, since damage never carries over.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Minion {
    pub card: CardId,
    pub attack: i32,
    pub health: i32,
    pub keywords: Keywords,
    pub golden: bool,
}
