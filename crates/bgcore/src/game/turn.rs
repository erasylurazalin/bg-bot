//! Turn flow: end of turn, start of turn, pairing, combat, knockouts.

use arrayvec::ArrayVec;
use crate::cards::{
    Effect,
    EndOfTurn,
    StartOfTurn,
    Tribe,
};
use crate::fight::{
    SideOutcome,
    fight_with_hands,
};
use crate::heroes::PowerEffect;
use crate::minion::{
    Keywords,
    Minion,
};
use crate::{
    BOARD,
    MAX_TIER,
    PLAYERS,
};
use super::{
    Game,
    Ghost,
    Opponent,
    Player,
    RefreshKind,
    SandboxFight,
    Seen,
    TavernBuff,
};
use super::shop::give_back;
use super::effects::buff;

impl Game {
    /// Knock `p` out in the middle of its turn. It takes the worst place
    /// still open, gives its cards back and stops acting; its board becomes
    /// the ghost, and the players still alive are paired again without it.
    pub(super) fn knock_out_mid_turn(&mut self, p: usize) {
        let place = self.alive() as u8;
        {
            let Game { data, players, ghost, lobby, .. } = self;
            let pl = &mut players[p];
            pl.alive = false;
            pl.placement = place;
            pl.final_tribes = pl.board_tribes(&data.cards);
            *ghost = Some(Ghost {
                player: p as u8,
                board: pl.board.clone(),
                tier: pl.tier,
                counters: pl.counters(data, *lobby),
            });
            pl.opponent = None;
            pl.done = true;
        }
        // The cards go back at the end of the action: effects that were
        // already running may still touch this board.
        let left: ArrayVec<u8, PLAYERS> =
            (0..PLAYERS as u8).filter(|&q| self.players[q as usize].alive).collect();
        if left.len() <= 1 {
            for &q in &left {
                let cards = &self.data.cards;
                let pl = &mut self.players[q as usize];
                pl.placement = 1;
                pl.final_tribes = pl.board_tribes(cards);
            }
            self.over = true;
            return;
        }
        self.pair();
    }

    /// A dead player holds no cards. Effects that were still running when
    /// the hero died can give it some, so the end of every action gives
    /// them back and clears anything waiting on a choice.
    pub(super) fn settle_dead(&mut self, p: usize) {
        let Game { data, players, pool, .. } = self;
        let pl = &mut players[p];
        if pl.alive {
            return;
        }
        let idle = pl.shop.is_empty()
            && pl.hand.is_empty()
            && pl.board.is_empty()
            && pl.discover.is_empty()
            && pl.shop_spell.is_none()
            && pl.discover_queue.is_empty()
            && pl.pending_target.is_none()
            && pl.pending_shop_target.is_none()
            && pl.pending_choose_one.is_none()
            && pl.pending_discard.is_none()
            && pl.choose_queue.is_empty()
            && pl.power_offer.is_empty();
        if idle {
            return;
        }
        pl.shop_spell = None;
        for m in pl.shop.drain(..).chain(pl.hand.drain(..)).chain(pl.board.drain(..)) {
            give_back(pool, &data.cards, &m);
        }
        for c in pl.discover.drain(..) {
            if !data.cards.get(c).is_spell() {
                let slot = data.cards.get(c).pool_slot as usize;
                pool[slot] += 1;
            }
        }
        pl.pending_choose_one = None;
        pl.pending_target = None;
        pl.pending_shop_target = None;
        pl.pending_discard = None;
        pl.choose_queue.clear();
        pl.discover_damage_from = None;
        pl.discover_locked = false;
        pl.discover_queue.clear();
        pl.tavern_buffs.clear();
        pl.power_offer.clear();
    }

    fn run_end_of_turn(&mut self, p: usize) {
        let runs = self.drakkari_mult(p);
        let mut pos = 0;
        while pos < self.players[p].board.len() {
            let m = self.players[p].board[pos];
            for _ in 0..runs {
                for (card, golden) in m.parts() {
                    let mul = if golden { 2 } else { 1 };
                    match self.data.cards.get(card).end_of_turn {
                        Some(EndOfTurn::BuffSelf { attack, health }) => {
                            buff(&mut self.players[p].board[pos], attack * mul, health * mul);
                        }
                        Some(EndOfTurn::ResourcefulRobot) => {
                            let n = if golden { 2 } else { 1 };
                            for _ in 0..n {
                                if let Some(v_id) = self.data.cards.random_volumizer(self.lobby, &mut self.rng)
                                {
                                    let v =
                                        self.players[p].make_card(&self.data.cards, v_id, false, 0);
                                    self.magnetize_to(p, pos, v, false);
                                    self.give_generated_tracking(p, v_id, &mut pos);
                                }
                            }
                        }
                        Some(EndOfTurn::BuffPerMagnetization { attack, health }) => {
                            for o in &mut self.players[p].board {
                                let mag = o.magnetizations as i32;
                                if mag > 0 {
                                    buff(o, attack * mul * mag, health * mul * mag);
                                }
                            }
                        }
                        Some(EndOfTurn::ImproveTier) => {
                            let imp = &mut self.players[p].board[pos].improvements;
                            *imp = (*imp + 1).min((MAX_TIER - 1) as u16);
                        }
                        Some(EndOfTurn::GetCard { card: tok, count }) => {
                            let n = count * (mul as u8);
                            for _ in 0..n {
                                self.give_generated_tracking(p, tok, &mut pos);
                            }
                        }
                        Some(EndOfTurn::Effect(e)) => {
                            self.run_effect(p, &mut pos, e, golden, 1);
                        }
                        Some(EndOfTurn::CopyLastSpell) => {
                            if let Some(sp) = self.players[p].last_spell {
                                let n = mul as u8;
                                for _ in 0..n {
                                    self.give_generated_tracking(p, sp, &mut pos);
                                }
                            }
                        }
                        None => {}
                    }
                }
            }
            pos += 1;
        }
        // Trinkets at the end of the turn: their own end-of-turn effects,
        // then an "every N turns" rule that has counted up to N.
        self.trinket_end_of_turn(p);
        // A Lockbox is an end of turn countdown too, so Drakkari doubles it.
        self.tick_lockboxes(p, runs);
    }

    /// The player is done for this turn. Anything still waiting on a choice
    /// takes the first option, then end-of-turn effects run.
    /// A played Tomb Turning Undead dies once its Battlecry, and whatever it
    /// asked the player to choose or Discover, is done.
    pub(super) fn choice_open(pl: &Player) -> bool {
        Self::other_choice_open(pl) || !pl.trinket_offer.is_empty()
    }

    /// A choice open beside the trinket offer (a Discover a trinket's own
    /// start-of-turn effect opened, a target, a discard): it is answered
    /// first, then the offer.
    pub(super) fn other_choice_open(pl: &Player) -> bool {
        pl.pending_target.is_some()
            || pl.pending_shop_target.is_some()
            || pl.pending_choose_one.is_some()
            || pl.pending_discard.is_some()
            || !pl.discover.is_empty()
            || !pl.discover_queue.is_empty()
            || !pl.choose_queue.is_empty()
            || !pl.power_offer.is_empty()
    }

    /// Turbo Hogrider fires for each Choose One card played once nothing
    /// that card started is still waiting for a pick, so the minion that was
    /// played (Intrepid Botanist...) is on the board with its choice done.
    pub(super) fn resolve_hogrider(&mut self, p: usize) {
        let pl = &self.players[p];
        if pl.hogrider_pending == 0 || !pl.alive || Self::choice_open(pl) {
            return;
        }
        let n = std::mem::take(&mut self.players[p].hogrider_pending);
        for _ in 0..n {
            self.after_choose_one_played(p);
        }
    }

    pub(super) fn resolve_doomed(&mut self, p: usize) {
        let pl = &self.players[p];
        if !pl.alive || Self::choice_open(pl) {
            return;
        }
        let victims: ArrayVec<usize, BOARD> =
            pl.board.iter().enumerate().filter(|(_, m)| m.doomed).map(|(i, _)| i).collect();
        if victims.is_empty() {
            return;
        }
        for m in &mut self.players[p].board {
            m.doomed = false;
        }
        self.destroy_in_shop(p, &victims, None);
    }

    /// Answer everything the player left open, taking the first option that
    /// can be taken (the end of their turn cannot ask).
    pub(super) fn resolve_open_choices(&mut self, p: usize) {
        loop {
            let pl = &self.players[p];
            if let Some(pt) = pl.pending_target {
                if let Some(s) = (0..pl.board.len()).find(|&s| self.valid_target(p, &pt, s)) {
                    self.resolve_target(p, s);
                } else if pt.kind.takes_shop()
                    && let Some(s) = (0..pl.shop.len())
                        .find(|&s| self.valid_shop_target(p, &pt, s))
                {
                    self.resolve_shop_target(p, s);
                } else {
                    self.players[p].pending_target = None;
                }
            } else if pl.pending_shop_target.is_some() {
                self.resolve_shop_target(p, 0);
            } else if pl.pending_discard.is_some() {
                // Throw away the first card a discard may take.
                match (0..pl.hand.len()).find(|&s| self.valid_discard(p, s)) {
                    Some(s) => self.resolve_discard(p, s),
                    None => self.players[p].pending_discard = None,
                }
            } else if let Some(pco) = pl.pending_choose_one {
                // The first option that can be chosen, or none: a Choose One
                // nothing can answer fizzles rather than panicking.
                match (0..2).find(|&k| self.can_choose_option(p, pco.options[k])) {
                    Some(k) => self.resolve_choose(p, k),
                    None => self.players[p].pending_choose_one = None,
                }
            } else if !pl.discover.is_empty() || !pl.power_offer.is_empty() {
                self.resolve_choose(p, 0);
            } else if !pl.trinket_offer.is_empty() {
                // The offer can't be declined: ending the turn with it open
                // buys the cheapest option.
                let k = (0..pl.trinket_offer.len())
                    .min_by_key(|&k| pl.trinket_offer[k].1)
                    .unwrap_or(0);
                self.pick_trinket(p, k);
            } else if !pl.choose_queue.is_empty() {
                // Both-effects Choose One options still waiting.
                self.maybe_start_discover(p);
            } else {
                break;
            }
        }
    }

    pub(super) fn finish_shop(&mut self, p: usize) {
        self.resolve_open_choices(p);
        self.resolve_doomed(p);
        self.resolve_hogrider(p);
        // A Tomb Turning Undead that wasn't played this turn is a normal one.
        for m in &mut self.players[p].hand {
            m.doomed = false;
        }
        self.run_end_of_turn(p);
        // An end-of-turn effect can still ask (a Battlecry it triggers may be
        // a Choose One, Young Murk-Eye's neighbours): answer it now, so no
        // prompt outlives the turn it was opened in.
        self.resolve_open_choices(p);
        let cards = &self.data.cards;
        // "This turn" effects end with the turn wherever the minion is: a
        // golden made from a triple holds its copies' in hand.
        let pl = &mut self.players[p];
        for m in pl.board.iter_mut().chain(pl.hand.iter_mut()) {
            m.extra_magnetize = 0;
            // An armed Living Prison that found no minion to buy is wasted.
            if m.improvements > 0
                && m.parts().any(|(c, _)| {
                    matches!(cards.get(c).activate, Some(a) if a.effect == Effect::ArmNextBuy)
                })
            {
                m.improvements = 0;
            }
        }
        self.players[p].end_turn_tavern_buffs(&self.data.cards);
        self.players[p].done = true;
    }

    pub(super) fn start_turn(&mut self) {
        self.turn += 1;
        let turn = self.turn;
        let base_turn_gold = self.data.rules.gold(turn);
        for p in 0..PLAYERS {
            if !self.players[p].alive {
                continue;
            }
            let (da, dh) = self.total_spell_bonus(p);
            let Game { data, rng, players, pool, sandbox, real_boards, .. } = self;
            let pl = &mut players[p];
            if pl.real {
                // A real seat doesn't shop: it takes a fresh random board of
                // this turn (the file's stats, golden flag, keywords and
                // scaling state), that player's hand, and the side-wide
                // numbers it had at the first attack (Blood Gem size, Beetle
                // and Undead bonuses, spell bonus, spells cast, Deity, the
                // "improve" counts). They replace the last turn's wholesale.
                // The old board just vanishes: real minions hold no pool
                // copies, so nothing goes back.
                let board = real_boards
                    .as_ref()
                    .expect("a real seat needs real boards")
                    .draw(turn, rng);
                pl.tier = board.tier;
                pl.board.clear();
                for m in &board.minions {
                    let _ = pl.board.try_push(*m);
                }
                pl.hand.clear();
                for m in &board.hand {
                    let _ = pl.hand.try_push(*m);
                }
                pl.take_real_counters(&board.counters, board.deity);
                pl.done = true;
                continue;
            }
            if let Some(sb) = sandbox {
                pl.free_rolls = pl.free_rolls.max(sb.free_rolls);
            }
            // the Beast bonus lasts until now; it never kills a Beast
            let (ba, bh) = std::mem::take(&mut pl.beast_bonus);
            if (ba, bh) != (0, 0) {
                for m in &mut pl.board {
                    if data.cards.get(m.card).tribes.has(Tribe::Beast) {
                        m.attack = (m.attack - ba).max(0);
                        m.health = (m.health - bh).max(1);
                    }
                }
            }
            if turn > 1 {
                pl.upgrade_cost = pl.upgrade_cost.saturating_sub(1);
            }
            let base_gold = base_turn_gold.saturating_add(pl.extra_max_gold);
            pl.gold = base_gold.saturating_add(pl.gold_next_turn);
            pl.gold_next_turn = 0;
            // The power button's uses come back, an unlock condition may be
            // met, and "at the start of your turn" passives run (Clairvoyance:
            // a free Refresh that stacks with any saved).
            if let Some(st) = pl.power {
                let power = &data.heroes.get(st.of).power;
                let (uses, unlock_tier, unlock_turn, effect) =
                    (power.uses_per_turn, power.unlock_tier, power.unlock_turn, power.effect);
                pl.power.as_mut().unwrap().uses_left = uses;
                pl.power.as_mut().unwrap().unlocked =
                    pl.tier >= unlock_tier && turn >= unlock_turn;
                if let PowerEffect::FreeRefreshPerTurn(n) = effect {
                    pl.free_rolls = pl.free_rolls.saturating_add(n);
                }
            }
            if turn > 1 {
                if pl.won_last_combat {
                    pl.gold = pl.gold.saturating_add(pl.overconfidence_win);
                } else if !pl.lost_last_combat {
                    pl.gold = pl.gold.saturating_add(pl.overconfidence_tie);
                }
            }
            pl.overconfidence_win = 0;
            pl.overconfidence_tie = 0;
            for m in &pl.board {
                for (card, golden) in m.parts() {
                    if let Some(StartOfTurn::GainGold(g)) = data.cards.get(card).start_of_turn {
                        let gain = g.saturating_mul(if golden { 2 } else { 1 });
                        pl.gold = pl.gold.saturating_add(gain);
                    }
                }
            }
            for (ba, bh, times) in pl.next_turn_board_buffs.drain(..) {
                for _ in 0..times {
                    for m in &mut pl.board {
                        buff(m, ba + da, bh + dh);
                    }
                }
            }
            let (wb_a, wb_h) = pl.winners_bread_buff;
            let won = pl.won_last_combat;
            for m in &mut pl.board {
                m.activated = false;
                // The limited uses of a new turn: a Trailblazer's combines,
                // a Mycologist's teachings, Malchezaar's Health Refreshes.
                m.uses_left = m.turn_uses(&data.cards);
                // Ichoron's Divine Shield lasts until the next shop phase.
                if m.temp_shield {
                    m.keywords.clear(Keywords::DIVINE_SHIELD);
                    m.temp_shield = false;
                }
                if m.winners_bread > 0 {
                    if won {
                        let n = m.winners_bread as i32;
                        buff(m, (wb_a + da) * n, (wb_h + dh) * n);
                    }
                    m.winners_bread = 0;
                }
            }
            for m in &mut pl.hand {
                m.winners_bread = 0;
            }
            for m in &mut pl.hand {
                m.locked_turns = m.locked_turns.saturating_sub(1);
            }
            pl.done = false;
            pl.actions = 0;
            pl.gold_spent_turn = 0;
            pl.elementals_played_turn = 0;
            let was_frozen = pl.frozen;
            if pl.frozen {
                pl.frozen = false;
            } else {
                for m in pl.shop.drain(..) {
                    give_back(pool, &data.cards, &m);
                }
                pl.shop_spell = None;
            }
            let size = data.rules.shop_size(pl.tier);
            // The roll at the start of a turn counts as a Refresh for
            // Waveling's amount and Barrage when the Tavern changed: always
            // when it was not frozen, and for a frozen one only when the roll
            // had to fill a slot (a bought card, a consumed or stolen one, a
            // new slot from levelling, an empty spell slot). It is still not
            // the player's own Refresh, so queued Fodders wait for an
            // explicit one.
            let refreshed = !was_frozen || pl.shop.len() < size || pl.shop_spell.is_none();
            Self::fill_shop(pl, pool, &data.cards, size, rng, self.lobby, RefreshKind::TurnStart);
            if refreshed {
                Self::apply_refresh_buff(pl, &data.cards, rng);
                Self::apply_barrage(pl, &data.cards);
            }
        }
        for p in 0..PLAYERS {
            // Trinkets at the start of the player's turn: the two offers (and
            // the one a hero power owes), the counters, the uses back and the
            // start-of-turn effects.
            if self.players[p].alive && !self.players[p].real {
                self.trinket_start_of_turn(p);
            }
        }
        for p in 0..PLAYERS {
            // A real board's stats already include everything its thresholds
            // gave it in the real game, so they don't fire here.
            if self.players[p].alive && !self.players[p].real {
                self.check_attack_thresholds(p);
            }
        }
        self.pair();
        self.order = (0..PLAYERS as u8).filter(|&p| self.players[p as usize].alive).collect();
        self.rng.shuffle(&mut self.order);
        // Real seats are done the moment the turn starts, so the first player
        // to act can sit past the cursor; with everyone done the turn goes
        // straight to the fights.
        match self.order.iter().position(|&p| !self.players[p as usize].done) {
            Some(c) => self.cursor = c,
            None => self.advance(),
        }
    }

    /// Pair the living players at random, avoiding last turn's opponent
    /// when a few shuffles can. With an odd count, one fights the ghost.
    fn pair(&mut self) {
        let mut alive: ArrayVec<u8, PLAYERS> =
            (0..PLAYERS as u8).filter(|&p| self.players[p as usize].alive).collect();
        let repeats = |order: &[u8], players: &[Player; PLAYERS]| {
            order
                .chunks_exact(2)
                .filter(|c| players[c[0] as usize].last_opponent == Some(Opponent::Player(c[1])))
                .count()
        };
        let mut best = alive.clone();
        let mut best_repeats = usize::MAX;
        for _ in 0..16 {
            self.rng.shuffle(&mut alive);
            let r = repeats(&alive, &self.players);
            if r < best_repeats {
                best_repeats = r;
                best = alive.clone();
            }
            if r == 0 {
                break;
            }
        }
        for c in best.chunks(2) {
            if let [a, b] = *c {
                self.players[a as usize].opponent = Some(Opponent::Player(b));
                self.players[b as usize].opponent = Some(Opponent::Player(a));
            } else {
                let ghost = self.ghost.as_ref().map(|g| Opponent::Ghost(g.player));
                self.players[c[0] as usize].opponent = ghost;
            }
        }
    }

    /// Take what a fight (or a Tavern attack, with `after_fight` false)
    /// leaves behind. After a Tavern attack the board is already up to date,
    /// so only the hand and the Tavern take the bonuses.
    pub(super) fn apply_side_outcome(&mut self, p: usize, so: &SideOutcome, after_fight: bool) {
        if after_fight {
            for (slot, &imp) in so.permanent_improvements.iter().enumerate() {
                if imp > 0 && slot < self.players[p].board.len() {
                    self.players[p].board[slot].improvements =
                        self.players[p].board[slot].improvements.saturating_add(imp);
                }
            }
            // Tarecgosa and Persistent Poet: stats and Bonus Keywords kept
            // from the fight.
            for (slot, &(da, dh)) in so.permanent_stats.iter().enumerate() {
                if (da != 0 || dh != 0) && slot < self.players[p].board.len() {
                    let m = &mut self.players[p].board[slot];
                    m.attack += da;
                    m.health += dh;
                }
            }
            for (slot, &kw) in so.permanent_keywords.iter().enumerate() {
                if kw != 0 && slot < self.players[p].board.len() {
                    self.players[p].board[slot].keywords.0 |= kw;
                }
            }
            // Very Hungry Winterfinner's hand buffs are permanent.
            for (slot, &(da, dh)) in so.hand_stats.iter().enumerate() {
                if (da != 0 || dh != 0) && slot < self.players[p].hand.len() {
                    self.players[p].hand[slot].attack += da;
                    self.players[p].hand[slot].health += dh;
                }
            }
            // Razorfen Vineweaver's permanent Blood Gems: the stats come as
            // permanent_stats, and the same gain is Blood Gem stats.
            for (slot, &(da, dh)) in so.permanent_gems.iter().enumerate() {
                if (da != 0 || dh != 0) && slot < self.players[p].board.len() {
                    let m = &mut self.players[p].board[slot];
                    m.gem_attack += da;
                    m.gem_health += dh;
                }
            }
        }
        if let Some(deity) = &mut self.players[p].deity {
            deity.attack += so.deity_gain.0;
            deity.health += so.deity_gain.1;
        }
        self.players[p].spells_cast = self.players[p].spells_cast.saturating_add(so.spells_cast_gain);
        let (sa, sh) = so.spell_bonus_gain;
        if sa != 0 || sh != 0 {
            self.players[p].spell_bonus.0 += sa;
            self.players[p].spell_bonus.1 += sh;
        }
        if so.beetle_attack_gain != 0 || so.beetle_health_gain != 0 {
            self.add_beetle_bonus_to(p, so.beetle_attack_gain, so.beetle_health_gain, after_fight);
        }
        for (n, &g) in self.players[p].improve.iter_mut().zip(&so.improve_gain) {
            *n = n.saturating_add(g);
        }
        if !after_fight {
            let pl = &mut self.players[p];
            pl.beast_bonus.0 += so.beast_bonus_gain.0;
            pl.beast_bonus.1 += so.beast_bonus_gain.1;
        }
        self.add_deathrattles_triggered(p, so.deathrattles_triggered as u32, after_fight);
        self.add_undead_bonus(p, so.undead_gain.0, so.undead_gain.1, after_fight);
        self.add_knight_deaths(p, so.knight_deaths, after_fight);
        // A Sanguine Refiner Rally in a fight raises the Gem value for good.
        if so.gem_gain != (0, 0) {
            let pl = &mut self.players[p];
            pl.gem_attack += so.gem_gain.0;
            pl.gem_health += so.gem_gain.1;
        }
        // Sacrificial Wrathguard's Deathrattle: a Tavern buff for the game.
        if so.tavern_buff_gain != (0, 0) {
            let b = TavernBuff {
                tribe: None,
                attack: so.tavern_buff_gain.0,
                health: so.tavern_buff_gain.1,
                this_turn: false,
            };
            self.players[p].add_tavern_buff(&self.data.cards, b);
        }
        // Waveling's Deathrattle: more for the Refreshes to come.
        self.players[p].refresh_buff.0 += so.refresh_buff_gain.0;
        self.players[p].refresh_buff.1 += so.refresh_buff_gain.1;
        // Trapped Clapper's Deathrattle: Fodders for upcoming Refreshes. A
        // fight's list is one entry per upcoming Refresh, and they stack on
        // any Refreshes the player already queued.
        for k in 0..so.n_fodder_refreshes as usize {
            self.players[p].add_fodder_to_refresh(k, so.fodder_refreshes[k]);
        }
        if after_fight {
            self.players[p].boon_of_beetles =
                self.players[p].boon_of_beetles.saturating_sub(so.boon_of_beetles_used);
            self.players[p].brood_of_nozdormu = 0;
            self.players[p].upper_hand = 0;
            self.players[p].sharing_is_caring = 0;
        }
        // A hero knocked out in this fight takes nothing home: its hand is
        // already gone (knockouts drained it), and a card given after that
        // would leave a dead player holding cards.
        if !self.players[p].alive {
            return;
        }
        for &c in so.cards_gained() {
            self.give_generated(p, c);
        }
        for &n in so.lockboxes_gained() {
            self.give_lockbox(p, n);
        }
    }

    pub(super) fn combat(&mut self) {
        if self.sandbox.is_some() {
            self.sandbox_combat();
            return;
        }
        let alive = self.alive();
        let max_attacks = self.data.rules.max_attacks;
        let cap = self.data.rules.damage_cap(self.turn, alive);
        let capped = |d: i32| cap.map_or(d, |c| d.min(c));
        let mut damage = [0i32; PLAYERS];
        for p in 0..PLAYERS {
            if !self.players[p].alive {
                continue;
            }
            // The hand only goes into the fight when something on the board
            // or in it reads the hand, so most fights skip it entirely.
            let cards = &self.data.cards;
            let reads_board = |b: &[Minion]| {
                b.iter().any(|m| m.parts().any(|(c, _)| cards.get(c).reads_hand()))
            };
            let reads_hand =
                |h: &[Minion]| h.iter().any(|m| cards.get(m.card).reads_hand());
            let (them, tier, their_counters, their_hand) = match self.players[p].opponent {
                Some(Opponent::Player(q)) if p < q as usize => {
                    let q = &self.players[q as usize];
                    let hand: &[Minion] = if reads_board(&q.board) || reads_hand(&q.hand) {
                        &q.hand
                    } else {
                        &[]
                    };
                    (q.board.clone(), q.tier, q.counters(&self.data, self.lobby), hand)
                }
                Some(Opponent::Ghost(_)) => {
                    let g = self.ghost.as_ref().unwrap();
                    (g.board.clone(), g.tier, g.counters, &[] as &[Minion])
                }
                _ => continue,
            };
            let my_hand: &[Minion] = if reads_board(&self.players[p].board)
                || reads_hand(&self.players[p].hand)
            {
                &self.players[p].hand
            } else {
                &[]
            };
            let me = self.players[p].board.clone();
            let my_counters = self.players[p].counters(&self.data, self.lobby);
            let turn = self.turn;
            let my_tier = self.players[p].tier;
            let hands: [&[Minion]; 2] = [my_hand, their_hand];
            let out = fight_with_hands(
                &self.data.cards,
                &me,
                &them,
                hands,
                [my_counters, their_counters],
                max_attacks,
                &mut self.rng,
                &mut (),
            );
            // What each side sees of the other: the boards that went in, and
            // the counters at the first attack, after Start of Combat.
            let their_seen = Seen::at_fight(them, tier, turn, out.sides[1].seen);
            let my_seen = Seen::at_fight(me, my_tier, turn, out.sides[0].seen);
            match self.players[p].opponent.unwrap() {
                Opponent::Player(q) => {
                    let q = q as usize;
                    self.players[p].seen[q] = their_seen;
                    self.players[q].seen[p] = my_seen;
                    match out.winner {
                        Some(0) => {
                            damage[q] += capped(my_tier as i32 + out.survivor_tiers);
                            self.players[p].lost_last_combat = false;
                            self.players[p].won_last_combat = true;
                            self.players[q].lost_last_combat = true;
                            self.players[q].won_last_combat = false;
                        }
                        Some(_) => {
                            damage[p] += capped(tier as i32 + out.survivor_tiers);
                            self.players[p].lost_last_combat = true;
                            self.players[p].won_last_combat = false;
                            self.players[q].lost_last_combat = false;
                            self.players[q].won_last_combat = true;
                        }
                        None => {
                            self.players[p].lost_last_combat = false;
                            self.players[p].won_last_combat = false;
                            self.players[q].lost_last_combat = false;
                            self.players[q].won_last_combat = false;
                        }
                    }
                    self.apply_side_outcome(p, &out.sides[0], true);
                    self.apply_side_outcome(q, &out.sides[1], true);
                }
                Opponent::Ghost(g) => {
                    self.players[p].seen[g as usize] = their_seen;
                    match out.winner {
                        Some(0) => {
                            self.players[p].lost_last_combat = false;
                            self.players[p].won_last_combat = true;
                        }
                        Some(_) => {
                            damage[p] += capped(tier as i32 + out.survivor_tiers);
                            self.players[p].lost_last_combat = true;
                            self.players[p].won_last_combat = false;
                        }
                        None => {
                            self.players[p].lost_last_combat = false;
                            self.players[p].won_last_combat = false;
                        }
                    }
                    self.apply_side_outcome(p, &out.sides[0], true);
                }
            }
        }
        for (p, d) in damage.into_iter().enumerate() {
            // A lost fight means the board wasn't in play, so no minion
            // reacts: no rewind, no "after your hero takes damage", no
            // damage event count. The hero just loses the Health (armor
            // first), and `knockouts` settles any that reach 0.
            let pl = &mut self.players[p];
            let to_armor = pl.armor.min(d);
            pl.armor -= to_armor;
            pl.health -= d - to_armor;
            pl.last_opponent = pl.opponent.take();
        }
        self.knockouts(alive);
    }

    /// The sandbox fight: the player against a copy of their own board,
    /// hand and counters. Only the player's side outcome is applied, and
    /// the fight never costs Health.
    fn sandbox_combat(&mut self) {
        let max_attacks = self.data.rules.max_attacks;
        let cards = &self.data.cards;
        let pl = &self.players[0];
        let reads_board =
            |b: &[Minion]| b.iter().any(|m| m.parts().any(|(c, _)| cards.get(c).reads_hand()));
        let reads_hand = |h: &[Minion]| h.iter().any(|m| cards.get(m.card).reads_hand());
        let hand: &[Minion] =
            if reads_board(&pl.board) || reads_hand(&pl.hand) { &pl.hand } else { &[] };
        let me = pl.board.clone();
        let counters = pl.counters(&self.data, self.lobby);
        let out = fight_with_hands(
            cards,
            &me,
            &me,
            [hand, hand],
            [counters, counters],
            max_attacks,
            &mut self.rng,
            &mut (),
        );
        let fight = SandboxFight {
            turn: self.turn,
            tier: pl.tier,
            health: pl.health,
            board: me.clone(),
            hand: pl.hand.clone(),
            winner: out.winner,
            side: out.sides[0],
        };
        let pl = &mut self.players[0];
        pl.won_last_combat = out.winner == Some(0);
        pl.lost_last_combat = out.winner == Some(1);
        self.apply_side_outcome(0, &out.sides[0], true);
        if let Some(sb) = &mut self.sandbox {
            sb.last_fight = Some(fight);
        }
        if self.turn >= self.data.rules.max_turns {
            self.players[0].placement = 1;
            self.over = true;
        }
    }

    fn knockouts(&mut self, alive: usize) {
        let mut dead: ArrayVec<u8, PLAYERS> = (0..PLAYERS as u8)
            .filter(|&p| self.players[p as usize].alive && self.players[p as usize].health <= 0)
            .collect();
        // best first: more health left, ties at random
        self.rng.shuffle(&mut dead);
        dead.sort_by_key(|&p| std::cmp::Reverse(self.players[p as usize].health));
        let first_place = alive - dead.len() + 1;
        for (k, &p) in dead.iter().enumerate() {
            let Game { data, players, pool, ghost, lobby, .. } = self;
            let pl = &mut players[p as usize];
            pl.alive = false;
            pl.placement = (first_place + k) as u8;
            pl.final_tribes = pl.board_tribes(&data.cards);
            if k == 0 {
                *ghost = Some(Ghost {
                    player: p,
                    board: pl.board.clone(),
                    tier: pl.tier,
                    counters: pl.counters(data, *lobby),
                });
            }
            pl.shop_spell = None;
            for m in pl.shop.drain(..).chain(pl.hand.drain(..)).chain(pl.board.drain(..)) {
                give_back(pool, &data.cards, &m);
            }
            for c in pl.discover.drain(..) {
                if !data.cards.get(c).is_spell() {
                    let slot = data.cards.get(c).pool_slot as usize;
                    pool[slot] += 1;
                }
            }
            pl.pending_choose_one = None;
            pl.pending_target = None;
            pl.pending_shop_target = None;
            pl.pending_discard = None;
            pl.choose_queue.clear();
            pl.tavern_buffs.clear();
            pl.discover_queue.clear();
            pl.power_offer.clear();
        }
        let mut left: ArrayVec<u8, PLAYERS> =
            (0..PLAYERS as u8).filter(|&p| self.players[p as usize].alive).collect();
        if left.len() <= 1 || self.turn >= self.data.rules.max_turns {
            self.rng.shuffle(&mut left);
            left.sort_by_key(|&p| std::cmp::Reverse(self.players[p as usize].health));
            for (k, &p) in left.iter().enumerate() {
                let pl = &mut self.players[p as usize];
                pl.placement = k as u8 + 1;
                pl.final_tribes = pl.board_tribes(&self.data.cards);
            }
            self.over = true;
        }
    }
}
