use serde::Deserialize;

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
    pub copies_per_tier: [u8; 6],
    pub damage_caps: Vec<DamageCap>,
    pub late_damage_cap: i32,
    pub uncapped_at_players: u8,
    pub max_turns: u32,
    pub max_actions_per_turn: u16,
    pub max_attacks: u32,
}

impl Rules {
    pub fn from_toml(s: &str) -> Result<Rules, String> {
        let r: Rules = toml::from_str(s).map_err(|e| format!("rules.toml: {e}"))?;
        if r.shop_sizes.iter().any(|&n| n as usize > crate::SHOP || n == 0) {
            return Err(format!("rules.toml: shop sizes must be 1 to {}", crate::SHOP));
        }
        if r.damage_caps.windows(2).any(|w| w[0].through_turn >= w[1].through_turn) {
            return Err("rules.toml: damage_caps must be in turn order".into());
        }
        if r.max_actions_per_turn == 0 {
            return Err("rules.toml: max_actions_per_turn must be at least 1".into());
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
