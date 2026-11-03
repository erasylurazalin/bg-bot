//! The hero pick, hero powers, armor and hero damage.

use arrayvec::ArrayVec;
use crate::cards::{
    Effect,
    EventKind,
    SpellEffect,
    Tribe,
};
use crate::heroes::PowerEffect;
use crate::trinkets::{CardMod, Event};
use crate::minion::{
    CardId,
    Keywords,
};
use crate::{
    BOARD,
    CHOICES,
    DISCOVER,
    HAND,
    PLAYERS,
    SHOP,
};
use crate::heroes::BuyKind;
use super::{
    DiscoverRequest,
    Game,
    PendingDiscard,
    PendingTarget,
    PowerState,
    RefreshKind,
    TavernBuff,
    TargetKind,
};
use super::effects::buff;
use crate::minion::Minion;

impl Game {
    /// Deal the hero offers and start the pick phase (turn 0): the living
    /// players, in a shuffled order, each answer their offer with
    /// `Action::Choose`; a player who picked is done, and once everyone has,
    /// turn 1 starts (`advance`). Offers are drawn without replacement from
    /// the heroes the lobby allows, round by round so a short hero table
    /// still gives everyone a hero.
    pub(super) fn open_hero_picks(&mut self) {
        let mut deck: Vec<u8> = self.data.heroes.lobby_allowed(self.lobby).collect();
        self.rng.shuffle(&mut deck);
        let n = self.data.rules.hero_choices as usize;
        let mut at = 0;
        for round in 0..n {
            for p in 0..PLAYERS {
                if !self.players[p].alive || self.players[p].real || at >= deck.len() {
                    continue;
                }
                self.players[p].hero_offer[round] = deck[at];
                at += 1;
            }
        }
        for p in 0..PLAYERS {
            // A player the hero table could not feed picks nothing and waits
            // the pick phase out (unreachable with the heroes built so far).
            let empty = self.players[p].hero_offer.iter().all(|&h| h == u8::MAX);
            self.players[p].done = !self.players[p].alive || empty;
        }
        self.order = (0..PLAYERS as u8).filter(|&p| self.players[p as usize].alive).collect();
        self.rng.shuffle(&mut self.order);
        self.cursor = 0;
    }

    /// Every living player takes a random hero the lobby allows, no two the
    /// same. A real seat gets none and uses up no hero from the deck. Turn 0
    /// then only waits for Finley's power Discover; with none open, turn 1
    /// starts right away.
    pub(super) fn deal_random_heroes(&mut self) {
        let mut deck: Vec<u8> = self.data.heroes.lobby_allowed(self.lobby).collect();
        self.rng.shuffle(&mut deck);
        let mut at = 0;
        for p in 0..PLAYERS {
            if self.players[p].alive && !self.players[p].real && at < deck.len() {
                self.take_hero(p, deck[at]);
                at += 1;
            }
        }
        for p in 0..PLAYERS {
            self.players[p].done = !self.players[p].alive || !Self::choice_open(&self.players[p]);
        }
        self.order = (0..PLAYERS as u8).filter(|&p| self.players[p as usize].alive).collect();
        self.rng.shuffle(&mut self.order);
        match self.order.iter().position(|&p| !self.players[p as usize].done) {
            Some(c) => self.cursor = c,
            None => self.start_turn(),
        }
    }

    /// Give player `p` this hero outright, no offer: the pick phase's
    /// effects (armor, extra Health, the power, Finley's Discover when
    /// `discover`, which the pick phase runs by itself). For tests (the
    /// sandbox's --hero goes through the pick at turn 0).
    ///
    /// A hero given this way while a later turn is already being played is
    /// the hero the game started with, so that turn's power work runs now:
    /// Faelin's Discovers open on turn 1, Clairvoyance's free Refresh lands.
    pub fn assign_hero(&mut self, p: usize, hero: u8) {
        self.take_hero(p, hero);
        if self.turn > 0 {
            self.power_start_of_turn(p);
        }
    }

    /// The player takes the hero: health, armor, power state, and Finley's
    /// power Discover if the hero's power is one.
    fn take_hero(&mut self, p: usize, hero: u8) {
        let h = self.data.heroes.get(hero);
        let (extra_health, armor, starting, excludes) =
            (h.extra_health, h.armor, h.power, h.power_excludes.clone());
        let effect = self.data.heroes.power(starting).effect;
        self.players[p].health += extra_health;
        self.players[p].armor = armor as i32;
        self.players[p].hero = Some(hero);
        self.players[p].power2 = None;
        self.players[p].storm_power = [false; 2];
        self.players[p].conviction = (0, 0, 0);
        let power = self.data.heroes.power(starting).clone();
        self.players[p].power = Some(PowerState::fresh(starting, &power));
        self.refresh_power_unlock(p);
        self.refresh_skip(p);
        if effect == PowerEffect::DiscoverPower {
            let excludes = self.data.heroes.hero_indices(&excludes);
            self.discover_hero_power(p, &excludes, false, 1);
        } else {
            // A power that is a rotating family (A Tale of Kings) starts on a
            // random tribe of the lobby, before turn 1.
            self.rotate_family_power(p, false);
        }
        // What a power does at the start of the game: For the Horde!'s Tavern
        // buff, Choose Your Champion's Tier 7 Discover, starting minions
        // (Menagerist, Avatar of N'Zoth, Pilot the Shredder), Conviction's
        // starting stats, and Power of the Storm's slot flag.
        for second in [false, true] {
            let Some(st) = self.players[p].power_at(second) else { continue };
            match self.data.heroes.power(st.of).effect {
                PowerEffect::TavernBuffAfterBuys { attack, health, .. } => {
                    self.players[p].add_tavern_buff(
                        &self.data.cards,
                        TavernBuff { tribe: None, attack, health, this_turn: false, typed_only: false },
                    );
                }
                PowerEffect::Champion { gold } => {
                    let gold = gold.min(u16::MAX as u32) as u16;
                    let _ = self.players[p]
                        .discover_queue
                        .try_push(DiscoverRequest::Champion(gold));
                    self.maybe_start_discover(p);
                }
                PowerEffect::StartMinion { card } => {
                    if !self.players[p].board.is_full() {
                        let mut m = self.players[p].make_card(&self.data.cards, card, false, 0);
                        m.uses_left = m.turn_uses(&self.data.cards);
                        self.players[p].board.push(m);
                        self.check_triple(p, card);
                    }
                }
                PowerEffect::Conviction => {
                    self.players[p].conviction = (1, 1, 1);
                }
                PowerEffect::PowerOfTheStorm => {
                    self.players[p].storm_power[second as usize] = true;
                }
                PowerEffect::FreeFirstMinion => {
                    self.players[p].free_minion_left = self.players[p].free_minion_left.max(1);
                }
                _ => {}
            }
        }
    }

    /// Set each held power's unlock flag from the player's tier and the turn
    /// ("Unlocks at Tier N", "Unlocks on Turn N").
    pub(super) fn refresh_power_unlock(&mut self, p: usize) {
        let (tier, turn) = (self.players[p].tier, self.turn);
        for second in [false, true] {
            let Some(st) = self.players[p].power_at(second) else { continue };
            let power = self.data.heroes.power(st.of);
            let unlocked = tier >= power.unlock_tier && turn >= power.unlock_turn;
            if let Some(st) = self.players[p].power_at_mut(second) {
                st.unlocked = unlocked;
            }
        }
    }

    /// Put this power in the given slot, its state fresh. `None` clears it.
    pub(super) fn set_power(&mut self, p: usize, second: bool, of: Option<u8>) {
        let state = of.map(|of| {
            let power = self.data.heroes.power(of).clone();
            PowerState::fresh(of, &power)
        });
        if second {
            self.players[p].power2 = state;
        } else {
            self.players[p].power = state;
        }
        if let Some(of) = of {
            match self.data.heroes.power(of).effect {
                PowerEffect::StartMinion { card } if self.turn == 0 => {
                    if !self.players[p].board.is_full() {
                        let mut m = self.players[p].make_card(&self.data.cards, card, false, 0);
                        m.uses_left = m.turn_uses(&self.data.cards);
                        self.players[p].board.push(m);
                        self.check_triple(p, card);
                    }
                }
                PowerEffect::Conviction => {
                    if self.players[p].conviction == (0, 0, 0) {
                        self.players[p].conviction = (1, 1, 1);
                    }
                }
                PowerEffect::PowerOfTheStorm => {
                    self.players[p].storm_power[second as usize] = true;
                }
                PowerEffect::FreeFirstMinion => {
                    let runs = self.players[p].tp.power_triggers.max(1);
                    self.players[p].free_minion_left =
                        self.players[p].free_minion_left.max(runs);
                }
                _ => {}
            }
        }
        self.refresh_power_unlock(p);
        self.refresh_skip(p);
        self.sync_power_flags(p);
    }

    /// Whether this power is one of the player's own (the Discover of a Hero
    /// Power never offers one the player already holds).
    fn holds_power(&self, p: usize, of: u8) -> bool {
        self.players[p].power.is_some_and(|st| st.of == of)
            || self.players[p].power2.is_some_and(|st| st.of == of)
    }

    /// A rotating power (A Tale of Kings) becomes a random other power of its
    /// family whose tribe is in the lobby, never the same tribe twice
    /// (docs/rules/heroes.md "King of Beasts, Rat King").
    pub(super) fn rotate_family_power(&mut self, p: usize, second: bool) {
        let Some(st) = self.players[p].power_at(second) else { return };
        let power = self.data.heroes.power(st.of);
        if power.family == 0 {
            return;
        }
        let options =
            self.data.heroes.family_options(power.family, self.lobby, power.tribe_of());
        if options.is_empty() {
            return;
        }
        let k = self.rng.below(options.len() as u32) as usize;
        self.set_power(p, second, Some(options[k]));
    }

    /// What the player's power button costs right now: its card cost plus the
    /// running change every press leaves behind (Lead Explorer's +1) or a
    /// buy takes off (Pirate Parrrrty!) or the end of a turn takes off (The
    /// Perfect Crime, The Galaxy's Lens), never below 0.
    pub fn power_cost(&self, p: usize, second: bool) -> u8 {
        let Some(st) = self.players[p].power_at(second) else { return 0 };
        let base = self.data.heroes.power(st.of).cost as i32;
        (base + st.cost_delta as i32).max(0) as u8
    }

    /// The Tavern prices a power sets (Manastorm): what a minion and a
    /// Refresh cost, and what upgrading costs on top of the tier's price.
    pub(super) fn power_prices(&self, p: usize) -> Option<(u8, u8, u8)> {
        for second in [false, true] {
            if let Some(st) = self.players[p].power_at(second) {
                match self.data.heroes.power(st.of).effect {
                    PowerEffect::Prices { minion, refresh, upgrade_extra } => {
                        return Some((minion, refresh, upgrade_extra));
                    }
                    // Stay Frosty: minions cost this; a Refresh and upgrading
                    // keep the rules' prices.
                    PowerEffect::StayFrosty { price } => {
                        return Some((price, self.data.rules.roll_cost, 0));
                    }
                    _ => {}
                }
            }
        }
        None
    }

    /// Read the standing rules the hero powers set into the player's fields
    /// (hero batch 2: Stay Frosty's Tavern shape, Dream Portal's Dragon, Hat
    /// Trick, Enhancification, Twice as Nice, Double Time, Come One, Come
    /// All!). Called whenever a power arrives and at the start of every turn,
    /// so a shop path reads one field and never the power table.
    pub(super) fn sync_power_flags(&mut self, p: usize) {
        let (main, second) = (self.players[p].power, self.players[p].power2);
        let mut shop_fewer = 0u8;
        let mut freeze_end = false;
        let mut extra_dragon = false;
        let mut double_time: Option<CardId> = None;
        let mut hat = (0, 0);
        let mut enhance_times = 0u8;
        let mut twice_nice = false;
        let mut ticket_chance = 0u8;
        for st in [main, second].into_iter().flatten() {
            match self.data.heroes.power(st.of).effect {
                PowerEffect::StayFrosty { .. } => {
                    shop_fewer = 1;
                    freeze_end = true;
                }
                PowerEffect::ExtraDragon => extra_dragon = true,
                PowerEffect::DoubleTime { coin } => double_time = Some(coin),
                PowerEffect::HatTrick { attack, health } => hat = (attack, health),
                PowerEffect::Enhancification { times } => enhance_times = times,
                PowerEffect::TwiceAsNice => twice_nice = true,
                PowerEffect::DarkmoonTickets { percent, .. } => ticket_chance = percent,
                _ => {}
            }
        }
        let pl = &mut self.players[p];
        pl.shop_fewer = shop_fewer;
        pl.freeze_end = freeze_end;
        pl.extra_dragon = extra_dragon;
        pl.double_time = double_time;
        pl.hat = hat;
        pl.enhance_times = enhance_times;
        pl.twice_nice = twice_nice;
        pl.ticket_chance = ticket_chance;
    }

    /// How many Darkmoon Tickets a Discover takes (Come One, Come All!), 3
    /// unless the power says otherwise.
    fn ticket_goal(&self, p: usize) -> u8 {
        for second in [false, true] {
            if let Some(st) = self.players[p].power_at(second)
                && let PowerEffect::DarkmoonTickets { count, .. } =
                    self.data.heroes.power(st.of).effect
            {
                return count.max(1);
            }
        }
        3
    }

    /// Whether the Tavern holds any card at all, the spell slot included.
    fn shop_has_card(&self, p: usize) -> bool {
        !self.players[p].shop.is_empty() || self.players[p].shop_spell.is_some()
    }

    /// Minions on the board and in the Tavern together.
    fn any_minions(&self, p: usize) -> usize {
        let pl = &self.players[p];
        pl.board.len()
            + pl.shop.iter().filter(|m| !self.data.cards.get(m.card).is_spell()).count()
    }

    /// Whether the player has a warband they last fought to copy (Bob's
    /// Burgles).
    fn burgle_has_board(&self, p: usize) -> bool {
        let pl = &self.players[p];
        let q = match pl.last_opponent {
            Some(super::Opponent::Player(q)) | Some(super::Opponent::Ghost(q)) => q as usize,
            None => return false,
        };
        pl.seen.get(q).is_some_and(|s| !s.board.is_empty())
    }

    /// Bob's Burgles: Refresh the Tavern with plain copies of the warband
    /// this player last fought, made fresh (no pool copy, printed stats).
    /// The Tavern's spell slot rolls as usual.
    fn burgle_last_opponent(&mut self, p: usize) {
        let Some(opp) = self.players[p].last_opponent else { return };
        let q = match opp {
            super::Opponent::Player(q) | super::Opponent::Ghost(q) => q as usize,
        };
        let mut board: ArrayVec<CardId, BOARD> =
            self.players[p].seen[q].board.iter().map(|m| m.card).collect();
        if board.is_empty() {
            return;
        }
        self.rng.shuffle(&mut board);
        let Game { data, rng, players, pool, lobby, .. } = self;
        let pl = &mut players[p];
        pl.frozen = false;
        for m in pl.shop.drain(..) {
            super::shop::give_back(pool, &data.cards, &m);
        }
        pl.shop_spell = None;
        let size = pl.shop_size(&data.rules);
        let mut next = 0;
        while pl.shop.len() < size {
            let card = if next < board.len() {
                next += 1;
                board[next - 1]
            } else {
                board[rng.below(board.len() as u32) as usize]
            };
            let mut m = pl.make_card(&data.cards, card, false, 0);
            pl.apply_tavern_buffs(&data.cards, &mut m);
            pl.shop.push(m);
        }
        pl.shop_spell = data.cards.random_spell(pl.tier, *lobby, rng);
        Self::apply_refresh_buff(pl, &data.cards, rng);
        Self::apply_barrage(pl, &data.cards);
        let bonus = data.rules.bonus_mask;
        Self::after_refresh_powers(pl, &data.cards, rng, bonus, size);
    }

    /// Take one Darkmoon Ticket from a Tavern card that carries one. At the
    /// goal, the player gets a Triple Reward spell that Discovers a minion of
    /// their own Tier (a Triple Reward is the Tier above, docs/rules/heroes.md).
    pub(super) fn take_ticket(&mut self, p: usize) {
        let goal = self.ticket_goal(p);
        let pl = &mut self.players[p];
        pl.tickets = pl.tickets.saturating_add(1);
        if pl.tickets < goal {
            return;
        }
        pl.tickets = 0;
        if pl.hand.is_full() {
            return;
        }
        let id = self
            .data
            .cards
            .triple_reward
            .expect("the card data has no Triple Reward spell");
        let tier = pl.tier.max(1);
        let mut m = pl.make_card(&self.data.cards, id, false, 0);
        m.improvements = tier as u16;
        self.add_to_hand(p, m);
    }

    /// What a Refresh costs this player: the rules' price, or a power's own.
    pub fn refresh_cost(&self, p: usize) -> u8 {
        match self.power_prices(p) {
            Some((_, refresh, _)) => refresh,
            None => self.data.rules.roll_cost,
        }
    }

    /// What upgrading the Tavern costs this player: the tier's price as it
    /// stands (the turns and Avalanche take from it) plus a power's extra.
    pub fn upgrade_price(&self, p: usize) -> u8 {
        let extra = self.power_prices(p).map_or(0, |(_, _, e)| e);
        self.players[p].upgrade_cost.saturating_add(extra)
    }

    /// What a Tavern spell costs this player before its discount: 0 when a
    /// power makes this one free (Reliquary Research's every third spell).
    /// A spell paid with Health still costs Health: the free one is free of
    /// Gold.
    pub(super) fn spell_price(&self, p: usize, card: CardId) -> u8 {
        let every_third =
            self.power_slot(p, |e| matches!(e, PowerEffect::FreeThirdSpell)).is_some();
        if every_third && (self.players[p].counts.tavern_spells_bought + 1).is_multiple_of(3) {
            return 0;
        }
        // Reserve Prices: Tavern spells cost less this turn.
        self.data
            .cards
            .get(card)
            .cost
            .saturating_sub(self.players[p].prize.spell_discount_turn)
    }

    /// Whether the player's MAIN power button may be pressed right now.
    pub fn power_usable(&self, p: usize) -> bool {
        self.power_usable_at(p, false)
    }

    /// Whether the player's SECOND power button may be pressed right now.
    pub fn power_usable2(&self, p: usize) -> bool {
        self.power_usable_at(p, true)
    }

    /// Whether one of the player's two power buttons may be pressed right
    /// now: a button, unlocked, uses left, not past its once-per-game, the
    /// Gold for it, and whatever its effect needs to land on something.
    pub fn power_usable_at(&self, p: usize, second: bool) -> bool {
        let Some(st) = self.players[p].power_at(second) else { return false };
        let power = self.data.heroes.power(st.of);
        let pl = &self.players[p];
        if power.passive
            // A power that became a trinket (Marin, Buttons) has no button.
            || pl.trinkets[super::trinkets::POWER_SLOT].of != 0
            || !st.unlocked
            || st.uses_left == 0
            || (power.uses_per_game > 0 && st.used_game >= power.uses_per_game as u32)
            || pl.gold < self.power_cost(p, second)
        {
            return false;
        }
        let hand_room = !pl.hand.is_full();
        let shop_minion = self.shop_has_minion(p);
        match power.effect {
            PowerEffect::GiveDivineShield | PowerEffect::MakeGolden => {
                !pl.board.is_empty() && pl.board.iter().any(|m| !m.golden)
            }
            PowerEffect::StealCardDamage { .. } => {
                hand_room && (shop_minion || pl.shop_spell.is_some())
            }
            PowerEffect::StealRandomDoubleHealth => hand_room && shop_minion,
            PowerEffect::GetRandomSpell => hand_room,
            // A Battlecry to trigger (Snicker-snack); the minion itself may
            // be the target, so no slot is excluded.
            PowerEffect::TriggerBattlecry => self.has_battlecry_target(p, None),
            // A card to throw away (Incubate): with none the press would do
            // nothing.
            PowerEffect::DiscardGetRandomMinion { .. } => self.has_discardable(p),
            // Both minions must fit in hand to stay a linked pair (Dark
            // Ritual).
            PowerEffect::GetLinkedMinions(n) => pl.hand.len() + n as usize <= HAND,
            // A minion to buff, to remove, or to throw in the pot.
            PowerEffect::BuffPerBuys { .. } | PowerEffect::DiscoverBelow => !pl.board.is_empty(),
            // A typed minion of the board or the Tavern to throw.
            PowerEffect::StirThePot { .. } => self.has_throwable(p),
            // A non-Golden minion of the board and a minion in the Tavern.
            PowerEffect::SwapWithShop => {
                shop_minion && pl.board.iter().any(|m| !m.golden)
            }
            // Two plain copies to make a pair, and room for the third.
            PowerEffect::ThreeWishes => hand_room && !self.three_wishes_pairs(p).is_empty(),
            // A Tavern spell to copy.
            PowerEffect::CopyLastSpell => {
                hand_room
                    && pl.last_spell.is_some_and(|c| self.data.cards.get(c).tavern_spell)
            }
            // The fourth dig hands out a Golden minion.
            PowerEffect::DigForGolden { .. } => hand_room,
            // A Tavern minion to replace, whether or not one has a Tier
            // above it: Galakrond's Greed was pressed on a Tavern of Tier 6
            // minions and did nothing but spend the press
            // (Hearthstone_2026_11_01_04_27_54-g2 turn 8).
            PowerEffect::ReplaceShopHigher => shop_minion,
            // Any minion, the Tavern's included.
            PowerEffect::TempReborn => shop_minion || !pl.board.is_empty(),
            PowerEffect::RefreshHighTier { .. } => true,
            // A combat Secret still missing from the zone (Prestidigitation).
            PowerEffect::DiscoverSecret => !pl.secrets.all_combat(),
            PowerEffect::SaturdayCthun { .. } => !pl.board.is_empty(),
            // A warband to copy, and a Tavern to put it in.
            PowerEffect::BurgleLastOpponent => self.burgle_has_board(p),
            // A card on the board or in the Tavern (the spell slot included)
            // to replace.
            PowerEffect::ReplaceSameTier => self.shop_has_card(p) || !pl.board.is_empty(),
            // A Tavern card to take, and room in hand for it.
            PowerEffect::Imprison { .. } => hand_room && self.shop_has_card(p),
            PowerEffect::StealShopAsStats { .. } => hand_room && shop_minion,
            // Two minions, one on the board or in the Tavern and another one
            // different from it.
            PowerEffect::SpiritSwap => {
                self.any_minions(p) >= 2
            }
            // A friendly minion to sell and another to take its stats.
            PowerEffect::Devour => pl.board.len() >= 2,
            // The next opponent's warband as it stood at the start of the
            // turn (docs/rules/heroes.md "Guessing").
            PowerEffect::IspyNextOpponent => hand_room && !pl.next_board.is_empty(),
            // A friendly Undead to destroy and room for the random one.
            PowerEffect::DestroyTribeGetRandom { tribe } => {
                hand_room
                    && pl.board.iter().any(|m| self.data.cards.tribes_of(m).has(tribe))
            }
            // A minion to buff, Attack or Health equal to the Tier.
            PowerEffect::Hymn { .. } => shop_minion || !pl.board.is_empty(),
            // An exact copy needs a space on the board.
            PowerEffect::CloningGallery => !pl.board.is_full(),
            // Conviction and Rapid Reanimation need a friendly minion; Lock
            // and Load needs a Tavern minion.
            PowerEffect::Conviction | PowerEffect::RapidReanimation => !pl.board.is_empty(),
            PowerEffect::LockAndLoad => shop_minion,
            PowerEffect::TakeFirstKilled
            | PowerEffect::EmbraceYourRage
            | PowerEffect::FriendlyWager { .. }
            | PowerEffect::DetectiveForHire { .. } => true,
            // A passive: no button.
            PowerEffect::StayFrosty { .. }
            | PowerEffect::HatTrick { .. }
            | PowerEffect::ExtraDragon
            | PowerEffect::TavernLighting { .. }
            | PowerEffect::DarkmoonTickets { .. }
            | PowerEffect::TwiceAsNice
            | PowerEffect::Enhancification { .. }
            | PowerEffect::DoubleTime { .. }
            | PowerEffect::Wingmen
            | PowerEffect::WaxWarband
            | PowerEffect::StartMinion { .. }
            | PowerEffect::DamageMilestone { .. }
            | PowerEffect::RandomMinionsEachTurn { .. }
            | PowerEffect::AllWillBurn
            | PowerEffect::AttackMilestone { .. }
            | PowerEffect::FreeFirstMinion
            | PowerEffect::SproutItOut
            | PowerEffect::GloryOfCombat
            | PowerEffect::PowerOfTheStorm
            | PowerEffect::Stance { .. }
            | PowerEffect::FragrantPhylactery
            | PowerEffect::CopyHighestWhenSpace { .. }
            | PowerEffect::MechGyver { .. }
            | PowerEffect::Broodmother { .. }
            | PowerEffect::Tentacular { .. } => false,
            // A press that only completes a chooser's pick needs nothing.
            _ => true,
        }
    }

    /// One of the hero power buttons is pressed: pay, spend a use, run the
    /// effect. "Your Hero Power triggers twice" (Ancient Wishbone) runs the
    /// effect twice for the one press and the one payment.
    pub(super) fn press_power(&mut self, p: usize, second: bool) {
        let (of, cost) = {
            let st = self.players[p].power_at(second).unwrap();
            (st.of, self.power_cost(p, second))
        };
        let effect = self.data.heroes.power(of).effect;
        let st = self.players[p].power_at_mut(second).unwrap();
        st.uses_left = st.uses_left.saturating_sub(1);
        st.used_game += 1;
        self.players[p].gold -= cost;
        let runs = self.players[p].tp.power_triggers.max(1);
        match effect {
            PowerEffect::ExtraMaxGold(n) => {
                let n = n.saturating_mul(runs);
                self.players[p].extra_max_gold = self.players[p].extra_max_gold.saturating_add(n);
            }
            PowerEffect::GiveDivineShield => {
                let pt = PendingTarget {
                    by_minion: false,
                    source: None,
                    kind: TargetKind::MinionBuff {
                        tribe: None,
                        attack: 0,
                        health: 0,
                        keyword: Keywords::DIVINE_SHIELD,
                        random_keyword: false,
                        shop: false,
                    },
                    runs,
                    tavern: false,
                    gem: false,
                };
                self.offer_target(p, pt);
            }
            PowerEffect::StealCardDamage(damage) => {
                let pt = PendingTarget {
                    by_minion: false,
                    source: None,
                    kind: TargetKind::StealShopCard { damage },
                    runs,
                    tavern: false,
                    gem: false,
                };
                self.offer_target(p, pt);
            }
            PowerEffect::StealRandomDoubleHealth => {
                for _ in 0..runs {
                    if !self.steal_random_shop_double_health(p) {
                        break;
                    }
                }
            }
            PowerEffect::MakeGolden => {
                let pt = PendingTarget {
                    by_minion: false,
                    source: None,
                    kind: TargetKind::MakeGolden { tribe: None, max_tier: crate::CARD_MAX_TIER },
                    runs,
                    tavern: false,
                    gem: false,
                };
                self.offer_target(p, pt);
            }
            PowerEffect::GetGems(n) => self.give_gems(p, n.saturating_mul(runs)),
            PowerEffect::DiscoverTribe(t) => {
                for _ in 0..runs {
                    let _ = self.players[p].discover_queue.try_push(DiscoverRequest::TribeMinion(t));
                }
                self.maybe_start_discover(p);
            }
            PowerEffect::SpellDiscount(n) => {
                let n = n.saturating_mul(runs);
                let pl = &mut self.players[p];
                pl.spell_discount = pl.spell_discount.saturating_add(n);
            }
            PowerEffect::GetRandomSpell => {
                let tier = self.players[p].tier;
                for _ in 0..runs {
                    if let Some(card) = self.data.cards.random_spell(tier, self.lobby, &mut self.rng) {
                        self.give_generated(p, card);
                    }
                }
            }
            PowerEffect::TriggerBattlecry => {
                let pt = PendingTarget {
                    by_minion: false,
                    source: None,
                    kind: TargetKind::TriggerBattlecry,
                    runs,
                    tavern: true,
                    gem: false,
                };
                self.offer_target(p, pt);
            }
            PowerEffect::DiscardGetRandomMinion { tribe: t } => {
                // The reward runs from `resolve_discard` once a card is gone,
                // the same path an Activate with `discard = true` takes.
                self.players[p].pending_discard = Some(PendingDiscard {
                    runs: 1,
                    source: None,
                    reward: Effect::GetRandomMinion { tribe: t },
                    golden: false,
                });
            }
            PowerEffect::GetLinkedMinions(n) => {
                let tier = self.players[p].tier;
                let n = n.saturating_mul(runs);
                let mut got: ArrayVec<CardId, 4> = ArrayVec::new();
                for _ in 0..n {
                    let drawn = self.data.cards.random_any_minion(tier, self.lobby, &mut self.rng);
                    if let Some(card) = drawn {
                        let _ = got.try_push(card);
                    }
                }
                self.give_linked(p, &got, false);
            }
            PowerEffect::RefreshWithSpells => {
                for _ in 0..runs {
                    let Game { data, rng, players, pool, lobby, .. } = self;
                    Self::refresh_with_spells(
                        &mut players[p], &data.cards, &data.rules, rng, pool, *lobby,
                    );
                }
            }
            // King of Duality: two Hero Power Discovers, the first into the
            // second slot and the second into the main one.
            PowerEffect::DiscoverPowers(n) => {
                self.players[p].power_offer_left = n;
                // The first pick is the second power (docs/rules/heroes.md).
                self.players[p].power_offer_second = !second;
                self.deal_power_offer(p);
            }
            // Embrace the Elements, Dungar's Gryphon: a fixed choice of
            // powers, never the one that just completed.
            PowerEffect::ChooseSubPowers => {
                let last = self.players[p].power_at(second).map_or(0, |st| st.last_pick);
                let mut picks: ArrayVec<u8, CHOICES> = ArrayVec::new();
                for &o in &self.data.heroes.power(of).options {
                    if o as u16 + 1 != last as u16 {
                        let _ = picks.try_push(o);
                    }
                }
                let pl = &mut self.players[p];
                pl.power_offer = picks;
                pl.power_offer_second = second;
            }
            // Sharpen Blades: the buff grows by 1 for every `every` cards
            // bought this game (docs/rules/heroes.md).
            PowerEffect::BuffPerBuys { attack, health, every } => {
                let grown = (self.players[p].counts.cards_bought / every.max(1) as u32) as i32;
                let pt = PendingTarget {
                    by_minion: false,
                    source: None,
                    kind: TargetKind::MinionBuff {
                        tribe: None,
                        attack: attack + grown,
                        health: health + grown,
                        keyword: 0,
                        random_keyword: false,
                        shop: false,
                    },
                    runs,
                    tavern: false,
                    gem: false,
                };
                self.offer_target(p, pt);
            }
            PowerEffect::DiscoverMagnetic => {
                for _ in 0..runs {
                    let _ = self.players[p].discover_queue.try_push(DiscoverRequest::MagneticMinion);
                }
                self.maybe_start_discover(p);
            }
            // Pirate Parrrrty!: a random Pirate, and the price goes back to
            // the card's own: the discount is for the next press only.
            PowerEffect::GetTribeCutCost { tribe, .. } => {
                let mut pos = 0;
                for _ in 0..runs {
                    self.run_effect(p, &mut pos, Effect::GetRandomMinion { tribe }, false, 1);
                }
                if let Some(st) = self.players[p].power_at_mut(second) {
                    st.cost_delta = 0;
                }
            }
            // Lead Explorer: a minion of the player's own Tier, and every
            // press costs more from here on.
            PowerEffect::DiscoverMyTierCostsMore { more } => {
                let tier = self.players[p].tier;
                for _ in 0..runs {
                    let _ = self.players[p].discover_queue.try_push(DiscoverRequest::MinionTier(tier));
                }
                self.maybe_start_discover(p);
                if let Some(st) = self.players[p].power_at_mut(second) {
                    st.cost_delta = st.cost_delta.saturating_add((more as i16 * runs as i16) as i8);
                }
            }
            // Buried Treasure: one dig a press; the fourth is a Golden minion
            // up to the player's Tier and the digging starts over.
            PowerEffect::DigForGolden { digs } => {
                let mut found = 0;
                for _ in 0..runs {
                    let Some(st) = self.players[p].power_at_mut(second) else { break };
                    st.progress += 1;
                    if st.progress >= digs as u32 {
                        st.progress = 0;
                        found += 1;
                    }
                }
                let tier = self.players[p].tier;
                for _ in 0..found {
                    self.give_golden_minion(p, 1, tier, false, false);
                }
            }
            // Trash for Treasure: remove a friendly minion, then Discover one
            // from the Tier below it.
            PowerEffect::DiscoverBelow => {
                let pt = PendingTarget {
                    by_minion: false,
                    source: None,
                    kind: TargetKind::RemoveTierBelow,
                    runs: 1,
                    tavern: false,
                    gem: false,
                };
                self.offer_target(p, pt);
            }
            // Piggy Bank: the Gold it holds, once a game.
            PowerEffect::GrowingGold { base } => {
                let waited = self.players[p].power_at(second).map_or(0, |st| st.progress);
                let gold = base.saturating_add(waited.min(u8::MAX as u32) as u8);
                self.players[p].gold = self.players[p].gold.saturating_add(gold);
            }
            PowerEffect::SwapWithShop => {
                let pt = PendingTarget {
                    by_minion: false,
                    source: None,
                    kind: TargetKind::SwapWithShop,
                    runs: 1,
                    tavern: false,
                    gem: false,
                };
                self.offer_target(p, pt);
            }
            // Three Wishes: the third copy of a pair, from the pool.
            PowerEffect::ThreeWishes => {
                let pairs = self.three_wishes_pairs(p);
                let pick = if pairs.is_empty() {
                    None
                } else {
                    Some(pairs[self.rng.below(pairs.len() as u32) as usize])
                };
                if let Some(card) = pick {
                    let slot = self.data.cards.get(card).pool_slot as usize;
                    if self.pool[slot] > 0 && !self.players[p].hand.is_full() {
                        self.pool[slot] -= 1;
                        let m = self.players[p].make_card(&self.data.cards, card, false, 1);
                        self.add_to_hand(p, m);
                        self.check_triple(p, card);
                    }
                }
            }
            // Stir the Pot: one throw a press.
            PowerEffect::StirThePot { count } => {
                let pt = PendingTarget {
                    by_minion: false,
                    source: None,
                    kind: TargetKind::ThrowInPot { count },
                    runs: 1,
                    tavern: false,
                    gem: false,
                };
                self.offer_target(p, pt);
            }
            PowerEffect::StealAllTavern => {
                for _ in 0..runs {
                    self.steal_all_tavern(p);
                }
            }
            // Reclaimed Souls: up to three plain copies of minions that died
            // in the last fight, either side.
            PowerEffect::DiscoverDeadMinion => {
                for _ in 0..runs {
                    let _ = self.players[p].discover_queue.try_push(DiscoverRequest::DeadLastFight);
                }
                self.maybe_start_discover(p);
            }
            // The Galaxy's Lens: a fresh copy of the last Tavern spell cast.
            PowerEffect::CopyLastSpell => {
                if let Some(card) = self.players[p].last_spell
                    && self.data.cards.get(card).tavern_spell
                {
                    for _ in 0..runs {
                        self.give_generated(p, card);
                    }
                }
            }
            // ---- hero batch 2 (2026-11-01) ----
            // Galakrond's Greed: a Tavern minion, then a Discover one Tier
            // higher takes its slot.
            PowerEffect::ReplaceShopHigher => {
                let pt = PendingTarget {
                    by_minion: false,
                    source: None,
                    kind: TargetKind::ReplaceHigher,
                    runs: 1,
                    tavern: false,
                    gem: false,
                };
                self.offer_target(p, pt);
            }
            // Reborn Rites: any minion, the Tavern's included.
            PowerEffect::TempReborn => {
                let pt = PendingTarget {
                    by_minion: false,
                    source: None,
                    kind: TargetKind::TempReborn,
                    runs,
                    tavern: false,
                    gem: false,
                };
                self.offer_target(p, pt);
            }
            // Temporal Tavern: a Refresh that also holds minions from a Tier
            // above the player's. The two higher-Tier cards go in before the
            // powers that watch a Refresh (Twice as Nice's copy, coming last,
            // may then be one of them).
            PowerEffect::RefreshHighTier { count } => {
                for _ in 0..runs {
                    let Game { data, rng, players, pool, lobby, .. } = self;
                    let pl = &mut players[p];
                    pl.frozen = false;
                    for m in pl.shop.drain(..) {
                        super::shop::give_back(pool, &data.cards, &m);
                    }
                    pl.shop_spell = None;
                    let size = pl.shop_size(&data.rules);
                    Self::fill_shop(
                        pl,
                        pool,
                        &data.cards,
                        &data.rules,
                        size,
                        rng,
                        *lobby,
                        RefreshKind::Explicit,
                    );
                    Self::add_high_tier_slots(pl, pool, &data.cards, rng, *lobby, count);
                    Self::apply_refresh_buff(pl, &data.cards, rng);
                    Self::apply_barrage(pl, &data.cards);
                    let bonus = data.rules.bonus_mask;
                    Self::after_refresh_powers(pl, &data.cards, rng, bonus, size);
                }
            }
            // Saturday C'Thuns!: pay, and the end of the turn gives the buff,
            // once more for every turn the power has lasted.
            PowerEffect::SaturdayCthun { .. } => {
                if let Some(st) = self.players[p].power_at_mut(second) {
                    st.armed = true;
                }
            }
            // Bob's Burgles: plain copies of the warband last fought.
            PowerEffect::BurgleLastOpponent => {
                for _ in 0..runs {
                    self.burgle_last_opponent(p);
                }
            }
            // Arcane Alteration: a card on the board or in the Tavern becomes a
            // random one of the same Tier.
            PowerEffect::ReplaceSameTier => {
                let pt = PendingTarget {
                    by_minion: false,
                    source: None,
                    kind: TargetKind::ReplaceSameTier,
                    runs: 1,
                    tavern: false,
                    gem: false,
                };
                self.offer_target(p, pt);
            }
            // Imprison: a Tavern card to hand, locked for its turns.
            PowerEffect::Imprison { turns } => {
                let pt = PendingTarget {
                    by_minion: false,
                    source: None,
                    kind: TargetKind::ImprisonShop { turns },
                    runs: 1,
                    tavern: false,
                    gem: false,
                };
                self.offer_target(p, pt);
            }
            // See the Light: a Tavern minion to hand as a 2/2.
            PowerEffect::StealShopAsStats { attack, health } => {
                let pt = PendingTarget {
                    by_minion: false,
                    source: None,
                    kind: TargetKind::StealShopAsStats { attack, health },
                    runs: 1,
                    tavern: false,
                    gem: false,
                };
                self.offer_target(p, pt);
            }
            // Spirit Swap and Devour: the first of two minions.
            PowerEffect::SpiritSwap => {
                let pt = PendingTarget {
                    by_minion: false,
                    source: None,
                    kind: TargetKind::SpiritSwapFirst,
                    runs: 1,
                    tavern: false,
                    gem: false,
                };
                self.offer_target(p, pt);
            }
            PowerEffect::Devour => {
                let pt = PendingTarget {
                    by_minion: false,
                    source: None,
                    kind: TargetKind::DevourFirst,
                    runs: 1,
                    tavern: false,
                    gem: false,
                };
                self.offer_target(p, pt);
            }
            // I Spy: plain copies of the next opponent's warband.
            PowerEffect::IspyNextOpponent => {
                for _ in 0..runs {
                    let _ = self.players[p]
                        .discover_queue
                        .try_push(DiscoverRequest::IspyNextOpponent);
                }
                self.maybe_start_discover(p);
            }
            // Rune of Damnation: a friendly Undead dies for a random one.
            PowerEffect::DestroyTribeGetRandom { tribe } => {
                let pt = PendingTarget {
                    by_minion: false,
                    source: None,
                    kind: TargetKind::DestroyGetTribe { tribe },
                    runs: 1,
                    tavern: false,
                    gem: false,
                };
                self.offer_target(p, pt);
            }
            // Major and Minor Hymn: Attack (or Health) equal to the Tier now.
            PowerEffect::Hymn { attack } => {
                let tier = self.players[p].tier as i32;
                let (a, h) = if attack { (tier, 0) } else { (0, tier) };
                let pt = PendingTarget {
                    by_minion: false,
                    source: None,
                    kind: TargetKind::MinionBuff {
                        tribe: None,
                        attack: a,
                        health: h,
                        keyword: 0,
                        random_keyword: false,
                        shop: false,
                    },
                    runs,
                    tavern: false,
                    gem: false,
                };
                self.offer_target(p, pt);
            }
            // Cloning Gallery: an exact copy, summoned.
            PowerEffect::CloningGallery => {
                let pt = PendingTarget {
                    by_minion: false,
                    source: None,
                    kind: TargetKind::SummonExactCopy,
                    runs: 1,
                    tavern: false,
                    gem: false,
                };
                self.offer_target(p, pt);
            }
            // Lucky Roll: a fair six-sided die, and the power is locked for
            // that many turns.
            PowerEffect::LuckyRoll => {
                for _ in 0..runs {
                    let roll = self.rng.below(6) as u8 + 1;
                    self.players[p].gold = self.players[p].gold.saturating_add(roll);
                    let Some(st) = self.players[p].power_at_mut(second) else { break };
                    st.progress = st.progress.saturating_add(roll as u32);
                    st.uses_left = 0;
                }
            }
            // ---- hero batch 3 (2026-11-01) ----
            PowerEffect::TakeFirstKilled | PowerEffect::EmbraceYourRage => {
                if let Some(st) = self.players[p].power_at_mut(second) {
                    st.armed = true;
                }
            }
            PowerEffect::Conviction => {
                let (count, attack, health) = self.players[p].conviction;
                let count = count.max(1) as usize;
                for _ in 0..runs {
                    let n = self.players[p].board.len();
                    if n == 0 {
                        break;
                    }
                    let mut indices: ArrayVec<usize, BOARD> = (0..n).collect();
                    self.rng.shuffle(&mut indices);
                    for &i in indices.iter().take(count) {
                        buff(&mut self.players[p].board[i], attack, health);
                    }
                }
            }
            PowerEffect::LockAndLoad => {
                let pt = PendingTarget {
                    by_minion: false,
                    source: None,
                    kind: TargetKind::LockAndLoad,
                    runs: 1,
                    tavern: false,
                    gem: false,
                };
                self.offer_target(p, pt);
            }
            PowerEffect::RapidReanimation => {
                let pt = PendingTarget {
                    by_minion: false,
                    source: None,
                    kind: TargetKind::RapidReanimation,
                    runs: 1,
                    tavern: false,
                    gem: false,
                };
                self.offer_target(p, pt);
            }
            PowerEffect::FriendlyWager { .. } => {
                self.open_friendly_wager(p);
            }
            PowerEffect::DetectiveForHire { .. } => {
                self.open_holmes_offer(p);
            }
            // Prestidigitation: one offer of every combat Secret the zone
            // lacks, however many that is, not the three of a Discover
            // (docs/rules/heroes.md "Secrets"). With all four up the press is not
            // legal, so there is always at least one option to offer.
            PowerEffect::DiscoverSecret => {
                let mut offer: ArrayVec<u8, CHOICES> = ArrayVec::new();
                for kind in 0..crate::secrets::COMBAT_SECRETS {
                    if !self.players[p].secrets.has(kind) {
                        let _ = offer.try_push(kind);
                    }
                }
                self.players[p].secret_offer = offer;
            }
            _ => {}
        }
        // "After you use your Hero Power, gain (1) Gold" (Sous Chef Sticker):
        // once per press, after the press's own cost.
        let back = self.players[p].tp.power_gold;
        if back > 0 {
            self.players[p].gold = self.players[p].gold.saturating_add(back);
        }
        self.fire_event(p, EventKind::GoldSpent, cost as u32);
    }

    /// Everything a player's two powers do at the start of their turn: the
    /// uses come back, an unlock condition may be met, a start-of-turn passive
    /// runs, a rotating power turns to another tribe, a chosen power counts
    /// down (or completes), and the powers that hand out cards do it.
    pub(super) fn power_start_of_turn(&mut self, p: usize) {
        self.players[p].free_minion_left = 0;
        self.players[p].locked_and_loaded = None;
        self.players[p].wager_offer = None;
        self.players[p].holmes_offer = None;
        for m in &mut self.players[p].board {
            m.reanimate = false;
        }
        // The standing rules the powers set, from whatever the slot holds now.
        self.sync_power_flags(p);
        for second in [false, true] {
            if self.players[p].storm_power[second as usize] {
                let excludes =
                    self.data.heroes.hero_indices(&self.data.cards.hero_power_excludes);
                self.discover_hero_power(p, &excludes, second, 1);
                continue;
            }
            let Some(st) = self.players[p].power_at(second) else { continue };
            // Only the Copy numbers of the power are read here, so nothing is
            // cloned or allocated on the start of every turn.
            let power = self.data.heroes.power(st.of);
            let (uses_per_turn, complete_turns) = (power.uses_per_turn, power.complete_turns);
            let counts_down = power.parent.is_some() && complete_turns > 0;
            let uses = uses_per_turn.saturating_add(self.players[p].tp.power_extra_uses);
            if let Some(sl) = self.players[p].power_at_mut(second) {
                sl.uses_left = uses;
                sl.armed = false;
            }
            // A power chosen from a chooser counts down; when it completes,
            // its bonus runs and the chooser comes back.
            if counts_down {
                let done = {
                    let sl = self.players[p].power_at_mut(second).unwrap();
                    sl.progress += 1;
                    sl.progress >= complete_turns as u32
                };
                if done {
                    self.complete_power(p, second);
                    continue;
                }
            }
            self.refresh_power_unlock(p);
            // A power that is part of a family (A Tale of Kings) swaps its
            // tribe at the start of every turn.
            self.rotate_family_power(p, second);
            self.run_power_start_of_turn_effect(p, second);
        }
        // Procrastinate and Expedition Plans: is this turn spent?
        self.refresh_skip(p);
    }

    fn run_power_start_of_turn_effect(&mut self, p: usize, second: bool) {
        let turn = self.turn;
        let runs = self.players[p].tp.power_triggers.max(1);
        let Some(st) = self.players[p].power_at(second) else { return };
        let effect = self.data.heroes.power(st.of).effect;
        match effect {
            PowerEffect::FreeRefreshPerTurn(n) => {
                let n = n.saturating_mul(runs);
                let pl = &mut self.players[p];
                pl.free_rolls = pl.free_rolls.saturating_add(n);
            }
            PowerEffect::Bananarama { mine, others, big_percent, banana, big } => {
                let n = mine.saturating_mul(runs);
                self.give_bananas(p, n, big_percent, banana, big);
                for q in 0..PLAYERS {
                    // A real seat is a logged board, not a shopper: its
                    // hand is the file's, so it takes no Banana.
                    if q == p || !self.players[q].alive || self.players[q].real {
                        continue;
                    }
                    self.give_bananas(q, others, big_percent, banana, big);
                }
            }
            // Void Power: at the start of the game (turn 1 here), a
            // Tier 5 minion with a Dark Gift, locked in hand until turn
            // 7, its gifts picked as if it were turn 7.
            PowerEffect::DarkGiftStart => {
                if turn == 1 {
                    let plan = crate::darkgifts::DarkGiftPlan {
                        slots: [crate::darkgifts::OfferSlot { tiers: 1 << 5, yours: false };
                            crate::DISCOVER],
                        gift_turn: crate::darkgifts::VOID_POWER_TURN,
                        golden: false,
                        locked_turns: (crate::darkgifts::VOID_POWER_TURN as u8).saturating_sub(1),
                    };
                    let _ = self.players[p]
                        .discover_queue
                        .try_push(DiscoverRequest::DarkGift(plan));
                    self.maybe_start_discover(p);
                }
            }
            // Feel Devastation: every `turns` turns, a Dark Gift offer of
            // the button's tiers for that turn.
            PowerEffect::DarkGiftEvery { turns } => {
                let done = {
                    let Some(sl) = self.players[p].power_at_mut(second) else { return };
                    sl.progress += 1;
                    if sl.progress >= turns as u32 {
                        sl.progress = 0;
                        true
                    } else {
                        false
                    }
                };
                if done {
                    self.open_dark_gift_offer(p);
                }
            }
            PowerEffect::TurnDiscovers { turn: at, tiers } => {
                if turn == at {
                    for &t in tiers.iter().filter(|&&t| t != 0) {
                        let _ = self.players[p]
                            .discover_queue
                            .try_push(DiscoverRequest::TierLockedMinion(t));
                    }
                    self.maybe_start_discover(p);
                }
            }
            // Puzzle Box: cast a random Tavern spell, once unlocked.
            PowerEffect::CastRandomSpell => {
                if self.players[p].power_at(second).is_some_and(|st| st.unlocked) {
                    for _ in 0..runs {
                        self.cast_random_spell(p);
                    }
                }
            }
            // Piggy Bank: one more Gold for every turn waited.
            PowerEffect::GrowingGold { .. } => {
                if turn > 1
                    && let Some(st) = self.players[p].power_at_mut(second)
                {
                    st.progress = st.progress.saturating_add(1);
                }
            }
            // Lucky Roll: a turn of the cooldown the die set comes off,
            // and the button stays locked while any is left.
            PowerEffect::LuckyRoll => {
                if let Some(st) = self.players[p].power_at_mut(second)
                    && st.progress > 0
                {
                    st.progress -= 1;
                    st.uses_left = 0;
                }
            }
            // Glaive Ricochet: "once per turn" starts over, and the three
            // minions it copies are this turn's.
            // Kel'Thuzad's Kitty: one Discover per hero that died in the
            // round before this one, back to back.
            PowerEffect::KittyDiscover => {
                let n = self.kitty_warbands.len();
                let mut queued = 0;
                for i in 0..n {
                    // Only the round that just ended: a warband stays for
                    // its offers, which the holder may answer later in the
                    // turn.
                    if self.kitty_warbands[i].0 + 1 != turn {
                        continue;
                    }
                    let _ = self.players[p]
                        .discover_queue
                        .try_push(DiscoverRequest::DeadWarband(i as u8));
                    queued += 1;
                }
                if queued > 0 {
                    self.maybe_start_discover(p);
                }
            }
            // Tavern Lighting: one more +1/+1 for Tavern spells at the
            // start of every third turn, on top of the spell power.
            PowerEffect::TavernLighting { attack, health } => {
                let n = {
                    let Some(sl) = self.players[p].power_at_mut(second) else { return };
                    sl.progress += 1;
                    sl.progress
                };
                if n == 1 || n % 3 == 0 {
                    let pl = &mut self.players[p];
                    pl.spell_bonus.0 += attack;
                    pl.spell_bonus.1 += health;
                }
            }
            // Saturday C'Thuns!: one more repeat every turn, all game, and
            // the press of the turn before is spent.
            PowerEffect::SaturdayCthun { .. } => {
                if let Some(sl) = self.players[p].power_at_mut(second) {
                    sl.progress = sl.progress.saturating_add(1);
                    sl.armed = false;
                }
            }
            PowerEffect::CopyBuyAfterMinions { .. } => {
                if let Some(st) = self.players[p].power_at_mut(second) {
                    st.progress = 0;
                }
                self.players[p].recent_buys.clear();
            }
            // Prize Wall: on turn 4, 8, 12, 16 and every four turns after,
            // Discover a Darkmoon Prize of that turn's Prize tier (turn 4 is
            // Tier 1); Tier 4 from turn 16 on (docs/rules/trinkets.md).
            PowerEffect::PrizeWall { every } => {
                let every = every.max(1) as u32;
                let on_prize = turn >= every && turn.is_multiple_of(every);
                // The turns to the next Prize, 0 on a Prize turn, so the
                // observation can carry it.
                let next = if on_prize {
                    turn
                } else if turn < every {
                    every
                } else {
                    (turn / every + 1) * every
                };
                if let Some(st) = self.players[p].power_at_mut(second) {
                    st.progress = next - turn;
                }
                if on_prize {
                    let tier = (turn / every).min(4) as u8;
                    let _ = self.players[p]
                        .discover_queue
                        .try_push(DiscoverRequest::Prize(tier));
                    self.maybe_start_discover(p);
                }
            }
            // Nether Portal (Bloodfury): random Demons at the start of each
            // turn.
            PowerEffect::RandomMinionsEachTurn { count, tribe } => {
                let mut pos = 0;
                let total = count.saturating_mul(runs);
                for _ in 0..total {
                    self.run_effect(
                        p,
                        &mut pos,
                        Effect::GetRandomMinion { tribe },
                        false,
                        1,
                    );
                }
            }
            // Spectral Sight (Demon Hunter Training): the first minion each
            // turn is free.
            PowerEffect::FreeFirstMinion => {
                self.players[p].free_minion_left =
                    self.players[p].free_minion_left.max(runs);
            }
            // Conviction: after each fight, choose one of the three stances
            // (United Stance is omitted once the 7-minion cap is reached).
            PowerEffect::Conviction => {
                if turn > 1
                    && !self.players[p].storm_power[second as usize]
                    && self.players[p].power_offer.is_empty()
                {
                    let cap_reached = self.players[p].conviction.0 >= 7;
                    let mut picks: ArrayVec<u8, CHOICES> = ArrayVec::new();
                    for &o in &self.data.heroes.power(st.of).options {
                        if cap_reached
                            && matches!(
                                self.data.heroes.power(o).effect,
                                PowerEffect::Stance { count, .. } if count > 0
                            )
                        {
                            continue;
                        }
                        let _ = picks.try_push(o);
                    }
                    self.players[p].power_offer = picks;
                    self.players[p].power_offer_second = second;
                }
            }
            _ => {}
        }
    }

    /// Hand `n` Bananas to one player, each the plain one or, `percent` of the
    /// time, the big one (Bananarama). A hand with no room simply misses it.
    fn give_bananas(&mut self, p: usize, n: u8, percent: u8, banana: CardId, big: CardId) {
        for _ in 0..n {
            let card = if percent > 0 && self.rng.below(100) < percent as u32 { big } else { banana };
            let m = self.players[p].make_card(&self.data.cards, card, false, 0);
            self.add_to_hand(p, m);
        }
    }

    /// A chosen power completes: its effect runs once, then the chooser it
    /// came from is back, remembering what just completed so it is not
    /// offered straight away (Galewing).
    fn complete_power(&mut self, p: usize, second: bool) {
        let Some(st) = self.players[p].power_at(second) else { return };
        let power = self.data.heroes.power(st.of);
        let Some(parent) = power.parent else { return };
        let effect = power.effect;
        let mut pos = 0usize;
        match effect {
            PowerEffect::Gold(n) => {
                self.execute_spell_effect(p, SpellEffect::GainGold(n), false)
            }
            PowerEffect::GetRandomSpellOfCost(c) => self.run_effect(
                p,
                &mut pos,
                Effect::GetRandomSpellUpTo { cost: Some(c), count: 1 },
                false,
                1,
            ),
            PowerEffect::DiscoverMyTier => {
                let tier = self.players[p].tier;
                let _ = self.players[p].discover_queue.try_push(DiscoverRequest::MinionTier(tier));
                self.maybe_start_discover(p);
            }
            _ => {}
        }
        self.set_power(p, second, Some(parent));
        if let Some(sl) = self.players[p].power_at_mut(second) {
            sl.last_pick = st.of + 1;
        }
    }

    /// The end-of-turn effects of the player's two powers (Sulfuras).
    pub(super) fn power_end_of_turn(&mut self, p: usize) {
        for second in [false, true] {
            let Some(st) = self.players[p].power_at(second) else { continue };
            let effect = self.data.heroes.power(st.of).effect;
            let runs = self.players[p].tp.power_triggers.max(1);
            if let PowerEffect::EndOfTurnEnds { attack, health } = effect {
                for _ in 0..runs {
                    let n = self.players[p].board.len();
                    if n == 0 {
                        continue;
                    }
                    buff(&mut self.players[p].board[0], attack, health);
                    // With one minion it is both ends, so it gets both buffs.
                    buff(&mut self.players[p].board[n - 1], attack, health);
                }
            }
            // Saturday C'Thuns!: pressed this turn, the end of it gives a
            // random friendly minion +1/+1, once more for every turn the power
            // has lasted (docs/rules/heroes.md).
            if let PowerEffect::SaturdayCthun { attack, health } = effect
                && st.armed
            {
                let times = st.progress.saturating_mul(runs as u32);
                for _ in 0..times {
                    let n = self.players[p].board.len();
                    if n == 0 {
                        break;
                    }
                    let k = self.rng.below(n as u32) as usize;
                    buff(&mut self.players[p].board[k], attack, health);
                }
                if let Some(sl) = self.players[p].power_at_mut(second) {
                    sl.armed = false;
                }
            }
            // Major and Minor Hymn: the power is the other one after this
            // turn (Attack on the first turn, then Health, docs/rules/heroes.md).
            // A power Master Nguyen borrowed for the turn does not turn over:
            // the game ends the turn still holding it and hands out a new one
            // at the start of the next (Hearthstone_2026_11_02_00_10_22-g6
            // turn 6 ends on Major Hymn, not on Minor).
            if matches!(effect, PowerEffect::Hymn { .. })
                && !self.players[p].storm_power[second as usize]
                && let Some(next) = self.data.heroes.power(st.of).swaps_to
            {
                self.set_power(p, second, Some(next));
            }
            // Stay Frosty: the Tavern Freezes at the end of every turn.
            if matches!(effect, PowerEffect::StayFrosty { .. }) && self.players[p].freeze_end {
                self.players[p].frozen = true;
            }
            // The Perfect Crime and The Galaxy's Lens: the next press costs 1
            // less, down to 0.
            if self.data.heroes.power(st.of).cheaper_each_turn
                && let Some(st) = self.players[p].power_at_mut(second)
            {
                st.cost_delta = st.cost_delta.saturating_sub(1);
            }
            // Upbeat Harmony: a plain copy of the left-most card in hand
            // every `turns` turns.
            if let PowerEffect::HandCopyEvery { turns } = effect {
                let done = {
                    let Some(st) = self.players[p].power_at_mut(second) else { continue };
                    st.progress += 1;
                    if st.progress >= turns as u32 {
                        st.progress = 0;
                        true
                    } else {
                        false
                    }
                };
                if done
                    && let Some(&card) = self.players[p].hand.first()
                {
                    for _ in 0..runs {
                        self.give_generated(p, card.card);
                    }
                }
            }
        }
    }

    /// A card was bought: the shared hero-power counters grow (`HeroCounts`),
    /// and every power that watches a buy runs. Free buys and Tavern spells
    /// count; a steal does not, as it never comes through here.
    pub(super) fn power_bought_card(&mut self, p: usize, bought: &Minion) {
        let card = self.data.cards.get(bought.card);
        let is_spell = card.is_spell();
        let (tier, tavern_spell, battlecry) =
            (card.tier, is_spell && card.tavern_spell, card.battlecry.is_some());
        let tribes = self.data.cards.tribes_of(bought);
        {
            let c = &mut self.players[p].counts;
            c.cards_bought = c.cards_bought.saturating_add(1);
            c.tiers_bought = c.tiers_bought.saturating_add(tier as u32);
            if tavern_spell {
                c.tavern_spells_bought = c.tavern_spells_bought.saturating_add(1);
            }
            if !is_spell {
                c.minions_bought = c.minions_bought.saturating_add(1);
                if battlecry {
                    c.battlecry_bought = c.battlecry_bought.saturating_add(1);
                }
            }
        }
        // Glaive Ricochet copies one of the three minions it has seen.
        if !is_spell {
            let r = &mut self.players[p].recent_buys;
            if r.is_full() {
                r.remove(0);
            }
            let _ = r.try_push(bought.card);
        }
        for second in [false, true] {
            let Some(st) = self.players[p].power_at(second) else { continue };
            let power = self.data.heroes.power(st.of);
            let effect = power.effect;
            // A passive that works once (or a few times) a game is used up.
            if power.passive
                && power.uses_per_game > 0
                && st.used_game >= power.uses_per_game as u32
            {
                continue;
            }
            match effect {
                PowerEffect::BuyMilestone { count, becomes } => {
                    if self.power_count_up(p, second, count as u32) {
                        self.set_power(p, second, Some(becomes));
                    }
                }
                // "After you buy a Pirate, gain 1 Gold" (I'm the Cap'n Now).
                PowerEffect::GoldAfterBuyTribe { tribe, gold } => {
                    if tribes.has(tribe) && !is_spell {
                        let runs = self.players[p].tp.power_triggers.max(1);
                        let gold = gold.saturating_mul(runs);
                        self.players[p].gold = self.players[p].gold.saturating_add(gold);
                    }
                }
                // "After you buy a Pirate, your next Hero Power costs (1)
                // less" (Pirate Parrrrty!), stacking until the next press.
                PowerEffect::GetTribeCutCost { tribe, gold } => {
                    if tribes.has(tribe) && !is_spell
                        && let Some(st) = self.players[p].power_at_mut(second)
                    {
                        st.cost_delta = st.cost_delta.saturating_sub(gold as i8);
                    }
                }
                // Verdant Spheres' Tavern Coin, Battle Brand's Brann.
                PowerEffect::CardAfterBuys { every, of, card: prize } => {
                    let counted = match of {
                        BuyKind::Cards => true,
                        BuyKind::Minions => !is_spell,
                        BuyKind::Battlecry => !is_spell && battlecry,
                    };
                    if counted && self.power_count_up(p, second, every as u32) {
                        self.give_generated(p, prize);
                        if let Some(st) = self.players[p].power_at_mut(second) {
                            st.used_game += 1;
                        }
                    }
                }
                // Natural Balance: 20 Tiers' worth of cards is a Triple
                // Reward, and the count starts over.
                PowerEffect::TripleRewardAfterTiers { every } => {
                    let Some(st) = self.players[p].power_at_mut(second) else { break };
                    st.progress += tier as u32;
                    if st.progress >= every {
                        st.progress -= every;
                        self.give_triple_reward(p);
                    }
                }
                // For the Horde!: the Tavern has +1/+1, and one more of each
                // every 3 minions bought.
                PowerEffect::TavernBuffAfterBuys { attack, health, every } => {
                    if !is_spell && self.power_count_up(p, second, every as u32) {
                        self.players[p].add_tavern_buff(
                            &self.data.cards,
                            TavernBuff { tribe: None, attack, health, this_turn: false, typed_only: false },
                        );
                    }
                }
                // Glaive Ricochet: once a turn, after 3 minions bought, a
                // plain copy of a random one of them.
                PowerEffect::CopyBuyAfterMinions { every } => {
                    let ready = {
                        let Some(st) = self.players[p].power_at_mut(second) else { break };
                        if is_spell {
                            false
                        } else {
                            st.progress += 1;
                            st.progress == every as u32
                        }
                    };
                    if ready {
                        let n = self.players[p].recent_buys.len();
                        if n > 0 {
                            let k = self.rng.below(n as u32) as usize;
                            let copy = self.players[p].recent_buys[k];
                            self.give_generated(p, copy);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// Add 1 to the power's own count and say whether it reached `every`,
    /// starting the count over when it does.
    fn power_count_up(&mut self, p: usize, second: bool, every: u32) -> bool {
        let Some(st) = self.players[p].power_at_mut(second) else { return false };
        st.progress += 1;
        if st.progress >= every {
            st.progress = 0;
            true
        } else {
            false
        }
    }

    /// A minion was sold (by any effect too): the count grows and a power
    /// that watches it runs (Gone Fishing, Tentacular).
    pub(super) fn power_sold_minion(&mut self, p: usize) {
        self.players[p].counts.minions_sold = self.players[p].counts.minions_sold.saturating_add(1);
        for second in [false, true] {
            let Some(st) = self.players[p].power_at(second) else { continue };
            match self.data.heroes.power(st.of).effect {
                PowerEffect::RandomTribeAfterSells { every, tribe } => {
                    if self.power_count_up(p, second, every as u32) {
                        let mut pos = 0;
                        self.run_effect(p, &mut pos, Effect::GetRandomMinion { tribe }, false, 1);
                    }
                }
                PowerEffect::Tentacular { .. } => {
                    if let Some(st) = self.players[p].power_at_mut(second) {
                        st.progress = st.progress.saturating_add(1);
                    }
                }
                _ => {}
            }
        }
    }

    /// An Elemental was played from hand: Avalanche takes 3 off the upgrade
    /// every third one, on top of the price it has now, down to 0.
    pub(super) fn power_elemental_played(&mut self, p: usize) {
        for second in [false, true] {
            let Some(st) = self.players[p].power_at(second) else { continue };
            let PowerEffect::UpgradeDiscountAfterElementals { every, discount } =
                self.data.heroes.power(st.of).effect
            else {
                continue;
            };
            if !self.power_count_up(p, second, every as u32) {
                continue;
            }
            let runs = self.players[p].tp.power_triggers.max(1);
            let pl = &mut self.players[p];
            pl.upgrade_cost = pl.upgrade_cost.saturating_sub(discount.saturating_mul(runs));
        }
    }

    /// My minions attacked `n` times in the fight that just ended: Heroic
    /// Inspiration gives a Triple Reward for every 15, and Demon Hunter
    /// Training becomes Spectral Sight at 16.
    pub(super) fn power_attacks(&mut self, p: usize, n: u32) {
        if n == 0 {
            return;
        }
        self.players[p].counts.attacks = self.players[p].counts.attacks.saturating_add(n);
        for second in [false, true] {
            let Some(st) = self.players[p].power_at(second) else { continue };
            match self.data.heroes.power(st.of).effect {
                PowerEffect::TripleRewardAfterAttacks { every } => {
                    // The count carries over the fight: a 15th attack in a
                    // fight of 8 and 7 swings fires once.
                    let mut due = 0;
                    {
                        let Some(st) = self.players[p].power_at_mut(second) else { continue };
                        st.progress = st.progress.saturating_add(n);
                        while st.progress >= every {
                            st.progress -= every;
                            due += 1;
                        }
                    }
                    for _ in 0..due {
                        self.give_triple_reward(p);
                    }
                }
                PowerEffect::AttackMilestone { count, becomes } => {
                    let done = {
                        let Some(st) = self.players[p].power_at_mut(second) else { continue };
                        st.progress = st.progress.saturating_add(n);
                        st.progress >= count as u32
                    };
                    if done {
                        self.set_power(p, second, Some(becomes));
                    }
                }
                _ => {}
            }
        }
    }

    /// Friendly minions died (in combat or in the shop): MechGyver gives a
    /// random Mech every `every` deaths.
    pub(super) fn power_friendly_deaths(&mut self, p: usize, n: u32) {
        if n == 0 {
            return;
        }
        for second in [false, true] {
            let Some(st) = self.players[p].power_at(second) else { continue };
            let PowerEffect::MechGyver { every } =
                self.data.heroes.power(st.of).effect
            else {
                continue;
            };
            let mut due = 0;
            {
                let Some(st) = self.players[p].power_at_mut(second) else { continue };
                st.progress = st.progress.saturating_add(n);
                while st.progress >= every as u32 {
                    st.progress -= every as u32;
                    due += 1;
                }
            }
            let runs = self.players[p].tp.power_triggers.max(1) as u32;
            let mut pos = 0;
            for _ in 0..(due * runs) {
                self.run_effect(
                    p,
                    &mut pos,
                    Effect::GetRandomMinion { tribe: Tribe::Mech },
                    false,
                    1,
                );
            }
        }
    }

    /// Friendly minions dealt `amount` damage: Bloodfury becomes Nether
    /// Portal once it reaches `damage`.
    pub(super) fn power_damage_dealt(&mut self, p: usize, amount: u32) {
        if amount == 0 {
            return;
        }
        for second in [false, true] {
            let Some(st) = self.players[p].power_at(second) else { continue };
            let PowerEffect::DamageMilestone { damage, becomes } =
                self.data.heroes.power(st.of).effect
            else {
                continue;
            };
            let done = {
                let Some(st) = self.players[p].power_at_mut(second) else { continue };
                st.progress = st.progress.saturating_add(amount);
                st.progress >= damage
            };
            if done {
                self.set_power(p, second, Some(becomes));
            }
        }
    }

    /// Broodmother's Whelps grew by `gain` in the fight that just ended,
    /// kept for the rest of the game.
    pub(super) fn power_broodmother_gain(&mut self, p: usize, gain: u32) {
        if gain == 0 {
            return;
        }
        for second in [false, true] {
            let Some(st) = self.players[p].power_at(second) else { continue };
            if !matches!(self.data.heroes.power(st.of).effect, PowerEffect::Broodmother { .. }) {
                continue;
            }
            if let Some(st) = self.players[p].power_at_mut(second) {
                st.progress = st.progress.saturating_add(gain);
            }
        }
    }

    /// The Tavern Coin card id from the power table (Friendly Wager,
    /// Detective for Hire, Double Time).
    pub(super) fn tavern_coin(&self, p: usize) -> Option<CardId> {
        for second in [false, true] {
            if let Some(st) = self.players[p].power_at(second) {
                match self.data.heroes.power(st.of).effect {
                    PowerEffect::FriendlyWager { coin }
                    | PowerEffect::DetectiveForHire { coin }
                    | PowerEffect::DoubleTime { coin } => return Some(coin),
                    _ => {}
                }
            }
        }
        for pw in &self.data.heroes.powers {
            if let PowerEffect::FriendlyWager { coin }
            | PowerEffect::DetectiveForHire { coin }
            | PowerEffect::DoubleTime { coin } = pw.effect
            {
                return Some(coin);
            }
        }
        None
    }

    /// Friendly Wager: pick a fight about to happen between two other heroes
    /// (or the player's own fight when only two players remain).
    fn open_friendly_wager(&mut self, p: usize) {
        let mut pairs: ArrayVec<(u8, u8), 4> = ArrayVec::new();
        for a in 0..PLAYERS as u8 {
            if a as usize == p || !self.players[a as usize].alive {
                continue;
            }
            if let Some(super::Opponent::Player(b)) = self.players[a as usize].opponent
                && a < b
                && b as usize != p
                && self.players[b as usize].alive
            {
                let _ = pairs.try_push((a, b));
            }
        }
        if pairs.is_empty() {
            for a in 0..PLAYERS as u8 {
                if a as usize == p || !self.players[a as usize].alive {
                    continue;
                }
                if let Some(super::Opponent::Ghost(g)) = self.players[a as usize].opponent {
                    let _ = pairs.try_push((a, g));
                }
            }
        }
        if pairs.is_empty()
            && let Some(super::Opponent::Player(q) | super::Opponent::Ghost(q)) =
                self.players[p].opponent
        {
            let _ = pairs.try_push((p as u8, q));
        }
        let (mut a, mut b) = if pairs.is_empty() {
            (0, 1)
        } else {
            pairs[self.rng.below(pairs.len() as u32) as usize]
        };
        if self.rng.below(2) == 1 {
            std::mem::swap(&mut a, &mut b);
        }
        self.players[p].wager_offer = Some((a, b));
    }

    /// Settle every player's Friendly Wagers after the round's fights: 3
    /// Tavern Coins for a win, 1 Gold next turn for a tie.
    pub(super) fn settle_wagers(&mut self) {
        for p in 0..PLAYERS {
            if !self.players[p].alive {
                self.players[p].wagers.clear();
                continue;
            }
            let wagers = std::mem::take(&mut self.players[p].wagers);
            for (picked, other) in wagers {
                // A wager on a player who has left the game (a ghost's seat,
                // never offered but settled here anyway) reads the other side
                // of the pair.
                let watched = if self.players[picked as usize].alive {
                    picked
                } else {
                    other
                } as usize;
                let (won, lost) = (
                    self.players[watched].won_last_combat,
                    self.players[watched].lost_last_combat,
                );
                if won {
                    if let Some(coin) = self.tavern_coin(p) {
                        for _ in 0..3 {
                            self.give_generated(p, coin);
                        }
                    }
                } else if !lost {
                    self.players[p].gold_next_turn =
                        self.players[p].gold_next_turn.saturating_add(1);
                }
            }
        }
    }

    /// Detective for Hire: offer two options from the next opponent's warband
    /// as it stood at the start of the turn (`next_board`); when that board
    /// was empty, one of the options is `u16::MAX` ("empty").
    fn open_holmes_offer(&mut self, p: usize) {
        let opp_tier = match self.players[p].opponent {
            Some(super::Opponent::Player(q) | super::Opponent::Ghost(q)) => {
                self.players[q as usize].tier.max(1)
            }
            None => self.players[p].tier.max(1),
        };
        let (right, wrong) = if self.players[p].next_board.is_empty() {
            let decoy = self
                .data
                .cards
                .random_any_minion(opp_tier, self.lobby, &mut self.rng)
                .unwrap_or(0);
            (u16::MAX, decoy)
        } else {
            let n = self.players[p].next_board.len();
            let k = self.rng.below(n as u32) as usize;
            let right = self.players[p].next_board[k].card;
            let mut decoy = u16::MAX;
            for tier in [opp_tier, crate::MAX_TIER] {
                for _ in 0..16 {
                    if let Some(c) =
                        self.data.cards.random_any_minion(tier, self.lobby, &mut self.rng)
                        && !self.players[p].next_board.iter().any(|m| m.card == c)
                    {
                        decoy = c;
                        break;
                    }
                }
                if decoy != u16::MAX {
                    break;
                }
            }
            (right, decoy)
        };
        let correct = self.rng.below(2) as u8;
        let cards = if correct == 0 { [right, wrong] } else { [wrong, right] };
        self.players[p].holmes_offer = Some((cards, correct));
    }

    /// Gold was spent (never a Health price): Choose Your Champion's count
    /// grows, and a card locked in hand until a Gold count unlocks itself
    /// (the lock reads the count, so nothing else has to happen).
    pub(super) fn power_gold_spent(&mut self, p: usize, amount: u32) {
        if amount == 0 {
            return;
        }
        self.players[p].counts.gold_spent =
            self.players[p].counts.gold_spent.saturating_add(amount);
    }

    /// Another hero died: their warband as it stood is what Kel'Thuzad's Kitty
    /// picks from. Heroes die at the end of a fight, so several can die in one
    /// round: the warbands wait on the game, and every holder gets one
    /// Discover per warband, back to back, at the start of its next turn
    /// (docs/rules/heroes.md).
    pub(super) fn kitty_watch(&mut self, who: usize, board: &[Minion], dying: &[u8]) {
        let holds = (0..PLAYERS).any(|p| {
            p != who
                && !dying.contains(&(p as u8))
                && self.players[p].alive
                && !self.players[p].real
                && self.power_slot(p, |e| matches!(e, PowerEffect::KittyDiscover)).is_some()
        });
        if !holds || self.kitty_warbands.is_full() {
            return;
        }
        let mut warband: ArrayVec<Minion, BOARD> = ArrayVec::new();
        for &m in board {
            let _ = warband.try_push(m);
        }
        let round = self.turn;
        let _ = self.kitty_warbands.try_push((round, warband));
    }

    /// A fight is over: a power chosen for the next Start of Combat has fired,
    /// and the chooser it came from is back (Bru'kan's Elements).
    pub(super) fn power_after_fight(&mut self, p: usize) {
        for second in [false, true] {
            let Some(st) = self.players[p].power_at(second) else { continue };
            let power = self.data.heroes.power(st.of);
            if power.parent.is_some() && power.complete_turns == 0 {
                self.complete_power(p, second);
            }
        }
    }

    /// Whether this player's turn is skipped (Procrastinate, Expedition
    /// Plans): only Freeze and End Turn are legal, and the fight still
    /// happens. `refresh_skip` keeps the flag, so this is one read on the
    /// legality path.
    pub fn turn_skipped(&self, p: usize) -> bool {
        self.players[p].skipped
    }

    /// Work out whether this player's turn is skipped now and keep it on the
    /// player, so the legality check stays one read. A hero's first turns are
    /// counted from turn 1, so the pick phase (turn 0) is never skipped.
    fn refresh_skip(&mut self, p: usize) {
        let turn = self.turn;
        let skipped = turn > 0
            && [false, true].iter().any(|&second| {
                self.players[p].power_at(second).is_some_and(|st| {
                    let skip = self.data.heroes.power(st.of).skip_turns;
                    skip > 0 && turn <= skip as u32
                })
            });
        self.players[p].skipped = skipped;
    }

    /// Steal a random minion from the Tavern into hand and double its Health
    /// (Brick by Brick). The pool copy leaves with the card, like a buy, and
    /// goes back where it stood when the hand is full (a press that triggers
    /// twice can fill it). False when nothing was stolen.
    fn steal_random_shop_double_health(&mut self, p: usize) -> bool {
        let slots: ArrayVec<usize, SHOP> = (0..self.players[p].shop.len())
            .filter(|&s| !self.data.cards.get(self.players[p].shop[s].card).is_spell())
            .collect();
        if slots.is_empty() {
            return false;
        }
        let k = self.rng.below(slots.len() as u32) as usize;
        let slot = slots[k];
        let mut m = self.players[p].shop.remove(slot);
        m.health *= 2;
        if self.add_to_hand(p, m) {
            return true;
        }
        self.players[p].shop.insert(slot, m);
        false
    }

    /// Steal the Tavern card the player picked into hand: a shop slot (any
    /// card in it) or, one past the last, the spell slot. Graveyard Shift's
    /// damage lands after the card is in hand. The hand can fill between the
    /// press and the pick (a "gold spent" trigger hands out a card), and then
    /// the steal fizzles: the card goes back where it stood (a pool copy
    /// never vanishes). Returns whether the card was taken.
    pub(super) fn steal_shop_card(&mut self, p: usize, slot: usize) -> bool {
        let from_spell = slot >= self.players[p].shop.len();
        let pl = &mut self.players[p];
        let m = if !from_spell {
            pl.shop.remove(slot)
        } else if let Some(id) = pl.shop_spell.take() {
            pl.make_card(&self.data.cards, id, false, 0)
        } else {
            return false;
        };
        if self.add_to_hand(p, m) {
            return true;
        }
        // The hand filled up: put the card back where it stood.
        let pl = &mut self.players[p];
        if from_spell {
            pl.shop_spell = Some(m.card);
        } else {
            pl.shop.insert(slot, m);
        }
        false
    }

    /// Open one Hero Power Discover for the player: `count` picks in all, the
    /// first of them into the slot `to_second` says (the first of King of
    /// Duality's two is the second power, the second pick the main one).
    pub(super) fn discover_hero_power(
        &mut self,
        p: usize,
        excludes: &[u8],
        to_second: bool,
        count: u8,
    ) {
        self.players[p].power_offer_left = count;
        self.players[p].power_offer_second = to_second;
        let list = &mut self.players[p].power_excludes;
        list.clear();
        for &h in excludes {
            let _ = list.try_push(h);
        }
        self.deal_power_offer(p);
    }

    /// Deal the open Hero Power Discover: three starting powers of built
    /// heroes (or two for Power of the Storm), never one the player already
    /// holds, never a hero on the exclusion list, never a hero tied only to
    /// tribes outside the lobby (docs/rules/design.md "Discovering a hero power"). The
    /// slot it answers into is the player's `power_offer_second`.
    fn deal_power_offer(&mut self, p: usize) {
        let storm = self.players[p].storm_power[self.players[p].power_offer_second as usize];
        let any_storm = self.players[p].storm_power.iter().any(|&b| b);
        let options: Vec<u8> = self
            .data
            .heroes
            .lobby_allowed(self.lobby)
            .filter(|&h| !self.players[p].power_excludes.contains(&h))
            .map(|h| self.data.heroes.get(h).power)
            .filter(|&of| {
                !(self.holds_power(p, of)
                    || any_storm
                        && matches!(
                            self.data.heroes.power(of).effect,
                            PowerEffect::PowerOfTheStorm
                        ))
            })
            .collect();
        if options.is_empty() {
            self.players[p].power_offer_left = 0;
            return;
        }
        let target_count = if storm { 2 } else { DISCOVER };
        // Draw `target_count` distinct ones (a reservoir sample over the candidates).
        let mut picks: ArrayVec<u8, CHOICES> = ArrayVec::new();
        let mut left = options.len() as u32;
        for &h in &options {
            let want = (target_count - picks.len()) as u32;
            if picks.len() < target_count && self.rng.below(left) < want {
                picks.push(h);
            }
            left -= 1;
        }
        self.players[p].power_offer = picks;
    }

    /// The player answered an open Hero Power offer: the power lands in the
    /// slot the offer named (or upgrades Conviction when a Stance was chosen),
    /// and King of Duality's second pick is dealt next.
    pub(super) fn pick_power(&mut self, p: usize, k: usize) {
        let Some(&of) = self.players[p].power_offer.get(k) else { return };
        let second = self.players[p].power_offer_second;
        self.players[p].power_offer.clear();
        if let PowerEffect::Stance { count, attack, health } = self.data.heroes.power(of).effect {
            let c = &mut self.players[p].conviction;
            c.0 = (c.0 + count).min(7);
            c.1 += attack;
            c.2 += health;
            self.players[p].power_offer_second = false;
            return;
        }
        self.set_power(p, second, Some(of));
        // A start-of-game power picked before turn 1 (Finley's Discover
        // finding All Patched Up) still does its part: the extra Health.
        if self.turn == 0 {
            self.players[p].health += self.power_extra_health(of);
        } else if self.players[p].storm_power[second as usize] {
            let extra = self.players[p].tp.power_extra_uses;
            if let Some(sl) = self.players[p].power_at_mut(second) {
                sl.uses_left = sl.uses_left.saturating_add(extra);
            }
            self.rotate_family_power(p, second);
            self.run_power_start_of_turn_effect(p, second);
        }
        let left = self.players[p].power_offer_left.saturating_sub(1);
        self.players[p].power_offer_left = left;
        if left > 0 {
            // The next pick goes to the main power (Genn's second pick).
            self.players[p].power_offer_second = false;
            self.deal_power_offer(p);
        } else {
            self.players[p].power_offer_second = false;
        }
    }

    /// Prestidigitation's pick: the chosen Secret joins the player's Secret
    /// zone, at the end of the order (docs/rules/heroes.md "Secrets").
    pub(super) fn pick_secret(&mut self, p: usize, k: usize) {
        let Some(&kind) = self.players[p].secret_offer.get(k) else { return };
        self.players[p].secret_offer.clear();
        self.players[p].secrets.add(kind);
    }

    /// The Health a power gives the hero for good when it is picked (All
    /// Patched Up), 0 for every other power.
    fn power_extra_health(&self, of: u8) -> i32 {
        self.data.heroes.heroes.iter().find(|h| h.power == of).map_or(0, |h| h.extra_health)
    }

    /// The player picked a hero from the offer (turn 0): the offer closes,
    /// the hero lands (health, armor, power, Finley's Discover).
    pub(super) fn pick_hero(&mut self, p: usize, k: usize) {
        let hero = self.players[p].hero_offer[k];
        self.players[p].hero_offer = [u8::MAX; DISCOVER];
        self.take_hero(p, hero);
    }

    /// The hero takes `amount` damage, then the "after your hero takes
    /// damage" effects and the damage event triggers run. They fire even
    /// when a rewind among them gives the Health back. Combat damage doesn't
    /// come through here (no minion reacts to it) and is settled by
    /// `knockouts` after the fights; Health costs are checked by
    /// `can_pay_health` before they're paid, so they can't kill.
    ///
    /// `source` is the board slot of the friendly Demon that dealt it, if
    /// any: a Health cost (Malchezaar) or the combat damage to the hero is
    /// not a Demon dealing damage, so they pass `None`.
    pub(super) fn take_hero_damage(&mut self, p: usize, amount: i32, source: Option<usize>) {
        let pl = &mut self.players[p];
        if amount <= 0 {
            return;
        }
        // Safety Patch: Immune for the rest of the turn, or the Ice Block
        // eats the one hit that would kill (docs/rules/trinkets.md).
        if pl.immune {
            return;
        }
        if pl.secrets.ice_block() && amount >= pl.health + pl.armor {
            Self::use_ice_block(pl);
            return;
        }
        // Armor takes it first (docs/rules/design.md "Health and armor").
        let to_armor = pl.armor.min(amount);
        pl.armor -= to_armor;
        pl.health -= amount - to_armor;
        pl.last_damage = amount;
        pl.last_damage_armor = to_armor;
        self.note_shop_demon_damage(p, source);
        let mut pos = 0;
        while pos < self.players[p].board.len() {
            let m = self.players[p].board[pos];
            for (card, golden) in m.parts() {
                for k in 0..self.data.cards.get(card).after_hero_damage.len() {
                    if pos >= self.players[p].board.len() {
                        break;
                    }
                    let e = self.data.cards.get(card).after_hero_damage[k];
                    self.run_effect(p, &mut pos, e, golden, 1);
                }
            }
            pos += 1;
        }
        self.fire_event(p, EventKind::HeroDamage, amount as u32);
        // "Every 3 times your hero takes damage outside combat" (Nether
        // Pendant): one count per damage, whatever the amount.
        self.trinket_event(p, Event::HeroDamage, None, 1);
        // The CardMods a portrait gives the wounded minions are read here.
        self.after_hero_damage_mods(p);
    }

    /// A Safety Patch's Ice Block goes off: the fatal damage is prevented (it
    /// never happened, so nothing reacts to it) and the hero is Immune for
    /// the rest of the turn. Once.
    fn use_ice_block(pl: &mut super::Player) {
        pl.secrets.remove(crate::secrets::ICE_BLOCK);
        pl.immune = true;
        pl.last_damage = 0;
        pl.last_damage_armor = 0;
    }

    /// A portrait that gives the card's "after your hero takes damage"
    /// reward more (Rewinder Portrait: your Soul Rewinders also gain Attack).
    /// The reward itself has just run, so this adds the portrait's share.
    fn after_hero_damage_mods(&mut self, p: usize) {
        let cards = &self.data.cards;
        let mods = self.players[p].tp.card_mods;
        for m in &mut self.players[p].board {
            let mut extra = 0;
            for (c, _) in m.parts() {
                if cards.get(c).after_hero_damage.is_empty() {
                    continue;
                }
                if let Some(CardMod::AlsoAttack(n)) =
                    mods.iter().flatten().find(|(card, _)| *card == c).map(|(_, k)| *k)
                {
                    extra += n;
                }
            }
            m.attack += extra;
        }
    }

    /// Damage to the hero in the shop phase (Wrath Weaver, Imposing
    /// Percussionist). Once its reactions have run, a hero at 0 Health or
    /// below is knocked out on the spot: a rewind on the board has already
    /// had its chance to give the Health back.
    pub(super) fn take_shop_damage(&mut self, p: usize, amount: i32, source: Option<usize>) {
        self.take_hero_damage(p, amount, source);
        if self.players[p].alive && self.players[p].health <= 0 {
            self.knock_out_mid_turn(p);
        }
    }

    /// Every minion this player holds two plain copies of, in hand or on the
    /// board, for Three Wishes: a Golden copy never counts as one of a pair
    /// (docs/rules/heroes.md).
    pub(super) fn three_wishes_pairs(&self, p: usize) -> ArrayVec<CardId, 4> {
        let pl = &self.players[p];
        let mut pairs: ArrayVec<CardId, 4> = ArrayVec::new();
        for m in pl.hand.iter().chain(&pl.board) {
            let c = self.data.cards.get(m.card);
            // A token (a Beetle, a Pointy Arrow) has no pool copy to find.
            if m.golden || c.is_spell() || c.token || pairs.contains(&m.card) {
                continue;
            }
            let copies = pl
                .hand
                .iter()
                .chain(&pl.board)
                .filter(|o| o.card == m.card && !o.golden)
                .count();
            if copies >= 2 && !pairs.is_full() {
                pairs.push(m.card);
            }
        }
        pairs
    }

    /// A minion that leaves the game for good: its pool copies are spent
    /// with it (Stir the Pot, The Perfect Crime's leftovers).
    fn consume_copies(&mut self, m: &Minion) {
        let c = self.data.cards.get(m.card);
        if m.pool_copies > 0 && !c.token && !c.is_spell() {
            self.consumed[c.pool_slot as usize] += m.pool_copies as u32;
        }
    }

    /// The Perfect Crime: steal every card in the Tavern, left to right. A
    /// hand with no room loses the rest (docs/rules/heroes.md).
    pub(super) fn steal_all_tavern(&mut self, p: usize) {
        while self.steal_shop_card(p, 0) {
            if self.players[p].hand.is_full() {
                break;
            }
        }
        // Whatever is left when the hand is full is gone for good.
        while !self.players[p].shop.is_empty() {
            let m = self.players[p].shop.remove(0);
            self.consume_copies(&m);
        }
        self.players[p].shop_spell = None;
    }

    /// Stir the Pot: a thrown minion (already out of the board or the Tavern,
    /// its pool copy spent) adds its type to the pot, and the `count`th throw
    /// Discovers one minion per type gathered. A slot holds the tribe plus
    /// one, so 0 is an empty one (Beast is 0).
    pub(super) fn stir_the_pot(&mut self, p: usize, thrown: Minion, count: u8) {
        // A thrown minion is gone for good: its pool copy is spent with it.
        self.consume_copies(&thrown);
        let tribes = self.data.cards.tribes_of(&thrown);
        // docs/rules/design.md "Types": a dual or all-type minion counts as one of its
        // types, picked at random.
        let some = tribes.random_type(&mut self.rng);
        if let Some(t) = some
            && !self.players[p].pot.is_full()
        {
            let _ = self.players[p].pot.try_push(t as u8 + 1);
        }
        let Some(second) = self.power_slot(p, |e| matches!(e, PowerEffect::StirThePot { .. }))
        else {
            return;
        };
        let full = {
            let Some(st) = self.players[p].power_at_mut(second) else { return };
            st.progress += 1;
            if st.progress >= count as u32 {
                st.progress = 0;
                true
            } else {
                false
            }
        };
        if !full {
            return;
        }
        let mut types = [0u8; 3];
        for (i, &t) in self.players[p].pot.iter().enumerate() {
            types[i] = t;
        }
        self.players[p].pot.clear();
        if types.iter().any(|&t| t != 0) {
            let _ = self.players[p].discover_queue.try_push(DiscoverRequest::PotTypes(types));
            self.maybe_start_discover(p);
        }
    }

    /// Which slot holds a power whose effect matches: the main one first.
    fn power_slot(&self, p: usize, want: impl Fn(PowerEffect) -> bool) -> Option<bool> {
        for second in [false, true] {
            if let Some(st) = self.players[p].power_at(second)
                && want(self.data.heroes.power(st.of).effect)
            {
                return Some(second);
            }
        }
        None
    }

    /// Swap, Lock, & Shop It: the picked minion goes into the Tavern where a
    /// random Tavern minion stood, and that one takes its place on the board.
    /// Both keep their buffs, and nothing triggers: it is not a buy and not a
    /// play (docs/rules/heroes.md).
    pub(super) fn swap_with_shop(&mut self, p: usize, s: usize) {
        let slots: ArrayVec<usize, SHOP> = (0..self.players[p].shop.len())
            .filter(|&i| !self.data.cards.get(self.players[p].shop[i].card).is_spell())
            .collect();
        if s >= self.players[p].board.len() || slots.is_empty() {
            return;
        }
        let k = self.rng.below(slots.len() as u32) as usize;
        let slot = slots[k];
        let mine = self.players[p].board[s];
        let mut theirs = self.players[p].shop[slot];
        theirs.uses_left = theirs.turn_uses(&self.data.cards);
        self.players[p].board[s] = theirs;
        self.players[p].shop[slot] = mine;
    }

    /// A friendly Demon dealt damage in the shop phase: every other friendly
    /// minion with "After another friendly Demon deals damage" gains its
    /// stats for good right away (Devout Hellcaller). A damage source that
    /// is no longer a friendly Demon does not count.
    fn note_shop_demon_damage(&mut self, p: usize, source: Option<usize>) {
        let Some(src) = source else { return };
        let cards = &self.data.cards;
        let Some(src_m) = self.players[p].board.get(src).copied() else { return };
        if !cards.tribes_of(&src_m).has(Tribe::Demon) {
            return;
        }
        let mut hits: ArrayVec<(usize, i32, i32), BOARD> = ArrayVec::new();
        for (i, m) in self.players[p].board.iter().enumerate() {
            if i == src {
                continue;
            }
            for (card, golden) in m.parts() {
                if let Some((a, h)) = cards.get(card).after_demon_damage {
                    let mul = if golden { 2 } else { 1 };
                    let _ = hits.try_push((i, a * mul, h * mul));
                }
            }
        }
        for (i, da, dh) in hits {
            buff(&mut self.players[p].board[i], da, dh);
        }
    }
}
