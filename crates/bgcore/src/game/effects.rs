//! run_effect, the big effect match, and its helpers.

use arrayvec::ArrayVec;
use crate::trinkets::CardMod;
use crate::cards::{
    ConsumeBy,
    ConsumePick,
    Effect,
    EventKind,
    Tribe,
    Tribes,
    TriggerWhen,
    pick_one_per_type,
};
use crate::minion::{
    CardId,
    Minion,
};
use crate::{
    BOARD,
    HAND,
    MAX_TIER,
};
use super::{
    DiscoverRequest,
    Game,
    PendingShopTarget,
    PendingTarget,
    ShopTargetKind,
    TargetKind,
    TavernBuff,
};
use super::shop::draw_filtered;
use super::shop::give_back;

pub(super) fn buff(m: &mut Minion, attack: i32, health: i32) {
    m.attack += attack;
    m.health += health;
}

impl Game {
    /// Improve every Magnetization to come by `da`/`dh`: a Volumizer this
    /// player holds, in the Tavern, in hand or inside a minion, grows by that
    /// much, and a later one starts that much bigger. A Magnetization calls
    /// this before it merges the Volumizer in: the improvement is for the
    /// Magnetizations after this one, so the minion it went into takes the
    /// card's own stats and the improvement only for the Volumizers it
    /// already held (log 2026_10_28_22_11_50-g5 turn 6: Relentless Deflector
    /// 13/19 to 16/28 for a Green Volumizer's 3/9).
    pub(super) fn add_volumizer_bonus(&mut self, p: usize, da: i32, dh: i32) {
        let pl = &mut self.players[p];
        pl.volumizer_bonus.0 += da;
        pl.volumizer_bonus.1 += dh;
        for m in pl.shop.iter_mut().chain(&mut pl.hand).chain(&mut pl.board) {
            if m.volumizers > 0 {
                let k = m.volumizers as i32;
                m.attack += da * k;
                m.health += dh * k;
            }
        }
    }

    fn add_beetle_bonus(&mut self, p: usize, da: i32, dh: i32) {
        self.add_beetle_bonus_to(p, da, dh, true);
    }

    /// Give the player's Beetles +da/+dh this game; `board` says whether the
    /// ones on the board get it too (a Tavern attack already gave it to them).
    pub(super) fn add_beetle_bonus_to(&mut self, p: usize, da: i32, dh: i32, board: bool) {
        let cards = &self.data.cards;
        let pl = &mut self.players[p];
        pl.beetle_bonus.0 += da;
        pl.beetle_bonus.1 += dh;
        let on_board = if board { &mut pl.board[..] } else { &mut [] };
        for m in pl.shop.iter_mut().chain(&mut pl.hand).chain(on_board) {
            if cards.get(m.card).is_beetle {
                m.attack += da;
                m.health += dh;
            }
        }
    }

    /// Your Undead have +da/+dh this game, wherever they are: the ones in the
    /// Tavern and the hand now (and the board when `board`), and every Undead
    /// made later. A fight or shop destroy that already gave the board its
    /// share passes `board = false`.
    pub(super) fn add_undead_bonus(&mut self, p: usize, da: i32, dh: i32, board: bool) {
        if da == 0 && dh == 0 {
            return;
        }
        let cards = &self.data.cards;
        let pl = &mut self.players[p];
        pl.undead_bonus.0 += da;
        pl.undead_bonus.1 += dh;
        let on_board = if board { &mut pl.board[..] } else { &mut [] };
        for m in pl.shop.iter_mut().chain(&mut pl.hand).chain(on_board) {
            let c = cards.get(m.card);
            if !c.is_spell() && !c.fixed_stats && cards.tribes_of(m).has(Tribe::Undead) {
                m.attack += da;
                m.health += dh;
            }
        }
    }

    /// `n` more of the player's Eternal Knights died: every Knight, in the
    /// Tavern, the hand and (when `board`) on the board, grows.
    pub(super) fn add_knight_deaths(&mut self, p: usize, n: u32, board: bool) {
        if n == 0 {
            return;
        }
        let cards = &self.data.cards;
        let pl = &mut self.players[p];
        pl.knight_deaths = pl.knight_deaths.saturating_add(n);
        let on_board = if board { &mut pl.board[..] } else { &mut [] };
        for m in pl.shop.iter_mut().chain(&mut pl.hand).chain(on_board) {
            if let Some((a, h)) = cards.get(m.card).buff_per_kind_death {
                let k = n as i32 * m.mul();
                m.attack += a * k;
                m.health += h * k;
            }
        }
    }

    /// The player triggered `n` more Deathrattles: Falling Sky Golem and its
    /// like grow, wherever they are.
    pub(super) fn add_deathrattles_triggered(&mut self, p: usize, n: u32, board: bool) {
        if n == 0 {
            return;
        }
        let cards = &self.data.cards;
        let pl = &mut self.players[p];
        pl.deathrattles_triggered = pl.deathrattles_triggered.saturating_add(n);
        let on_board = if board { &mut pl.board[..] } else { &mut [] };
        for m in pl.shop.iter_mut().chain(&mut pl.hand).chain(on_board) {
            for (card, golden) in m.parts().collect::<ArrayVec<_, { crate::MAX_ATTACHED + crate::MAX_GIFTS + 1 }>>() {
                if let Some((a, h)) = cards.get(card).buff_per_deathrattle {
                    let k = n as i32 * if golden { 2 } else { 1 };
                    m.attack += a * k;
                    m.health += h * k;
                }
            }
        }
    }

    /// Run a generic effect for the minion in board slot `*pos`. `golden`
    /// doubles it and `runs` repeats it (Brann). `pos` follows the minion
    /// when a triple takes board minions away to its left.
    pub(super) fn run_effect(&mut self, p: usize, pos: &mut usize, e: Effect, golden: bool, runs: u8) {
        let mul = if golden { 2 } else { 1 };
        let bonus_mask = self.data.rules.bonus_mask;
        match e {
            Effect::BuffDeity { attack, health, per_spell } => {
                let n = self.players[p].spells_cast as i32;
                if let Some(deity) = &mut self.players[p].deity {
                    deity.attack += (attack + per_spell.0 * n) * mul * runs as i32;
                    deity.health += (health + per_spell.1 * n) * mul * runs as i32;
                }
            }
            Effect::BuffSelfAndDeity { attack, health } => {
                if let Some(m) = self.players[p].board.get_mut(*pos) {
                    buff(m, attack * mul * runs as i32, health * mul * runs as i32);
                }
                if let Some(deity) = &mut self.players[p].deity {
                    deity.attack += attack * mul * runs as i32;
                    deity.health += health * mul * runs as i32;
                }
            }
            Effect::BuffDeityAndImprove { attack, health } => {
                for _ in 0..runs {
                    let imp = self.players[p].board.get(*pos).map_or(0, |m| m.improvements as i32);
                    if let Some(deity) = &mut self.players[p].deity {
                        deity.attack += attack * (1 + imp) * mul;
                        deity.health += health * (1 + imp) * mul;
                    }
                    if let Some(m) = self.players[p].board.get_mut(*pos) {
                        m.improvements = m.improvements.saturating_add(1);
                    }
                }
            }
            Effect::BuffLeftmostPerDiscard { attack, health } => {
                let n = 1 + self.players[p].discarded as i32;
                if let Some(m) = self.players[p].board.first_mut() {
                    buff(m, attack * n * mul * runs as i32, health * n * mul * runs as i32);
                }
            }
            Effect::GetRandomSpellUpTo { cost, count } => {
                let tier = self.players[p].tier;
                for _ in 0..count.saturating_mul(mul as u8).saturating_mul(runs) {
                    let card = match cost {
                        Some(cost) => {
                            self.data.cards.random_spell_of_cost_up_to(cost, tier, self.lobby, &mut self.rng)
                        }
                        None => self.data.cards.random_spell(tier, self.lobby, &mut self.rng),
                    };
                    if let Some(card) = card {
                        self.give_generated_tracking(p, card, pos);
                    }
                }
            }
            Effect::GetLinkedSpells { count } => {
                let tier = self.players[p].tier;
                let mut cards = ArrayVec::<CardId, HAND>::new();
                for _ in 0..count.saturating_mul(mul as u8).saturating_mul(runs) {
                    if let Some(card) = self.data.cards.random_spell(tier, self.lobby, &mut self.rng) {
                        let _ = cards.try_push(card);
                    }
                }
                self.give_linked(p, &cards, golden);
            }
            Effect::GetLinkedMinions { tribe, count } => {
                let tier = self.players[p].tier;
                let mut cards = ArrayVec::<CardId, HAND>::new();
                for _ in 0..count.saturating_mul(mul as u8).saturating_mul(runs) {
                    if let Some(card) = self.data.cards.random_pool_minion(tribe, tier, self.lobby, &mut self.rng) {
                        let _ = cards.try_push(card);
                    }
                }
                self.give_linked(p, &cards, golden);
            }
            Effect::DiscardLeftmostSpells { count, attack, health } => {
                for _ in 0..count.saturating_mul(mul as u8).saturating_mul(runs) {
                    let cards = &self.data.cards;
                    let slot = self.players[p].hand.iter()
                        .position(|m| cards.get(m.card).tavern_spell);
                    let Some(slot) = slot else { break };
                    self.discard_card(p, slot);
                    if let Some(m) = self.players[p].board.get_mut(*pos) {
                        buff(m, attack, health);
                    }
                }
            }
            Effect::SummonAndBuffAll { .. }
            | Effect::SummonDeadAberrations { .. }
            | Effect::DamageHighestHealth => {}
            Effect::BuffOthers { tribe, attack, health, per_keyword } => {
                let cards = &self.data.cards;
                let pl = &mut self.players[p];
                let k = pl.distinct_bonus_keywords(bonus_mask) as i32;
                let (da, dh) = (attack + per_keyword.0 * k, health + per_keyword.1 * k);
                for _ in 0..runs {
                    for (i, o) in pl.board.iter_mut().enumerate() {
                        if i != *pos && cards.tribes_of(o).is(tribe) {
                            buff(o, da * mul, dh * mul);
                        }
                    }
                }
            }
            Effect::BuffSelf { attack, health } => {
                if let Some(m) = self.players[p].board.get_mut(*pos) {
                    buff(m, attack * mul * runs as i32, health * mul * runs as i32);
                }
            }
            Effect::BuffChosen { tribe, attack, health, per_keyword, random_keyword, in_shop } => {
                let k = self.players[p].distinct_bonus_keywords(bonus_mask) as i32;
                let pt = PendingTarget {
                    by_minion: false,
                    source: Some(*pos as u8),
                    kind: TargetKind::MinionBuff {
                        tribe,
                        attack: (attack + per_keyword.0 * k) * mul,
                        health: (health + per_keyword.1 * k) * mul,
                        keyword: 0,
                        random_keyword,
                        shop: in_shop,
                    },
                    runs,
                        tavern: true,
                        gem: false,
                };
                self.offer_target(p, pt);
            }
            Effect::RandomKeyword => {
                for _ in 0..mul as u8 * runs {
                    self.give_random_keyword(p, *pos);
                }
            }
            Effect::Consume { by, pick, keywords, count } => {
                // A golden consumer takes double stats; repeated Battlecries
                // (Brann) consume once more each.
                let count = count.saturating_mul(runs);
                let stat_mul = mul as u8;
                match by {
                    ConsumeBy::This => self.start_consume(p, *pos, pick, keywords, count, stat_mul),
                    ConsumeBy::Chosen(tribe) => {
                        let pt = PendingTarget {
                            by_minion: false,
                            source: Some(*pos as u8),
                            kind: TargetKind::Consume { tribe, pick, keywords, count, stat_mul },
                            runs: 1,
                                tavern: true,
                                gem: false,
                        };
                        self.offer_target(p, pt);
                    }
                }
            }
            Effect::TavernBuff { tribe, attack, health, this_turn } => {
                let n = mul * runs as i32;
                let b = TavernBuff {
                    tribe,
                    attack: attack * n,
                    health: health * n,
                    this_turn,
                    typed_only: false,
                };
                self.players[p].add_tavern_buff(&self.data.cards, b);
                // Azerite Portrait: the Living Azerites also give the stats
                // to your Elementals on the board.
                let also = self
                    .players[p]
                    .board
                    .get(*pos)
                    .and_then(|m| self.player_card_mod(p, m.card));
                if let Some(CardMod::AlsoBoard(want)) = also {
                    let cards = &self.data.cards;
                    for o in &mut self.players[p].board {
                        if cards.tribes_of(o).has(want) {
                            buff(o, attack * n, health * n);
                        }
                    }
                }
            }
            Effect::TavernBuffRandomKeyword { attack, health, mask } => {
                let bits = mask.count_ones();
                if bits == 0 {
                    return;
                }
                for _ in 0..runs {
                    let mut k = self.rng.below(bits);
                    let mut keyword = 0u8;
                    for bit in 0..8 {
                        let b = 1u8 << bit;
                        if mask & b != 0 {
                            if k == 0 {
                                keyword = b;
                                break;
                            }
                            k -= 1;
                        }
                    }
                    let cards = &self.data.cards;
                    for m in &mut self.players[p].shop {
                        if cards.get(m.card).is_spell() {
                            continue;
                        }
                        buff(m, attack * mul, health * mul);
                        m.keywords.set(keyword);
                    }
                }
            }
            Effect::AddFodderRefreshes(n) => {
                let per = mul as u8;
                let times = (n as u32).saturating_mul(runs as u32);
                self.players[p].add_fodder_refreshes(per, times);
            }
            Effect::AddFodderToNextRefresh(n) => {
                let add = n.saturating_mul(mul as u8).saturating_mul(runs);
                if let Some(first) = self.players[p].fodder_refreshes.first_mut() {
                    *first = first.saturating_add(add);
                } else {
                    let _ = self.players[p].fodder_refreshes.try_push(add);
                }
            }
            Effect::DemonsConsume { count } => {
                let n = count.saturating_mul(runs);
                let cards = &self.data.cards;
                let demons: ArrayVec<usize, BOARD> = self.players[p]
                    .board
                    .iter()
                    .enumerate()
                    .filter(|(_, m)| cards.tribes_of(m).has(Tribe::Demon))
                    .map(|(i, _)| i)
                    .collect();
                for i in demons {
                    self.start_consume(p, i, ConsumePick::Random, false, n, mul as u8);
                }
            }
            Effect::ImproveSelf(n) => {
                // The count is Activates, so a golden Activate adds one too.
                // Its Deathrattle doubles the per-Activate number instead.
                if let Some(m) = self.players[p].board.get_mut(*pos) {
                    let add = n.saturating_mul(runs);
                    m.improvements = m.improvements.saturating_add(add as u16);
                }
            }
            Effect::DiscoverDemonTierDamage(n) => {
                let times =
                    (n as u32).saturating_mul(mul as u32).saturating_mul(runs as u32);
                let tier = self.players[p].tier;
                let source = *pos as u8;
                for _ in 0..times {
                    let _ = self.players[p]
                        .discover_queue
                        .try_push(DiscoverRequest::DemonMinion { max_tier: tier, source });
                }
            }
            Effect::BuffAnyMinion { attack, health } => {
                let pt = PendingTarget {
                    by_minion: false,
                    source: Some(*pos as u8),
                    kind: TargetKind::AnyBuff {
                        attack: attack * mul,
                        health: health * mul,
                    },
                    runs,
                        tavern: true,
                        gem: false,
                };
                self.offer_target(p, pt);
            }
            Effect::SetStatsAnyMinion { attack, health } => {
                let pt = PendingTarget {
                    by_minion: false,
                    source: Some(*pos as u8),
                    kind: TargetKind::AnySetStats {
                        attack: attack * mul,
                        health: health * mul,
                    },
                    runs,
                        tavern: true,
                        gem: false,
                };
                self.offer_target(p, pt);
            }
            Effect::StealTavern { count } => {
                let n = count.saturating_mul(mul as u8).saturating_mul(runs);
                self.steal_highest_tavern(p, n);
            }
            Effect::DiscoverSpell { count } => {
                let n = count.saturating_mul(mul as u8).saturating_mul(runs);
                let tier = self.players[p].tier;
                for _ in 0..n {
                    let _ = self.players[p]
                        .discover_queue
                        .try_push(DiscoverRequest::SpellMaxTier(tier));
                }
            }
            Effect::DamageHero(n) => {
                self.take_shop_damage(p, n * mul * runs as i32, Some(*pos));
            }
            Effect::RewindDamage => {
                let pl = &mut self.players[p];
                // What the damage took comes back as it was taken: the Health
                // as Health, the Armor as Armor (docs/rules/design.md "Heroes").
                pl.health += pl.last_damage - pl.last_damage_armor;
                pl.armor += pl.last_damage_armor;
                pl.last_damage = 0;
                pl.last_damage_armor = 0;
            }
            Effect::MagnetizeVolumizerAndCopy => {
                for _ in 0..runs {
                    if let Some(v_id) = self.data.cards.random_volumizer(self.lobby, &mut self.rng) {
                        let v = self.players[p].make_card(&self.data.cards, v_id, false, 0);
                        self.magnetize_to(p, *pos, v, false);
                        for _ in 0..mul {
                            self.give_generated_tracking(p, v_id, pos);
                        }
                    }
                }
            }
            Effect::BeetleBuff { attack, health } => {
                for _ in 0..runs {
                    self.add_beetle_bonus(p, attack * mul, health * mul);
                }
            }
            Effect::GoldNextTurn(g) => {
                let gain = g.saturating_mul(mul as u8).saturating_mul(runs);
                self.players[p].gold_next_turn =
                    self.players[p].gold_next_turn.saturating_add(gain);
            }
            Effect::RandomVolumizer => {
                let total = (mul as u8).saturating_mul(runs);
                for _ in 0..total {
                    if let Some(v) = self.data.cards.random_volumizer(self.lobby, &mut self.rng) {
                        self.give_generated_tracking(p, v, pos);
                    }
                }
            }
            Effect::SpellDiscount(d) => {
                let gain = d.saturating_mul(mul as u8).saturating_mul(runs);
                self.players[p].spell_discount =
                    self.players[p].spell_discount.saturating_add(gain);
            }
            Effect::GetCard { card: tok, count } => {
                let total = count.saturating_mul(mul as u8).saturating_mul(runs);
                for _ in 0..total {
                    self.give_generated_tracking(p, tok, pos);
                }
            }
            Effect::BuffAll { tribe, attack, health } => {
                let cards = &self.data.cards;
                for _ in 0..runs {
                    for o in &mut self.players[p].board {
                        if cards.tribes_of(o).is(tribe) && !cards.get(o.card).fixed_stats {
                            buff(o, attack * mul, health * mul);
                        }
                    }
                }
            }
            Effect::BuffRandomFriendly { tribe, attack, health, improve } => {
                for _ in 0..runs {
                    let scale = improve.map_or(1, |c| 1 + self.players[p].improve[c.index()] as i32);
                    let cards = &self.data.cards;
                    let board = &mut self.players[p].board;
                    let ok = |m: &Minion| cards.tribes_of(m).is(tribe) && !cards.get(m.card).fixed_stats;
                    let n = board.iter().filter(|m| ok(m)).count() as u32;
                    if n > 0 {
                        let k = self.rng.below(n) as usize;
                        let m = board.iter_mut().filter(|m| ok(m)).nth(k).unwrap();
                        buff(m, attack * mul * scale, health * mul * scale);
                    }
                    if let Some(c) = improve {
                        let n = &mut self.players[p].improve[c.index()];
                        *n = n.saturating_add(1);
                    }
                }
            }
            Effect::GainKeyword { keyword } => {
                if let Some(m) = self.players[p].board.get_mut(*pos) {
                    m.keywords.set(keyword);
                }
            }
            Effect::GetRandomMinion { tribe } => {
                let tier = self.players[p].tier;
                for _ in 0..mul * runs as i32 {
                    if let Some(c) =
                        self.data.cards.random_pool_minion(tribe, tier, self.lobby, &mut self.rng)
                    {
                        self.give_generated_tracking(p, c, pos);
                    }
                }
            }
            // Dark Gifts Charisma, triggered in the shop (a Rally an
            // Activate triggers): a random minion of the board's most common
            // type.
            Effect::GetMostCommonTypeMinion => {
                let tribes: ArrayVec<crate::cards::Tribes, { crate::BOARD }> =
                    self.players[p].board.iter().map(|m| self.data.cards.tribes_of(m)).collect();
                let tier = self.players[p].tier;
                // A golden minion gets one per `mul`, and a triggered Rally
                // repeats with `runs`.
                let times = mul * runs as i32;
                for _ in 0..times {
                    if let Some(t) = crate::darkgifts::most_common_type(&tribes, &mut self.rng)
                        && let Some(c) = self.data.cards.random_pool_minion(
                            t,
                            tier,
                            self.lobby,
                            &mut self.rng,
                        )
                    {
                        self.give_generated_tracking(p, c, pos);
                    }
                }
            }
            // Dark Gifts Mystic Essence: a random Tavern spell of any cost.
            Effect::GetAnyTavernSpell => {
                if let Some(c) = self.data.cards.random_spell(crate::MAX_TIER, self.lobby, &mut self.rng) {
                    self.give_generated_tracking(p, c, pos);
                }
            }
            Effect::GetRandomSpell { cost, count } => {
                let total = count as i32 * mul * runs as i32;
                for _ in 0..total {
                    if let Some(c) = self.data.cards.random_spell_of_cost(cost, self.lobby, &mut self.rng) {
                        self.give_generated_tracking(p, c, pos);
                    }
                }
            }
            Effect::ReplaceWithFishbait { card } => {
                self.start_fishbait(p, *pos, card, golden);
            }
            Effect::RefreshWithFishbait { card } => {
                for _ in 0..runs {
                    self.refresh_with_fishbait(p, card, golden);
                }
            }
            Effect::ExtraMagnetization(n) => {
                if let Some(m) = self.players[p].board.get_mut(*pos) {
                    let add = (n as i32 * mul) as u8 * runs;
                    m.extra_magnetize = m.extra_magnetize.saturating_add(add);
                }
            }
            // A Rally triggered in the shop can summon (Sky-hatch Runaway
            // triggering Flittering Bat): the copies go next to the minion.
            Effect::Summon { card, count, golden_doubles_count } => {
                let (n, g) = if golden_doubles_count {
                    (count * mul as u8, false)
                } else {
                    (count, golden)
                };
                let mut at = *pos + 1;
                for _ in 0..n {
                    if self.players[p].board.is_full() || at > self.players[p].board.len() {
                        break;
                    }
                    let mut m = self.players[p].make_card(&self.data.cards, card, g, 0);
                    if self.data.cards.tribes_of(&m).has(Tribe::Beast) {
                        let (ba, bh) = self.players[p].beast_bonus;
                        m.attack += ba;
                        m.health += bh;
                    }
                    // A minion that reaches the board without being played
                    // gets its limited uses at once (docs/notes/KNOWN-GAPS.md):
                    // Malchezaar's Health Refreshes, a Trailblazer's combines,
                    // a Mycologist's teachings.
                    m.uses_left = m.turn_uses(&self.data.cards);
                    self.players[p].board.insert(at, m);
                    at += 1;
                }
                // Three summoned copies of one card are a triple too.
                if n > 1 {
                    self.check_triples_on_board(p);
                }
            }
            Effect::SpellBonus { attack, health } => {
                let n = mul * runs as i32;
                let pl = &mut self.players[p];
                pl.spell_bonus.0 += attack * n;
                pl.spell_bonus.1 += health * n;
            }
            Effect::RandomChromadrake => {
                for _ in 0..mul as u8 * runs {
                    if let Some(c) = self.data.cards.random_chromadrake(self.lobby, &mut self.rng) {
                        self.give_generated_tracking(p, c, pos);
                    }
                }
            }
            // A spell a minion's own effect casts for the player: Gatekeeper
            // Amalgam's Misplaced Tea Set, an Air Revenant's Easterly Winds, a
            // Crimson Vindicator's Rally spell. It happens and buffs as
            // printed, but it is not a spell the player cast (Era 2026-11-02,
            // docs/notes/UNSURE.md). A trinket's own `cast` act is the player's
            // cast: it calls `cast_named_spell` itself (trinkets.rs).
            Effect::CastSpell { card, times } => {
                self.cast_named_spell(p, card, times as u32 * mul as u32 * runs as u32, true);
            }
            Effect::TriggerRally => {
                let pt = PendingTarget {
                    by_minion: false,
                    source: Some(*pos as u8),
                    kind: TargetKind::TriggerRally,
                    runs: (mul as u8).saturating_mul(runs),
                    tavern: true,
                    gem: false,
                };
                self.offer_target(p, pt);
            }
            Effect::TriggerBattlecry => {
                let pt = PendingTarget {
                    by_minion: false,
                    source: Some(*pos as u8),
                    kind: TargetKind::TriggerBattlecry,
                    runs: (mul as u8).saturating_mul(runs),
                    tavern: true,
                    gem: false,
                };
                self.offer_target(p, pt);
            }
            Effect::BuffLeftmostHand { attack, health } => {
                let n = mul * runs as i32;
                let cards = &self.data.cards;
                let slot = self.players[p].hand.iter().position(|m| !cards.get(m.card).is_spell());
                if let Some(i) = slot {
                    let m = &mut self.players[p].hand[i];
                    m.attack += attack * n;
                    m.health += health * n;
                }
            }
            Effect::BuffTribeBoardAndHand { tribe, attack, health } => {
                let n = mul * runs as i32;
                let cards = &self.data.cards;
                for o in &mut self.players[p].board {
                    if cards.tribes_of(o).has(tribe) {
                        buff(o, attack * n, health * n);
                    }
                }
                for o in &mut self.players[p].hand {
                    if cards.tribes_of(o).has(tribe) {
                        buff(o, attack * n, health * n);
                    }
                }
            }
            Effect::DiscoverTribeIfAnother { tribe, count } => {
                let n = count.saturating_mul(mul as u8).saturating_mul(runs);
                let cards = &self.data.cards;
                let another = self.players[p]
                    .board
                    .iter()
                    .enumerate()
                    .any(|(i, m)| i != *pos && cards.tribes_of(m).has(tribe));
                if another {
                    for _ in 0..n {
                        let _ = self.players[p]
                            .discover_queue
                            .try_push(DiscoverRequest::TribeMinion(tribe));
                    }
                }
            }
            Effect::TriggerAdjacentBattlecries(count) => {
                let times = count.saturating_mul(mul as u8).saturating_mul(runs);
                let here = *pos;
                for neighbour in [here.checked_sub(1), Some(here + 1)].into_iter().flatten() {
                    for _ in 0..times {
                        if neighbour >= self.players[p].board.len() {
                            break;
                        }
                        self.battlecry_or_choose_one(p, neighbour);
                        if self.players[p].pending_target.is_some()
                            || self.players[p].pending_choose_one.is_some()
                        {
                            break;
                        }
                    }
                }
                self.maybe_start_discover(p);
            }
            // A Magicfin token casts the spell it was taught: the minion's
            // cast, not the player's.
            Effect::CastTaughtSpell => {
                let Some(sp) = self.players[p].board.get(*pos).and_then(|m| m.taught_spell)
                else {
                    return;
                };
                self.cast_named_spell(p, sp, (mul as u8 * runs) as u32, true);
                self.maybe_start_discover(p);
            }
            Effect::GetRandomTierMinion { tier } => {
                for _ in 0..mul * runs as i32 {
                    if let Some(c) =
                        self.data.cards.random_tier_minion(tier, self.lobby, &mut self.rng)
                    {
                        self.give_generated_tracking(p, c, pos);
                    }
                }
            }
            // A Rally triggered in the shop (Sky-hatch Runaway): the buffs
            // are permanent in the shop anyway.
            Effect::BuffEachType { attack, health } => {
                for _ in 0..mul * runs as i32 {
                    let cards = &self.data.cards;
                    let board = &mut self.players[p].board;
                    let tribes: ArrayVec<Tribes, BOARD> =
                        board.iter().map(|m| cards.tribes_of(m)).collect();
                    for i in pick_one_per_type(&tribes, &mut self.rng) {
                        if !cards.get(board[i].card).fixed_stats {
                            buff(&mut board[i], attack, health);
                        }
                    }
                }
            }
            Effect::GiveOwnStatsToLeftmostHand => {
                let Some(host) = self.players[p].board.get(*pos).copied() else { return };
                let n = mul * runs as i32;
                let cards = &self.data.cards;
                let slot = self.players[p].hand.iter().position(|m| !cards.get(m.card).is_spell());
                if let Some(i) = slot {
                    buff(&mut self.players[p].hand[i], host.attack * n, host.health * n);
                }
            }
            Effect::GetLockbox { sooner } => {
                for _ in 0..runs {
                    self.give_lockbox(p, sooner.saturating_mul(mul as u8));
                }
            }
            Effect::GetRandomBounty => {
                for _ in 0..mul as u8 * runs {
                    if let Some(c) = self.data.cards.random_bounty(self.lobby, &mut self.rng) {
                        self.give_generated_tracking(p, c, pos);
                    }
                }
            }
            Effect::BuffAdjacentPerGolden { attack } => {
                // Once, and again for each friendly Golden minion.
                let golden_count = self.players[p].board.iter().filter(|m| m.golden).count() as i32;
                let n = (1 + golden_count) * runs as i32;
                let here = *pos;
                let board = &mut self.players[p].board;
                for j in [here.checked_sub(1), Some(here + 1)].into_iter().flatten() {
                    if let Some(o) = board.get_mut(j) {
                        buff(o, attack * mul * n, 0);
                    }
                }
            }
            Effect::BuffChosenPerGold { tribe, attack, health, per_gold, in_shop } => {
                let g = self.players[p].gold_spent_turn as i32;
                let pt = PendingTarget {
                    by_minion: false,
                    source: Some(*pos as u8),
                    kind: TargetKind::MinionBuff {
                        tribe,
                        attack: (attack + per_gold.0 * g) * mul,
                        health: (health + per_gold.1 * g) * mul,
                        keyword: 0,
                        random_keyword: false,
                        shop: in_shop,
                    },
                    runs,
                        tavern: true,
                        gem: false,
                };
                self.offer_target(p, pt);
            }
            Effect::MakeGoldenChosen { tribe, max_tier, count } => {
                let pt = PendingTarget {
                    by_minion: false,
                    source: Some(*pos as u8),
                    kind: TargetKind::MakeGolden { tribe, max_tier },
                    runs: count.saturating_mul(mul as u8).saturating_mul(runs),
                        tavern: true,
                        gem: false,
                };
                self.offer_target(p, pt);
            }
            Effect::GetGoldenMinion { tier, count } => {
                let n = count.saturating_mul(mul as u8).saturating_mul(runs);
                for _ in 0..n {
                    if !self.give_golden_minion(p, tier, tier, false, true) {
                        break;
                    }
                }
            }
            Effect::MaxGold(n) => {
                let pl = &mut self.players[p];
                pl.extra_max_gold = pl.extra_max_gold.saturating_add(n.saturating_mul(mul as u8).saturating_mul(runs));
            }
            Effect::BecomeCopyChosen => {
                let pt = PendingTarget {
                    by_minion: false,
                    source: Some(*pos as u8),
                    kind: TargetKind::BecomeCopy,
                    runs,
                    tavern: true,
                    gem: false,
                };
                self.offer_target(p, pt);
            }
            Effect::PlainCopyRandomFriendly => {
                let n = (mul as u8).saturating_mul(runs);
                for _ in 0..n {
                    let len = self.players[p].board.len();
                    if len == 0 {
                        break;
                    }
                    let k = self.rng.below(len as u32) as usize;
                    let card = self.players[p].board[k].card;
                    self.give_generated(p, card);
                }
            }
            Effect::CopyLastDiscarded => {
                let Some(m) = self.players[p].last_discarded else { return };
                for _ in 0..runs {
                    if self.players[p].hand.is_full() {
                        break;
                    }
                    // "a copy of it with double stats": the doubling is the
                    // card's own, and a golden one doubles it again.
                    let mut c = self.players[p].make_card(&self.data.cards, m.card, m.golden, 0);
                    c.attack = m.attack.saturating_mul(2 * mul);
                    c.health = m.health.saturating_mul(2 * mul);
                    self.add_to_hand(p, c);
                }
            }
            Effect::BuffOthersPerGolden { tribe, attack, health } => {
                let k = 1 + self.players[p].golden_played as i32;
                let cards = &self.data.cards;
                let pl = &mut self.players[p];
                for _ in 0..runs {
                    for (i, o) in pl.board.iter_mut().enumerate() {
                        if i != *pos && cards.tribes_of(o).is(tribe) {
                            buff(o, attack * mul * k, health * mul * k);
                        }
                    }
                }
            }
            Effect::RefreshBuff { attack, health } => {
                let n = mul * runs as i32;
                let pl = &mut self.players[p];
                pl.refresh_buff.0 += attack * n;
                pl.refresh_buff.1 += health * n;
            }
            Effect::FreeRefreshes(n) => {
                let gain = n.saturating_mul(mul as u8).saturating_mul(runs);
                self.players[p].free_rolls = self.players[p].free_rolls.saturating_add(gain);
            }
            Effect::BuffAllImproving { attack, health, counter } => {
                for _ in 0..runs {
                    let k = 1 + self.players[p].improve[counter.index()] as i32;
                    let cards = &self.data.cards;
                    for o in &mut self.players[p].board {
                        if !cards.get(o.card).fixed_stats {
                            buff(o, attack * mul * k, health * mul * k);
                        }
                    }
                    let n = &mut self.players[p].improve[counter.index()];
                    *n = n.saturating_add(1);
                }
            }
            Effect::ArmNextBuy => {
                if let Some(m) = self.players[p].board.get_mut(*pos) {
                    // A golden Prison's own press is worth two shares.
                    m.improvements = m.improvements.saturating_add(mul as u16);
                }
            }
            Effect::GainHighestTavernStats => {
                let cards = &self.data.cards;
                let pl = &mut self.players[p];
                let mut best: Option<(i32, i32)> = None;
                let mut best_health = i32::MIN;
                for m in &pl.shop {
                    if !cards.get(m.card).is_spell() && m.health > best_health {
                        best_health = m.health;
                        best = Some((m.attack, m.health));
                    }
                }
                if let Some((a, h)) = best
                    && let Some(host) = pl.board.get_mut(*pos)
                    && !cards.get(host.card).fixed_stats
                {
                    let n = mul * runs as i32;
                    buff(host, a * n, h * n);
                }
            }
            Effect::UndeadBonus { attack, health, outside_attack } => {
                let a = if outside_attack != 0 { outside_attack } else { attack };
                let n = mul * runs as i32;
                self.add_undead_bonus(p, a * n, health * n, true);
            }
            Effect::DestroyChosenDiscover { tribe, count } => {
                let pt = PendingTarget {
                    by_minion: false,
                    source: Some(*pos as u8),
                    kind: TargetKind::DestroyDiscover {
                        tribe,
                        discover: count.saturating_mul(mul as u8).saturating_mul(runs),
                    },
                    runs: 1,
                        tavern: true,
                        gem: false,
                };
                self.offer_target(p, pt);
            }
            Effect::DestroyChosenGetCopy { tribe, copies } => {
                let pt = PendingTarget {
                    by_minion: false,
                    source: Some(*pos as u8),
                    kind: TargetKind::DestroyCopy {
                        tribe,
                        copies: copies.saturating_mul(mul as u8).saturating_mul(runs),
                    },
                    runs: 1,
                        tavern: true,
                        gem: false,
                };
                self.offer_target(p, pt);
            }
            Effect::GiveRebornDestroy { tribe, attack, health } => {
                let pt = PendingTarget {
                    by_minion: false,
                    source: Some(*pos as u8),
                    kind: TargetKind::RebornDestroy {
                        tribe,
                        attack: attack * mul,
                        health: health * mul,
                    },
                    runs: 1,
                    tavern: true,
                    gem: false,
                };
                self.offer_target(p, pt);
            }
            Effect::GetGems(n) => {
                let total = n.saturating_mul(mul as u8).saturating_mul(runs);
                self.give_gems(p, total);
            }
            Effect::GetGemsCastOverflow(n) => {
                let total = n.saturating_mul(mul as u8).saturating_mul(runs);
                let Some(gem) = self.data.cards.blood_gem else { return };
                for _ in 0..total {
                    if self.players[p].hand.is_full() {
                        // Cast on the left-most friendly minion instead.
                        self.play_gems_on_board(p, 0, 1);
                    } else {
                        let m = self.players[p].make_card(&self.data.cards, gem, false, 0);
                        self.add_to_hand(p, m);
                    }
                }
            }
            // A Rally triggered in the shop (Sky-hatch Runaway): the Gems are
            // permanent in the shop anyway.
            Effect::GemSelf { count, permanent: _ } => {
                let times = count.saturating_mul(mul as u8).saturating_mul(runs);
                self.play_gems_on_board(p, *pos, times);
            }
            Effect::GemOthers(count) => {
                let times = count.saturating_mul(mul as u8).saturating_mul(runs);
                let board_len = self.players[p].board.len();
                for i in 0..board_len {
                    if i != *pos {
                        self.play_gems_on_board(p, i, times);
                    }
                }
            }
            Effect::GemBonus { attack, health } => {
                let n = mul * runs as i32;
                let pl = &mut self.players[p];
                pl.gem_attack += attack * n;
                pl.gem_health += health * n;
            }
            Effect::GetRandomChooseOne(n) => {
                let total = n as i32 * mul * runs as i32;
                let tier = self.players[p].tier;
                for _ in 0..total {
                    if let Some(c) =
                        self.data.cards.random_choose_one_minion(tier, self.lobby, &mut self.rng)
                    {
                        self.give_generated_tracking(p, c, pos);
                    }
                }
            }
            // The shop copy of Jailbird Juggernaut's Rally (Sky-hatch
            // Runaway): summon the stat copy; there is no target to attack.
            Effect::SummonStatCopyAttacker { card } => {
                let Some(host) = self.players[p].board.get(*pos) else { return };
                let mut m = self.players[p].make_card(&self.data.cards, card, false, 0);
                m.attack = host.attack * mul;
                m.health = host.health * mul;
                m.uses_left = m.turn_uses(&self.data.cards);
                let at = *pos + 1;
                if !self.players[p].board.is_full() && at <= self.players[p].board.len() {
                    self.players[p].board.insert(at, m);
                }
            }
            // These only happen in fights.
            Effect::GiveRebornRandom { .. }
            | Effect::SummonAttacker { .. }
            | Effect::GetPlainCopy
            | Effect::GetRandomMinionHeld { .. }
            | Effect::SummonHeld
            | Effect::DestroyLeftNeighbor
            | Effect::SummonSalvaged
            | Effect::BuffOthersPerImprovement { .. }
            | Effect::BuffAllKeepGolden { .. }
            | Effect::DamageTargetAndNeighbor
            | Effect::BuffKiller { .. }
            | Effect::BuffAttacker { .. }
            | Effect::BuffRandomOther { .. }
            | Effect::SummonRandomDeathrattle { .. }
            | Effect::TriggerDeathrattle { .. }
            | Effect::SummonFromHand { .. }
            | Effect::BuffRandomHand { .. }
            | Effect::GainTargetAttack
            | Effect::StripTargetKeywords { .. }
            | Effect::DestroyKiller
            | Effect::ShieldDragons { .. }
            | Effect::GemAttacker(_) => {}
        }
    }

    /// Makeshift Master: the stats the watched minion gained since the last
    /// action go to the Deity too. The mark is on the card (`Minion.watched`),
    /// so the watch follows it wherever it goes: a Tavern minion I bought,
    /// the copy in my hand, the minion I then played. A card that leaves the
    /// game (sold, consumed, tripled away) takes the watch with it.
    pub(super) fn watch_deity_gain(&mut self, p: usize) {
        let tp = self.players[p].tp;
        if !tp.deity_watching {
            return;
        }
        let pl = &self.players[p];
        let watched = pl
            .board
            .iter()
            .chain(pl.hand.iter())
            .chain(pl.shop.iter())
            .find(|m| m.watched)
            .copied();
        let Some(m) = watched else {
            return;
        };
        let (a0, h0) = tp.deity_watch_stats;
        let (da, dh) = ((m.attack - a0).max(0), (m.health - h0).max(0));
        self.players[p].tp.deity_watch_stats = (m.attack, m.health);
        if da == 0 && dh == 0 {
            return;
        }
        if let Some(deity) = &mut self.players[p].deity {
            deity.attack += da;
            deity.health += dh;
        }
    }

    /// Mishmash: "Whenever your Amalgam gains stats, this gains them twice."
    /// Run after every action, the way Makeshift Master's watch is: the stats
    /// each board slot held after the last action are on the player, so
    /// whatever a friendly Amalgam gained while this one ran lands on every
    /// Mishmash. One pass over the board, and only while one is held.
    pub(super) fn note_amalgam_gains(&mut self, p: usize) {
        let cards = &self.data.cards;
        let has = self.players[p].board.iter().any(|m| cards.get(m.card).amalgam_gain_copy > 0);
        if !has {
            if self.players[p].amalgam_seen != [(0, 0, 0); crate::BOARD] {
                self.players[p].amalgam_seen = [(0, 0, 0); crate::BOARD];
            }
            return;
        }
        let mut gain = (0i32, 0i32);
        let mut seen = [(0, 0i32, i32::MIN); crate::BOARD];
        for i in 0..self.players[p].board.len() {
            let m = self.players[p].board[i];
            seen[i] = (m.card, m.attack, m.health);
            if !cards.get(m.card).amalgam {
                continue;
            }
            // A slot whose minion changed is a new minion, not a gain.
            let (c0, a0, h0) = self.players[p].amalgam_seen[i];
            if c0 != m.card {
                continue;
            }
            gain.0 += (m.attack - a0).max(0);
            gain.1 += (m.health - h0).max(0);
        }
        self.players[p].amalgam_seen = seen;
        if gain == (0, 0) {
            return;
        }
        for m in &mut self.players[p].board {
            let n = cards.get(m.card).amalgam_gain_copy as i32 * m.mul();
            if n > 0 {
                buff(m, gain.0 * n, gain.1 * n);
            }
        }
    }

    /// Scarlet Survivor: once its Attack reaches the threshold, it gains the
    /// keyword (once; `improvements` records that it fired).
    pub(super) fn check_attack_thresholds(&mut self, p: usize) {
        let cards = &self.data.cards;
        // "Once this reaches N Attack, gain X" (Scarlet Survivor) has no zone
        // in its text: a minion in the Tavern and one in the hand are buffed
        // there as a board minion is, so all three are checked.
        let pl = &mut self.players[p];
        for m in pl
            .board
            .iter_mut()
            .chain(pl.hand.iter_mut())
            .chain(pl.shop.iter_mut())
        {
            if m.improvements != 0 {
                continue;
            }
            let t = m.parts().find_map(|(c, _)| cards.get(c).attack_threshold);
            if let Some(t) = t
                && m.attack >= t.total as i32
            {
                m.keywords.set(t.keyword);
                m.improvements = 1;
            }
        }
    }

    /// Give the minion in board slot `pos` a random Bonus Keyword it doesn't
    /// have yet, if there is one.
    pub(super) fn give_random_keyword(&mut self, p: usize, pos: usize) {
        let mask = self.data.rules.bonus_mask;
        let Some(m) = self.players[p].board.get_mut(pos) else { return };
        if let Some(k) = m.keywords.random_missing(mask, &mut self.rng) {
            m.keywords.set(k);
        }
    }

    /// A random Bonus Keyword the Tavern minion in `slot` does not have yet
    /// (Vibrant Bubble cast on the Tavern).
    pub(super) fn give_random_keyword_in_shop(&mut self, p: usize, slot: usize) {
        let mask = self.data.rules.bonus_mask;
        let Some(m) = self.players[p].shop.get_mut(slot) else { return };
        if let Some(k) = m.keywords.random_missing(mask, &mut self.rng) {
            m.keywords.set(k);
        }
    }

    /// The minion in board slot `pos` consumes `count` minions from the
    /// Tavern. A chosen one waits for the player to pick (or takes the only
    /// choice there is).
    pub(super) fn start_consume(
        &mut self,
        p: usize,
        pos: usize,
        pick: ConsumePick,
        keywords: bool,
        count: u8,
        stat_mul: u8,
    ) {
        for left in (1..=count).rev() {
            let shop_len = self.players[p].shop.len();
            if shop_len == 0 || pos >= self.players[p].board.len() {
                return;
            }
            let slot = match pick {
                ConsumePick::Random => self.rng.below(shop_len as u32) as usize,
                ConsumePick::HighestHealth => {
                    let shop = &self.players[p].shop;
                    // the first of equals, so the leftmost
                    (0..shop_len).fold(0, |best, i| if shop[i].health > shop[best].health { i } else { best })
                }
                ConsumePick::Chosen if shop_len == 1 => 0,
                ConsumePick::Chosen => {
                    self.players[p].pending_shop_target = Some(PendingShopTarget {
                        source: pos as u8,
                        kind: ShopTargetKind::Consume { keywords, remaining: left, stat_mul },
                    });
                    return;
                }
            };
            self.consume(p, pos, slot, keywords, stat_mul);
        }
    }

    /// The minion in board slot `pos` takes the Tavern minion in `slot`: its
    /// stats (`stat_mul` times), and its Bonus Keywords if `keywords`. The
    /// copy goes back to the pool (a generated Fodder has no copy to return).
    pub(super) fn consume(&mut self, p: usize, pos: usize, slot: usize, keywords: bool, stat_mul: u8) {
        let neighbours = matches!(
            self.players[p].board.get(pos).and_then(|m| self.player_card_mod(p, m.card)),
            Some(CardMod::AlsoNeighbors)
        );
        let Game { data, players, pool, .. } = self;
        let pl = &mut players[p];
        if slot >= pl.shop.len() {
            return;
        }
        let eaten = pl.shop.remove(slot);
        give_back(pool, &data.cards, &eaten);
        // Consuming Claw: a friendly Demon's consume also takes Bonus
        // Keywords and +5/+5.
        let claw = pl.tp.consume_claw;
        let by_demon = claw != (0, 0)
            && data.cards.tribes_of(&pl.board[pos]).has(Tribe::Demon);
        let m = &mut pl.board[pos];
        let k = stat_mul.max(1) as i32;
        buff(m, eaten.attack * k, eaten.health * k);
        if keywords || by_demon {
            m.keywords.0 |= eaten.keywords.0 & data.rules.bonus_mask;
        }
        if by_demon {
            buff(m, claw.0, claw.1);
        }
        // Flaming Portrait: the Flaming Enforcers' consume also gives the
        // stats to their neighbours.
        if neighbours {
            let (da, dh) = (eaten.attack * k, eaten.health * k);
            let mut sibs = [usize::MAX; 2];
            let mut found = 0;
            for j in [pos.checked_sub(1), Some(pos + 1)].into_iter().flatten() {
                if j < pl.board.len() {
                    sibs[found] = j;
                    found += 1;
                }
            }
            for &j in &sibs[..found] {
                buff(&mut pl.board[j], da, dh);
            }
        }
        let card = eaten.card;
        self.trinket_event(p, crate::trinkets::Event::Consume, Some(card), 1);
    }

    /// Robust Evolution on a board minion: a random pool minion exactly one
    /// tier higher (a tier 6 into another tier 6), keeping only its Attack
    /// and Health. The old copy goes back to the pool, the new comes from it.
    pub(super) fn robust_evolution_board(&mut self, p: usize, slot: usize) {
        let old = self.players[p].board[slot];
        let Some(new_id) = self.robust_evolution_draw(p, old) else { return };
        let mut m = self.data.cards.make(new_id, false);
        m.pool_copies = 1;
        m.attack = old.attack;
        m.health = old.health;
        self.players[p].board[slot] = m;
    }

    pub(super) fn robust_evolution_shop(&mut self, p: usize, slot: usize) {
        let old = self.players[p].shop[slot];
        let Some(new_id) = self.robust_evolution_draw(p, old) else { return };
        let mut m = self.data.cards.make(new_id, false);
        m.pool_copies = 1;
        m.attack = old.attack;
        m.health = old.health;
        self.players[p].shop[slot] = m;
    }

    /// Return the old copies to the pool and draw the new one. The result is
    /// never golden; a golden old minion keeps its Attack and Health and
    /// returns its copies (an assumption, see docs/rules/design.md).
    fn robust_evolution_draw(&mut self, _p: usize, old: Minion) -> Option<CardId> {
        let old_tier = self.data.cards.get(old.card).tier;
        // Tier 6 and tier 7 minions stay on their tier: no minion is ever
        // evolved up to tier 7.
        let new_tier = if old_tier >= MAX_TIER { old_tier } else { old_tier + 1 };
        let new_id = draw_filtered(
            &mut self.pool,
            &self.data.cards,
            &mut self.rng,
            new_tier,
            new_tier,
            &[],
            self.lobby,
            |_| true,
        )?;
        let (slot, copies, token) = {
            let c = self.data.cards.get(old.card);
            (c.pool_slot, old.pool_copies, c.token || c.is_spell())
        };
        if copies > 0 && !token {
            self.pool[slot as usize] += copies;
        }
        Some(new_id)
    }

    /// Channel the Devourer: sell the chosen board minion (a normal sale)
    /// and give its stats to a random other friendly minion.
    pub(super) fn channel_the_devourer(&mut self, p: usize, slot: usize) {
        if slot >= self.players[p].board.len() {
            return;
        }
        let stats = (self.players[p].board[slot].attack, self.players[p].board[slot].health);
        self.sell_minion(p, slot);
        let n = self.players[p].board.len();
        if n == 0 {
            return;
        }
        let k = self.rng.below(n as u32) as usize;
        buff(&mut self.players[p].board[k], stats.0, stats.1);
    }

    /// Faceless Manipulator: the minion in board slot `src` becomes `copy`,
    /// exactly as it stands (buffs, keywords, Golden status, attachments),
    /// but it holds no pool copy and gives no Triple Reward: it is a copy
    /// that was never played.
    pub(super) fn become_copy_of(&mut self, p: usize, src: usize, copy: Minion) {
        if src >= self.players[p].board.len() {
            return;
        }
        let mut m = copy;
        m.pool_copies = 0;
        m.no_reward = true;
        self.players[p].board[src] = m;
    }

    /// Run every event trigger of `kind` on the player's board. Each adds
    /// `amount` to its count and fires once for every full `every` in it,
    /// keeping the remainder.
    pub(super) fn fire_event(&mut self, p: usize, kind: EventKind, amount: u32) {
        self.fire_event_skipping(p, kind, amount, None);
    }

    /// `fire_event`, leaving out the board minion in slot `skip` (a card
    /// never counts its own play).
    pub(super) fn fire_event_skipping(&mut self, p: usize, kind: EventKind, amount: u32, skip: Option<usize>) {
        if amount == 0 {
            return;
        }
        if kind == EventKind::GoldSpent {
            let pl = &mut self.players[p];
            pl.gold_spent_turn = pl.gold_spent_turn.saturating_add(amount.min(u16::MAX as u32) as u16);
        }
        if kind == EventKind::SpellsCast {
            self.players[p].spells_cast = self.players[p].spells_cast.saturating_add(amount);
        }
        // EventKind::AllSpellsCast counts every spell but has no player
        // scalar of its own: it is for the minions that say "spells". The
        // observation's "Tavern spells cast this game" is the counter above
        // and stays Tavern-only.
        if kind == EventKind::GoldSpent {
            self.trinket_event(p, crate::trinkets::Event::GoldSpent, None, amount);
            // The hero powers' own Gold count (Choose Your Champion).
            self.power_gold_spent(p, amount);
        }
        self.run_event_triggers(p, kind, amount, skip, TriggerWhen::Now);
    }

    /// The board minions' own triggers for an event, without the player-level
    /// counters and trinkets `fire_event` takes first. A Choose One spell's
    /// cast counts itself at the play but its triggers wait for the choice
    /// (docs/rules/design.md "Targets in the shop"), so the two halves are fired apart.
    /// `when` picks the half: a "whenever you cast a spell" trigger fires as
    /// the spell is cast, an "after you cast a spell" one once it has
    /// resolved (`EventTrigger::resolved`).
    pub(super) fn run_event_triggers(
        &mut self,
        p: usize,
        kind: EventKind,
        amount: u32,
        skip: Option<usize>,
        when: TriggerWhen,
    ) {
        if amount == 0 {
            return;
        }
        let mut pos = 0;
        while pos < self.players[p].board.len() {
            if skip == Some(pos) {
                pos += 1;
                continue;
            }
            let m = self.players[p].board[pos];
            for (card, golden) in m.parts() {
                let Some(t) = self.data.cards.get(card).on_event else { continue };
                if t.kind != kind
                    || t.resolved != (when == TriggerWhen::Resolved)
                    || pos >= self.players[p].board.len()
                {
                    continue;
                }
                let imp = &mut self.players[p].board[pos].improvements;
                // An every-1 trigger has no remainder to keep, so it leaves
                // `improvements` alone: an effect that improves itself on
                // every fire (Harbinger Aph'lass) stores its level there.
                let mut count = if t.every == 1 { amount } else { *imp as u32 + amount };
                let mut fires = 0;
                while count >= t.every as u32 {
                    count -= t.every as u32;
                    fires += 1;
                }
                if t.every > 1 {
                    *imp = count as u16;
                }
                for _ in 0..fires {
                    self.run_effect(p, &mut pos, t.effect, golden, 1);
                }
            }
            pos += 1;
        }
    }
}
