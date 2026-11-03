use serde::Deserialize;

use crate::minion::Keywords;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DamageCap {
    pub through_turn: u32,
    pub cap: i32,
}

/// Every number in rules.toml. See the comments there.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rules {
    pub starting_health: i32,
    pub first_turn_gold: u8,
    pub max_gold: u8,
    pub buy_cost: u8,
    pub sell_gold: u8,
    pub roll_cost: u8,
    pub upgrade_costs: [u8; 5],
    pub shop_sizes: [u8; 6],
    /// Pool copies per card, tiers 1 to 7 (`CARD_MAX_TIER`).
    pub copies_per_tier: [u8; 7],
    pub damage_caps: Vec<DamageCap>,
    pub late_damage_cap: i32,
    pub uncapped_at_players: u8,
    pub max_turns: u32,
    pub max_actions_per_turn: u16,
    pub max_attacks: u32,
    /// Heroes each player is offered before turn 1; at most `DISCOVER`.
    pub hero_choices: u8,
    /// Damage a player deals to their own hero (card effects, Health costs)
    /// can't take them below this. Combat damage ignores it.
    /// Health a Refresh costs when it is paid in Health.
    pub health_refresh_cost: i32,
    /// Trinkets: the two offer turns, the tribe counts that guarantee a
    /// tribe's trinket (and keep its price), the discount for a tribe trinket
    /// whose tribe falls short, and the draw weights of the other options.
    pub trinket_lesser_turn: u32,
    pub trinket_greater_turn: u32,
    pub trinket_lesser_need: u8,
    pub trinket_greater_need: u8,
    pub trinket_discount: u8,
    pub trinket_offer_neutral: u32,
    pub trinket_offer_own: u32,
    pub trinket_offer_other: u32,
    pub trinket_offer_mechanic: u32,
    /// The Dark Discovery button: the Gold it costs, its uses a game and the
    /// first turn it can be pressed (docs/rules/darkgifts.md).
    pub dark_gift_cost: u8,
    pub dark_gift_uses: u8,
    pub dark_gift_turn: u8,
    /// The keywords "random Bonus Keyword" picks from and "different Bonus
    /// Keywords" counts, by name (see `Keywords::ALL`).
    pub bonus_keywords: Vec<String>,
    /// `bonus_keywords` as a keyword bit set.
    #[serde(skip)]
    pub bonus_mask: u8,
}

impl Rules {
    pub fn from_toml(s: &str) -> Result<Rules, String> {
        let mut r: Rules = toml::from_str(s).map_err(|e| format!("rules.toml: {e}"))?;
        for name in &r.bonus_keywords {
            let bit = Keywords::ALL
                .iter()
                .find(|(_, n)| n == name)
                .map(|(b, _)| *b)
                .ok_or_else(|| format!("rules.toml: unknown bonus keyword {name:?}"))?;
            r.bonus_mask |= bit;
        }
        // A tavern's minion slots plus its spell slot must fit in the shop.
        if r.shop_sizes.iter().any(|&n| n as usize + 1 > crate::SHOP || n == 0) {
            return Err(format!("rules.toml: shop sizes must be 1 to {}", crate::SHOP - 1));
        }
        if r.damage_caps.windows(2).any(|w| w[0].through_turn >= w[1].through_turn) {
            return Err("rules.toml: damage_caps must be in turn order".into());
        }
        if r.max_actions_per_turn == 0 {
            return Err("rules.toml: max_actions_per_turn must be at least 1".into());
        }
        if r.hero_choices == 0 || r.hero_choices as usize > crate::DISCOVER {
            return Err(format!("rules.toml: hero_choices must be 1 to {}", crate::DISCOVER));
        }
        if r.trinket_lesser_turn == 0 || r.trinket_greater_turn <= r.trinket_lesser_turn {
            return Err("rules.toml: the greater trinket turn must come after the lesser one".into());
        }
        if r.trinket_lesser_need == 0 || r.trinket_greater_need < r.trinket_lesser_need {
            return Err("rules.toml: trinket tribe counts must rise from lesser to greater".into());
        }
        if [r.trinket_offer_neutral, r.trinket_offer_own, r.trinket_offer_other].contains(&0) {
            return Err("rules.toml: every trinket offer weight must be at least 1".into());
        }
        Ok(r)
    }

    pub fn gold(&self, turn: u32) -> u8 {
        let g = self.first_turn_gold as u32 + turn.saturating_sub(1);
        g.min(self.max_gold as u32) as u8
    }

    /// Cost to upgrade from `tier` to the next one, before discounts.
    pub fn upgrade_cost(&self, tier: u8) -> u8 {
        if tier >= crate::MAX_TIER { 0 } else { self.upgrade_costs[tier as usize - 1] }
    }

    pub fn shop_size(&self, tier: u8) -> usize {
        self.shop_sizes[tier as usize - 1] as usize
    }

    pub fn copies(&self, tier: u8) -> u8 {
        self.copies_per_tier[tier as usize - 1]
    }

    /// The most damage a hero can take this turn, None for no cap.
    pub fn damage_cap(&self, turn: u32, alive: usize) -> Option<i32> {
        if alive <= self.uncapped_at_players as usize {
            return None;
        }
        let cap = self.damage_caps.iter().find(|c| turn <= c.through_turn);
        Some(cap.map_or(self.late_damage_cap, |c| c.cap))
    }
}
