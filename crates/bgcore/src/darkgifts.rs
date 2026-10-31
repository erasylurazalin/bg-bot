//! Dark Gifts: the 47 gift cards, when each can be offered and which hosts it
//! fits (docs/darkgifts.md). What a gift does is a token in cards.toml, so
//! every effect a gift shares with a card runs through the same code; this
//! file only holds the gift table, the offer rules and the landing.
//!
//! The game side of the Dark Discovery button (the press, the three offers,
//! the pick) is in `game/darkgifts.rs`.

use serde::Deserialize;

use crate::cards::{Card, CardDb, Tribe};
use crate::minion::{CardId, Keywords, Minion};
use crate::rng::Rng;

/// Turns the gifts of a Dark Discovery offer pick from: where the window and
/// the host condition are read, and where a turn-dependent number (Steady
/// Growth) is fixed. Void Power picks its gifts as if it were turn 7.
pub const VOID_POWER_TURN: u32 = 7;

/// Which hosts a gift may land on (docs/darkgifts.md "Era's rules"). Every
/// gift whose logs never showed a condition goes on any minion.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GiftCond {
    Any,
    /// A minion with a Battlecry (Echoing Voice, 6 of 6 in the logs).
    Battlecry,
    /// A minion with a Deathrattle (Jaws of Death, 12 of 12).
    Deathrattle,
    /// A minion of one type (Consanguinity, Polarization, Tarecgosa's
    /// Blessing, Demonology).
    Tribe(Tribe),
    /// A minion with no type at all (Amalgamation, 3 of 3).
    Typeless,
    /// A minion with an end of turn effect (Time Turning, 4 of 4).
    EndOfTurn,
    /// A Murloc that isn't Venomous already (Toxicity).
    MurlocNotVenomous,
    /// A minion with an Activate (Efficiency).
    Activate,
    /// Only while you have a Deity (Worship).
    HasDeity,
    /// A minion whose Spellcrafts could be permanent (Sunken Persistence).
    Spellcraft,
    /// Never on a Dragon (Titanic Strength; Era thinks Admiration too).
    NotDragon,
    /// Never on a minion with no type (Persisting Horror).
    NotTypeless,
    /// Never on a Taunt minion or one with no type (Invulnerability).
    NotTauntOrTypeless,
}

impl GiftCond {
    /// Whether the host meets the condition. `deity` is whether the player
    /// has one. Every tribe question goes through `tribes`, so a minion given
    /// all types by Amalgamation counts.
    pub fn fits(self, card: &Card, tribes: crate::cards::Tribes, deity: bool) -> bool {
        match self {
            GiftCond::Any => true,
            GiftCond::Battlecry => card.battlecry.is_some() || card.choose_one.is_some(),
            GiftCond::Deathrattle => card.deathrattle.is_some(),
            GiftCond::Tribe(t) => tribes.has(t),
            GiftCond::Typeless => tribes.is_neutral(),
            GiftCond::EndOfTurn => card.end_of_turn.is_some(),
            GiftCond::MurlocNotVenomous => {
                tribes.has(Tribe::Murloc) && !card.keywords.has(Keywords::VENOMOUS)
            }
            GiftCond::Activate => card.activate.is_some(),
            GiftCond::HasDeity => deity,
            GiftCond::Spellcraft => card.spellcraft || card.teaches_spell,
            GiftCond::NotDragon => !tribes.has(Tribe::Dragon),
            GiftCond::NotTypeless => !tribes.is_neutral(),
            GiftCond::NotTauntOrTypeless => {
                !card.keywords.has(Keywords::TAUNT) && !tribes.is_neutral()
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct Gift {
    pub name: String,
    /// The game data's card_id (`BG36_MidGameEffect_000t`), for the docs.
    pub id: String,
    /// The card in cards.toml whose effect the host runs.
    pub card: CardId,
    /// Earliest and last turn it can be offered (game tags 4853, 4854); 0
    /// for the last one means "any later turn".
    pub first: u8,
    pub last: u8,
    /// Game tag 4856: 50 means half as likely to be picked for an offer. The
    /// 200s are not used (the logs show they are not more common).
    pub weight: u8,
    pub cond: GiftCond,
}

impl Gift {
    /// Whether this gift's window holds `turn`.
    pub fn in_window(&self, turn: u32) -> bool {
        turn >= self.first as u32 && (self.last == 0 || turn <= self.last as u32)
    }
}

/// One slot of an offer: the tiers it may be, and whether it wants a minion
/// of the player's most common type.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct OfferSlot {
    /// A bit mask of tiers: bit 2 is tier 2.
    pub tiers: u8,
    pub yours: bool,
}

/// The tiers of the three offers for one turn, and every later turn until
/// the next step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OfferStep {
    pub turn: u8,
    pub slots: [OfferSlot; crate::DISCOVER],
}

/// The pool the offers draw from (darkgifts.toml `[pool]`).
#[derive(Clone, Debug, Default)]
pub struct GiftPool {
    /// No Battlecry minion before this turn, 0 for never excluded.
    pub no_battlecry_before: u8,
    pub no_choose_one_before: u8,
    /// Minions left out for balance, by card.
    pub exclude: Vec<CardId>,
}

/// Steady Growth's numbers by the turn the gift was offered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SteadyGrowth {
    pub base_attack: i32,
    pub base_health: i32,
    pub base_turn: u8,
    pub attack_every: u8,
    pub health_every: u8,
    pub max_attack: i32,
    pub max_health: i32,
}

impl Default for SteadyGrowth {
    fn default() -> SteadyGrowth {
        SteadyGrowth {
            base_attack: 2,
            base_health: 2,
            base_turn: 3,
            attack_every: 2,
            health_every: 1,
            max_attack: 10,
            max_health: 20,
        }
    }
}

impl SteadyGrowth {
    /// The +attack/+health a Steady Growth gift carries when it lands on
    /// `turn`: +2/+2 on turn 3, +2/+3 on turn 4, +3/+4 on turn 5, one step
    /// per turn after that (+3/+5, +4/+6, ...), capped.
    pub fn numbers(&self, turn: u32) -> (i32, i32) {
        let t = turn.saturating_sub(self.base_turn as u32);
        let a = self.base_attack + (t / self.attack_every.max(1) as u32) as i32;
        let h = self.base_health + (t / self.health_every.max(1) as u32) as i32;
        (a.min(self.max_attack), h.min(self.max_health))
    }
}


/// The gift table, loaded from darkgifts.toml.
#[derive(Clone, Debug, Default)]
pub struct GiftDb {
    pub gifts: Vec<Gift>,
    /// Gift index + 1 for every card id that is a gift's effect card, 0 for
    /// every other card.
    by_card: Vec<u16>,
    pub pool: GiftPool,
    pub offers: Vec<OfferStep>,
    pub steady_growth: SteadyGrowth,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    #[serde(default)]
    pool: Option<RawPool>,
    #[serde(default)]
    offer: Vec<RawOffer>,
    #[serde(default)]
    steady_growth: Option<RawSteady>,
    #[serde(default)]
    gift: Vec<RawGift>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPool {
    #[serde(default)]
    no_battlecry_before: u8,
    #[serde(default)]
    no_choose_one_before: u8,
    #[serde(default)]
    exclude: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawOffer {
    turn: u8,
    /// "2", "4,5", ...
    tiers: [String; crate::DISCOVER],
    yours: [bool; crate::DISCOVER],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSteady {
    base_attack: i32,
    base_health: i32,
    base_turn: u8,
    attack_every: u8,
    health_every: u8,
    max_attack: i32,
    max_health: i32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawGift {
    name: String,
    id: String,
    card: String,
    #[serde(default)]
    window: Option<Vec<u8>>,
    #[serde(default)]
    weight: u8,
    condition: String,
}

fn condition(name: &str, err: &impl Fn(String) -> String) -> Result<GiftCond, String> {
    let tribe = |t: &str| match t {
        "beast" => Ok(Tribe::Beast),
        "mech" => Ok(Tribe::Mech),
        "dragon" => Ok(Tribe::Dragon),
        "murloc" => Ok(Tribe::Murloc),
        "demon" => Ok(Tribe::Demon),
        "pirate" => Ok(Tribe::Pirate),
        "elemental" => Ok(Tribe::Elemental),
        "undead" => Ok(Tribe::Undead),
        "quilboar" => Ok(Tribe::Quilboar),
        "aberration" => Ok(Tribe::Aberration),
        other => Err(err(format!("unknown gift condition tribe {other:?}"))),
    };
    Ok(match name {
        "any" => GiftCond::Any,
        "battlecry" => GiftCond::Battlecry,
        "deathrattle" => GiftCond::Deathrattle,
        "typeless" => GiftCond::Typeless,
        "end_of_turn" => GiftCond::EndOfTurn,
        "murloc_not_venomous" => GiftCond::MurlocNotVenomous,
        "activate" => GiftCond::Activate,
        "deity" => GiftCond::HasDeity,
        "spellcraft" => GiftCond::Spellcraft,
        "not_dragon" => GiftCond::NotDragon,
        "not_typeless" => GiftCond::NotTypeless,
        "not_taunt_or_typeless" => GiftCond::NotTauntOrTypeless,
        other => match other.strip_prefix("tribe:") {
            Some(t) => GiftCond::Tribe(tribe(t)?),
            None => return Err(err(format!("unknown gift condition {other:?}"))),
        },
    })
}

fn tiers(mask: &str, err: &impl Fn(String) -> String) -> Result<u8, String> {
    let mut out = 0u8;
    for part in mask.split(',') {
        let t: u8 = part
            .trim()
            .parse()
            .map_err(|_| err(format!("bad tier {part:?}")))?;
        if !(1..=crate::CARD_MAX_TIER).contains(&t) {
            return Err(err(format!("tier {t} is not 1 to {}", crate::CARD_MAX_TIER)));
        }
        out |= 1 << t;
    }
    Ok(out)
}

impl GiftDb {
    /// Read darkgifts.toml. The gift cards are looked up by name in the card
    /// set, so a misspelt name is a load error.
    pub fn from_toml(s: &str, cards: &CardDb) -> Result<GiftDb, String> {
        if s.trim().is_empty() {
            return Ok(GiftDb::default());
        }
        let raw: RawFile = toml::from_str(s).map_err(|e| format!("darkgifts.toml: {e}"))?;
        let err = |e: String| format!("darkgifts.toml: {e}");
        let pool = match raw.pool {
            None => GiftPool::default(),
            Some(p) => GiftPool {
                no_battlecry_before: p.no_battlecry_before,
                no_choose_one_before: p.no_choose_one_before,
                exclude: p
                    .exclude
                    .iter()
                    .map(|n| cards.by_name(n).ok_or_else(|| err(format!("unknown card {n:?}"))))
                    .collect::<Result<Vec<_>, _>>()?,
            },
        };
        let mut db = GiftDb { pool, ..GiftDb::default() };
        for o in &raw.offer {
            let mut slots = [OfferSlot::default(); crate::DISCOVER];
            for k in 0..crate::DISCOVER {
                slots[k] = OfferSlot {
                    tiers: tiers(&o.tiers[k], &err)?,
                    yours: o.yours[k],
                };
            }
            db.offers.push(OfferStep { turn: o.turn, slots });
        }
        db.offers.sort_by_key(|o| o.turn);
        db.steady_growth = match raw.steady_growth {
            None => SteadyGrowth::default(),
            Some(g) => SteadyGrowth {
                base_attack: g.base_attack,
                base_health: g.base_health,
                base_turn: g.base_turn,
                attack_every: g.attack_every,
                health_every: g.health_every,
                max_attack: g.max_attack,
                max_health: g.max_health,
            },
        };
        for g in &raw.gift {
            let card = cards
                .by_name(&g.card)
                .ok_or_else(|| err(format!("gift {}: unknown card {:?}", g.name, g.card)))?;
            if card > crate::minion::GIFT_CARD_MAX {
                return Err(err(format!(
                    "gift {}: card id {card} does not fit a gift",
                    g.name
                )));
            }
            let (first, last) = match &g.window {
                None => (1, 0),
                Some(w) if w.len() == 1 => (w[0], 0),
                Some(w) if w.len() == 2 => (w[0], w[1]),
                Some(_) => return Err(err(format!("gift {}: bad window", g.name))),
            };
            if db.gifts.iter().any(|o| o.name == g.name) {
                return Err(err(format!("gift {} appears twice", g.name)));
            }
            db.gifts.push(Gift {
                name: g.name.clone(),
                id: g.id.clone(),
                card,
                first,
                last,
                weight: g.weight,
                cond: condition(&g.condition, &err)?,
            });
        }
        db.by_card = vec![0; cards.cards.len()];
        for (i, g) in db.gifts.iter().enumerate() {
            db.by_card[g.card as usize] = i as u16 + 1;
        }
        Ok(db)
    }

    pub fn len(&self) -> usize {
        self.gifts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.gifts.is_empty()
    }

    /// The index of the gift whose effect card is `card`.
    pub fn gift_of_card(&self, card: CardId) -> Option<usize> {
        self.by_card
            .get(card as usize)
            .copied()
            .filter(|&g| g != 0)
            .map(|g| g as usize - 1)
    }

    /// The offer step for `turn`, None before the first (the button is locked
    /// then).
    pub fn offer_step(&self, turn: u32) -> Option<&OfferStep> {
        if turn < self.offers.first()?.turn as u32 {
            return None;
        }
        self.offers.iter().rev().find(|o| turn >= o.turn as u32)
    }

    /// Whether gift `i` may be offered on this host this turn.
    pub fn eligible(&self, i: usize, turn: u32, card: &Card, tribes: crate::cards::Tribes, deity: bool) -> bool {
        let g = &self.gifts[i];
        g.in_window(turn) && g.cond.fits(card, tribes, deity)
    }

    /// The gifts that fit this host this turn, in table order.
    pub fn candidates(
        &self,
        turn: u32,
        card: &Card,
        tribes: crate::cards::Tribes,
        deity: bool,
    ) -> arrayvec::ArrayVec<u8, 64> {
        let mut out = arrayvec::ArrayVec::new();
        for i in 0..self.gifts.len() {
            if self.eligible(i, turn, card, tribes, deity) {
                let _ = out.try_push(i as u8);
            }
        }
        out
    }

    /// One gift for an offered minion: at random among the gifts whose
    /// window holds `turn` and whose condition the host meets, never one in
    /// `used` (the gifts already on this press's other offers). The four
    /// weight-50 gifts are half as likely; weight 200 is ignored.
    pub fn pick(
        &self,
        turn: u32,
        card: &Card,
        tribes: crate::cards::Tribes,
        deity: bool,
        used: &[u8],
        rng: &mut Rng,
    ) -> Option<usize> {
        let candidates = self.candidates(turn, card, tribes, deity);
        let mut total = 0u32;
        for &i in &candidates {
            if used.contains(&i) {
                continue;
            }
            total += self.weight_of(i as usize);
        }
        if total == 0 {
            return None;
        }
        let mut r = rng.below(total);
        for &i in &candidates {
            if used.contains(&i) {
                continue;
            }
            let w = self.weight_of(i as usize);
            if r < w {
                return Some(i as usize);
            }
            r -= w;
        }
        None
    }

    /// How likely a gift is to be picked: 1, or 1 in 2 for the weight-50
    /// gifts, scaled by 100 so a 50 counts half. Weight 200 and no weight
    /// alike count 100.
    fn weight_of(&self, i: usize) -> u32 {
        match self.gifts[i].weight {
            50 => 50,
            _ => 100,
        }
    }

    /// Whether the Dark Discovery pool leaves this minion out on `turn`
    /// (docs/darkgifts.md "The pool the offers come from"). The flags are on
    /// the card: a Magnetic minion, one with "when you sell this", one whose
    /// effect works in hand, and the balance list in darkgifts.toml.
    pub fn pool_allows(&self, card: &Card, id: CardId, turn: u32) -> bool {
        if card.is_spell() || card.token || card.keywords.has(Keywords::MAGNETIC) {
            return false;
        }
        if card.on_sell.is_some()
            || card.reads_hand()
            || card.while_in_hand.is_some()
            || card.on_card_added.is_some()
            || card.teaches_spell
            || card.spellcraft
        {
            return false;
        }
        if card.battlecry.is_some() && turn < self.pool.no_battlecry_before as u32 {
            return false;
        }
        if card.choose_one.is_some() && turn < self.pool.no_choose_one_before as u32 {
            return false;
        }
        !self.pool.exclude.contains(&id)
    }
}

/// One Dark Gift offer: the three tiers to offer (and whether a slot wants
/// the player's most common type), the turn the gifts are picked as, and
/// whether every offer is Golden.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DarkGiftPlan {
    pub slots: [OfferSlot; crate::DISCOVER],
    /// The turn the gifts are picked as if it were: the button and the
    /// trinkets use the current turn, Void Power turn 7.
    pub gift_turn: u32,
    /// Every offer is the Golden copy of its minion (Sinister Invitation).
    pub golden: bool,
    /// How many turns the pick stays locked in hand (Void Power: until turn
    /// 7).
    pub locked_turns: u8,
}

/// The most common minion type of a board: the tribe a plurality of its
/// minions have, a tie broken at random among the tied ones. None when no
/// minion on it has a type.
pub fn most_common_type(tribes: &[crate::cards::Tribes], rng: &mut Rng) -> Option<Tribe> {
    let mut counts = [0u32; Tribe::EVERY.len()];
    for ts in tribes {
        for (k, t) in Tribe::EVERY.into_iter().enumerate() {
            if ts.has(t) {
                counts[k] += 1;
            }
        }
    }
    let best = *counts.iter().max().unwrap_or(&0);
    if best == 0 {
        return None;
    }
    let tied: arrayvec::ArrayVec<Tribe, 10> = Tribe::EVERY
        .into_iter()
        .enumerate()
        .filter(|&(k, _)| counts[k] == best)
        .map(|(_, t)| t)
        .collect();
    Some(tied[rng.below(tied.len() as u32) as usize])
}

/// The Dark Gifts on a minion as gift indexes, for the observation and the
/// tests. A card that is not a gift's effect card is skipped.
pub fn gifts_of(db: &GiftDb, m: &Minion) -> arrayvec::ArrayVec<usize, { crate::MAX_GIFTS }> {
    m.gift_slice()
        .iter()
        .filter_map(|g| db.gift_of_card(g.card()))
        .collect()
}
