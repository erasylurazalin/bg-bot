//! The exact pool and state accounting the fuzzer checks.

use crate::{
    MAX_TIER,
    PLAYERS,
};
use super::{
    Game,
    MAX_POOL_CARDS,
};

impl Game {
    /// Every card copy is in the pool, held by someone, or consumed by a
    /// Magnetization; nothing is over its limit; and the dead hold nothing.
    /// Checked after every step in debug builds; the fuzzer calls it in
    /// release too.
    pub fn check_invariants(&self) -> Result<(), String> {
        let cards = &self.data.cards;
        let mut held = [0u32; MAX_POOL_CARDS];
        for (i, pl) in self.players.iter().enumerate() {
            for m in pl.shop.iter().chain(&pl.hand).chain(&pl.board) {
                let c = cards.get(m.card);
                if !c.token && !c.is_spell() {
                    let slot = c.pool_slot as usize;
                    held[slot] += m.pool_copies as u32;
                }
            }
            for &c in &pl.discover {
                if !cards.get(c).is_spell() {
                    let slot = cards.get(c).pool_slot as usize;
                    held[slot] += 1;
                }
            }
            // A Tavern minion may be above the player's tier: Robust
            // Evolution (which ignores the Tavern tier) can put one there.
            // Generated cards (Fishbait) may be any Tier too, and a Golden
            // Touch makes a Tavern minion golden.
            if let Some(sp) = pl.shop_spell
                && (!cards.get(sp).is_spell() || cards.get(sp).tier > pl.tier)
            {
                return Err(format!("player {i}: shop_spell invalid"));
            }
            // The minion slots plus the spell slot, which a Fodder can take
            // (and fill with a minion once it feeds).
            if pl.shop.len() > self.data.rules.shop_size(pl.tier) + 1 {
                return Err(format!("player {i}: shop too big"));
            }
            if pl.shop.len() == self.data.rules.shop_size(pl.tier) + 1
                && pl.shop_spell.is_some()
            {
                return Err(format!("player {i}: shop spell in a taken slot"));
            }
            if let Some(pt) = pl.pending_target
                && let Some(src) = pt.source
                && src as usize >= pl.board.len()
            {
                return Err(format!("player {i}: pending target source off the board"));
            }
            if let Some(pst) = pl.pending_shop_target
                && (pst.source as usize >= pl.board.len() || pl.shop.is_empty())
            {
                return Err(format!("player {i}: pending shop target has nothing to act on"));
            }
            if let Some(pd) = pl.pending_discard
                && let Some(src) = pd.source
                && src as usize >= pl.board.len()
            {
                return Err(format!("player {i}: pending discard source off the board"));
            }
            if let Some(pco) = pl.pending_choose_one
                && let Some(src) = pco.source
                && src as usize >= pl.board.len()
            {
                return Err(format!("player {i}: pending choose_one source off the board"));
            }
            for (slot, st) in pl.trinkets.iter().enumerate() {
                if st.of == 0 {
                    continue;
                }
                if st.of as usize > self.data.trinkets.len() {
                    return Err(format!("player {i}: trinket slot {slot} holds no trinket"));
                }
                // Either slot can hold either kind now: Trip Vouchers puts a
                // greater offer into the lesser slot, and Mysterious Orb
                // makes the next offer a lesser one (DESIGN.md, second
                // round). The power slot holds whatever the power offered.
            }
            {
                let mut mask = 0;
                for st in pl.trinkets.iter() {
                    if st.of != 0 {
                        mask |= self.data.trinkets.get(st.of - 1).event_mask();
                    }
                }
                if mask != pl.trinket_events {
                    return Err(format!(
                        "player {i}: the trinket event mask is stale ({:#x} not {:#x})",
                        pl.trinket_events, mask
                    ));
                }
            }
            if pl.trinket_offer_slot > crate::TRINKET_SLOTS as u8 {
                return Err(format!("player {i}: trinket offer for no slot"));
            }
            for &(of, price) in &pl.trinket_offer {
                if of as usize >= self.data.trinkets.len() {
                    return Err(format!("player {i}: a trinket offer holds no trinket"));
                }
                if price > self.data.trinkets.get(of).cost {
                    return Err(format!("player {i}: a trinket offer is dearer than the card"));
                }
            }
            if !pl.trinket_offer.is_empty() && pl.trinket_offer_slot == 0 {
                return Err(format!("player {i}: a trinket offer with no slot"));
            }
            let holds_cards = !(pl.shop.is_empty()
                && pl.shop_spell.is_none()
                && pl.hand.is_empty()
                && pl.board.is_empty()
                && pl.discover.is_empty());
            if !pl.alive && holds_cards {
                return Err(format!(
                    "player {i}: dead but holds cards at turn {} (place {} done {} shop {} hand {} board {} discover {} spell {})",
                    self.turn, pl.placement, pl.done,
                    pl.shop.len(), pl.hand.len(), pl.board.len(), pl.discover.len(),
                    pl.shop_spell.is_some()
                ));
            }
            if !(1..=MAX_TIER).contains(&pl.tier) {
                return Err(format!("player {i}: tier {}", pl.tier));
            }
            if pl.armor < 0 {
                return Err(format!("player {i}: armor {}", pl.armor));
            }
            // No hero is offered twice in a lobby, so no two players share one.
            if let Some(h) = pl.hero {
                if (0..PLAYERS).any(|q| q != i && self.players[q].hero == Some(h)) {
                    return Err(format!("player {i}: hero {} twice", self.data.heroes.get(h).name));
                }
                if pl.power.is_none() {
                    return Err(format!("player {i}: a hero without a power"));
                }
            }
            if let Some(st) = pl.power {
                if st.of as usize >= self.data.heroes.heroes.len() {
                    return Err(format!("player {i}: power of hero {}", st.of));
                }
                let power = &self.data.heroes.get(st.of).power;
                if st.uses_left > power.uses_per_turn {
                    return Err(format!("player {i}: {} uses left", st.uses_left));
                }
            }
            if pl
                .hero_offer
                .iter()
                .any(|&h| h != u8::MAX && h as usize >= self.data.heroes.heroes.len())
            {
                return Err(format!("player {i}: hero offer out of range"));
            }
        }
        for c in 0..cards.pool_cards {
            let card = &cards.cards[c];
            // A card outside the lobby holds no copies anywhere: it was never
            // in this game's pool.
            let want = if card.pool_slot as usize == c && card.lobby_ok(self.lobby) {
                self.data.rules.copies(card.tier) as u32
            } else {
                0
            };
            let have = self.pool[c] as u32 + held[c] + self.consumed[c];
            if have != want {
                return Err(format!("{}: {have} copies, should be {want}", card.name));
            }
        }
        if self.over {
            let mut places = self.placements();
            places.sort();
            if places != [1, 2, 3, 4, 5, 6, 7, 8] {
                return Err(format!("placements {places:?}"));
            }
        }
        Ok(())
    }
}
