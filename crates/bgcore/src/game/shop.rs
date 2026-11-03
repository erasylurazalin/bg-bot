//! The Tavern: buying, selling, Refresh, filling the shop, and the pool.

use arrayvec::ArrayVec;
use crate::trinkets::Event;
use crate::cards::{
    Card,
    CardDb,
    Effect,
    EventKind,
    OnSell,
    Tribe,
    Tribes,
};
use crate::heroes::PowerEffect;
use crate::minion::{
    CardId,
    Minion,
};
use crate::rng::Rng;
use crate::rules::Rules;
use crate::{
    MAX_TIER,
    SHOP,
};
use super::{
    DiscoverRequest,
    Game,
    MAX_POOL_CARDS,
    PendingShopTarget,
    Player,
    RefreshKind,
    ShopTargetKind,
};
use super::effects::buff;

/// Draw one card from the pool, weighted by copies left, among cards of
/// tier `lo..=hi` that satisfy `pred` and aren't in `exclude`. When the shared
/// Volumizer slot is drawn, picks a random Volumizer color not in `exclude`.
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_filtered(
    pool: &mut [u8],
    cards: &CardDb,
    rng: &mut Rng,
    lo: u8,
    hi: u8,
    exclude: &[CardId],
    lobby: Tribes,
    pred: impl Fn(&Card) -> bool,
) -> Option<CardId> {
    // A card with no copies left (or one that is not a pool slot, which never
    // holds copies) adds nothing, so the cheap checks run first.
    let ok = |c: usize| {
        if pool[c] == 0 {
            return false;
        }
        let t = cards.tiers[c];
        if !(lo <= t && t <= hi) {
            return false;
        }
        let card = &cards.cards[c];
        if card.pool_slot as usize != c || !card.lobby_ok(lobby) || !pred(card) {
            return false;
        }
        if card.is_volumizer {
            cards.volumizers.iter().any(|v| !exclude.contains(v))
        } else {
            exclude.is_empty() || !exclude.contains(&(c as CardId))
        }
    };
    let mut eligible: ArrayVec<(u8, u8), MAX_POOL_CARDS> = ArrayVec::new();
    let mut total = 0u32;
    // Ids in ascending order, so the walk below sees cards in the same order
    // as a scan of the whole pool; `ok` still checks everything.
    for &c in &cards.ids_up_to_tier[(hi as usize).min(crate::CARD_MAX_TIER as usize)] {
        let c = c as usize;
        if ok(c) {
            total += pool[c] as u32;
            eligible.push((c as u8, pool[c]));
        }
    }
    if total == 0 {
        return None;
    }
    let mut r = rng.below(total);
    for &(c, n) in &eligible {
        let c = c as usize;
        let n = n as u32;
        if r < n {
            pool[c] -= 1;
            if cards.cards[c].is_volumizer {
                let mut avail: ArrayVec<CardId, 3> = ArrayVec::new();
                for &v in &cards.volumizers {
                    if !exclude.contains(&v) {
                        let _ = avail.try_push(v);
                    }
                }
                let k = rng.below(avail.len() as u32) as usize;
                return Some(avail[k]);
            }
            return Some(c as CardId);
        }
        r -= n;
    }
    unreachable!()
}

/// Draw one Golden minion from the pool: a card of tier `lo..=hi` (with a
/// type when `typed`), weighted by copies left, taking the three copies a
/// Golden minion stands for (fewer when the pool has fewer; cards with all
/// three left are preferred). The lobby filters it.
pub(super) fn draw_golden(
    pool: &mut [u8],
    cards: &CardDb,
    rng: &mut Rng,
    lo: u8,
    hi: u8,
    typed: bool,
    lobby: Tribes,
) -> Option<(CardId, u8)> {
    for min in [3u8, 1] {
        let ok = |c: usize| {
            let card = &cards.cards[c];
            pool[c] >= min
                && card.pool_slot as usize == c
                && lo <= card.tier
                && card.tier <= hi
                && (!typed || !card.tribes.is_neutral())
                && card.lobby_ok(lobby)
        };
        let total: u32 = (0..cards.pool_cards).filter(|&c| ok(c)).map(|c| pool[c] as u32).sum();
        if total == 0 {
            continue;
        }
        let mut r = rng.below(total);
        for c in 0..cards.pool_cards {
            if !ok(c) {
                continue;
            }
            let n = pool[c] as u32;
            if r < n {
                let take = pool[c].min(3);
                pool[c] -= take;
                let id = if cards.cards[c].is_volumizer {
                    cards.volumizers[rng.below(cards.volumizers.len() as u32) as usize]
                } else {
                    c as CardId
                };
                return Some((id, take));
            }
            r -= n;
        }
    }
    None
}

/// Make a minion Golden where it stands: its stats go to the golden card's
/// (double the printed ones), keeping every buff.
pub(super) fn golden_up(cards: &CardDb, m: &mut Minion) {
    if m.golden {
        return;
    }
    let base = cards.get(m.card);
    let (ga, gh) = base.golden_stats.unwrap_or((base.attack * 2, base.health * 2));
    m.golden = true;
    m.attack += ga - base.attack;
    m.health += gh - base.health;
}

pub(crate) fn give_back(pool: &mut [u8], cards: &CardDb, m: &Minion) {
    let c = cards.get(m.card);
    if m.pool_copies > 0 && !c.token && !c.is_spell() {
        let slot = c.pool_slot as usize;
        pool[slot] += m.pool_copies;
    }
}

/// `give_back`'s opposite: take a card's pool copies again, for a minion that
/// survived something that put them back.
fn take_copy(pool: &mut [u8], cards: &CardDb, m: &Minion) {
    let c = cards.get(m.card);
    if m.pool_copies > 0 && !c.token && !c.is_spell() {
        let slot = c.pool_slot as usize;
        pool[slot] = pool[slot].saturating_sub(m.pool_copies);
    }
}

impl Game {
    /// The Attack and Health of the highest-Health minion in the player's
    /// Tavern, a random one of them on a Health tie (no random number is
    /// drawn without a tie). None with no minion there.
    pub(super) fn highest_tavern_stats(&mut self, p: usize) -> Option<(i32, i32)> {
        let cards = &self.data.cards;
        let mut best: ArrayVec<(i32, i32), { crate::SHOP }> = ArrayVec::new();
        for m in &self.players[p].shop {
            if cards.get(m.card).is_spell() {
                continue;
            }
            if best.first().is_none_or(|&(_, h)| m.health > h) {
                best.clear();
            }
            if best.first().is_none_or(|&(_, h)| m.health == h) {
                best.push((m.attack, m.health));
            }
        }
        match best.len() {
            0 => None,
            1 => Some(best[0]),
            n => Some(best[self.rng.below(n as u32) as usize]),
        }
    }

    /// Throw the Tavern back and draw a new one. Unfreezes it. `kind` says
    /// whether this is the player's own Refresh (which takes queued
    /// Fodders) or an effect that merely refreshes the Tavern.
    pub(super) fn refresh_shop(
        pl: &mut Player,
        pool: &mut [u8],
        cards: &CardDb,
        rules: &Rules,
        rng: &mut Rng,
        lobby: Tribes,
        kind: RefreshKind,
    ) {
        pl.frozen = false;
        for m in pl.shop.drain(..) {
            give_back(pool, cards, &m);
        }
        pl.shop_spell = None;
        let size = pl.shop_size(rules);
        Self::fill_shop(pl, pool, cards, rules, size, rng, lobby, kind);
        Self::apply_refresh_buff(pl, cards, rng);
        Self::apply_barrage(pl, cards);
        Self::after_refresh_powers(pl, cards, rng, rules.bonus_mask, size);
    }

    /// The hero powers that watch the Tavern's shape (hero batch 2), run by
    /// every Refresh: Come One, Come All! marks some of the new cards with a
    /// Darkmoon Ticket, Twice as Nice copies the highest-Tier minion and
    /// Freezes both cards, and Enhancification gives random Bonus Keywords.
    pub(super) fn after_refresh_powers(
        pl: &mut Player,
        cards: &CardDb,
        rng: &mut Rng,
        bonus_mask: u8,
        size: usize,
    ) {
        // A Ticket roll: 1 for a card that carries one, 2 for one that was
        // rolled for without getting one, 0 for a card the Tavern never
        // marked (so a card that stays put is not rolled again).
        if pl.ticket_chance > 0 {
            let chance = pl.ticket_chance as u32;
            for m in &mut pl.shop {
                if m.tickets == 0 {
                    m.tickets = if rng.below(100) < chance { 1 } else { 2 };
                }
            }
            if pl.shop_spell.is_some() && pl.shop_spell_ticket == 0 {
                pl.shop_spell_ticket = if rng.below(100) < chance { 1 } else { 2 };
            }
        }
        // Twice as Nice: the highest-Tier minion in the Tavern, and an exact
        // copy of it, both frozen. The copy sits outside the normal slots.
        if pl.twice_nice && pl.shop.len() <= size && !pl.shop.is_full() {
            let mut best: Option<usize> = None;
            for i in 0..pl.shop.len() {
                let c = cards.get(pl.shop[i].card);
                if c.is_spell() {
                    continue;
                }
                match best {
                    Some(b) if cards.get(pl.shop[b].card).tier >= c.tier => {}
                    _ => best = Some(i),
                }
            }
            if let Some(b) = best {
                let mut copy = pl.shop[b];
                copy.pool_copies = 0;
                copy.frozen = true;
                pl.shop[b].frozen = true;
                pl.shop.push(copy);
            }
        }
        // Enhancification: a random minion in the Tavern gains a random Bonus
        // Keyword, twice; both may land on the same one, and one holding every
        // keyword is skipped (docs/rules/heroes.md).
        for _ in 0..pl.enhance_times {
            let mut cands: ArrayVec<usize, { crate::SHOP }> = ArrayVec::new();
            for i in 0..pl.shop.len() {
                let c = cards.get(pl.shop[i].card);
                if c.is_spell() || pl.shop[i].keywords.0 & bonus_mask == bonus_mask {
                    continue;
                }
                let _ = cands.try_push(i);
            }
            if cands.is_empty() {
                break;
            }
            let k = rng.below(cands.len() as u32) as usize;
            let m = &mut pl.shop[cands[k]];
            if let Some(kw) = m.keywords.random_missing(bonus_mask, rng) {
                m.keywords.set(kw);
            }
        }
    }

    /// Temporal Tavern: the last `count` Tavern minion slots hold minions from
    /// a Tier above the player's, each drawn from a random Tier up to Tier 6;
    /// the higher-Tier cards take normal slots, they are not extra ones
    /// (docs/rules/heroes.md).
    pub(super) fn add_high_tier_slots(
        pl: &mut Player,
        pool: &mut [u8],
        cards: &CardDb,
        rng: &mut Rng,
        lobby: Tribes,
        count: u8,
    ) {
        let n = pl.shop.len();
        if n == 0 || pl.tier >= crate::MAX_TIER {
            return;
        }
        for k in 0..(count as usize).min(n) {
            let at = n - 1 - k;
            let tier = pl.tier + 1 + rng.below((crate::MAX_TIER - pl.tier) as u32) as u8;
            let drawn = draw_filtered(pool, cards, rng, tier, tier, &[], lobby, |_| true);
            let Some(c) = drawn else { continue };
            let old = pl.shop[at];
            give_back(pool, cards, &old);
            let mut m = pl.make_card(cards, c, false, 1);
            pl.apply_tavern_buffs(cards, &mut m);
            pl.shop[at] = m;
        }
    }

    /// "After the Tavern is Refreshed this game" (Waveling, En-Djinn Blazer,
    /// Easterly Winds): the whole amount goes to one random minion in the
    /// Tavern. Called by every Refresh, the turn-start roll included when
    /// the Tavern changed (see "Two kinds of Refresh" in docs/rules/design.md). It
    /// draws no random number when there is nothing to give.
    pub(super) fn apply_refresh_buff(pl: &mut Player, cards: &CardDb, rng: &mut Rng) {
        let (a, h) = pl.refresh_buff;
        if a == 0 && h == 0 {
            return;
        }
        let ok = |m: &Minion| !cards.get(m.card).is_spell() && !cards.get(m.card).fixed_stats;
        let n = pl.shop.iter().filter(|m| ok(m)).count() as u32;
        if n == 0 {
            return;
        }
        let k = rng.below(n) as usize;
        if let Some(m) = pl.shop.iter_mut().filter(|m| ok(m)).nth(k) {
            buff(m, a, h);
        }
    }

    /// Blood Gem Barrage: after every Refresh (the turn-start roll included
    /// when the Tavern changed), every minion in the Tavern gains the whole
    /// amount cast so far, as Blood Gem stats.
    pub(super) fn apply_barrage(pl: &mut Player, cards: &CardDb) {
        let (a, h) = pl.barrage;
        if a == 0 && h == 0 {
            return;
        }
        for m in &mut pl.shop {
            if !cards.get(m.card).is_spell() && !cards.get(m.card).fixed_stats {
                buff(m, a, h);
                m.gem_attack += a;
                m.gem_health += h;
            }
        }
    }

    /// Fill the Tavern: `size` minion slots and the spell slot. Only the
    /// player's own Refresh takes Fodders queued for upcoming Refreshes; the
    /// automatic roll at the start of a turn and an effect that refreshes
    /// the Tavern leave the queue alone. The first `fodders` slots hold
    /// Demon Fodders (they are not extra slots). A Fodder can also take the
    /// spell slot, which then has no spell this Refresh; Fodders beyond all
    /// slots are lost.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn fill_shop(
        pl: &mut Player,
        pool: &mut [u8],
        cards: &CardDb,
        rules: &Rules,
        size: usize,
        rng: &mut Rng,
        lobby: Tribes,
        kind: RefreshKind,
    ) {
        let fodders = match kind {
            RefreshKind::Explicit => pl.take_fodder_for_refresh(),
            RefreshKind::TurnStart | RefreshKind::Effect => 0,
        };
        // A price a Refresh gave the Tavern lasts only as long as those
        // cards do (Chillmere Mosaic).
        pl.tp.tavern_price = 0;
        let slots = size + 1;
        let fodders = (fodders as usize).min(slots);
        // The tier's own size: the extra slots below fill whatever the
        // player's shape adds on top of it.
        let base = if pl.tp.gauntlet { 6 } else { rules.shop_size(pl.tier) };
        let base = base.saturating_sub(pl.shop_fewer as usize);
        // New Recruit: the extra slot is drawn with the rest of them.
        let base = base + u8::from(pl.prize.extra_shop) as usize;
        let normal = base.saturating_sub(fodders);
        // The tier bounds this Refresh draws in: Bob-blehead's floor, and
        // Guiding Candle's first Refreshes of a turn, which hold Tier 6 cards
        // only (free access to the tier, as the card says).
        let (lo, hi) = Self::tavern_tiers(pl);
        // Warband Whistle: this Refresh holds one plain copy (the card's own
        // stats) of each minion in the warband, and nothing else random
        // (docs/rules/design.md, docs/rules/trinkets.md). A warband of 7 fills the Tavern and
        // leaves no spell slot; 6 or fewer copies sit next to the spell.
        let whistle = pl.tp.whistle && !pl.board.is_empty();
        if whistle {
            pl.tp.whistle = false;
            let mut order: ArrayVec<usize, { crate::BOARD }> = (0..pl.board.len()).collect();
            rng.shuffle(&mut order);
            for k in order {
                if pl.shop.len() >= crate::SHOP {
                    break;
                }
                let mut m = pl.make_card(cards, pl.board[k].card, false, 0);
                pl.apply_tavern_buffs(cards, &mut m);
                pl.shop.push(m);
            }
        } else {
            while pl.shop.len() < normal {
                match draw_filtered(pool, cards, rng, lo, hi, &[], lobby, |_| true) {
                    Some(c) => {
                        let mut m = pl.make_card(cards, c, false, 1);
                        pl.apply_tavern_buffs(cards, &mut m);
                        pl.shop.push(m);
                    }
                    None => break,
                }
            }
        }
        // The extra slots (Innkeeper's Stein, Electrode Attractor): one
        // Magnetic Mech, then one minion of the tier above. `size` already
        // counts them, so a slot only appears when the size allows it.
        if pl.tp.extra_magnetic && pl.shop.len() < size && !whistle {
            let mech = draw_filtered(pool, cards, rng, lo, hi, &[], lobby, |c| {
                c.keywords.has(crate::minion::Keywords::MAGNETIC) && c.tribes.has(Tribe::Mech)
            });
            if let Some(c) = mech {
                let mut m = pl.make_card(cards, c, false, 1);
                pl.apply_tavern_buffs(cards, &mut m);
                pl.shop.push(m);
            }
        }
        // Dream Portal: one more minion, a Dragon, in a normal slot.
        if pl.extra_dragon && pl.shop.len() < size && !whistle {
            let dragon = draw_filtered(pool, cards, rng, lo, hi, &[], lobby, |c| {
                c.tribes.has(Tribe::Dragon)
            });
            if let Some(c) = dragon {
                let mut m = pl.make_card(cards, c, false, 1);
                pl.apply_tavern_buffs(cards, &mut m);
                pl.shop.push(m);
            }
        }
        // New Recruit: the extra slot is drawn like any other. Its +2/+2 is
        // the after-Refresh buff below, the one Waveling gives.
        if pl.prize.extra_shop
            && pl.shop.len() < size
            && !whistle
            && let Some(c) = draw_filtered(pool, cards, rng, lo, hi, &[], lobby, |_| true)
        {
            let mut m = pl.make_card(cards, c, false, 1);
            pl.apply_tavern_buffs(cards, &mut m);
            pl.shop.push(m);
        }
        if pl.tp.extra_high_tier && pl.tier < crate::MAX_TIER && pl.shop.len() < size && !whistle {
            let up = (pl.tier + 1).max(lo);
            let drawn = draw_filtered(pool, cards, rng, up, up, &[], lobby, |_| true);
            if let Some(c) = drawn {
                let mut m = pl.make_card(cards, c, false, 1);
                pl.apply_tavern_buffs(cards, &mut m);
                pl.shop.push(m);
            }
        }
        if let Some(tok) = cards.fodder_token
            && !whistle
        {
            let mut made: ArrayVec<Minion, SHOP> = ArrayVec::new();
            for _ in 0..fodders {
                if pl.shop.len() + made.len() >= slots {
                    break;
                }
                let mut m = pl.make_card(cards, tok, false, 0);
                pl.apply_tavern_buffs(cards, &mut m);
                made.push(m);
            }
            for (k, m) in made.into_iter().enumerate() {
                pl.shop.insert(k, m);
            }
        }
        // The spell slot is free unless Fodders took it too; a frozen Tavern
        // keeps the spell it already had. The Warband Whistle's Tavern holds a
        // copy per warband minion, so with six copies the spell takes the
        // seventh slot and with seven there is no spell.
        let room = if whistle { pl.shop.len() < crate::SHOP } else { pl.shop.len() <= size };
        if room {
            if pl.shop_spell.is_none() {
                pl.shop_spell = cards.random_spell(pl.tier, lobby, rng);
            }
        } else {
            pl.shop_spell = None;
        }
        // Only an explicit Refresh makes Fodders; they feed at once.
        if kind == RefreshKind::Explicit {
            Self::feed_fodders(pl, pool, cards, rng, lobby);
        }
    }

    /// The tier range the Tavern draws minions from right now: Bob-blehead's
    /// floor, or Tier 6 alone during one of Guiding Candle's Refreshes.
    pub(super) fn tavern_tiers(pl: &Player) -> (u8, u8) {
        if pl.tp.refresh_tier6 > 0 {
            (crate::MAX_TIER, crate::MAX_TIER)
        } else {
            (pl.tp.min_tier.max(1), pl.tier)
        }
    }

    /// Glowing Gauntlet: the Tavern always holds seven cards, six minions and
    /// the spell, so a bought, consumed or stolen minion is replaced by a
    /// minion, and a bought spell by a spell (docs/rules/design.md). Called after every
    /// action; a player without the trinket returns on one flag.
    pub(super) fn refill_tavern(&mut self, p: usize) {
        if !self.players[p].tp.gauntlet {
            return;
        }
        let Game { data, rng, players, pool, lobby, .. } = self;
        let cards = &data.cards;
        let pl = &mut players[p];
        let size = pl.shop_size(&data.rules);
        let (lo, hi) = Self::tavern_tiers(pl);
        while pl.shop.len() < size {
            match draw_filtered(pool, cards, rng, lo, hi, &[], *lobby, |_| true) {
                Some(c) => {
                    let mut m = pl.make_card(cards, c, false, 1);
                    pl.apply_tavern_buffs(cards, &mut m);
                    pl.shop.push(m);
                }
                None => break,
            }
        }
        if pl.shop.len() <= size && pl.shop_spell.is_none() {
            pl.shop_spell = cards.random_spell(pl.tier, *lobby, rng);
        }
        // A refilled card is a new card in the Tavern: it takes its Darkmoon
        // Ticket roll like any other (Come One, Come All!).
        if pl.ticket_chance > 0 {
            let chance = pl.ticket_chance as u32;
            for m in pl.shop.iter_mut().filter(|m| m.tickets == 0) {
                m.tickets = if rng.below(100) < chance { 1 } else { 2 };
            }
            if pl.shop_spell.is_some() && pl.shop_spell_ticket == 0 {
                pl.shop_spell_ticket = if rng.below(100) < chance { 1 } else { 2 };
            }
        }
    }

    /// Saloon's Finest: every Tavern slot becomes a Tavern spell up to the
    /// player's tier. Spells have no pool, so repeats are allowed. This is a
    /// card effect, not the player's own Refresh, so no Fodders.
    pub(super) fn refresh_with_spells(
        pl: &mut Player,
        cards: &CardDb,
        rules: &Rules,
        rng: &mut Rng,
        pool: &mut [u8],
        lobby: Tribes,
    ) {
        pl.frozen = false;
        for m in pl.shop.drain(..) {
            give_back(pool, cards, &m);
        }
        pl.shop_spell = None;
        let size = pl.shop_size(rules);
        for _ in 0..size {
            match cards.random_spell(pl.tier, lobby, rng) {
                Some(id) => {
                    let m = pl.make_card(cards, id, false, 0);
                    pl.shop.push(m);
                }
                None => break,
            }
        }
        pl.shop_spell = cards.random_spell(pl.tier, lobby, rng);
        Self::apply_refresh_buff(pl, cards, rng);
        Self::apply_barrage(pl, cards);
        Self::after_refresh_powers(pl, cards, rng, rules.bonus_mask, size);
    }

    /// Chillmere Mosaic's Spellcraft spell: refresh the Tavern with Battlecry
    /// minions, which cost `price` while they stay. The draw comes from the
    /// pool like any other, so a small pool can leave slots empty; it is an
    /// effect, so no Fodders feed it.
    pub(super) fn refresh_with_battlecries(
        pl: &mut Player,
        pool: &mut [u8],
        cards: &CardDb,
        rules: &Rules,
        rng: &mut Rng,
        lobby: Tribes,
        price: u8,
    ) {
        pl.frozen = false;
        for m in pl.shop.drain(..) {
            give_back(pool, cards, &m);
        }
        pl.shop_spell = None;
        let size = pl.shop_size(rules);
        while pl.shop.len() < size {
            let drawn =
                draw_filtered(pool, cards, rng, 1, pl.tier, &[], lobby, |c| c.battlecry.is_some());
            match drawn {
                Some(c) => {
                    let mut m = pl.make_card(cards, c, false, 1);
                    pl.apply_tavern_buffs(cards, &mut m);
                    pl.shop.push(m);
                }
                None => break,
            }
        }
        pl.shop_spell = cards.random_spell(pl.tier, lobby, rng);
        pl.tp.tavern_price = price;
        Self::apply_refresh_buff(pl, cards, rng);
        Self::apply_barrage(pl, cards);
        Self::after_refresh_powers(pl, cards, rng, rules.bonus_mask, size);
    }

    /// Lost Staff of Hamuul: refresh the Tavern with minions of one type,
    /// up to the player's tier. A card effect, not the player's own
    /// Refresh, so no Fodders.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn refresh_with_tribe(
        pl: &mut Player,
        pool: &mut [u8],
        cards: &CardDb,
        rules: &Rules,
        rng: &mut Rng,
        lobby: Tribes,
        tribe: Tribe,
    ) {
        pl.frozen = false;
        for m in pl.shop.drain(..) {
            give_back(pool, cards, &m);
        }
        pl.shop_spell = None;
        let size = pl.shop_size(rules);
        while pl.shop.len() < size {
            match draw_filtered(pool, cards, rng, 1, pl.tier, &[], lobby, |c| c.tribes.has(tribe)) {
                Some(c) => {
                    let mut m = pl.make_card(cards, c, false, 1);
                    pl.apply_tavern_buffs(cards, &mut m);
                    pl.shop.push(m);
                }
                None => break,
            }
        }
        pl.shop_spell = cards.random_spell(pl.tier, lobby, rng);
        Self::apply_refresh_buff(pl, cards, rng);
        Self::apply_barrage(pl, cards);
        Self::after_refresh_powers(pl, cards, rng, rules.bonus_mask, size);
    }

    /// After an action: Fodders that waited in the Tavern because there was
    /// no Demon feed themselves as soon as one is on the board.
    pub(super) fn feed_waiting_fodders(&mut self, p: usize) {
        let Game { data, rng, players, pool, lobby, .. } = self;
        let cards = &data.cards;
        if players[p].shop.iter().any(|m| cards.get(m.card).is_fodder) {
            Self::feed_fodders(&mut players[p], pool, cards, rng, *lobby);
        }
    }

    /// The Fodders in the Tavern feed themselves if the player has a Demon:
    /// each goes to a random friendly Demon, which gains its current stats,
    /// and its slot gets a new card from the pool. With no Demon they wait as
    /// plain 2/2 Demons, and feed once a Demon lands (feed_waiting_fodders).
    fn feed_fodders(pl: &mut Player, pool: &mut [u8], cards: &CardDb, rng: &mut Rng, lobby: Tribes) {
        let is_demon = |m: &Minion| cards.tribes_of(m).has(Tribe::Demon);
        if !pl.board.iter().any(is_demon) {
            return;
        }
        let demons = pl.board.iter().filter(|m| is_demon(m)).count() as u32;
        let mut i = 0;
        while i < pl.shop.len() {
            if !cards.get(pl.shop[i].card).is_fodder {
                i += 1;
                continue;
            }
            let fodder = pl.shop.remove(i);
            let k = rng.below(demons) as usize;
            let di = pl
                .board
                .iter()
                .enumerate()
                .filter(|(_, m)| is_demon(m))
                .nth(k)
                .map(|(j, _)| j)
                .unwrap();
            buff(&mut pl.board[di], fodder.attack, fodder.health);
            if let Some(c) = draw_filtered(pool, cards, rng, 1, pl.tier, &[], lobby, |_| true) {
                let mut m = pl.make_card(cards, c, false, 1);
                pl.apply_tavern_buffs(cards, &mut m);
                pl.shop.insert(i, m);
                i += 1;
            }
        }
    }

    /// Destroy the board minions in `victims` in the shop phase: they die for
    /// real (Deathrattles, Reborn, Knight deaths, Banshee and Phantom). Returns
    /// where the `track`ed board slot ended up.
    pub(super) fn destroy_in_shop(&mut self, p: usize, victims: &[usize], track: Option<usize>) -> Option<usize> {
        let Game { data, rng, players, pool, lobby, .. } = self;
        let pl = &mut players[p];
        let out = crate::fight::shop_destroy(
            &data.cards,
            &pl.board,
            victims,
            track,
            pl.counters(data, *lobby),
            rng,
        );
        // The minions that died leave the warband for good, and take their
        // pool copies back to the pool.
        for m in &out.dead {
            give_back(pool, &data.cards, m);
        }
        pl.board.clear();
        pl.board.extend(out.board);
        self.apply_side_outcome(p, &out.side, false);
        // "Whenever you destroy a minion outside combat" (Maw Caster
        // Portrait, Plaguerunner Portrait).
        for m in &out.dead {
            self.trinket_event(p, Event::Destroy, Some(m.card), 1);
        }
        out.tracked
    }

    /// Fire the Deathrattles of the `n` left-most friendly minions that have
    /// one, outside combat (Wolfhead Flail, Floating Candle Set). Nothing
    /// dies, so each minion keeps its place; a summon its Deathrattle makes
    /// lands next to it.
    pub(super) fn trigger_deathrattles_in_shop(&mut self, p: usize, n: u8) {
        let mut pos = 0;
        for _ in 0..n {
            let at = (pos..self.players[p].board.len()).find(|&i| {
                self.players[p].board[i]
                    .parts()
                    .any(|(c, _)| self.data.cards.get(c).deathrattle.is_some())
            });
            let Some(at) = at else { return };
            self.trigger_one_deathrattle(p, at);
            pos = at + 1;
        }
    }

    /// Fire one friendly minion's Deathrattles outside combat.
    pub(super) fn trigger_one_deathrattle(&mut self, p: usize, at: usize) {
        let Game { data, rng, players, pool, lobby, .. } = self;
        let pl = &mut players[p];
        let out = crate::fight::shop_trigger_deathrattles(
            &data.cards,
            &pl.board,
            at,
            pl.counters(data, *lobby),
            rng,
        );
        for m in &out.dead {
            give_back(pool, &data.cards, m);
        }
        pl.board.clear();
        pl.board.extend(out.board);
        self.apply_side_outcome(p, &out.side, false);
    }

    /// A minion was just bought into the last hand slot (before it can
    /// triple): every Stone Age Slab that has not fired this turn buffs and
    /// multiplies it, left to right, then an armed Living Prison gains its
    /// stats (the Slab buffs included).
    pub(super) fn after_buy_effects(&mut self, p: usize) {
        let cards = &self.data.cards;
        let pl = &mut self.players[p];
        let Some(last) = pl.hand.len().checked_sub(1) else { return };
        if cards.get(pl.hand[last].card).is_spell() {
            return;
        }
        if !cards.get(pl.hand[last].card).fixed_stats {
            for i in 0..pl.board.len() {
                let slab = pl.board[i].parts().find_map(|(card, golden)| {
                    cards.get(card).after_buy.map(|a| (a, golden))
                });
                if let Some((a, golden)) = slab
                    && !pl.board[i].activated
                {
                    pl.board[i].activated = true;
                    let b = &mut pl.hand[last];
                    b.attack += a.attack;
                    b.health += a.health;
                    let times = if golden { a.golden_multiply } else { a.multiply };
                    b.attack = b.attack.saturating_mul(times);
                    b.health = b.health.saturating_mul(times);
                }
            }
        }
        let bought = pl.hand[last];
        for m in &mut pl.board {
            // A Prison's held arms are shares of the bought minion's stats
            // (its own golden press is worth two); arms from several copies
            // add up.
            let arms = m.improvements;
            if arms == 0 {
                continue;
            }
            if m.parts().any(|(card, _)| {
                matches!(cards.get(card).activate, Some(a) if a.effect == Effect::ArmNextBuy)
            }) {
                m.improvements = 0;
                buff(m, bought.attack * arms as i32, bought.health * arms as i32);
            }
        }
    }

    /// Sell the board minion in `slot`: Gold, the copy back to the pool and
    /// the sell triggers, exactly as the Sell action does (Channel the
    /// Devourer sells through here too).
    pub(super) fn sell_minion(&mut self, p: usize, slot: usize) -> Minion {
        let m = self.players[p].board.remove(slot);
        // Hat Trick: a sold minion's hats move to a random other friendly
        // minion, so they are only lost when the whole board is sold or the
        // minion is destroyed (docs/rules/heroes.md).
        if m.hats > 0 && !self.players[p].board.is_empty() {
            let n = self.players[p].board.len();
            let k = self.rng.below(n as u32) as usize;
            let hat = self.players[p].hat;
            let b = &mut self.players[p].board[k];
            b.attack += hat.0 * m.hats as i32;
            b.health += hat.1 * m.hats as i32;
            b.hats = b.hats.saturating_add(m.hats);
        }
        let mut sell_gold = self.data.rules.sell_gold;
        if self.players[p].lost_last_combat {
            for (card, golden) in m.parts() {
                if let Some(OnSell::GoldIfLost(g)) = self.data.cards.get(card).on_sell {
                    let mul = if golden { 2 } else { 1 };
                    sell_gold = g * mul;
                }
            }
        }
        self.players[p].gold = self.players[p].gold.saturating_add(sell_gold);
        // "After you sell a minion, gain 1 Gold next turn" (Smart Savings).
        if let Some(st) = self.players[p].power
            && let PowerEffect::GoldNextTurnAfterSell(g) =
                self.data.heroes.power(st.of).effect
        {
            let runs = self.players[p].tp.power_triggers.max(1);
            let pl = &mut self.players[p];
            pl.gold_next_turn = pl.gold_next_turn.saturating_add(g.saturating_mul(runs));
        }
        give_back(&mut self.pool, &self.data.cards, &m);
        for (card, golden) in m.parts() {
            match self.data.cards.get(card).on_sell {
                Some(OnSell::GetCard { card: tok, count }) => {
                    let n = count * (if golden { 2 } else { 1 });
                    for _ in 0..n {
                        self.give_generated(p, tok);
                    }
                }
                Some(OnSell::DiscoverByTier) => {
                    let tier = (1 + m.improvements as u8).min(MAX_TIER);
                    let n = if golden { 2 } else { 1 };
                    for _ in 0..n {
                        let _ = self.players[p]
                            .discover_queue
                            .try_push(DiscoverRequest::MinionTier(tier));
                    }
                    self.maybe_start_discover(p);
                }
                Some(OnSell::Effect(e)) => {
                    let mut pos = 0;
                    self.run_effect(p, &mut pos, e, golden, 1);
                }
                Some(OnSell::DiscoverTier7IfGolden) => {
                    if golden {
                        self.discover_tier7_minion(p);
                    }
                }
                // Handled below: the minion with this text is on the board,
                // not the one being sold.
                Some(OnSell::GainSoldStats { .. }) | Some(OnSell::GoldIfLost(_)) | None => {}
            }
        }
        // "After you sell an Elemental, gain its stats" (Arcane Behemoth):
        // the text sits on a minion that stays on the board, so it gains the
        // sold minion's stats, doubled for a golden one. Two copies of the
        // text on the same minion do not stack.
        let sold_tribes = self.data.cards.tribes_of(&m);
        let cards = &self.data.cards;
        for o in &mut self.players[p].board {
            let mul = o
                .parts()
                .filter_map(|(c, golden)| match cards.get(c).on_sell {
                    Some(OnSell::GainSoldStats { tribe })
                        if tribe.is_none_or(|t| sold_tribes.has(t)) =>
                    {
                        Some(if golden { 2 } else { 1 })
                    }
                    _ => None,
                })
                .max()
                .unwrap_or(0);
            if mul > 0 {
                o.attack = o.attack.saturating_add(m.attack * mul);
                o.health = o.health.saturating_add(m.health * mul);
            }
        }
        self.fire_event(p, EventKind::MinionsSold, 1);
        self.trinket_event(p, Event::Sell, Some(m.card), 1);
        self.power_sold_minion(p);
        self.check_golden_when_rich(p);
        m
    }

    /// Replace the Tavern card in `slot` with a Fishbait and have the
    /// left-most Beast attack it. The spell slot, one past the minions, is a
    /// Tavern card too: the Fishbait takes its place at the end.
    pub(super) fn put_fishbait(&mut self, p: usize, slot: usize, card: CardId, golden: bool) {
        // The Tavern's spell slot, one past the last minion: the spell is
        // replaced by the Fishbait, which takes a minion slot (Era,
        // 2026-11-03: Lionfish may pick a Tavern spell).
        if slot >= self.players[p].shop.len() {
            if self.players[p].shop_spell.take().is_none() {
                return;
            }
            let bait = self.data.cards.make(card, golden);
            if self.players[p].shop.try_push(bait).is_err() {
                return;
            }
            let at = self.players[p].shop.len() - 1;
            self.tavern_attack(p, at);
            return;
        }
        let Game { data, players, pool, .. } = self;
        let pl = &mut players[p];
        let old = std::mem::replace(&mut pl.shop[slot], data.cards.make(card, golden));
        give_back(pool, &data.cards, &old);
        self.tavern_attack(p, slot);
    }

    /// A Fishbait effect that picks the Tavern card: ask, unless there is
    /// only one card.
    pub(super) fn start_fishbait(&mut self, p: usize, source: usize, card: CardId, golden: bool) {
        let cards = self.players[p].shop.len()
            + usize::from(self.players[p].shop_spell.is_some());
        match cards {
            // Nothing to replace.
            0 => {}
            // One card, a minion or the spell: no question to ask.
            1 => self.put_fishbait(p, 0, card, golden),
            _ => {
                self.players[p].pending_shop_target = Some(PendingShopTarget {
                    source: source as u8,
                    kind: ShopTargetKind::Fishbait { card, golden },
                });
            }
        }
    }

    /// Refresh the Tavern for free with a Fishbait in its left-most slot.
    /// An effect that says "Refresh" is not the player's own Refresh, so it
    /// leaves Fodders queued for upcoming Refreshes alone.
    pub(super) fn refresh_with_fishbait(&mut self, p: usize, card: CardId, golden: bool) {
        {
            let Game { data, rng, players, pool, lobby, .. } = self;
            Self::refresh_shop(
                &mut players[p],
                pool,
                &data.cards,
                &data.rules,
                rng,
                *lobby,
                RefreshKind::Effect,
            );
        }
        self.trinket_event(p, Event::Refresh, None, 1);
        if !self.players[p].shop.is_empty() {
            self.put_fishbait(p, 0, card, golden);
        }
    }

    /// The left-most Beast on the player's board attacks the Tavern minion in
    /// `slot`. It dies, or stays if the Beast can't hurt it.
    fn tavern_attack(&mut self, p: usize, slot: usize) {
        let Game { data, rng, players, pool, lobby, .. } = self;
        let pl = &mut players[p];
        let Some(ai) = pl.board.iter().position(|m| data.cards.tribes_of(m).has(Tribe::Beast))
        else {
            return;
        };
        let out = crate::fight::tavern_attack(
            &data.cards,
            &pl.board,
            ai,
            pl.shop[slot],
            pl.counters(data, *lobby),
            rng,
        );
        // A Beast that dies to the Fishbait dies for real, so its pool copy
        // goes back with it (the same rule a shop-phase destroy follows);
        // the ones still standing keep theirs. Only the copies change hands
        // here: each old copy goes back to the pool and the survivors take
        // their own out again.
        let old: ArrayVec<Minion, { crate::BOARD }> = pl.board.iter().copied().collect();
        pl.board.clear();
        pl.board.extend(out.board);
        for m in &old {
            give_back(pool, &data.cards, m);
        }
        for m in &pl.board {
            take_copy(pool, &data.cards, m);
        }
        match out.target {
            Some(t) => pl.shop[slot] = t,
            None => {
                pl.shop.remove(slot);
            }
        }
        self.apply_side_outcome(p, &out.side, false);
        // A Rally can summon the third copy of a card (Flittering Bat's 1/1
        // Beast next to two of them), and in the shop phase that triples at
        // once. `check_triple` does nothing below three copies.
        let mut held: ArrayVec<CardId, { crate::BOARD }> = ArrayVec::new();
        for m in &self.players[p].board {
            if !m.golden && !held.contains(&m.card) {
                let _ = held.try_push(m.card);
            }
        }
        for card in held {
            self.check_triple(p, card);
        }
    }

    /// Steal the `n` highest-Attack Tavern minions into hand (Decoy
    /// Conjurer). Ties go at random among them (Era, docs/rules/design.md
    /// "Shop replay answers"). The copies leave the pool like a buy and come
    /// back when sold.
    pub(super) fn steal_highest_tavern(&mut self, p: usize, n: u8) {
        for _ in 0..n {
            if self.players[p].hand.is_full() {
                break;
            }
            let cards = &self.data.cards;
            let shop = &self.players[p].shop;
            let mut best: Option<i32> = None;
            let mut tied: ArrayVec<usize, SHOP> = ArrayVec::new();
            for (i, m) in shop.iter().enumerate() {
                if cards.get(m.card).is_spell() {
                    continue;
                }
                match best {
                    Some(a) if m.attack < a => {}
                    Some(a) if m.attack == a => {
                        let _ = tied.try_push(i);
                    }
                    _ => {
                        best = Some(m.attack);
                        tied.clear();
                        let _ = tied.try_push(i);
                    }
                }
            }
            if tied.is_empty() {
                break;
            }
            let i = tied[self.rng.below(tied.len() as u32) as usize];
            let m = self.players[p].shop.remove(i);
            let card = m.card;
            self.add_to_hand(p, m);
            self.check_triple(p, card);
        }
    }

    /// Steal a random Tavern minion into hand (Enchanted Lasso).
    pub(super) fn steal_random_tavern(&mut self, p: usize) {
        if self.players[p].hand.is_full() {
            return;
        }
        let cards = &self.data.cards;
        let mut ok: ArrayVec<usize, SHOP> = ArrayVec::new();
        for (i, m) in self.players[p].shop.iter().enumerate() {
            if !cards.get(m.card).is_spell() {
                let _ = ok.try_push(i);
            }
        }
        if ok.is_empty() {
            return;
        }
        let k = self.rng.below(ok.len() as u32) as usize;
        let m = self.players[p].shop.remove(ok[k]);
        let card = m.card;
        self.add_to_hand(p, m);
        self.check_triple(p, card);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Data, Tribe, Tribes};
    use std::sync::Arc;

    /// A Beast that dies to a Fishbait dies for real, so its pool copy comes
    /// back (2026-11-02: the board was cleared and the survivors pushed back
    /// without it, so the copy vanished and the pool invariant caught it).
    /// The Fishbait's own stats are fixed, so the attack is set here to make
    /// the Beast die; nothing else about the attack changes.
    #[test]
    fn a_beast_that_dies_to_a_fishbait_gives_its_pool_copy_back() {
        let mut g = Game::new_with_lobby(
            Arc::new(Data::bundled()),
            7,
            Tribes::NONE.with(Tribe::Beast).with(Tribe::Quilboar),
        );
        let c = g.data.cards.by_name("Wolf Pup").expect("Wolf Pup");
        let slot = g.data.cards.get(c).pool_slot as usize;
        g.pool[slot] -= 1;
        let mut m = g.players[0].make_card(&g.data.cards, c, false, 1);
        m.attack = 1;
        m.health = 1;
        g.players[0].board.push(m);
        // The Tavern: a Fishbait that hits hard enough to kill the Beast.
        let old: ArrayVec<Minion, { crate::BOARD }> = g.players[0].shop.drain(..).collect();
        for m in &old {
            give_back(&mut g.pool, &g.data.cards, m);
        }
        let fish = g.data.cards.by_name("Fishbait").expect("Fishbait");
        let mut f = g.data.cards.make(fish, false);
        f.attack = 5;
        g.players[0].shop.push(f);
        let before = g.pool[slot];
        g.tavern_attack(0, 0);
        assert!(g.players[0].board.is_empty(), "the Beast died to the Fishbait");
        assert!(g.players[0].shop.is_empty(), "and it ate the Fishbait on the way");
        assert_eq!(g.pool[slot], before + 1, "its pool copy came back with it");
        g.check_invariants().unwrap();
    }

    /// Lurking Lionfish may pick a Tavern spell: the Fishbait takes the spell
    /// slot's place, as a minion slot (Era, 2026-11-03; log
    /// 2026_10_28_22_11_50-g2 turn 7 step 13).
    #[test]
    fn a_fishbait_may_take_the_tavern_spell_slot() {
        let mut g = Game::new_with_lobby(
            Arc::new(Data::bundled()),
            8,
            Tribes::NONE.with(Tribe::Beast).with(Tribe::Quilboar),
        );
        // No Beast on the board: the Fishbait is not attacked and stays.
        let spell = g.data.cards.by_name("Blood Gem").expect("Blood Gem");
        g.players[0].shop_spell = Some(spell);
        let bait = g.data.cards.by_name("Fishbait").expect("Fishbait");
        let minions = g.players[0].shop.len();
        g.put_fishbait(0, minions, bait, false);
        assert_eq!(g.players[0].shop_spell, None, "the Tavern spell is gone");
        assert_eq!(g.players[0].shop.len(), minions + 1, "the Fishbait took a minion slot");
        assert_eq!(
            g.data.cards.get(g.players[0].shop[minions].card).name,
            "Fishbait"
        );
        g.check_invariants().unwrap();
    }

    /// A Rally during a Tavern attack can summon the third copy of a card,
    /// and in the shop phase that triples at once: two Foraging Bats plus the
    /// one a Flittering Bat's Rally summons make a golden one in hand (log
    /// 2026_10_28_02_28_32-g5 turn 5 step 1).
    #[test]
    fn a_rally_summon_during_a_tavern_attack_triples() {
        let mut g = Game::new_with_lobby(
            Arc::new(Data::bundled()),
            9,
            Tribes::NONE.with(Tribe::Beast).with(Tribe::Quilboar),
        );
        // A token: generated, so no pool copy.
        let bats = g.data.cards.by_name("Foraging Bat").expect("Foraging Bat");
        for _ in 0..2 {
            let m = g.players[0].make_card(&g.data.cards, bats, false, 0);
            g.players[0].board.push(m);
        }
        let flitter = g.data.cards.by_name("Flittering Bat").expect("Flittering Bat");
        let slot = g.data.cards.get(flitter).pool_slot as usize;
        g.pool[slot] -= 1;
        let mut m = g.players[0].make_card(&g.data.cards, flitter, false, 1);
        m.attack = 4;
        m.health = 20;
        g.players[0].board.insert(0, m);
        let fish = g.data.cards.by_name("Fishbait").expect("Fishbait");
        let old: ArrayVec<Minion, { crate::BOARD }> = g.players[0].shop.drain(..).collect();
        for m in &old {
            give_back(&mut g.pool, &g.data.cards, m);
        }
        g.players[0].shop.push(g.data.cards.make(fish, false));
        g.tavern_attack(0, 0);
        assert!(
            g.players[0].hand.iter().any(|m| m.card == bats && m.golden),
            "the third copy tripled into a golden one in hand"
        );
        g.check_invariants().unwrap();
    }
}
