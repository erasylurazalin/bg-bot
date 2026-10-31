//! The shop-phase state of one player (impl Player).

use arrayvec::ArrayVec;
use crate::cards::{
    N_COUNTERS,
    CardDb,
    Effect,
    OnSell,
    Tribe,
    Tribes,
};
use crate::fight::{Counters, Deity};
use crate::heroes::PowerEffect;
use crate::minion::{
    CardId,
    Minion,
};
use crate::real_boards::RealCounters;
use crate::rules::Rules;
use crate::{
    DISCOVER,
    Data,
    MAX_TIER,
    TRIBES,
};
use super::{
    Player,
    TavernBuff,
    TrinketPowers,
    TrinketState,
};
use super::effects::buff;

impl Player {
    pub(super) fn new(data: &Data) -> Player {
        Player {
            health: data.rules.starting_health,
            alive: true,
            hero: None,
            armor: 0,
            power: None,
            hero_offer: [u8::MAX; DISCOVER],
            power_offer: ArrayVec::new(),
            trinkets: [TrinketState::default(); crate::TRINKET_SLOTS],
            trinket_offer: ArrayVec::new(),
            trinket_offer_slot: 0,
            trinket_events: 0,
            tp: TrinketPowers::default(),
            dark_gifts: 0,
            real: false,
            placement: 0,
            tier_turns: [1, 0, 0, 0, 0, 0],
            final_tribes: [0; TRIBES],
            tavern_buffs: ArrayVec::new(),
            last_damage: 0,
            last_damage_armor: 0,
            gold: 0,
            gold_next_turn: 0,
            extra_max_gold: 0,
            free_rolls: 0,
            tier: 1,
            upgrade_cost: data.rules.upgrade_cost(1),
            frozen: false,
            lost_last_combat: false,
            won_last_combat: false,
            overconfidence_win: 0,
            overconfidence_tie: 0,
            winners_bread_buff: (4, 6),
            next_turn_board_buffs: ArrayVec::new(),
            boon_of_beetles: 0,
            brood_of_nozdormu: 0,
            upper_hand: 0,
            sharing_is_caring: 0,
            volumizer_bonus: (0, 0),
            beetle_bonus: (0, 0),
            improve: [0; N_COUNTERS],
            beast_bonus: (0, 0),
            deathrattles_triggered: 0,
            golden_played: 0,
            gold_spent_turn: 0,
            elementals_played_turn: 0,
            spells_cast_turn: 0,
            elementals_played: 0,
            magnetizations: 0,
            refresh_buff: (0, 0),
            spell_bonus: (0, 0),
            spell_discount: 0,
            last_spell: None,
            shop_spell: None,
            shop: ArrayVec::new(),
            hand: ArrayVec::new(),
            board: ArrayVec::new(),
            pending_choose_one: None,
            pending_target: None,
            pending_shop_target: None,
            pending_discard: None,
            hogrider_pending: 0,
            choose_queue: ArrayVec::new(),
            discover: ArrayVec::new(),
            discover_queue: ArrayVec::new(),
            discover_damage_from: None,
            discover_locked: false,
            discover_doomed: false,
            discover_both: false,
            undead_bonus: (0, 0),
            knight_deaths: 0,
            fodder_refreshes: ArrayVec::new(),
            gem_attack: 1,
            gem_health: 1,
            barrage: (0, 0),
            deity: None,
            discarded: 0,
            last_discarded: None,
            spells_cast: 0,
            next_link: 1,
            actions: 0,
            done: false,
            opponent: None,
            last_opponent: None,
            seen: Default::default(),
        }
    }

    /// Cost of the spell currently in the shop, after Ominous Seer discounts.
    pub fn spell_cost(&self, cards: &CardDb) -> Option<u8> {
        let id = self.shop_spell?;
        Some(cards.get(id).cost.saturating_sub(self.spell_discount))
    }

    /// Highest discover tier of any Patient Scout currently on the board, or 0
    /// when none is present.
    pub fn patient_scout_tier(&self, cards: &CardDb) -> u8 {
        let mut best = 0u8;
        for m in &self.board {
            for (card, _) in m.parts() {
                if matches!(cards.get(card).on_sell, Some(OnSell::DiscoverByTier)) {
                    let tier = (1 + m.improvements as u8).min(MAX_TIER);
                    best = best.max(tier);
                }
            }
        }
        best
    }

    /// Counts per `TRIBES` slot (the ten tribes in `Tribe::EVERY` order, then
    /// neutral) of the minions currently on this player's board. A
    /// multi-tribe minion counts once in each of its tribes.
    pub fn board_tribes(&self, cards: &CardDb) -> [u8; TRIBES] {
        let mut out = [0u8; TRIBES];
        for m in &self.board {
            let t = cards.get(m.card).tribes;
            if t.is_neutral() {
                out[Tribe::EVERY.len()] += 1;
            }
            for (slot, tribe) in Tribe::EVERY.into_iter().enumerate() {
                if t.has(tribe) {
                    out[slot] += 1;
                }
            }
        }
        out
    }

    /// How many different Bonus Keywords the minions on the board have
    /// between them.
    pub fn distinct_bonus_keywords(&self, mask: u8) -> u32 {
        (self.board.iter().fold(0, |acc, m| acc | m.keywords.0) & mask).count_ones()
    }

    /// Queue `per` Fodders for each of the next `count` Refreshes. It stacks:
    /// a Refresh that already gets Fodders gets these on top.
    pub(super) fn add_fodder_refreshes(&mut self, per: u8, count: u32) {
        for i in 0..count as usize {
            if !self.add_fodder_to_refresh(i, per) {
                break;
            }
        }
    }

    /// Add `per` Fodders to the Refresh `index` queues from now (0 is the
    /// next one), filling any gap with a Refresh that gets none. Returns
    /// false when the queue is full.
    pub(super) fn add_fodder_to_refresh(&mut self, index: usize, per: u8) -> bool {
        if per == 0 {
            return true;
        }
        while self.fodder_refreshes.len() < index {
            if self.fodder_refreshes.try_push(0).is_err() {
                return false;
            }
        }
        match self.fodder_refreshes.get_mut(index) {
            Some(n) => {
                *n = n.saturating_add(per);
                true
            }
            None => self.fodder_refreshes.try_push(per).is_ok(),
        }
    }

    /// Take the Fodders the next Refresh gets, if any are queued.
    pub(super) fn take_fodder_for_refresh(&mut self) -> u8 {
        if self.fodder_refreshes.is_empty() {
            0
        } else {
            self.fodder_refreshes.remove(0)
        }
    }

    /// Health Refreshes left this turn, from the uses on the Malchezaars on
    /// the board. Uses belong to the minion: a golden from a triple starts
    /// fresh, and sold copies take theirs along.
    pub fn health_refreshes_left(&self, cards: &CardDb) -> u8 {
        self.board
            .iter()
            .filter(|m| m.parts().any(|(c, _)| cards.get(c).health_refreshes > 0))
            .map(|m| m.uses_left)
            .fold(0u8, u8::saturating_add)
    }

    /// Spend one Health Refresh: the first Malchezaar left to right that
    /// still has one loses it.
    pub fn spend_health_refresh(&mut self, cards: &CardDb) {
        let Some(i) = (0..self.board.len()).find(|&i| {
            self.board[i].uses_left > 0
                && self.board[i].parts().any(|(c, _)| cards.get(c).health_refreshes > 0)
        }) else { return };
        self.board[i].uses_left -= 1;
    }

    /// Whether a minion on the board undoes hero damage ("rewind it").
    pub fn can_rewind(&self, cards: &CardDb) -> bool {
        self.board.iter().any(|m| {
            m.parts().any(|(card, _)| cards.get(card).after_hero_damage.contains(&Effect::RewindDamage))
        })
    }

    /// Whether a Health cost can be paid: it can't kill the hero, unless a
    /// rewind on the board gives the Health straight back. Armor absorbs the
    /// cost first, so only what gets past it counts.
    pub fn can_pay_health(&self, cost: i32, cards: &CardDb) -> bool {
        self.health - (cost - self.armor).max(0) >= 1 || self.can_rewind(cards)
    }

    /// Whether the next Refresh would be paid in Health.
    pub fn can_refresh_with_health(&self, cards: &CardDb, rules: &Rules) -> bool {
        self.health_refreshes_left(cards) > 0 && self.can_pay_health(rules.health_refresh_cost, cards)
    }

    /// Add a Tavern buff: it goes onto the minions in the Tavern now and is
    /// remembered for the ones that show up later.
    pub fn add_tavern_buff(&mut self, cards: &CardDb, b: TavernBuff) {
        match self
            .tavern_buffs
            .iter_mut()
            .find(|o| o.tribe == b.tribe && o.this_turn == b.this_turn)
        {
            Some(o) => {
                o.attack += b.attack;
                o.health += b.health;
            }
            None => self.tavern_buffs.push(b),
        }
        for m in &mut self.shop {
            if !cards.get(m.card).is_spell() && cards.get(m.card).is(b.tribe) {
                buff(m, b.attack, b.health);
            }
        }
    }

    /// Put every Tavern buff that applies on a minion that just showed up in
    /// the Tavern.
    pub fn apply_tavern_buffs(&self, cards: &CardDb, m: &mut Minion) {
        if cards.get(m.card).is_spell() {
            return;
        }
        for b in &self.tavern_buffs {
            if cards.get(m.card).is(b.tribe) {
                buff(m, b.attack, b.health);
            }
        }
    }

    /// The player's turn is over: "this turn" Tavern buffs come off the
    /// minions still in the Tavern. Bought minions keep what they had.
    pub fn end_turn_tavern_buffs(&mut self, cards: &CardDb) {
        for b in self.tavern_buffs.iter().filter(|b| b.this_turn) {
            for m in &mut self.shop {
                if !cards.get(m.card).is_spell() && cards.get(m.card).is(b.tribe) {
                    buff(m, -b.attack, -b.health);
                }
            }
        }
        self.tavern_buffs.retain(|b| !b.this_turn);
    }

    /// Create a copy of `id` with this player's current Volumizer and Beetle
    /// bonuses applied.
    pub fn make_card(&self, cards: &CardDb, id: CardId, golden: bool, pool_copies: u8) -> Minion {
        let mut m = cards.make(id, golden);
        m.pool_copies = pool_copies;
        if m.volumizers > 0 {
            m.attack += self.volumizer_bonus.0 * m.volumizers as i32;
            m.health += self.volumizer_bonus.1 * m.volumizers as i32;
        }
        let c = cards.get(id);
        if c.is_beetle {
            m.attack += self.beetle_bonus.0;
            m.health += self.beetle_bonus.1;
        }
        if let Some((a, h)) = c.buff_per_deathrattle {
            let n = self.deathrattles_triggered as i32 * m.mul();
            m.attack += a * n;
            m.health += h * n;
        }
        if let Some((a, h)) = c.buff_per_golden_played {
            let n = self.golden_played as i32 * m.mul();
            m.attack += a * n;
            m.health += h * n;
        }
        if let Some((a, h)) = c.buff_per_elemental_played {
            let n = self.elementals_played as i32 * m.mul();
            m.attack += a * n;
            m.health += h * n;
        }
        if let Some((a, h)) = c.buff_per_magnetize {
            let n = self.magnetizations as i32 * m.mul();
            m.attack += a * n;
            m.health += h * n;
        }
        if c.tribes.has(Tribe::Undead) && !c.fixed_stats {
            m.attack += self.undead_bonus.0;
            m.health += self.undead_bonus.1;
        }
        if let Some((a, h)) = c.buff_per_kind_death {
            let n = self.knight_deaths as i32 * m.mul();
            m.attack += a * n;
            m.health += h * n;
        }
        m
    }

    /// The minion slots this player's Tavern has right now. `SHOP` holds the
    /// spell slot too, so a Tavern never offers more than `SHOP - 1` minions
    /// at once (DESIGN.md: never more than 7 slots, one almost always the
    /// spell). Glowing Gauntlet fixes it at six (six plus the spell is the
    /// seven it promises); Innkeeper's Stein and Electrode Attractor add one
    /// each, and Stein's needs a tier above yours to draw from.
    pub fn shop_size(&self, rules: &Rules) -> usize {
        let base = if self.tp.gauntlet { 6 } else { rules.shop_size(self.tier) };
        let mut extra = 0;
        if self.tp.extra_high_tier && self.tier < crate::MAX_TIER {
            extra += 1;
        }
        if self.tp.extra_magnetic {
            extra += 1;
        }
        (base + extra).min(crate::SHOP - 1)
    }

    /// The player's Tavern spell stat bonus: permanent gains plus board
    /// auras (Enchanted Sentinel, including when Magnetized into a host,
    /// and Humon'gozz).
    pub fn total_spell_bonus(&self, cards: &CardDb) -> (i32, i32) {
        let (mut da, mut dh) = self.spell_bonus;
        for m in &self.board {
            for (card, golden) in m.parts() {
                if let Some((a, h)) = cards.get(card).spell_buff {
                    let mul = if golden { 2 } else { 1 };
                    da += a * mul;
                    dh += h * mul;
                }
            }
        }
        // Multilayered Shield: so much for each different friendly minion
        // type, and Honeycomb Ring: what this turn's spells on minions added.
        let (pa, ph) = self.tp.spell_bonus_per_type;
        if (pa, ph) != (0, 0) {
            let mut seen = crate::cards::Tribes::NONE;
            let mut types = 0i32;
            for m in &self.board {
                let ts = cards.get(m.card).tribes;
                if ts.is_empty() || seen.overlaps(ts) {
                    continue;
                }
                seen = seen.with_all(ts);
                types += 1;
            }
            da += pa * types;
            dh += ph * types;
        }
        let (ta, th) = self.tp.spell_bonus_turn;
        da += ta;
        dh += th;
        (da, dh)
    }

    /// Give a real seat the numbers and the Deity of the board it just drew,
    /// replacing the ones the last turn's board left: nothing accumulates on
    /// a real seat, and a number the log did not show is the base value
    /// (`RealCounters::default`). Its Health, armor, tier and hand are the
    /// seat's own and set elsewhere.
    pub(super) fn take_real_counters(&mut self, c: &RealCounters, deity: Option<Deity>) {
        self.gem_attack = c.gem_attack;
        self.gem_health = c.gem_health;
        self.beetle_bonus = (c.beetle_attack, c.beetle_health);
        self.undead_bonus = (c.undead_attack, c.undead_health);
        self.spell_bonus = (c.spell_attack, c.spell_health);
        self.spells_cast = c.spells_cast;
        self.knight_deaths = c.knight_deaths;
        self.improve = c.improve;
        self.deity = deity;
        // A fight's outcome can move these too; they start from the base
        // again. None of them is read without a shop phase.
        self.beast_bonus = (0, 0);
        self.boon_of_beetles = 0;
        self.brood_of_nozdormu = 0;
        self.upper_hand = 0;
        self.sharing_is_caring = 0;
    }

    /// The trinkets' effects inside a fight, as the fight's small `Copy`
    /// bundle: the slots in order (lesser, greater, a power's), each act with
    /// the slot's own "improve this" number. Allocation-free, and empty for
    /// a player with no trinket, which is one check in the fight.
    pub fn fight_trinkets(&self, data: &Data) -> crate::fight::FightTrinkets {
        let mut ft = crate::fight::FightTrinkets::default();
        // Nobody holds a trinket before turn 6: one check, no table read.
        if self.trinket_events & crate::trinkets::HAS_TRINKET == 0 {
            return ft;
        }
        for slot in 0..crate::TRINKET_SLOTS {
            let st = &self.trinkets[slot];
            if st.of == 0 {
                continue;
            }
            let t = data.trinkets.get(st.of - 1);
            for e in &t.fight {
                ft.push(e, slot as u8, st.value as i16);
            }
            if t.first_deathrattle {
                ft.set_first_deathrattle(true);
            }
            if let Some(r) = &t.rule
                && let crate::trinkets::Event::DeathTribe(w) = r.event
            {
                ft.count_death_tribes |= crate::cards::Tribes::of(w).mask();
            }
        }
        ft.count_deaths = self.trinket_events & crate::trinkets::Event::Death.bit() != 0;
        ft.count_summons = self.trinket_events & crate::trinkets::Event::Summon.bit() != 0;
        ft
    }

    /// Whether one of this player's trinkets reads the hand inside a fight
    /// (Dramaloc Sticker), so the fight has to be given the hand at all.
    pub fn trinket_reads_hand(&self, data: &Data) -> bool {
        (0..crate::TRINKET_SLOTS).any(|slot| {
            let of = self.trinkets[slot].of;
            of != 0
                && data.trinkets.get(of - 1).fight.iter().any(|e| {
                    matches!(e.act, crate::trinkets::FightAct::HighestHandAttack { .. })
                })
        })
    }

    pub fn counters(&self, data: &Data, lobby: Tribes) -> Counters {
        let cards = &data.cards;
        let (spell_attack, spell_health) = self.total_spell_bonus(cards);
        Counters {
            health: Some(self.health),
            beetle_attack: self.beetle_bonus.0,
            beetle_health: self.beetle_bonus.1,
            improve: self.improve,
            beast_attack: self.beast_bonus.0,
            beast_health: self.beast_bonus.1,
            tier: self.tier,
            boon_of_beetles: self.boon_of_beetles,
            brood_of_nozdormu: self.brood_of_nozdormu,
            upper_hand: self.upper_hand,
            trinkets: self.fight_trinkets(data),
            soc_leftmost: self.power.map_or(0, |st| {
                match data.heroes.get(st.of).power.effect {
                    PowerEffect::SocLeftmost { keywords } => keywords,
                    _ => 0,
                }
            }),
            sharing_is_caring: self.sharing_is_caring,
            spell_attack,
            spell_health,
            undead_attack: self.undead_bonus.0,
            undead_health: self.undead_bonus.1,
            // Hammer of Twilight: the base plus a step per card discarded
            // this game, for the minions the fight summons.
            aura_attack: self.tp.hammer_base.0
                + self.tp.hammer_per.0 * self.discarded as i32,
            aura_health: self.tp.hammer_base.1
                + self.tp.hammer_per.1 * self.discarded as i32,
            knight_deaths: self.knight_deaths,
            gem_attack: self.gem_attack,
            gem_health: self.gem_health,
            deity: self.deity,
            spells_cast: self.spells_cast,
            lobby,
        }
    }
}
