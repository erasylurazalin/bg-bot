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
use crate::minion::{
    CardId,
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
        // Kel'Thuzad's Kitty picks from the warband as it stands now: the
        // cards go back below.
        let board = self.players[p].board.clone();
        self.kitty_watch(p, &board, &[p as u8]);
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
            && pl.hand_queue.is_empty()
            && pl.board.is_empty()
            && pl.discover.is_empty()
            && pl.shop_spell.is_none()
            && pl.discover_queue.is_empty()
            && pl.pending_target.is_none()
            && pl.pending_shop_target.is_none()
            && pl.pending_choose_one.is_none()
            && pl.pending_discard.is_none()
            && pl.choose_queue.is_empty()
            && pl.power_offer.is_empty()
            && pl.secret_offer.is_empty();
        if idle {
            return;
        }
        pl.shop_spell = None;
        for m in pl
            .shop
            .drain(..)
            .chain(pl.hand.drain(..))
            .chain(pl.hand_queue.drain(..))
            .chain(pl.board.drain(..))
        {
            give_back(pool, &data.cards, &m);
        }
        pl.abandon_discover(pool, &data.cards);
        pl.pending_choose_one = None;
        pl.cast_after_choice = None;
        pl.pending_battlecries = 0;
        pl.pending_gold_spent = 0;
        pl.pending_target = None;
        pl.pending_shop_target = None;
        pl.pending_discard = None;
        pl.choose_queue.clear();
        pl.repeats.clear();
        pl.discover_damage_from = None;
        pl.discover_locked = false;
        pl.discover_locked_tier = 0;
        pl.discover_queue.clear();
        pl.repeat_discover = 0;
        pl.discover_silent = false;
        pl.tavern_buffs.clear();
        pl.power_offer.clear();
        pl.secret_offer.clear();
    }

    fn run_end_of_turn(&mut self, p: usize) {
        let runs = self.drakkari_mult(p);
        // The trinkets' end-of-turn triggers run before the board's: the log
        // of Hearthstone_2026_10_24_04_08_00-g1 turn 10 runs Sphere of
        // Memory's three copies before any Nightmare Corroder's Sludge
        // Corrosion, which the hand's ten-card limit makes visible.
        // docs/rules/design.md "Settled" has the rest of the end of turn.
        self.trinket_end_of_turn(p);
        let mut pos = 0;
        while pos < self.players[p].board.len() {
            self.run_minion_end_of_turn(p, &mut pos, runs);
            pos += 1;
        }
        // The Dark Gift countdowns (Incubation, Replication, Affinity) tick
        // and fire after the board's own effects (Hearthstone_2026_10_26
        // _00_48_15-g2 turn 5: Trench Fighter's Gem Confiscation lands before
        // the Replication gift's copy). Incubation only ticks here: its
        // doubling lands with the start of the next turn
        // (`dark_gift_start_of_turn`, log `10_28_19_47_59-g1` turn 7).
        self.dark_gift_end_of_turn(p);
        // The powers' own end-of-turn effects (Sulfuras gives your ends
        // +8/+8).
        self.power_end_of_turn(p);
        // A Lockbox is an end of turn countdown too, so Drakkari doubles it.
        self.tick_lockboxes(p, runs);
        // The Unlimited Coin comes back to hand at the end of every turn (a
        // generated copy, no pool copy), whatever became of the old one.
        if let Some(card) = self.players[p].prize.returns_each_turn {
            self.give_generated(p, card);
        }
    }

    /// One board minion's end of turn effects, `runs` times. A gift that puts
    /// its minion's end of turn effects at the start of the turn as well
    /// (Time Turning) runs the same code from `start_turn`.
    pub(super) fn run_minion_end_of_turn(&mut self, p: usize, pos: &mut usize, runs: u8) {
        // `pos` is a `&mut usize`; the shadow keeps the body reading like the
        // old local variable.
        let pos = &mut *pos;
        {
            let m = self.players[p].board[*pos];
            for _ in 0..runs {
                for (card, golden) in m.parts() {
                    let mul = if golden { 2 } else { 1 };
                    match self.data.cards.get(card).end_of_turn {
                        Some(EndOfTurn::BuffSelf { attack, health }) => {
                            buff(&mut self.players[p].board[*pos], attack * mul, health * mul);
                        }
                        Some(EndOfTurn::ResourcefulRobot) => {
                            let n = if golden { 2 } else { 1 };
                            for _ in 0..n {
                                // getting the Volumizer can change the board
                                // (a triple, a "when you get a card" trinket),
                                // so the Robot's slot may be gone (2026-10-31)
                                if *pos >= self.players[p].board.len() {
                                    break;
                                }
                                if let Some(v_id) = self.data.cards.random_volumizer(self.lobby, &mut self.rng)
                                {
                                    let v =
                                        self.players[p].make_card(&self.data.cards, v_id, false, 0);
                                    self.magnetize_to(p, *pos, v, false);
                                    self.give_generated_tracking(p, v_id, pos);
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
                            let imp = &mut self.players[p].board[*pos].improvements;
                            *imp = (*imp + 1).min((MAX_TIER - 1) as u16);
                        }
                        Some(EndOfTurn::GetCard { card: tok, count }) => {
                            let n = count * (mul as u8);
                            for _ in 0..n {
                                self.give_generated_tracking(p, tok, pos);
                            }
                        }
                        Some(EndOfTurn::Effect(e)) => {
                            self.run_effect(p, pos, e, golden, 1);
                        }
                        Some(EndOfTurn::CopyLastSpell) => {
                            if let Some(sp) = self.players[p].last_spell {
                                let n = mul as u8;
                                for _ in 0..n {
                                    self.give_generated_tracking(p, sp, pos);
                                }
                            }
                        }
                        // Dark Gifts Echoing Voice: its own Battlecries, now.
                        Some(EndOfTurn::TriggerOwnBattlecry) => {
                            let at = *pos;
                            self.run_battlecry_at(p, at, true);
                            if self.players[p].board.len() <= at {
                                return;
                            }
                        }
                        // Dark Gifts Polarization: a random Mech from the
                        // pool Magnetizes onto it, like Resourceful Robot's
                        // Volumizer.
                        Some(EndOfTurn::MagnetizeRandomMech) => {
                            if *pos >= self.players[p].board.len() {
                                break;
                            }
                            let Some(v_id) = self
                                .data
                                .cards
                                .random_magnetic_mech(self.lobby, &mut self.rng)
                            else {
                                break;
                            };
                            let v =
                                self.players[p].make_card(&self.data.cards, v_id, false, 0);
                            self.magnetize_to(p, *pos, v, false);
                            self.give_generated_tracking(p, v_id, pos);
                        }
                        // Dark Gifts Steady Growth: the +attack/+health the
                        // turn it landed gives it, read from the counter.
                        Some(EndOfTurn::GiftBuffSelf) => {
                            let landed = m.gift_state(card).unwrap_or(0) as u32;
                            let (a, h) = self.data.gifts.steady_growth.numbers(landed);
                            if *pos < self.players[p].board.len() {
                                buff(&mut self.players[p].board[*pos], a, h);
                            }
                        }
                        None => {}
                    }
                }
                // Gruul Rules: what a Prize gave this minion to gain at the
                // end of every turn (its own card, not a part). An earlier
                // part can have taken the slot away, so it is read again.
                if let Some(m) = self.players[p].board.get_mut(*pos) {
                    let (ga, gh) = m.end_turn_gain;
                    if (ga, gh) != (0, 0) {
                        buff(m, ga, gh);
                    }
                }
            }
        }
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
            || !pl.secret_offer.is_empty()
            || pl.wager_offer.is_some()
            || pl.holmes_offer.is_some()
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
            } else if !pl.discover.is_empty()
                || !pl.power_offer.is_empty()
                || pl.wager_offer.is_some()
                || pl.holmes_offer.is_some()
            {
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
            } else if !pl.discover_queue.is_empty() {
                // A Discover a Battlecry queued and nothing had started yet:
                // an end-of-turn Battlecry (a Dark Gift's Echoing Voice, Young
                // Murk-Eye triggering a Magicfin token's) leaves its Discover
                // in the queue, and the end of the turn cannot leave it
                // unanswered. The next turn of this loop takes its first
                // option, like every other choice here.
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
        self.resolve_after_play(p);
        // A Tomb Turning Undead that wasn't played this turn is a normal one.
        for m in &mut self.players[p].hand {
            m.doomed = false;
        }
        self.run_end_of_turn(p);
        // An end-of-turn effect can still ask (a Battlecry it triggers may be
        // a Choose One, Young Murk-Eye's neighbours): answer it now, so no
        // prompt outlives the turn it was opened in.
        self.resolve_open_choices(p);
        // Spellcraft: a spell a trinket handed out that was not cast leaves
        // the hand now (docs/rules/design.md "Spellcraft"). Leaving is not a discard, so
        // no discard trigger fires.
        {
            let cards = &self.data.cards;
            let pl = &mut self.players[p];
            let mut k = 0;
            while k < pl.hand.len() {
                if cards.get(pl.hand[k].card).spellcraft {
                    pl.hand.remove(k);
                } else {
                    k += 1;
                }
            }
            // A Makeshift Master's watch lasts this turn only.
            if pl.tp.deity_watching {
                for m in pl.board.iter_mut().chain(pl.hand.iter_mut()).chain(pl.shop.iter_mut()) {
                    m.watched = false;
                }
                pl.tp.deity_watching = false;
            }
        }
        // A Spellcraft spell leaving frees a hand slot, and a card the start
        // of the turn queued with a full hand joins as soon as there is room
        // (docs/rules/design.md "A full hand").
        self.drain_hand_queue(p);
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
                // copies, so nothing goes back. A card an effect put there
                // (a Lockbox a real seat opened at the end of its turn) does
                // hold copies, and those go back before the zones are
                // replaced (2026-10-31: the pool lost three).
                for m in pl.hand.iter().chain(pl.board.iter()) {
                    if m.pool_copies > 0 {
                        give_back(pool, &data.cards, m);
                    }
                }
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
            // Everything the start of the turn hands out (a compass trinket's
            // tribe card and the like) waits in the queue when the hand is
            // full, where a card got later in the turn would be lost
            // (docs/rules/design.md "A full hand"). The flag is cleared at the end of
            // this function, after every start-of-turn effect has run.
            pl.hand_queue_on = true;
            // the Beast bonus lasts until now; it never kills a Beast
            let (ba, bh) = std::mem::take(&mut pl.beast_bonus);
            if (ba, bh) != (0, 0) {
                for m in &mut pl.board {
                    if data.cards.tribes_of(m).has(Tribe::Beast) {
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
                // Reborn Rites' Reborn and Spirit Swap's Attack last until
                // the next turn too; a Reborn printed on the card stays.
                if m.temp_reborn {
                    if !data.cards.get(m.card).keywords.has(Keywords::REBORN) {
                        m.keywords.clear(Keywords::REBORN);
                    }
                    m.temp_reborn = false;
                }
                if m.temp_attack != 0 {
                    m.attack -= m.temp_attack;
                    m.temp_attack = 0;
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
                if m.temp_reborn {
                    if !data.cards.get(m.card).keywords.has(Keywords::REBORN) {
                        m.keywords.clear(Keywords::REBORN);
                    }
                    m.temp_reborn = false;
                }
                if m.temp_attack != 0 {
                    m.attack -= m.temp_attack;
                    m.temp_attack = 0;
                }
            }
            pl.done = false;
            pl.actions = 0;
            pl.gold_spent_turn = 0;
            pl.elementals_played_turn = 0;
            pl.spells_cast_turn = 0;
            // "This turn only" trinket numbers (Honeycomb Ring).
            pl.tp.spell_bonus_turn = (0, 0);
            // The Ice Block's Immunity covered the rest of that turn only.
            pl.immune = false;
            // The Prizes' per-turn numbers, and the free Refreshes Rocking
            // and Rolling and Open Bar give at every start of turn.
            let rolls = pl.prize.free_rolls_per_turn;
            pl.free_rolls = pl.free_rolls.saturating_add(rolls);
            pl.prize.spell_discount_turn = 0;
            pl.prize.battlecry_extra_turn = 0;
            let was_frozen = pl.frozen;
            let had_frozen = pl.shop.iter().any(|m| m.frozen);
            if pl.frozen {
                pl.frozen = false;
                for m in pl.shop.iter_mut() {
                    m.frozen = false;
                }
            } else {
                // A card frozen on its own (Twice as Nice froze the two cards
                // it copied) stays for one more turn, like a frozen Tavern.
                let mut kept: ArrayVec<Minion, { crate::SHOP }> = ArrayVec::new();
                for m in pl.shop.drain(..) {
                    if m.frozen {
                        let mut m = m;
                        m.frozen = false;
                        kept.push(m);
                    } else {
                        give_back(pool, &data.cards, &m);
                    }
                }
                pl.shop = kept;
                if !had_frozen {
                    pl.shop_spell = None;
                }
            }
            // Guiding Candle's two Tier 6 Refreshes come back each turn, and
            // the roll here is the first of them when the Tavern changed
            // (docs/rules/design.md, "Two kinds of Refresh").
            pl.tp.refresh_tier6 = pl.tp.refresh_tier6_base;
            // Replica Cathedral: a fresh "first spell" each turn.
            pl.tp.first_spell_double = pl.tp.first_spell_double_base;
            let size = pl.shop_size(&data.rules);
            // The roll at the start of a turn counts as a Refresh for
            // Waveling's amount and Barrage when the Tavern changed: always
            // when it was not frozen, and for a frozen one only when the roll
            // had to fill a slot (a bought card, a consumed or stolen one, a
            // new slot from levelling, an empty spell slot). It is still not
            // the player's own Refresh, so queued Fodders wait for an
            // explicit one.
            let refreshed =
                !was_frozen || had_frozen || pl.shop.len() < size || pl.shop_spell.is_none();
            let _ = Self::fill_shop(
                pl, pool, &data.cards, &data.rules, size, rng, self.lobby, RefreshKind::TurnStart,
            );
            if refreshed {
                Self::apply_refresh_buff(pl, &data.cards, rng);
                Self::apply_barrage(pl, &data.cards, rng);
                let bonus = data.rules.bonus_mask;
                Self::after_refresh_powers(pl, &data.cards, rng, bonus);
                pl.tp.refresh_tier6 = pl.tp.refresh_tier6.saturating_sub(1);
            }
        }
        for p in 0..PLAYERS {
            // The two hero powers at the start of the player's turn: the uses
            // back, the unlock conditions, and what a passive does now
            // (Clairvoyance's free Refresh, A Tale of Kings' new tribe, a
            // flightpath's countdown, Bananarama's Bananas).
            if self.players[p].alive && !self.players[p].real {
                self.power_start_of_turn(p);
            }
        }
        // Every turn of the new round has started, so Kel'Thuzad's Kitty has
        // queued its Discover for each warband of the round before it: the
        // warbands of older rounds are done with.
        let turn = self.turn;
        self.kitty_warbands.retain(|(died, _)| *died + 1 >= turn);
        for p in 0..PLAYERS {
            // Trinkets at the start of the player's turn: the two offers (and
            // the one a hero power owes), the counters, the uses back and the
            // start-of-turn effects.
            if self.players[p].alive && !self.players[p].real {
                self.trinket_start_of_turn(p);
            }
        }
        for p in 0..PLAYERS {
            // Dark Gifts at the start of the player's turn: the button is
            // pressable again, and a Time Turning minion runs its end of turn
            // effects again. The countdown gifts tick at the end of the turn.
            if self.players[p].alive && !self.players[p].real {
                self.players[p].dark_gift_pressed = false;
                self.dark_gift_start_of_turn(p);
            }
        }
        for p in 0..PLAYERS {
            // A real board's stats already include everything its thresholds
            // gave it in the real game, so they don't fire here.
            if self.players[p].alive && !self.players[p].real {
                self.check_attack_thresholds(p);
            }
        }
        // The start of the turn's own effects are done: a card they had to
        // queue with a full hand joins now if a slot opened, and anything got
        // from here on is lost rather than queued (docs/rules/design.md "A full hand").
        for p in 0..PLAYERS {
            if self.players[p].hand_queue_on {
                self.players[p].hand_queue_on = false;
                self.drain_hand_queue(p);
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
        // I Spy reads the next opponent's warband as it stands now, before
        // anyone has played this turn (docs/rules/heroes.md "Guessing").
        for p in 0..PLAYERS {
            let board = match self.players[p].opponent {
                Some(Opponent::Player(q)) => self.players[q as usize].board.clone(),
                Some(Opponent::Ghost(q)) => self
                    .ghost
                    .as_ref()
                    .filter(|g| g.player == q)
                    .map(|g| g.board.clone())
                    .unwrap_or_default(),
                None => ArrayVec::new(),
            };
            self.players[p].next_board = board;
        }
    }

    /// Take what a fight (or a Tavern attack, with `after_fight` false)
    /// leaves behind. After a Tavern attack the board is already up to date,
    /// so only the hand and the Tavern take the bonuses.
    pub fn apply_side_outcome(&mut self, p: usize, so: &SideOutcome, after_fight: bool) {
        // Nothing triples during the fights (docs/rules/design.md "Nothing
        // triples during combat"): what a fight gives the hand here (a Rally's
        // Beast, a Deathrattle's card, a trinket's or a hero power's) only
        // gets its triple check at the start of the next turn.
        let defer_triples = std::mem::replace(&mut self.defer_triples, after_fight);
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
        // A Secret that fired in this fight is gone from the zone for good
        // (docs/rules/heroes.md "Secrets").
        for kind in 0..crate::secrets::MAX_ZONE as u8 {
            if so.secrets_fired & (1 << kind) != 0 {
                self.players[p].secrets.remove(kind);
            }
        }
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
                typed_only: false,
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
            self.defer_triples = defer_triples;
            return;
        }
        // Trinket gains from the fight: free Refreshes (Herding Horn), the
        // trinket's own "improve this" numbers, and the deaths and summons
        // its shop-side "after N" counters are following (Bleeding Heart,
        // Wildfeather Duster). docs/rules/design.md: shop deaths count for these too, so
        // a shop destroy reports its dead the same way.
        let pl = &mut self.players[p];
        pl.free_rolls = pl.free_rolls.saturating_add(so.free_refresh_gain);
        for (slot, &n) in so.trinket_improve.iter().enumerate() {
            if n > 0 {
                let v = &mut pl.trinkets[slot].value;
                *v = v.saturating_add(n);
            }
        }
        if so.friendly_deaths > 0 {
            self.trinket_event(p, crate::trinkets::Event::Death, None, so.friendly_deaths as u32);
            self.power_friendly_deaths(p, so.friendly_deaths as u32);
        }
        if so.friendly_summons > 0 {
            self.trinket_event(p, crate::trinkets::Event::Summon, None, so.friendly_summons as u32);
        }
        for (i, &n) in so.summon_tribes.iter().enumerate() {
            if n > 0 {
                let t = crate::cards::Tribe::EVERY[i];
                self.trinket_event(p, crate::trinkets::Event::SummonTribe(t), None, n as u32);
            }
        }
        if so.friendly_attacks > 0 {
            self.trinket_event(p, crate::trinkets::Event::Attack, None, so.friendly_attacks as u32);
            // Heroic Inspiration and Demon Hunter Training count friendly attacks.
            self.power_attacks(p, so.friendly_attacks as u32);
        }
        if so.damage_dealt > 0 {
            self.power_damage_dealt(p, so.damage_dealt);
        }
        if so.broodmother_gain > 0 {
            self.power_broodmother_gain(p, so.broodmother_gain);
        }
        // Deaths of one named card (Snarky Portrait counts Fishbaits). The
        // bulk count above covers every other death counter, so a rule with
        // no card of its own ignores these.
        for i in 0..so.n_death_cards as usize {
            let (card, n) = so.death_cards[i];
            if n > 0 {
                self.trinket_event(p, crate::trinkets::Event::Death, Some(card), n as u32);
            }
        }
        for (i, &n) in so.death_tribes.iter().enumerate() {
            if n > 0 {
                let t = crate::cards::Tribe::EVERY[i];
                self.trinket_event(p, crate::trinkets::Event::DeathTribe(t), None, n as u32);
            }
        }
        for &c in so.cards_gained() {
            self.give_generated(p, c);
        }
        for &n in so.lockboxes_gained() {
            self.give_lockbox(p, n);
        }
        self.defer_triples = defer_triples;
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
                    let hand: &[Minion] = if reads_board(&q.board)
                        || reads_hand(&q.hand)
                        || q.trinket_reads_hand(&self.data)
                        || q.locked_and_loaded
                            .as_ref()
                            .is_some_and(|m| m.parts().any(|(c, _)| cards.get(c).reads_hand()))
                    {
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
                || self.players[p].trinket_reads_hand(&self.data)
                || self.players[p]
                    .locked_and_loaded
                    .as_ref()
                    .is_some_and(|m| m.parts().any(|(c, _)| cards.get(c).reads_hand()))
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
            // Reclaimed Souls: every minion that died in this fight, either
            // side, in the order they died. Both players get the same list.
            let mut dead_last: ArrayVec<CardId, 16> = ArrayVec::new();
            for so in &out.sides {
                for &c in &so.dead_cards[..so.n_dead_cards as usize] {
                    let _ = dead_last.try_push(c);
                }
            }
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
                    self.players[p].dead_last_fight = dead_last.clone();
                    self.players[q].dead_last_fight = dead_last;
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
                    self.players[p].dead_last_fight = dead_last;
                }
            }
        }
        for (p, d) in damage.into_iter().enumerate() {
            // A lost fight means the board wasn't in play, so no minion
            // reacts: no rewind, no "after your hero takes damage", no
            // damage event count. The hero just loses the Health (armor
            // first), and `knockouts` settles any that reach 0.
            let pl = &mut self.players[p];
            // A Safety Patch's Ice Block: the hit that would knock the hero
            // out is prevented and the hero is Immune for the rest of the
            // turn, once (docs/rules/trinkets.md).
            if !pl.immune && pl.secrets.ice_block() && d >= pl.health + pl.armor {
                pl.secrets.remove(crate::secrets::ICE_BLOCK);
                pl.immune = true;
                pl.last_damage = 0;
                pl.last_damage_armor = 0;
                pl.last_opponent = pl.opponent.take();
                continue;
            }
            if pl.immune {
                pl.last_opponent = pl.opponent.take();
                continue;
            }
            let to_armor = pl.armor.min(d);
            pl.armor -= to_armor;
            pl.health -= d - to_armor;
            pl.last_opponent = pl.opponent.take();
        }
        self.knockouts(alive);
        self.settle_wagers();
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
        let hand: &[Minion] = if reads_board(&pl.board)
            || reads_hand(&pl.hand)
            || pl.trinket_reads_hand(&self.data)
            || pl
                .locked_and_loaded
                .as_ref()
                .is_some_and(|m| m.parts().any(|(c, _)| cards.get(c).reads_hand()))
        {
            &pl.hand
        } else {
            &[]
        };
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
        self.settle_wagers();
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
        // Kel'Thuzad's Kitty picks from the dead warbands before they are
        // given back, and the players dying in this round get nothing.
        for &p in &dead {
            let board = self.players[p as usize].board.clone();
            self.kitty_watch(p as usize, &board, &dead);
        }
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
            pl.abandon_discover(pool, &data.cards);
            pl.pending_choose_one = None;
            pl.cast_after_choice = None;
            pl.pending_battlecries = 0;
            pl.pending_gold_spent = 0;
            pl.pending_target = None;
            pl.pending_shop_target = None;
            pl.pending_discard = None;
            pl.choose_queue.clear();
            pl.repeats.clear();
            pl.tavern_buffs.clear();
            pl.discover_queue.clear();
            pl.repeat_discover = 0;
            pl.discover_silent = false;
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
