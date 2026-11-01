//! The exact pool and state accounting the fuzzer checks.

use crate::minion::CardId;
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
            for (i, &c) in pl.discover.iter().enumerate() {
                // A Discover whose options are ready-made minions
                // (Kel'Thuzad's Kitty) took no pool copy at all.
                if pl.discover_minions.is_empty() && !cards.get(c).is_spell() && !cards.get(c).token {
                    let slot = cards.get(c).pool_slot as usize;
                    // A Dark Gift offer holds one copy, three for a Golden one
                    // and none for a Dagger copy.
                    let n = if pl.dark_offer {
                        pl.discover_copies.get(i).copied().unwrap_or(1)
                    } else {
                        1
                    };
                    held[slot] += n as u32;
                }
            }
            // A Tavern minion may be above the player's tier: Robust
            // Evolution (which ignores the Tavern tier) can put one there.
            // Generated cards (Fishbait) may be any Tier too, and a Golden
            // Touch makes a Tavern minion golden. The spell slot's card may be
            // above the tier for the same reason: Evolving Tavern replaces it
            // with one a Tier higher (Era's rule), so only "it is a spell" is
            // checked here.
            if let Some(sp) = pl.shop_spell
                && !cards.get(sp).is_spell()
            {
                return Err(format!("player {i}: shop_spell invalid"));
            }
            // The minion slots plus the spell slot, which a Fodder can take
            // (and fill with a minion once it feeds).
            // The Tavern holds its minion slots plus the spell slot, which a
            // Fodder can take: one card more. Twice as Nice's copy sits
            // outside the normal slots, one more again, and a Tavern filled
            // before a power or trinket changed its size (Stay Frosty takes a
            // slot away, a trinket adds one) keeps what it had.
            // The Warband Whistle's Refresh holds one copy per warband
            // minion, up to the seven slots and with no spell then, so its
            // Tavern can be bigger than the tier's size (`fill_shop`).
            let size = pl.shop_size(&self.data.rules).max(crate::SHOP - 1);
            if pl.shop.len() > size + 2 {
                return Err(format!("player {i}: shop too big"));
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
                // The card a trinket remembers (Portable Factory) is a real
                // card of the table.
                if let Some(c) = st.card
                    && c as usize >= cards.cards.len()
                {
                    return Err(format!("player {i}: trinket slot {slot} remembers card {c}"));
                }
            }
            // A Discover waiting to be remembered belongs to a held slot.
            if pl.tp.remember_in != 0 {
                let slot = pl.tp.remember_in as usize - 1;
                if slot >= crate::TRINKET_SLOTS || pl.trinkets[slot].of == 0 {
                    return Err(format!(
                        "player {i}: a Discover would be remembered by an empty slot"
                    ));
                }
            }
            {
                let mut mask = 0;
                let mut cube = false;
                for st in pl.trinkets.iter() {
                    cube |= st.cube;
                    if st.of != 0 {
                        mask |= self.data.trinkets.get(st.of - 1).event_mask();
                    }
                }
                // A cube slot wants the start of the turn whatever it holds
                // (see `refresh_trinket_mask`).
                if cube {
                    mask |= crate::trinkets::STARTS_TURN | crate::trinkets::HAS_TRINKET;
                }
                // The fight bundle holds every held trinket's fight acts at
                // once (`FIGHT_ACTS`); a table that outgrows it would drop
                // one silently in `fight_trinkets`.
                let acts: usize = pl
                    .trinkets
                    .iter()
                    .filter(|st| st.of != 0)
                    .map(|st| self.data.trinkets.get(st.of - 1).fight.len())
                    .sum();
                if acts > crate::trinkets::FIGHT_ACTS {
                    return Err(format!(
                        "player {i}: {acts} fight acts held, the bundle holds {}",
                        crate::trinkets::FIGHT_ACTS
                    ));
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
                // A trinket offer holds a plain index into the table, while a
                // slot's `of` is that index plus one.
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
            for st in [pl.power, pl.power2].into_iter().flatten() {
                if st.of as usize >= self.data.heroes.powers.len() {
                    return Err(format!("player {i}: power {}", st.of));
                }
                let power = self.data.heroes.power(st.of);
                if st.uses_left > power.uses_per_turn.saturating_add(pl.tp.power_extra_uses) {
                    return Err(format!("player {i}: {} uses left", st.uses_left));
                }
            }
            // The Secret zone holds each kind once, and an open offer holds
            // only kinds the zone lacks (one each), so a press can never offer
            // a Secret the player already has.
            if pl.secrets.len() > crate::secrets::MAX_ZONE {
                return Err(format!("player {i}: {} Secrets held", pl.secrets.len()));
            }
            for (k, &kind) in pl.secret_offer.iter().enumerate() {
                if kind >= crate::secrets::COMBAT_SECRETS {
                    return Err(format!("player {i}: a Secret offer of kind {kind}"));
                }
                if pl.secrets.has(kind) || pl.secret_offer[..k].contains(&kind) {
                    return Err(format!("player {i}: a Secret offered twice or already held"));
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
            // Dark Paradox: only the one version this game picked is in the
            // pool, with its tier's usual copies.
            let want = if card.paradox && self.paradox != Some(c as CardId) {
                0
            } else if card.pool_slot as usize == c && card.lobby_ok(self.lobby) {
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
