//! The fight: a pure function of two boards, their counters, and an RNG.
//! The order of events follows `docs/rules/design.md` Part 1, and matches the
//! older private simulator's engine where the design leaves room (attack
//! pointer, Windfury swings, death order), so the two can be cross-checked.
//!
//! Rally, "whenever" and "after a friendly minion attacks" triggers, Avenge,
//! damage milestones and Deathrattles all run the same `Effect`s, through
//! `Side::run_effect`. The Tavern attack of a Fishbait runs through here too
//! (`tavern_attack`), so everything that reacts to an attack reacts to it.

use arrayvec::ArrayVec;

use crate::cards::{
    N_COUNTERS, AttackFilter, Aura, CardDb, Deathrattle, DeityKind, Effect, KillReward,
    OnSummonBeast, PermanentKeep, OnCastSpell, OnReborn, SpellEffect, StartOfCombat, TimesRole,
    Tribe, Tribes, how_many_times, pick_one_per_type,
};
use crate::minion::{Attached, CardId, Keywords, Minion};
use crate::trinkets::{FIGHT_ACTS, FightAct, FightEntry, GrantedDr, NO_CAP, Pick, Scale, Trigger, When};
use crate::rng::Rng;
use crate::{BOARD, HAND};

/// Deathrattles one Fish of N'Zoth can gain in one fight. A fight offers at
/// most seven friendly Deathrattle minions at a time and they can be
/// summoned again, so this is well past any real board. It also holds a dead
/// minion's own parts (its card, its attachments and its Dark Gifts) when it
/// fires its Deathrattles.
pub const FISH_DR: usize = 40;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Deity {
    pub card: CardId,
    pub attack: i32,
    pub health: i32,
    pub golden: bool,
}

/// Per-player counters passed into a fight.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Counters {
    /// The owner's hero Health; None when unknown. A fight reads nothing off
    /// it (Holy Vanguard's +30/+30 is in the board's own stats, the shop keeps
    /// it there), but a seat and the observations do.
    pub health: Option<i32>,
    pub beetle_attack: i32,
    pub beetle_health: i32,
    /// How many times each "improve your future ..." effect has triggered
    /// (`Counter::index`), which scales its next buff.
    pub improve: [u16; N_COUNTERS],
    /// The Beast bonus the player has until next turn (Goldrinn in the
    /// shop). Beasts already on the board carry it in their stats; Beasts
    /// that show up in the fight get it.
    pub beast_attack: i32,
    pub beast_health: i32,
    /// The owner's Tavern tier (Ghastcoiler picks Deathrattles up to it).
    /// 0 means unknown, which allows any tier.
    pub tier: u8,
    /// Gold the owner has spent this game, which a card locked in hand until
    /// a Gold count (Choose Your Champion) reads before it may be summoned
    /// from the hand.
    pub gold_spent: u32,
    pub boon_of_beetles: u8,
    pub brood_of_nozdormu: u8,
    pub upper_hand: u8,
    /// A hero power's Start of Combat: the keyword mask the left-most
    /// friendly minion gains before any minion's own Start of Combat runs
    /// (Al'Akir's Swatting Insects). 0 for none.
    pub soc_leftmost: u8,
    /// The keyword mask the RIGHT-most friendly minion gains (Bru'kan's Water
    /// Invocation).
    pub soc_rightmost: u8,
    /// How many times the left-most minion's Attack is doubled (Fire
    /// Invocation: once).
    pub soc_double_left: u8,
    /// How many random friendly minions get "Deathrattle: summon
    /// `soc_earth_card`" for this fight (Earth Invocation).
    pub soc_earth: u8,
    /// The token Earth Invocation's Deathrattle summons, 0 for none.
    pub soc_earth_card: CardId,
    /// How many random enemy minions take `soc_lightning_damage` damage
    /// (Lightning Invocation).
    pub soc_lightning: u8,
    pub soc_lightning_damage: i32,
    /// The owner's trinkets' effects inside the fight, as a small `Copy`
    /// bundle built from the trinket slots (`Player::counters`). Empty for a
    /// board without one, which every hook answers from one field.
    pub trinkets: FightTrinkets,
    /// Pending Sharing is Caring Start of Combat triggers.
    pub sharing_is_caring: u8,
    /// The player's Tavern spell bonus, for spells cast inside the fight
    /// (Runic Arcanist, Crimson Vindicator).
    pub spell_attack: i32,
    pub spell_health: i32,
    /// Timewarped Swirler: your Elementals give this much extra stats.
    pub elemental_attack: i32,
    pub elemental_health: i32,
    /// The player's Undead bonus "wherever they are" (Nerubian Deathswarmer,
    /// Plaguerunner, Butchering, Forsaken Weaver). Undead on the board and in
    /// the hand already carry it; Undead made in the fight get it.
    pub undead_attack: i32,
    pub undead_health: i32,
    /// "Your minions have +X/+Y" from a trinket (Hammer of Twilight, its
    /// base plus a step per card discarded). The board already carries it;
    /// a minion summoned in the fight gets it from `summon_with`.
    pub aura_attack: i32,
    pub aura_health: i32,
    /// Eternal Knights that died this game, which every Knight reads.
    pub knight_deaths: u32,
    /// What one of this side's Blood Gems gives (Sanguine Refiner's Rally
    /// raises it inside a fight, for good through `SideOutcome::gem_gain`).
    pub gem_attack: i32,
    pub gem_health: i32,
    pub deity: Option<Deity>,
    pub spells_cast: u32,
    /// The game's lobby: every card draw inside the fight (Ghastcoiler, a
    /// random Chromadrake, random spells) stays inside it. `Tribes::ALL`
    /// (a fight without a game behind it) draws from everything.
    pub lobby: crate::cards::Tribes,
    /// rules.toml's Bonus Keyword mask, which "is Reborn with full stats and
    /// Bonus Keywords" (Dark Gifts Persisting Horror) reads.
    pub bonus_keywords: u8,
    // ---- hero batch 3 ----
    /// Minions sold this game (Ozumat's Tentacular).
    pub minions_sold: u32,
    /// Wingmen (Illidan Stormrage) Start of Combat runs.
    pub wingmen: u8,
    /// ALL Will Burn! (Deathwing) Start of Combat runs.
    pub all_will_burn: u8,
    /// Wax Warband (Queen Wagtoggle) Start of Combat runs.
    pub wax_warband: u8,
    /// Fragrant Phylactery (Tamsin Roame) Start of Combat runs.
    pub phylactery: u8,
    /// Embrace Your Rage (Y'Shaarj) Start of Combat runs.
    pub embrace_rage: u8,
    /// Sprout It Out! (Greybough) combat summon buff runs.
    pub sprout_it_out: u8,
    /// Glory of Combat (Rokara) permanent Attack on kill runs.
    pub glory_of_combat: u8,
    /// I'll Take That! (Arch-Villain Rafaam) copies of the first killed enemy.
    pub take_first_killed: u8,
    /// Whether to count damage dealt by friendly minions (Bloodfury).
    pub count_damage: bool,
    /// Frostwolf Fervor (Drek'Thar) copies when space opens.
    pub frostwolf_fervor: u8,
    /// Stormpike Strength (Vanndar Stormpike) copies when space opens.
    pub stormpike_strength: u8,
    /// Tentacular (Ozumat): `(Tentacle card_id, runs)`.
    pub tentacular: Option<(CardId, u8)>,
    /// Broodmother (Onyxia): `(Onyxian Whelp card_id, bonus_stats, runs)`.
    pub broodmother: Option<(CardId, i32, u8)>,
    /// Lock and Load (Tavish Stormpike): `(card, golden, attack, health,
    /// keywords, runs)`.
    pub locked_and_loaded: Option<(CardId, bool, i32, i32, Keywords, u8)>,
    /// Diremuck Forager's owed summons: how many Murlocs it still has to pull
    /// out of the hand as board slots open. A real board file's `waits` sets
    /// it; a fight without one owes none.
    pub waits: u8,
    /// Rapid Reanimation (Teron Gorefiend) runs when a marked minion is on
    /// the board.
    pub rapid_reanimation: u8,
    /// The owner's Secret zone (docs/rules/heroes.md "Secrets"), in the order the
    /// Secrets were got: they fire in that order on the event each answers.
    /// Empty for a fight without a game behind it, which every hook answers
    /// from this one field.
    pub secrets: crate::secrets::Secrets,
    /// Timewarped Magnanimoose: one other player's warband as last seen,
    /// chosen by the game before the fight. Empty for a fight without a game
    /// behind it, which falls back to the actual opponent.
    pub warband: [Option<Minion>; BOARD],
    pub n_warband: u8,
}

/// Every player's Blood Gems start at +1/+1, so a fight without the player's
/// real counters still plays them at that size.
impl Default for Counters {
    fn default() -> Counters {
        Counters {
            health: None,
            beetle_attack: 0,
            beetle_health: 0,
            improve: [0; N_COUNTERS],
            beast_attack: 0,
            beast_health: 0,
            tier: 0,
            gold_spent: 0,
            boon_of_beetles: 0,
            brood_of_nozdormu: 0,
            upper_hand: 0,
            soc_leftmost: 0,
            soc_rightmost: 0,
            soc_double_left: 0,
            soc_earth: 0,
            soc_earth_card: 0,
            soc_lightning: 0,
            soc_lightning_damage: 0,
            trinkets: FightTrinkets::default(),
            sharing_is_caring: 0,
            spell_attack: 0,
            spell_health: 0,
            elemental_attack: 0,
            elemental_health: 0,
            warband: [None; BOARD],
            n_warband: 0,
            undead_attack: 0,
            undead_health: 0,
            aura_attack: 0,
            aura_health: 0,
            knight_deaths: 0,
            gem_attack: 1,
            gem_health: 1,
            deity: None,
            spells_cast: 0,
            lobby: crate::cards::Tribes::ALL,
            bonus_keywords: Keywords::TAUNT
                | Keywords::DIVINE_SHIELD
                | Keywords::WINDFURY
                | Keywords::VENOMOUS
                | Keywords::REBORN
                | Keywords::STEALTH,
            minions_sold: 0,
            wingmen: 0,
            all_will_burn: 0,
            wax_warband: 0,
            phylactery: 0,
            embrace_rage: 0,
            sprout_it_out: 0,
            glory_of_combat: 0,
            take_first_killed: 0,
            count_damage: false,
            frostwolf_fervor: 0,
            stormpike_strength: 0,
            tentacular: None,
            broodmother: None,
            locked_and_loaded: None,
            waits: 0,
            rapid_reanimation: 0,
            secrets: crate::secrets::Secrets::new(),
        }
    }
}

/// One of a side's trinket effects in a fight: what it does, when, and the
/// counters the fight keeps for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FightAction {
    pub act: FightAct,
    pub when: When,
    /// What the minion the trigger is about must be.
    pub trigger: Trigger,
    /// Fire on every Nth event (Avenge N).
    pub every: u8,
    /// Fire once per fight.
    pub once: bool,
    /// Uses left this fight, `NO_CAP` for no limit.
    pub uses: u8,
    /// Events counted this fight.
    pub count: u16,
    /// The trinket's own "improve this" number when the fight started.
    pub value: i16,
    /// The trinket slot it came from, for gains that go home.
    pub slot: u8,
    pub done: bool,
}

const NO_ACTION: FightAction = FightAction {
    act: FightAct::Improve(0),
    when: When::Soc,
    trigger: Trigger::Any,
    every: 1,
    once: false,
    uses: NO_CAP,
    count: 0,
    value: 0,
    slot: 0,
    done: false,
};

/// One side's trinkets, as the fight sees them: `Copy`, no allocation, and
/// no pointer into the shared card table, so a fight (and every hook in it)
/// pays nothing for trinkets it does not have.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FightTrinkets {
    acts: [FightAction; FIGHT_ACTS],
    n: u8,
    /// "Your first Deathrattle each combat triggers an extra time"
    /// (Deathly Phylactery).
    first_deathrattle: bool,
    first_deathrattle_used: bool,
    /// The owner's trinkets count friendly deaths and summons for their
    /// shop-side "after N" rules, so the fight has to report them.
    pub count_deaths: bool,
    pub count_summons: bool,
    /// The tribes a death has to be counted by (`death_<tribe>` rules), as a
    /// `Tribes` mask; 0 when none.
    pub count_death_tribes: u16,
}

impl Default for FightTrinkets {
    fn default() -> FightTrinkets {
        FightTrinkets {
            acts: [NO_ACTION; FIGHT_ACTS],
            n: 0,
            first_deathrattle: false,
            first_deathrattle_used: false,
            count_deaths: false,
            count_summons: false,
            count_death_tribes: 0,
        }
    }
}

impl FightTrinkets {
    pub fn is_empty(&self) -> bool {
        self.n == 0 && !self.count_deaths && !self.count_summons
    }

    pub fn len(&self) -> usize {
        self.n as usize
    }

    /// Add one trinket effect, from `slot`, with the slot's own "improve
    /// this" number. Past `FIGHT_ACTS` the act is dropped; the table check in
    /// `check_invariants` keeps that from happening.
    pub fn push(&mut self, e: &FightEntry, slot: u8, value: i16) {
        let n = self.n as usize;
        if n >= FIGHT_ACTS {
            return;
        }
        self.acts[n] = FightAction {
            act: e.act,
            when: e.when,
            trigger: e.trigger,
            every: e.every.max(1),
            once: e.once,
            uses: e.uses,
            count: 0,
            value,
            slot,
            done: false,
        };
        self.n += 1;
    }

    pub fn first_deathrattle(self) -> bool {
        self.first_deathrattle
    }

    pub fn set_first_deathrattle(&mut self, on: bool) {
        self.first_deathrattle = on;
    }

    /// The first Deathrattle of the fight gets one extra run; the rest do
    /// not (Deathly Phylactery).
    fn take_first_deathrattle(&mut self) -> bool {
        if !self.first_deathrattle || self.first_deathrattle_used {
            return false;
        }
        self.first_deathrattle_used = true;
        true
    }
}

/// Side effects from a fight that carry over to one player after combat.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SideOutcome {
    /// Permanent improvement stacks gained by the minion that started in each
    /// board slot (Lurking Leviathan, Treasure Parrot damage).
    pub permanent_improvements: [u16; BOARD],
    /// Increase to the player's Beetle bonus during this fight (Turquoise
    /// Skitterer, Ravaging Scorpid).
    pub beetle_attack_gain: i32,
    pub beetle_health_gain: i32,
    /// Increase to the player's improve counts (Tasty Lobster).
    pub improve_gain: [u16; N_COUNTERS],
    /// Stats kept permanently by the minion in each starting board slot
    /// (Tarecgosa, Persistent Poet). The game adds them to the board after
    /// the fight.
    pub permanent_stats: [(i32, i32); BOARD],
    /// Bonus Keywords kept permanently, by starting board slot.
    pub permanent_keywords: [u8; BOARD],
    /// Increase to the player's maximum Gold for good (Timewarped
    /// Recycler).
    pub max_gold_gain: u8,
    /// Increase to the player's Tavern spell bonus this game (Blue Whelp,
    /// and any spell bonus an effect grants in a fight).
    pub spell_bonus_gain: (i32, i32),
    /// Increase to the player's Beast bonus until next turn (Goldrinn). Only
    /// a Tavern attack keeps it, a fight's is gone with its board.
    pub beast_bonus_gain: (i32, i32),
    /// Deathrattles the player triggered in this fight (Falling Sky Golem).
    pub deathrattles_triggered: u16,
    /// Deathrattle triggers a Deathstrider caused here, its golden second
    /// trigger included.
    pub deathstrider_fires: u16,
    /// Generated cards added to the player's hand during this fight
    /// (Headhunter Gryphon, Conveyor Construct, Rescue Bot).
    pub cards: [CardId; HAND],
    pub n_cards: u8,
    /// Permanent stat gains for the minions in each starting hand slot
    /// (Very Hungry Winterfinner). The game adds them to the hand after the
    /// fight; combat changes to minions summoned from hand are not kept.
    pub hand_stats: [(i32, i32); HAND],
    /// Number of Boon of Beetles Beetles summoned during this fight.
    pub boon_of_beetles_used: u8,
    /// Tavern stats the player's minions gained for the rest of the game
    /// (Sacrificial Wrathguard's Deathrattle). The game applies it as an
    /// ongoing Tavern buff after the fight.
    pub tavern_buff_gain: (i32, i32),
    /// Increase to the player's "after the Tavern is Refreshed this game"
    /// amount (Waveling's Deathrattle).
    pub refresh_buff_gain: (i32, i32),
    /// Upcoming Refreshes that get Fodders (Trapped Clapper), oldest first.
    pub fodder_refreshes: [u8; 16],
    pub n_fodder_refreshes: u8,
    /// Lockboxes the player got in this fight, in order, each as the number
    /// of turns it opens sooner when one is already held (Locked-up
    /// Mutineer).
    pub lockboxes: [u8; 8],
    pub n_lockboxes: u8,
    /// Increase to the player's Undead bonus (Plaguerunner's Deathrattle).
    /// The game adds it to the Undead that are not in the fight's copy of the
    /// board.
    pub undead_gain: (i32, i32),
    /// Eternal Knights of the player that died in this fight.
    pub knight_deaths: u32,
    /// Increase to the player's Blood Gem value, for good (Sanguine Refiner's
    /// Rally, Sanguine Champion).
    pub gem_gain: (i32, i32),
    pub deity_gain: (i32, i32),
    pub spells_cast_gain: u32,
    /// Permanent Blood Gem stats gained by the minion in each starting board
    /// slot (Razorfen Vineweaver): the game adds them to the board minion's
    /// stats and Blood Gem stats after the fight.
    pub permanent_gems: [(i32, i32); BOARD],
    /// Free Refreshes a trinket gave in this fight (Herding Horn).
    pub free_refresh_gain: u16,
    /// Increase to each trinket slot's own "improve this" number, by slot
    /// (All-Purpose Kibble, Fang Anklet).
    pub trinket_improve: [u8; crate::TRINKET_SLOTS],
    /// Friendly minions that died here, for the trinkets whose "after N
    /// friendly minions die" counter carries across turns (Bleeding Heart).
    pub friendly_deaths: u16,
    /// Friendly deaths here, by `Tribe::EVERY` index, for a rule of a tribe
    /// ("after N friendly Beasts die").
    pub death_tribes: [u16; Tribe::EVERY.len()],
    /// Friendly deaths here of a named card, for a counter that names one
    /// (Snarky Portrait: a Fishbait).
    pub death_cards: [(CardId, u16); 4],
    pub n_death_cards: u8,
    /// Every minion that died on this side, tokens and summoned ones
    /// included, in the order they died: Reclaimed Souls picks from both
    /// sides' lists.
    pub dead_cards: [CardId; DEAD_CARDS],
    pub n_dead_cards: u8,
    /// Friendly minions summoned here, by `Tribe::EVERY` index, for the
    /// "after N Beasts summoned" counters (Wildfeather Duster).
    pub summon_tribes: [u16; Tribe::EVERY.len()],
    /// Everything summoned here, the neutral ones included, for a counter of
    /// summons whatever their tribe (Soul Battery).
    pub friendly_summons: u16,
    /// Friendly minions that attacked here, once per swing, so a Windfury
    /// minion's two attacks count 2 (Dragonhead Staff).
    pub friendly_attacks: u16,
    /// Damage dealt by friendly minions here (Bloodfury).
    pub damage_dealt: u32,
    /// Permanent +1/+1 steps Broodmother gained here (Onyxia).
    pub broodmother_gain: u32,
    /// The counters the real client shows during combat, captured at the
    /// first attack, after Start of Combat effects: what the other player
    /// "last saw" of this one.
    pub seen: SeenCounters,
    /// The Secrets that fired here, as a bit per `secrets` kind: each one is
    /// gone from the owner's zone for good (docs/rules/heroes.md "Secrets").
    pub secrets_fired: u8,
}

/// The Deity, the Blood Gem value and the Tavern spell bonus of one side at
/// one moment of its fight.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SeenCounters {
    pub deity: Option<Deity>,
    pub gem: (i32, i32),
    pub spell: (i32, i32),
}

/// The minions one side may record as dead for Reclaimed Souls.
pub const DEAD_CARDS: usize = 16;

impl SideOutcome {
    /// One more minion died here, whatever its side: a token and a summoned
    /// minion count (Reclaimed Souls can find a Beetle).
    pub fn note_dead_card(&mut self, card: CardId) {
        if (self.n_dead_cards as usize) < DEAD_CARDS {
            self.dead_cards[self.n_dead_cards as usize] = card;
            self.n_dead_cards += 1;
        }
    }

    /// One more friendly death of `card` here (Snarky Portrait's Fishbait).
    pub fn note_death_card(&mut self, card: CardId) {
        for i in 0..self.n_death_cards as usize {
            if self.death_cards[i].0 == card {
                self.death_cards[i].1 = self.death_cards[i].1.saturating_add(1);
                return;
            }
        }
        if (self.n_death_cards as usize) < self.death_cards.len() {
            self.death_cards[self.n_death_cards as usize] = (card, 1);
            self.n_death_cards += 1;
        }
    }

    pub fn cards_gained(&self) -> &[CardId] {
        &self.cards[..self.n_cards as usize]
    }

    pub fn lockboxes_gained(&self) -> &[u8] {
        &self.lockboxes[..self.n_lockboxes as usize]
    }

    fn push_lockbox(&mut self, sooner: u8) {
        let n = self.n_lockboxes as usize;
        if n < self.lockboxes.len() {
            self.lockboxes[n] = sooner;
            self.n_lockboxes += 1;
        }
    }

    fn push_card(&mut self, c: CardId) {
        let n = self.n_cards as usize;
        if n < HAND {
            self.cards[n] = c;
            self.n_cards += 1;
        }
    }

    /// Take back the last gained card `c` (a card that was summoned from the
    /// hand it was just put in).
    fn remove_card(&mut self, c: CardId) {
        let n = self.n_cards as usize;
        if let Some(i) = self.cards[..n].iter().rposition(|&x| x == c) {
            self.cards.copy_within(i + 1..n, i);
            self.n_cards -= 1;
        }
    }

    /// Queue `count` upcoming Refreshes that each get `per` Fodders. It
    /// stacks: Refreshes already queued get these on top.
    fn push_fodder_refreshes(&mut self, per: u8, count: u32) {
        for i in 0..count as usize {
            if i < self.n_fodder_refreshes as usize {
                self.fodder_refreshes[i] = self.fodder_refreshes[i].saturating_add(per);
            } else if i < self.fodder_refreshes.len() {
                self.fodder_refreshes[i] = per;
                self.n_fodder_refreshes += 1;
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// 0 for the first board, 1 for the second, None for a tie.
    pub winner: Option<usize>,
    /// Sum of the tiers of the winner's surviving minions, 0 on a tie.
    pub survivor_tiers: i32,
    pub attacks: u32,
    pub sides: [SideOutcome; 2],
}

/// Watches a fight, for tests and debugging. Every method defaults to doing
/// nothing, and `()` watches nothing, so a plain fight pays nothing for it.
pub trait Watch {
    fn attack(&mut self, _side: usize, _attacker: &Minion, _defender: &Minion) {}
    fn death(&mut self, _side: usize, _pos: usize, _m: &Minion) {}
    fn summon(&mut self, _side: usize, _pos: usize, _m: &Minion) {}
}

impl Watch for () {}

#[derive(Clone, Copy)]
struct Fm {
    m: Minion,
    uid: u16,
    start_slot: Option<u8>,
    /// Friendly minions that died while this one lived (Avenge).
    avenge: u16,
    /// Times this minion's "whenever this takes damage" trigger has fired
    /// this fight (Timewarped Piper's cap).
    damaged_fires: u8,
    /// Timewarped Pagle's once-per-combat Triple Reward.
    triple_reward_used: bool,
    /// The uid of the minion that dealt the damage that killed this one.
    killer: Option<u16>,
    /// Damage this minion's Health has taken in this fight. Its maximum
    /// Health is its Health plus this (Stalwart Kodo).
    taken: i32,
    /// How many times this minion's "after you summon" gift has fired this
    /// fight (Stalwart Kodo).
    gifts: u8,
}

/// Who an effect runs for and where it summons.
#[derive(Clone, Copy)]
struct Fx {
    side: usize,
    /// uid of the minion that has the effect (on the board, unless it died).
    host: u16,
    /// uid of the minion that killed the host, for a Deathrattle.
    killer: Option<u16>,
    /// uid of the minion that just attacked, for "give it +..." triggers.
    attacker: Option<u16>,
}

struct Side<'a> {
    ms: ArrayVec<Fm, BOARD>,
    /// The opponent's board as the fight began, for "a random opponent's
    /// warband" (Timewarped Magnanimoose).
    enemy: &'a [Minion],
    /// The hand as of the start of the fight, borrowed: it is empty unless
    /// something on the board (or in the hand) reads it. The fight never
    /// writes to it; combat-time gains live in `hand_bonus` and permanent
    /// ones in `SideOutcome::hand_stats`.
    hand: &'a [Minion],
    /// Combat-only stat gains on the hand, by hand slot.
    hand_bonus: [(i32, i32); HAND],
    /// Whether any minion on the board reacts to taking damage.
    damage_trigger: bool,
    /// A friendly minion was attacked: Timewarped Arm.
    attacked_trigger: bool,
    /// A Reborn copy is being placed: do not let summon triggers see it.
    no_summon_hooks: bool,
    /// Whether any minion on the board gives stats to minions summoned
    /// after it (Stalwart Kodo).
    gift_trigger: bool,
    /// Bit `i` marks the hand minion in slot `i` summoned for this combat
    /// only: later hand checks no longer see it.
    hand_taken: u16,
    /// Index of the next minion to attack.
    ptr: usize,
    counters: Counters,
    /// Combat-only buff to all Beasts on this side (Humming Bird, Goldrinn).
    beast_buff: (i32, i32),
    /// Buffs waiting for a minion that killed a Fishbait, by its uid. They
    /// are handed out by `resolve_deaths`, which sees both sides.
    killer_buffs: ArrayVec<(u16, i32, i32), 8>,
    /// Minions to destroy, by uid, from a "destroy the minion that killed
    /// this" Deathrattle (Leeroy). Applied by `resolve_deaths`, which sees
    /// both sides.
    killer_destroy: ArrayVec<u16, 8>,
    /// The cards of the minions that started the fight on the board
    /// (Lichling Hoarder copies one of them).
    start_cards: ArrayVec<CardId, BOARD>,
    /// Minions this fight's Deathly Strikers got, to be summoned from the hand
    /// by a Deathrattle.
    held: ArrayVec<CardId, 8>,
    /// What Stitched Salvagers destroyed: the host's uid and an exact copy
    /// of the minion as it stood, with the damage it had taken, for the
    /// Deathrattle to summon.
    salvage: ArrayVec<(u16, Minion, i32), 4>,
    /// Minions waiting to attack as soon as the current attack is over
    /// (Eternal Tycoon's Knight), by uid.
    immediate: ArrayVec<u16, 8>,
    /// The card whose Deathrattle is running ("a different Undead").
    dr_card: Option<CardId>,
    deity_left: u8,
    deity_awakened: bool,
    dead_aberrations: ArrayVec<Minion, 4>,
    /// This side is in the shop phase, not a fight (Plaguerunner gives more).
    outside: bool,
    /// Whether deaths count for Avenge. A shop destroy doesn't: its counts
    /// would start from 0 every time.
    count_avenge: bool,
    /// Deathrattles trinkets gave this side's minions for the fight, by the
    /// uid of the minion that carries them.
    granted: ArrayVec<(u16, GrantedDr), 8>,
    /// Deathrattles a Fish of N'Zoth gained from the friendly Deathrattle
    /// minions that died this fight, by the Fish's uid. They fire when the
    /// Fish dies, like its own.
    gained_dr: ArrayVec<(u16, Attached), FISH_DR>,
    /// Minions a Soul Fermenter destroyed at Start of Combat, with the slot
    /// they stood in, exactly as they were then. They come back when this
    /// side's last minion dies.
    fermenter: ArrayVec<(usize, Minion), 3>,
    /// Dead minions taken out of `ms` so a summon could use their board slot
    /// before the deaths were handled, with the slot each stood in. `resolve`
    /// still runs their Deathrattles and Reborn.
    held_dead: ArrayVec<(usize, Fm), BOARD>,
    /// Minions Rapid Reanimation destroyed at Start of Combat, with the slot
    /// they stood in, waiting for a board slot.
    /// The minions Rapid Reanimation destroyed at Start of Combat, with the
    /// slot they stood in and the damage each had taken.
    reanimate: ArrayVec<(usize, Minion, i32), BOARD>,
    /// Friendly minions destroyed by their own side at Start of Combat (Soul
    /// Fermenter, Stitched Salvagers, Rapid Reanimation), so I'll Take That!
    /// does not treat them as killed by the enemy.
    self_destroyed: ArrayVec<u16, BOARD>,
    /// The card of the first minion on this side killed by the enemy side
    /// (for the enemy's I'll Take That!).
    first_killed_by_enemy: Option<CardId>,
    /// Friendly deaths counted toward Broodmother's Avenge (4).
    broodmother_avenge: u16,
    frostwolf_left: u8,
    stormpike_left: u8,
    tentacular_left: u8,
    locked_and_loaded: Option<(CardId, bool, i32, i32, Keywords, u8)>,
    /// Diremuck Forager summons this side still owes the board.
    waits_left: u8,
    take_first_killed_left: u8,
    /// The minion that died last and where it stood (Blood Golem Sticker).
    last_dead: Option<Minion>,
    dead_at: usize,
    /// This side's Secret zone as the fight finds it: a Secret leaves it when
    /// it fires (`out.secrets_fired` says which), and one that summons with
    /// no room stays in it, unfired (docs/rules/heroes.md "Secrets").
    secrets: crate::secrets::Secrets,
    /// Redemption's copy waiting for a board slot, with the slot the minion
    /// died in. The Secret itself stays armed until this lands, and it owes
    /// one copy at most (there is one Redemption).
    secret_wait: Option<(usize, Minion)>,
    out: SideOutcome,
}


fn has_taunt(db: &CardDb, m: &Minion) -> bool {
    m.keywords.has(Keywords::TAUNT)
        || (m.keywords.has(Keywords::DIVINE_SHIELD)
            && m.parts().any(|(c, _)| db.get(c).aura == Some(Aura::TauntWhileShielded)))
}

fn filter_ok(db: &CardDb, f: AttackFilter, attacker: &Minion) -> bool {
    f.tribe.is_none_or(|t| db.tribes_of(attacker).has(t))
        && (!f.rally || attacker.parts().any(|(c, _)| db.get(c).rally.is_some()))
}

impl<'a> Side<'a> {
    fn new(
        db: &CardDb,
        board: &[Minion],
        hand: &'a [Minion],
        counters: Counters,
        uid: &mut u16,
    ) -> Side<'a> {
        let mut ms = ArrayVec::new();
        for (slot, &m) in board.iter().take(BOARD).enumerate() {
            ms.push(Fm { m, uid: *uid, start_slot: Some(slot as u8), avenge: 0, damaged_fires: 0, triple_reward_used: false, killer: None, taken: 0, gifts: 0 });
            *uid += 1;
        }
        let damage_trigger = ms
            .iter()
            .any(|f| f.m.parts().any(|(c, _)| db.get(c).when_damaged.is_some()));
        let attacked_trigger = ms
            .iter()
            .any(|f| f.m.parts().any(|(c, _)| db.get(c).when_friendly_attacked.is_some()));
        let gift_trigger = ms
            .iter()
            .any(|f| f.m.parts().any(|(c, _)| db.get(c).after_summon_give_stats.is_some()));
        // Holy Vanguard's +30/+30 (at 15 Health or less) is in the card's own
        // stats already: the shop puts it there and takes it off when the
        // Health crosses 15 (`Game::sync_vanguard`, docs/rules/design.md
        // "Triples"), so a fight adds nothing on top.
        Side {
            ms,
            enemy: &[],
            hand: &hand[..hand.len().min(HAND)],
            hand_bonus: [(0, 0); HAND],
            damage_trigger,
            attacked_trigger,
            no_summon_hooks: false,
            gift_trigger,
            hand_taken: 0,
            ptr: 0,
            counters,
            beast_buff: (counters.beast_attack, counters.beast_health),
            killer_buffs: ArrayVec::new(),
            killer_destroy: ArrayVec::new(),
            start_cards: board.iter().take(BOARD).map(|m| m.card).collect(),
            held: ArrayVec::new(),
            salvage: ArrayVec::new(),
            immediate: ArrayVec::new(),
            dr_card: None,
            deity_left: 4,
            deity_awakened: false,
            dead_aberrations: ArrayVec::new(),
            outside: false,
            count_avenge: true,
            granted: ArrayVec::new(),
            gained_dr: ArrayVec::new(),
            fermenter: ArrayVec::new(),
            held_dead: ArrayVec::new(),
            reanimate: ArrayVec::new(),
            self_destroyed: ArrayVec::new(),
            first_killed_by_enemy: None,
            broodmother_avenge: 0,
            frostwolf_left: counters.frostwolf_fervor,
            stormpike_left: counters.stormpike_strength,
            tentacular_left: counters.tentacular.map_or(0, |(_, n)| n),
            locked_and_loaded: counters.locked_and_loaded,
            waits_left: counters.waits,
            take_first_killed_left: counters.take_first_killed,
            last_dead: None,
            dead_at: 0,
            secrets: counters.secrets,
            secret_wait: None,
            out: SideOutcome::default(),
        }
    }

    /// A hand minion as it stands now: its card copy plus this combat's
    /// stat gains.
    fn hand_at(&self, i: usize) -> Minion {
        let mut m = self.hand[i];
        m.attack += self.hand_bonus[i].0;
        m.health += self.hand_bonus[i].1;
        m
    }

    fn hand_taken(&self, i: usize) -> bool {
        self.hand_taken & (1 << i) != 0
    }

    /// The hand minion with the highest Attack (or Health when `by_health`)
    /// that is still in hand; ties go to the left-most. Spells are not
    /// minions.
    fn best_in_hand(&self, db: &CardDb, tribe: Option<Tribe>, by_health: bool) -> Option<usize> {
        let mut best: Option<usize> = None;
        for (i, m) in self.hand.iter().enumerate() {
            if self.hand_taken(i)
                || db.get(m.card).is_spell()
                || tribe.is_some_and(|t| !db.tribes_of(m).has(t))
                // A card locked in hand (Imprison, Search Through Time,
                // Faelin's Tier, Thorim's Gold) cannot be summoned either, not
                // even by a Battlecry or a Start of Combat that reaches into
                // the hand.
                || m.locked_in_hand(self.counters.tier, self.counters.gold_spent)
            {
                continue;
            }
            // The best is the highest Attack, or the highest Health for
            // Bassgill and Sneed's New Shredder; a tie keeps the earlier one
            // (the lower hand slot).
            let stat = |k: usize| {
                if by_health {
                    self.hand[k].health + self.hand_bonus[k].1
                } else {
                    self.hand[k].attack + self.hand_bonus[k].0
                }
            };
            if best.is_none_or(|b| stat(i) > stat(b)) {
                best = Some(i);
            }
        }
        best
    }

    /// Summon the `n` highest-Attack (or highest-Health) minions from hand for
    /// this combat. A summoned copy leaves the hand; it is not put back.
    #[allow(clippy::too_many_arguments)]
    fn summon_from_hand(
        &mut self,
        db: &CardDb,
        side: usize,
        at: &mut usize,
        n: u32,
        tribe: Option<Tribe>,
        by_health: bool,
        shield: bool,
        uid: &mut u16,
        rng: &mut Rng,
        w: &mut impl Watch,
    ) {
        for _ in 0..n {
            if self.ms.is_full() {
                break;
            }
            let Some(i) = self.best_in_hand(db, tribe, by_health) else { break };
            let mut m = self.hand_at(i);
            if shield {
                m.keywords.set(Keywords::DIVINE_SHIELD);
            }
            self.hand_taken |= 1 << i;
            if !self.summon_baked(db, side, *at, m, uid, rng, w) {
                self.hand_taken &= !(1 << i);
                break;
            }
            *at += 1;
        }
    }

    /// Give the minion in hand slot `i` stats, permanently (the game adds
    /// them to the real hand after the fight).
    fn hand_buff(&mut self, db: &CardDb, i: usize, da: i32, dh: i32) {
        if db.get(self.hand[i].card).fixed_stats {
            return;
        }
        self.hand_bonus[i].0 = self.hand_bonus[i].0.saturating_add(da);
        self.hand_bonus[i].1 = self.hand_bonus[i].1.saturating_add(dh);
        self.out.hand_stats[i].0 = self.out.hand_stats[i].0.saturating_add(da);
        self.out.hand_stats[i].1 = self.out.hand_stats[i].1.saturating_add(dh);
    }

    /// Give a random minion still in hand stats (Very Hungry Winterfinner).
    fn buff_random_hand(&mut self, db: &CardDb, da: i32, dh: i32, rng: &mut Rng) {
        let mut ok: ArrayVec<usize, HAND> = ArrayVec::new();
        for (i, m) in self.hand.iter().enumerate() {
            if !self.hand_taken(i) && !db.get(m.card).is_spell() {
                ok.push(i);
            }
        }
        if ok.is_empty() {
            return;
        }
        let i = ok[rng.below(ok.len() as u32) as usize];
        self.hand_buff(db, i, da, dh);
    }

    /// A friendly minion took damage while in board slot `i`.
    fn when_damaged(
        &mut self,
        db: &CardDb,
        side: usize,
        i: usize,
        uid: &mut u16,
        rng: &mut Rng,
        w: &mut impl Watch,
    ) {
        // Most minions have nothing to do here; check before copying.
        if !self.ms[i].m.parts().any(|(c, _)| db.get(c).when_damaged.is_some()) {
            return;
        }
        let m = self.ms[i].m;
        let host = self.ms[i].uid;
        for (card, golden) in m.parts() {
            if let Some(e) = db.get(card).when_damaged {
                let max = db.get(card).when_damaged_max;
                if max > 0 && self.ms[i].damaged_fires >= max {
                    continue;
                }
                self.ms[i].damaged_fires = self.ms[i].damaged_fires.saturating_add(1);
                let fx = Fx { side, host, killer: None, attacker: None };
                let mut at = i + 1;
                self.run_effect(db, e, golden, fx, &mut at, uid, rng, w);
            }
        }
    }

    fn index_of(&self, uid: u16) -> Option<usize> {
        self.ms.iter().position(|f| f.uid == uid)
    }

    /// How much of a stat gain at board slot `i` is kept permanently: 1, 2
    /// for a golden keeper, None when nothing keeps it. Tarecgosa keeps its
    /// own gains; a Dragon keeps its when a living Persistent Poet sits next
    /// to it, or when a living Timewarped Poet is anywhere on its side (that
    /// one keeps its own gains too: it is one of "all your Dragons"), checked
    /// at the moment of the gain.
    fn keep_mul(&self, db: &CardDb, i: usize) -> Option<i32> {
        let m = &self.ms[i].m;
        let mut mul = 0;
        for (card, golden) in m.parts() {
            if db.get(card).permanent_keep == Some(PermanentKeep::SelfKeep) {
                mul = mul.max(if golden { 2 } else { 1 });
            }
        }
        if db.tribes_of(m).has(Tribe::Dragon) {
            for j in [i.checked_sub(1), Some(i + 1)].into_iter().flatten() {
                if j >= self.ms.len() || self.ms[j].m.health <= 0 {
                    continue;
                }
                for (card, golden) in self.ms[j].m.parts() {
                    if db.get(card).permanent_keep == Some(PermanentKeep::AdjacentDragons) {
                        mul = mul.max(if golden { 2 } else { 1 });
                    }
                }
            }
            for f in &self.ms {
                if f.m.health <= 0 {
                    continue;
                }
                for (card, golden) in f.m.parts() {
                    if db.get(card).permanent_keep == Some(PermanentKeep::AllDragons) {
                        mul = mul.max(if golden { 2 } else { 1 });
                    }
                }
            }
        }
        (mul > 0).then_some(mul)
    }

    /// Mishmash: "Whenever your Amalgam gains stats, this gains them too"
    /// (twice on a golden one). A gain an Amalgam takes in combat lands on
    /// every Mishmash of that side for the rest of the fight, through the
    /// ordinary `gain_at`, so it is gone when the fight ends unless a
    /// Persistent Poet next to it keeps it (Mishmash is every type, so it is
    /// a Dragon for the Poet).
    fn amalgam_gain_at(&mut self, db: &CardDb, i: usize, da: i32, dh: i32) {
        if !db.get(self.ms[i].m.card).amalgam || (da == 0 && dh == 0) {
            return;
        }
        let me = self.ms[i].uid;
        let mut hits: ArrayVec<(u16, i32, i32), BOARD> = ArrayVec::new();
        for f in &self.ms {
            if f.m.health <= 0 || f.uid == me {
                continue;
            }
            let n = f
                .m
                .parts()
                .map(|(c, golden)| {
                    db.get(c).amalgam_gain_copy as i32 * if golden { 2 } else { 1 }
                })
                .max()
                .unwrap_or(0);
            if n > 0 {
                let _ = hits.try_push((f.uid, da.saturating_mul(n), dh.saturating_mul(n)));
            }
        }
        for (uid, a, h) in hits {
            if let Some(j) = self.index_of(uid) {
                self.gain_at(db, j, a, h);
            }
        }
    }

    /// Give the minion in slot `i` stats. A keeper keeps them; Scarlet
    /// Survivor gains its keyword when the gain takes it to its threshold.
    fn gain_at(&mut self, db: &CardDb, i: usize, da: i32, dh: i32) {
        if db.get(self.ms[i].m.card).fixed_stats {
            return;
        }
        let keep = self.keep_mul(db, i);
        let mut threshold = 0u8;
        let f = &mut self.ms[i];
        f.m.attack = f.m.attack.saturating_add(da);
        f.m.health = f.m.health.saturating_add(dh);
        if let Some(t) = db.get(f.m.card).attack_threshold
            && f.m.improvements == 0
            && f.m.attack >= t.total as i32
        {
            f.m.keywords.set(t.keyword);
            f.m.improvements = 1;
            threshold = t.keyword;
        }
        let slot = f.start_slot;
        if let Some(mul) = keep
            && let Some(slot) = slot
        {
            let s = slot as usize;
            if threshold != 0 {
                self.out.permanent_keywords[s] |= threshold;
            }
            self.out.permanent_stats[s].0 =
                self.out.permanent_stats[s].0.saturating_add(da.saturating_mul(mul));
            self.out.permanent_stats[s].1 =
                self.out.permanent_stats[s].1.saturating_add(dh.saturating_mul(mul));
        }
        self.amalgam_gain_at(db, i, da, dh);
        if da > 0 {
            self.attack_gain_watchers(db, i, da);
        }
        self.note_gain_to_hand(db, i, da, dh);
    }

    /// Timewarped Expeditioner: this minion gained stats, so the two
    /// left-most minions in hand gain the same.
    fn note_gain_to_hand(&mut self, db: &CardDb, i: usize, da: i32, dh: i32) {
        if da == 0 && dh == 0 {
            return;
        }
        let mut gifts: ArrayVec<(i32, i32, u8), BOARD> = ArrayVec::new();
        for (card, golden) in self.ms[i].m.parts() {
            if let Some(count) = db.get(card).after_gain_stats_to_hand {
                let mul = if golden { 2 } else { 1 };
                let _ = gifts.try_push((da * mul, dh * mul, count));
            }
        }
        for (ga, gh, count) in gifts {
            let mut given = 0;
            for k in 0..self.hand.len() {
                if given >= count as usize {
                    break;
                }
                if !self.hand_taken(k) && !db.get(self.hand[k].card).is_spell() {
                    self.hand_buff(db, k, ga, gh);
                    given += 1;
                }
            }
        }
    }

    /// Timewarped Embalmer: one summoned minion each turn gains Reborn. A
    /// Reborn copy does not count (the flag is set around its insert).
    fn embalmer_on_summon(&mut self, db: &CardDb, pos: usize) {
        if self.no_summon_hooks || pos >= self.ms.len() {
            return;
        }
        let mut hosts: ArrayVec<usize, BOARD> = ArrayVec::new();
        for i in 0..self.ms.len() {
            if self.ms[i].m.health > 0
                && self.ms[i]
                    .m
                    .parts()
                    .any(|(c, _)| db.get(c).on_summon_give_reborn)
            {
                let _ = hosts.try_push(i);
            }
        }
        for i in hosts {
            if self.ms[i].m.uses_left == 0 {
                continue;
            }
            self.ms[i].m.uses_left -= 1;
            self.ms[pos].m.keywords.set(Keywords::REBORN);
        }
    }

    /// Timewarped Whelp Smuggler: a friendly minion gained Attack, so every
    /// one on this side gives it Health. The Health gain has no Attack, so
    /// it never re-enters this.
    fn attack_gain_watchers(&mut self, db: &CardDb, i: usize, da: i32) {
        if da <= 0 || i >= self.ms.len() {
            return;
        }
        let mut heals: ArrayVec<i32, BOARD> = ArrayVec::new();
        for f in &self.ms {
            if f.m.health <= 0 {
                continue;
            }
            for (card, golden) in f.m.parts() {
                if let Some((_, health)) = db.get(card).on_friendly_gain_attack {
                    let _ = heals.try_push(health * if golden { 2 } else { 1 });
                }
            }
        }
        for health in heals {
            self.gain_at(db, i, 0, health);
        }
    }

    /// Give the minion in slot `i` stats that it keeps after the fight
    /// regardless of other keepers (Devout Hellcaller).
    fn gain_permanent_at(&mut self, db: &CardDb, i: usize, da: i32, dh: i32) {
        // A shop destroy's Side is "outside" a real combat: its board is the
        // game's board when the dust settles, so the gain is a normal one.
        if self.outside {
            self.gain_at(db, i, da, dh);
            return;
        }
        if db.get(self.ms[i].m.card).fixed_stats {
            return;
        }
        let f = &mut self.ms[i];
        f.m.attack = f.m.attack.saturating_add(da);
        f.m.health = f.m.health.saturating_add(dh);
        if let Some(slot) = f.start_slot {
            let s = slot as usize;
            self.out.permanent_stats[s].0 = self.out.permanent_stats[s].0.saturating_add(da);
            self.out.permanent_stats[s].1 = self.out.permanent_stats[s].1.saturating_add(dh);
        }
        self.amalgam_gain_at(db, i, da, dh);
        if da > 0 {
            self.attack_gain_watchers(db, i, da);
        }
        self.note_gain_to_hand(db, i, da, dh);
    }

    /// A friendly Demon (`attacker_uid`) dealt damage: every other friendly
    /// minion with "After another friendly Demon deals damage" gains its
    /// stats for good (Devout Hellcaller).
    fn note_demon_damage(&mut self, db: &CardDb, attacker_uid: u16) {
        let mut hits: ArrayVec<(u16, i32, i32), BOARD> = ArrayVec::new();
        for f in &self.ms {
            if f.m.health <= 0 || f.uid == attacker_uid {
                continue;
            }
            for (card, golden) in f.m.parts() {
                if let Some((a, h)) = db.get(card).after_demon_damage {
                    let mul = if golden { 2 } else { 1 };
                    let _ = hits.try_push((f.uid, a * mul, h * mul));
                }
            }
        }
        for (host, da, dh) in hits {
            if let Some(i) = self.index_of(host) {
                self.gain_permanent_at(db, i, da, dh);
            }
        }
    }

    /// "Give your minions +attack/+health" cast inside a fight, with the
    /// player's spell bonus.
    fn cast_board_buff(&mut self, db: &CardDb, attack: i32, health: i32) {
        let (da, dh) = (attack + self.counters.spell_attack, health + self.counters.spell_health);
        for i in 0..self.ms.len() {
            if self.ms[i].m.health > 0 {
                self.gain_at(db, i, da, dh);
            }
        }
    }

    /// Mighty Dragonbreath: all minions, again for Dragons, again for
    /// minions with Divine Shield.
    fn cast_mighty_dragonbreath(&mut self, db: &CardDb, attack: i32, health: i32) {
        let (da, dh) = (attack + self.counters.spell_attack, health + self.counters.spell_health);
        for pass in 0..3 {
            for i in 0..self.ms.len() {
                if self.ms[i].m.health <= 0 {
                    continue;
                }
                let ok = match pass {
                    0 => true,
                    1 => db.tribes_of(&self.ms[i].m).has(Tribe::Dragon),
                    _ => self.ms[i].m.keywords.has(Keywords::DIVINE_SHIELD),
                };
                if ok {
                    self.gain_at(db, i, da, dh);
                }
            }
        }
    }

    /// The minion in slot `i` gains a keyword. A keeper keeps it.
    fn gain_keyword_at(&mut self, db: &CardDb, i: usize, keyword: u8) {
        self.ms[i].m.keywords.set(keyword);
        if self.keep_mul(db, i).is_some()
            && let Some(slot) = self.ms[i].start_slot
        {
            self.out.permanent_keywords[slot as usize] |= keyword;
        }
    }

    /// The minion in slot `i` gains a keyword it keeps after the fight
    /// regardless of other keepers (Timewarped Guard).
    fn gain_keyword_permanent_at(&mut self, i: usize, keyword: u8) {
        self.ms[i].m.keywords.set(keyword);
        if let Some(slot) = self.ms[i].start_slot {
            self.out.permanent_keywords[slot as usize] |= keyword;
        }
    }

    /// A friendly minion in `def_uid` was attacked: every Timewarped Arm on
    /// its side gives it Attack for good, before the hit lands. Golden doubles
    /// the amount.
    fn when_friendly_attacked(&mut self, db: &CardDb, def_uid: u16) {
        let Some(di) = self.index_of(def_uid) else { return };
        let mut hits: ArrayVec<(i32, i32), BOARD> = ArrayVec::new();
        for f in &self.ms {
            if f.m.health <= 0 {
                continue;
            }
            for (card, golden) in f.m.parts() {
                if let Some((attack, health)) = db.get(card).when_friendly_attacked {
                    let mul = if golden { 2 } else { 1 };
                    let _ = hits.try_push((attack * mul, health * mul));
                }
            }
        }
        for (attack, health) in hits {
            self.gain_permanent_at(db, di, attack, health);
        }
    }

    /// A friendly minion just lost Divine Shield: every Hopebringer on this
    /// side improves by one for the rest of the game. The count lives in its
    /// `improvements` and is carried to the player's real board.
    fn note_shield_lost(&mut self, db: &CardDb) {
        for i in 0..self.ms.len() {
            if self.ms[i].m.health <= 0 {
                continue;
            }
            let n = self.ms[i]
                .m
                .parts()
                .filter(|(c, _)| {
                    matches!(db.get(*c).start_of_combat, Some(StartOfCombat::Hopebringer { .. }))
                })
                .count() as u16;
            if n == 0 {
                continue;
            }
            self.ms[i].m.improvements = self.ms[i].m.improvements.saturating_add(n);
            if let Some(slot) = self.ms[i].start_slot {
                let s = slot as usize;
                self.out.permanent_improvements[s] =
                    self.out.permanent_improvements[s].saturating_add(n);
            }
        }
        // Timewarped Grease Bot: a friendly minion lost Divine Shield, so
        // every one on this side gives all friendly minions stats for good.
        let mut buffs: ArrayVec<(i32, i32), BOARD> = ArrayVec::new();
        for f in &self.ms {
            if f.m.health <= 0 {
                continue;
            }
            for (card, golden) in f.m.parts() {
                if let Some((attack, health)) = db.get(card).on_friendly_shield_lost {
                    let mul = if golden { 2 } else { 1 };
                    let _ = buffs.try_push((attack * mul, health * mul));
                }
            }
        }
        for (attack, health) in buffs {
            for i in 0..self.ms.len() {
                if self.ms[i].m.health > 0 {
                    self.gain_permanent_at(db, i, attack, health);
                }
            }
        }
    }

    /// Your Undead have +da/+dh this game, wherever they are: the living
    /// ones on the board, the ones in the hand (for a summon from it), and
    /// the ones summoned later. The game gives the rest through
    /// `SideOutcome::undead_gain`.
    fn gain_undead(&mut self, db: &CardDb, da: i32, dh: i32) {
        self.counters.undead_attack += da;
        self.counters.undead_health += dh;
        self.out.undead_gain.0 += da;
        self.out.undead_gain.1 += dh;
        for i in 0..self.ms.len() {
            if self.ms[i].m.health > 0 && db.tribes_of(&self.ms[i].m).has(Tribe::Undead) {
                self.gain_at(db, i, da, dh);
            }
        }
        for i in 0..self.hand.len() {
            let c = db.get(self.hand[i].card);
            if !self.hand_taken(i)
                && !c.is_spell()
                && !c.fixed_stats
                && db.tribes_of(&self.hand[i]).has(Tribe::Undead)
            {
                self.hand_bonus[i].0 = self.hand_bonus[i].0.saturating_add(da);
                self.hand_bonus[i].1 = self.hand_bonus[i].1.saturating_add(dh);
            }
        }
    }

    /// The side cast a Tavern spell in the fight: permanent bonuses leave in
    /// the outcome and combat buffs stay on this side.
    fn on_spell_cast(&mut self, db: &CardDb) {
        self.counters.spells_cast = self.counters.spells_cast.saturating_add(1);
        self.out.spells_cast_gain = self.out.spells_cast_gain.saturating_add(1);
        let mut triggers: ArrayVec<(u16, OnCastSpell, bool), 32> = ArrayVec::new();
        for f in &self.ms {
            if f.m.health <= 0 {
                continue;
            }
            for (card, golden) in f.m.parts() {
                let c = db.get(card);
                // A fight cast has no pick to wait for, so the "whenever" and
                // the "after" halves both fire here.
                for trigger in [c.on_cast_spell, c.on_cast_spell_after].into_iter().flatten() {
                    let _ = triggers.try_push((f.uid, trigger, golden));
                }
            }
        }
        for (host, trigger, golden) in triggers {
            let mul = if golden { 2 } else { 1 };
            match trigger {
                OnCastSpell::UndeadBonus { attack, health } => self.gain_undead(db, attack * mul, health * mul),
                OnCastSpell::BuffSelfAndDeity { attack, health } => {
                    if let Some(i) = self.index_of(host) {
                        self.gain_at(db, i, attack * mul, health * mul);
                    }
                    self.buff_deity(attack * mul, health * mul);
                }
                OnCastSpell::BuffAllAndDeity { attack, health } => {
                    for i in 0..self.ms.len() {
                        if self.ms[i].m.health > 0 {
                            self.gain_at(db, i, attack * mul, health * mul);
                        }
                    }
                    self.buff_deity(attack * mul, health * mul);
                }
                _ => {}
            }
        }
    }

    /// A friendly Eternal Knight died: every Knight, on the board or in the
    /// hand, grows (golden ones double), and the count goes up for the Knights
    /// summoned later.
    fn note_knight_death(&mut self, db: &CardDb) {
        self.counters.knight_deaths += 1;
        self.out.knight_deaths += 1;
        for i in 0..self.ms.len() {
            if self.ms[i].m.health <= 0 {
                continue;
            }
            if let Some((a, h)) = db.get(self.ms[i].m.card).buff_per_kind_death {
                let mul = self.ms[i].m.mul();
                self.gain_at(db, i, a * mul, h * mul);
            }
        }
        for i in 0..self.hand.len() {
            if self.hand_taken(i) {
                continue;
            }
            if let Some((a, h)) = db.get(self.hand[i].card).buff_per_kind_death {
                let mul = self.hand[i].mul();
                self.hand_bonus[i].0 = self.hand_bonus[i].0.saturating_add(a * mul);
                self.hand_bonus[i].1 = self.hand_bonus[i].1.saturating_add(h * mul);
            }
        }
    }

    /// The minion in slot `idx` just came back through Reborn: every other
    /// friendly minion that reacts to a Reborn does (Barrier Banshee, Snazzy
    /// Phantom). A minion never reacts to its own Reborn.
    fn note_reborn(&mut self, db: &CardDb, idx: usize, dead_slot: Option<u8>) {
        let rb_uid = self.ms[idx].uid;
        let rb_attack = self.ms[idx].m.attack;
        let mut hosts: ArrayVec<(u16, OnReborn, bool), BOARD> = ArrayVec::new();
        for f in &self.ms {
            if f.m.health <= 0 || f.uid == rb_uid {
                continue;
            }
            for (card, golden) in f.m.parts() {
                if let Some(r) = db.get(card).on_reborn {
                    let _ = hosts.try_push((f.uid, r, golden));
                }
            }
        }
        for (host, r, golden) in hosts {
            let mul = if golden { 2 } else { 1 };
            match r {
                OnReborn::Banshee { attack, health } => {
                    if let Some(i) = self.index_of(host) {
                        self.gain_at(db, i, attack * mul, health * mul);
                        self.ms[i].m.keywords.set(Keywords::DIVINE_SHIELD);
                    }
                }
                OnReborn::BuffAll { attack, health } => {
                    for i in 0..self.ms.len() {
                        if self.ms[i].m.health > 0 {
                            self.gain_permanent_at(db, i, attack * mul, health * mul);
                        }
                    }
                    // The reborn copy is a summoned minion with no starting
                    // slot. It still stands in the dead minion's slot, so
                    // its share of a permanent buff comes back there
                    // (Timewarped Jelly Belly). This runs only for a card
                    // with this OnReborn, so normal games are unchanged.
                    if let Some(slot) = dead_slot
                        && self.ms.get(idx).is_some_and(|f| {
                            f.start_slot.is_none() && f.m.health > 0
                        })
                    {
                        self.out.permanent_stats[slot as usize].0 = self.out.permanent_stats
                            [slot as usize]
                            .0
                            .saturating_add(attack * mul);
                        self.out.permanent_stats[slot as usize].1 = self.out.permanent_stats
                            [slot as usize]
                            .1
                            .saturating_add(health * mul);
                    }
                }
                OnReborn::Phantom => {
                    let last = self.ms.iter().rposition(|f| {
                        f.m.health > 0 && db.tribes_of(&f.m).has(Tribe::Undead)
                    });
                    if let Some(j) = last {
                        self.gain_at(db, j, rb_attack * mul, rb_attack * mul);
                    }
                }
            }
        }
    }

    fn next_attacker(&self) -> Option<usize> {
        let n = self.ms.len();
        if n == 0 {
            return None;
        }
        let p = self.ptr % n;
        (0..n).map(|k| (p + k) % n).find(|&i| self.ms[i].m.attack > 0)
    }

    fn remove(&mut self, i: usize) -> Fm {
        if i < self.ptr {
            self.ptr -= 1;
        }
        self.ms.remove(i)
    }

    /// False when the board is full and the minion is lost.
    /// Timewarped Magnanimoose: summon and get minions from the opponent's
    /// warband as the fight began. The hand copy is a plain card.
    #[allow(clippy::too_many_arguments)]
    fn summon_opponent_warband(
        &mut self,
        db: &CardDb,
        side: usize,
        mut pos: usize,
        count: usize,
        uid: &mut u16,
        rng: &mut Rng,
        w: &mut impl Watch,
    ) {
        let pool_b: ArrayVec<Minion, BOARD> = if self.counters.n_warband > 0 {
            self.counters.warband.iter().flatten().copied().collect()
        } else {
            self.enemy.iter().copied().collect()
        };
        if pool_b.is_empty() {
            return;
        }
        for _ in 0..count {
            let k = rng.below(pool_b.len() as u32) as usize;
            let mut m = pool_b[k];
            m.pool_copies = 0;
            if self.summon(db, side, pos, m, uid, rng, w) {
                pos += 1;
            }
            self.out.push_card(m.card);
        }
    }

    fn insert(&mut self, i: usize, f: Fm) -> bool {
        if self.ms.is_full() {
            return false;
        }
        self.ms.insert(i, f);
        if i < self.ptr {
            self.ptr += 1;
        }
        true
    }

    fn add_beetle_buff(&mut self, db: &CardDb, da: i32, dh: i32) {
        self.counters.beetle_attack += da;
        self.counters.beetle_health += dh;
        self.out.beetle_attack_gain += da;
        self.out.beetle_health_gain += dh;
        for i in 0..self.ms.len() {
            if self.ms[i].m.health > 0 && db.get(self.ms[i].m.card).is_beetle {
                self.gain_at(db, i, da, dh);
            }
        }
    }

    fn add_combat_beast_buff(&mut self, db: &CardDb, da: i32, dh: i32) {
        self.beast_buff.0 += da;
        self.beast_buff.1 += dh;
        self.out.beast_bonus_gain.0 += da;
        self.out.beast_bonus_gain.1 += dh;
        for i in 0..self.ms.len() {
            if self.ms[i].m.health > 0 && db.tribes_of(&self.ms[i].m).has(Tribe::Beast) {
                self.gain_at(db, i, da, dh);
            }
        }
    }

    /// A Deathrattle of this side triggered.
    fn note_deathrattle(&mut self, db: &CardDb) {
        self.out.deathrattles_triggered = self.out.deathrattles_triggered.saturating_add(1);
        for i in 0..self.ms.len() {
            if self.ms[i].m.health <= 0 {
                continue;
            }
            let m = self.ms[i].m;
            for (card, golden) in m.parts() {
                if let Some((a, h)) = db.get(card).buff_per_deathrattle {
                    let mul = if golden { 2 } else { 1 };
                    self.gain_at(db, i, a * mul, h * mul);
                }
            }
        }
        // Timewarped Ghoul-acabra: a Deathrattle was triggered, so every one
        // on this side gives all friendly minions stats for good.
        let mut buffs: ArrayVec<(i32, i32), BOARD> = ArrayVec::new();
        for f in &self.ms {
            if f.m.health <= 0 {
                continue;
            }
            for (card, golden) in f.m.parts() {
                if let Some((attack, health)) = db.get(card).after_deathrattle_buff_all {
                    let mul = if golden { 2 } else { 1 };
                    let _ = buffs.try_push((attack * mul, health * mul));
                }
            }
        }
        for (attack, health) in buffs {
            for i in 0..self.ms.len() {
                if self.ms[i].m.health > 0 {
                    self.gain_permanent_at(db, i, attack, health);
                }
            }
        }
    }

    /// The living friendly minions an act reaches, as board indices. `target`
    /// is the uid of the minion the trigger is about; a dead or missing one
    /// reaches nobody.
    fn pick(
        &self,
        db: &CardDb,
        pick: Pick,
        target: Option<u16>,
        rng: &mut Rng,
    ) -> ArrayVec<usize, BOARD> {
        let mut out: ArrayVec<usize, BOARD> = ArrayVec::new();
        let alive = |i: usize| self.ms[i].m.health > 0;
        let of_tribe = |i: usize, t: Tribe| db.tribes_of(&self.ms[i].m).has(t);
        match pick {
            Pick::Itself => {
                if let Some(i) = target.and_then(|t| self.index_of(t))
                    && alive(i)
                {
                    out.push(i);
                }
            }
            Pick::All => {
                for i in 0..self.ms.len() {
                    if alive(i) {
                        out.push(i);
                    }
                }
            }
            Pick::Tribe(t) => {
                for i in 0..self.ms.len() {
                    if alive(i) && of_tribe(i, t) {
                        out.push(i);
                    }
                }
            }
            Pick::Neutral => {
                for i in 0..self.ms.len() {
                    if alive(i) && db.tribes_of(&self.ms[i].m).is_empty() {
                        out.push(i);
                    }
                }
            }
            Pick::Ends { tribe } => {
                let ok = |i: usize| alive(i) && tribe.is_none_or(|t| of_tribe(i, t));
                if let Some(i) = (0..self.ms.len()).find(|&i| ok(i)) {
                    out.push(i);
                }
                if let Some(i) = (0..self.ms.len()).rev().find(|&i| ok(i))
                    && out.first() != Some(&i)
                {
                    out.push(i);
                }
            }
            Pick::Random { tribe, count } => {
                let mut ok: ArrayVec<usize, BOARD> = ArrayVec::new();
                for i in 0..self.ms.len() {
                    if alive(i) && tribe.is_none_or(|t| of_tribe(i, t)) {
                        ok.push(i);
                    }
                }
                for _ in 0..count {
                    if ok.is_empty() {
                        break;
                    }
                    let k = rng.below(ok.len() as u32) as usize;
                    out.push(ok.remove(k));
                }
            }
            Pick::LowestAttack(count) => {
                let mut ok: ArrayVec<usize, BOARD> =
                    (0..self.ms.len()).filter(|&i| alive(i)).collect();
                for _ in 0..count {
                    if ok.is_empty() {
                        break;
                    }
                    let low = ok.iter().map(|&i| self.ms[i].m.attack).min().unwrap_or(0);
                    let ties: ArrayVec<usize, BOARD> = ok
                        .iter()
                        .copied()
                        .filter(|&i| self.ms[i].m.attack == low)
                        .collect();
                    let i = ties[rng.below(ties.len() as u32) as usize];
                    ok.retain(|j| *j != i);
                    out.push(i);
                }
            }
            Pick::Rally => {
                for i in 0..self.ms.len() {
                    if alive(i) && self.ms[i].m.parts().any(|(c, _)| db.get(c).rally.is_some()) {
                        out.push(i);
                    }
                }
            }
            Pick::Card(want) => {
                for i in 0..self.ms.len() {
                    if alive(i) && self.ms[i].m.parts().any(|(c, _)| c == want) {
                        out.push(i);
                    }
                }
            }
        }
        out
    }

    /// A friendly minion died, a minion was summoned, a minion attacked, a
    /// minion was Reborn or the Deity awakened: run the trinket effects that
    /// listen for it. A side without trinket effects answers from one field.
    #[allow(clippy::too_many_arguments)]
    fn trinket_hook(
        &mut self,
        db: &CardDb,
        side: usize,
        when: When,
        target: Option<(u16, Minion)>,
        uid: &mut u16,
        rng: &mut Rng,
        w: &mut impl Watch,
    ) {
        if self.counters.trinkets.n == 0 {
            return;
        }
        // A shop destroy's deaths count toward nothing (its Avenge counts
        // would start from 0 every time, and a trinket's Avenge is Avenge).
        if when == When::Avenge && !self.count_avenge {
            return;
        }
        // Which `when`s need a minion to be about: the per-minion triggers.
        // Start of Combat, Avenge and a spell cast are the side's own.
        let trigger_only = !matches!(when, When::Soc | When::Avenge | When::CastSpell);
        for k in 0..self.counters.trinkets.len() {
            let mut e = self.counters.trinkets.acts[k];
            if e.when != when || e.done || e.uses == 0 {
                continue;
            }
            match target {
                Some((_, m)) if e.trigger.fits(db, &m) => {}
                Some(_) => continue,
                None if trigger_only => continue,
                None => {}
            }
            e.count = e.count.saturating_add(1);
            if !e.count.is_multiple_of(e.every.max(1) as u16) {
                self.counters.trinkets.acts[k] = e;
                continue;
            }
            if e.uses != NO_CAP {
                e.uses -= 1;
            }
            if e.once {
                e.done = true;
            }
            self.counters.trinkets.acts[k] = e;
            self.run_fight_act(db, side, k, target.map(|t| t.0), uid, rng, w);
        }
    }

    /// Run one of the side's trinket effects (entry `k` of the bundle).
    #[allow(clippy::too_many_arguments)]
    fn run_fight_act(
        &mut self,
        db: &CardDb,
        side: usize,
        k: usize,
        target: Option<u16>,
        uid: &mut u16,
        rng: &mut Rng,
        w: &mut impl Watch,
    ) {
        let e = self.counters.trinkets.acts[k];
        let value = e.value as i32;
        match e.act {
            FightAct::Keywords { pick, keywords } => {
                for i in self.pick(db, pick, target, rng) {
                    self.gain_keyword_at(db, i, keywords);
                }
            }
            FightAct::Stats { pick, attack, health, grow } => {
                // "Improve this" grows each number the act gives, so a
                // +3/+0 stays +0 Health however often it improves.
                let grown = |n: i32| if n == 0 { 0 } else { grow * value };
                let (da, dh) = (attack + grown(attack), health + grown(health));
                for i in self.pick(db, pick, target, rng) {
                    self.gain_at(db, i, da, dh);
                }
            }
            FightAct::Scale { pick, kind } => {
                for i in self.pick(db, pick, target, rng) {
                    let (a, h) = (self.ms[i].m.attack, self.ms[i].m.health);
                    let (da, dh) = match kind {
                        Scale::Double => (a, h),
                        Scale::Triple => (2 * a, 2 * h),
                        Scale::Health => (0, h),
                        Scale::Attack => (a, 0),
                    };
                    self.gain_at(db, i, da, dh);
                }
            }
            FightAct::HalfAttackToHealth { pick } => {
                for i in self.pick(db, pick, target, rng) {
                    let a = self.ms[i].m.attack;
                    // "Half" rounds up (docs/rules/design.md).
                    let h = if a > 0 { (a + 1) / 2 } else { 0 };
                    self.gain_at(db, i, 0, h);
                }
            }
            FightAct::TriggerDeathrattles { pick } => {
                let uids: ArrayVec<u16, BOARD> =
                    self.pick(db, pick, target, rng).iter().map(|&i| self.ms[i].uid).collect();
                for u in uids {
                    let Some(i) = self.index_of(u) else { continue };
                    if self.ms[i].m.health <= 0 {
                        continue;
                    }
                    let (m, host) = (self.ms[i].m, u);
                    let runs = self.deathrattle_fires(db);
                    self.fire_deathrattles(
                        db, side, &m, m.health, host, None, i + 1, runs, uid, rng, w,
                    );
                }
            }
            FightAct::GrantDr { pick, dr } => {
                for i in self.pick(db, pick, target, rng) {
                    let u = self.ms[i].uid;
                    let _ = self.granted.try_push((u, dr));
                }
            }
            FightAct::HighestHandAttack { pick } => {
                let mut best = 0;
                for (i, m) in self.hand.iter().enumerate() {
                    if !self.hand_taken(i) && !db.get(m.card).is_spell() {
                        best = best.max(m.attack + self.hand_bonus[i].0);
                    }
                }
                for i in self.pick(db, pick, target, rng) {
                    self.gain_at(db, i, best, 0);
                }
            }
            FightAct::Gems { pick, count } => {
                for i in self.pick(db, pick, target, rng) {
                    self.gem_at(db, i, count as i32, false, false, rng);
                }
            }
            FightAct::Summon { card, count, keywords } => {
                // A trinket has no source minion, so its summons go to the
                // right end (docs/rules/design.md).
                let mut at = self.ms.len();
                for _ in 0..count {
                    if !self.summon(db, side, at, db.make(card, false), uid, rng, w) {
                        break;
                    }
                    self.ms[at].m.keywords.set(keywords);
                    at += 1;
                }
            }
            FightAct::Bonus { gem, spell } => {
                if gem != (0, 0) {
                    self.counters.gem_attack += gem.0;
                    self.counters.gem_health += gem.1;
                    self.out.gem_gain.0 += gem.0;
                    self.out.gem_gain.1 += gem.1;
                }
                if spell != (0, 0) {
                    self.counters.spell_attack += spell.0;
                    self.counters.spell_health += spell.1;
                    self.out.spell_bonus_gain.0 += spell.0;
                    self.out.spell_bonus_gain.1 += spell.1;
                }
            }
            FightAct::RandomSpell(n) => {
                for _ in 0..n {
                    if let Some(c) = db.random_spell(self.counters.tier, self.counters.lobby, rng) {
                        self.out.push_card(c);
                    }
                }
            }
            FightAct::RandomMagnetic(n) => {
                for _ in 0..n {
                    if let Some(c) =
                        db.random_magnetic_minion(self.counters.tier, self.counters.lobby, rng)
                    {
                        self.out.push_card(c);
                    }
                }
            }
            FightAct::FreeRefreshes(n) => {
                self.out.free_refresh_gain = self.out.free_refresh_gain.saturating_add(n);
            }
            FightAct::Improve(amount) => {
                let slot = e.slot;
                let n = self.counters.trinkets.len();
                for a in self.counters.trinkets.acts[..n].iter_mut() {
                    if a.slot == slot {
                        a.value = a.value.saturating_add(amount as i16);
                    }
                }
                let s = slot as usize % crate::TRINKET_SLOTS;
                self.out.trinket_improve[s] =
                    self.out.trinket_improve[s].saturating_add(amount);
            }
            FightAct::CopyToHand { pick } => {
                for i in self.pick(db, pick, target, rng) {
                    self.out.push_card(self.ms[i].m.card);
                }
            }
            FightAct::PermanentStats { pick, attack, health } => {
                // Stats the minions keep after the fight (Bewitched
                // Ribbon's in-combat numbers): the gain goes home by start
                // slot, like Tarecgosa's.
                for i in self.pick(db, pick, target, rng) {
                    self.gain_permanent_at(db, i, attack, health);
                }
            }
            FightAct::Fermenter { count } => {
                // Soul Fermenter: the `count` left-most minions are
                // destroyed now and held, exactly as they stand, until this
                // side's last minion dies (docs/rules/trinkets.md).
                let n = (count as usize).min(self.ms.len());
                for k in 0..n {
                    let m = self.ms[k].m;
                    let _ = self.fermenter.try_push((k, m));
                    let _ = self.self_destroyed.try_push(self.ms[k].uid);
                }
                for k in 0..n {
                    self.ms[k].m.health = 0;
                }
            }
            FightAct::GemGolem { card } => {
                // Blood Golem Sticker: a Golem with the Blood Gem stats of
                // the Quilboar that just died, where it stood. The Golem is
                // Neutral (its token has no race), so its own death is no
                // Quilboar death and the fight cannot chain Golems for ever.
                let dead = self.last_dead.unwrap_or_else(|| db.make(card, false));
                let (ga, gh) = (dead.gem_attack, dead.gem_health);
                let mut m = db.make(card, false);
                m.attack = ga.max(1);
                m.health = gh.max(1);
                let at = self.dead_at;
                self.summon(db, side, at, m, uid, rng, w);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn summon(
        &mut self,
        db: &CardDb,
        side: usize,
        pos: usize,
        m: Minion,
        uid: &mut u16,
        rng: &mut Rng,
        w: &mut impl Watch,
    ) -> bool {
        self.summon_with(db, side, pos, m, uid, rng, w, true)
    }

    /// Summon a minion that already carries the player's bonuses (a hand
    /// minion, an exact copy): the Undead and Knight bonuses are not added.
    #[allow(clippy::too_many_arguments)]
    fn summon_baked(
        &mut self,
        db: &CardDb,
        side: usize,
        pos: usize,
        m: Minion,
        uid: &mut u16,
        rng: &mut Rng,
        w: &mut impl Watch,
    ) -> bool {
        self.summon_with(db, side, pos, m, uid, rng, w, false)
    }

    #[allow(clippy::too_many_arguments)]
    fn summon_with(
        &mut self,
        db: &CardDb,
        side: usize,
        pos: usize,
        mut m: Minion,
        uid: &mut u16,
        rng: &mut Rng,
        w: &mut impl Watch,
        bonus: bool,
    ) -> bool {
        if self.ms.is_full() {
            // A dead minion does not hold a board slot. The log of Era's
            // 2026-11-17 fight against Tickatus (Power.log 274478) shows a
            // golden Ravaging Scorpid's Deathrattle, fired by Deathstrider
            // after the attacking Headhunter Gryphon died, filling that
            // Gryphon's slot before the DEATHS block ran: one Beetle fitted,
            // the second was lost. Take the left-most dead minion out and let
            // `resolve` handle its death after this summon, so its own
            // Deathrattle and Reborn still run in their place.
            let Some(j) = self.ms.iter().position(|f| f.m.health <= 0) else {
                return false;
            };
            let fm = self.remove(j);
            let _ = self.held_dead.try_push((j, fm));
        }
        let pos = pos.min(self.ms.len());
        let c = db.get(m.card);
        if bonus && !c.fixed_stats {
            // "Your minions have +X/+Y" (Hammer of Twilight): the board
            // carries it already, so only a minion summoned here needs it.
            m.attack += self.counters.aura_attack;
            m.health += self.counters.aura_health;
            if db.tribes_of(&m).has(Tribe::Undead) {
                m.attack += self.counters.undead_attack;
                m.health += self.counters.undead_health;
            }
            if let Some((a, h)) = c.buff_per_kind_death {
                let n = self.counters.knight_deaths as i32 * m.mul();
                m.attack += a * n;
                m.health += h * n;
            }
        }
        if c.when_damaged.is_some() {
            self.damage_trigger = true;
        }
        if c.when_friendly_attacked.is_some() {
            self.attacked_trigger = true;
        }
        if c.after_summon_give_stats.is_some() {
            self.gift_trigger = true;
        }
        if c.is_beetle {
            m.attack += self.counters.beetle_attack;
            m.health += self.counters.beetle_health;
        }
        self.insert(
            pos,
            Fm { m, uid: *uid, start_slot: None, avenge: 0, damaged_fires: 0, triple_reward_used: false, killer: None, taken: 0, gifts: 0 },
        );
        *uid += 1;
        self.embalmer_on_summon(db, pos);
        // Sprout It Out! (Greybough): minions summoned during combat gain
        // +1/+2 and Taunt.
        if !self.outside && self.counters.sprout_it_out > 0 {
            let n = self.counters.sprout_it_out as i32;
            self.gain_at(db, pos, n, 2 * n);
            self.gain_keyword_at(db, pos, Keywords::TAUNT);
        }
        if db.tribes_of(&self.ms[pos].m).has(Tribe::Beast) {
            self.ms[pos].m.attack += self.beast_buff.0;
            self.ms[pos].m.health += self.beast_buff.1;
            for j in 0..self.ms.len() {
                if j == pos || self.ms[j].m.health <= 0 {
                    continue;
                }
                let imp = self.ms[j].m.improvements;
                let start_slot = self.ms[j].start_slot;
                let mut da_total = 0;
                let mut dh_total = 0;
                let mut imp_gain = 0u16;
                for (card, golden) in self.ms[j].m.parts() {
                    match db.get(card).on_summon_beast {
                        Some(OnSummonBeast::Buff { attack, health, improve_permanently }) => {
                            let mul = if golden { 2 } else { 1 };
                            let stacks = if improve_permanently {
                                1 + (imp + imp_gain) as i32
                            } else {
                                1
                            };
                            da_total += attack * mul * stacks;
                            dh_total += health * mul * stacks;
                            if improve_permanently {
                                imp_gain += 1;
                            }
                        }
                        Some(OnSummonBeast::BuffOnSpell { attack, health }) => {
                            let mul = if golden { 2 } else { 1 };
                            let stacks = 1 + (imp + imp_gain) as i32;
                            da_total += attack * mul * stacks;
                            dh_total += health * mul * stacks;
                        }
                        _ => {}
                    }
                }
                if imp_gain > 0 {
                    self.ms[j].m.improvements = self.ms[j].m.improvements.saturating_add(imp_gain);
                    if let Some(slot) = start_slot {
                        let s = slot as usize;
                        self.out.permanent_improvements[s] =
                            self.out.permanent_improvements[s].saturating_add(imp_gain);
                    }
                }
                self.gain_at(db, pos, da_total, dh_total);
            }
            // Banana Slamma sets the Beast's base Attack to twice (golden:
            // three times) its current Attack and keeps every buff on top, so
            // a 65 Attack Beetle with +63 of buffs goes to 130 + 63 = 193
            // (Power.log 2026_11_11_02_43_24 around line 152571; an unbuffed
            // 9/4 goes to 18/4). The base starts at the printed Attack and
            // moves with each Slamma.
            let mut base = db.make(self.ms[pos].m.card, self.ms[pos].m.golden).attack;
            for j in 0..self.ms.len() {
                if j == pos || self.ms[j].m.health <= 0 {
                    continue;
                }
                let mj = self.ms[j].m;
                for (card, golden) in mj.parts() {
                    if matches!(db.get(card).on_summon_beast, Some(OnSummonBeast::MultiplyAttackInCombat)) {
                        let factor = if golden { 3 } else { 2 };
                        let attack = self.ms[pos].m.attack;
                        let buffs = attack - base;
                        base = attack * factor;
                        self.ms[pos].m.attack = (base + buffs).max(0);
                    }
                }
            }
        }
        // Stalwart Kodo: each one on the board gives the new minion a
        // multiple of its current Attack and maximum Health (double, triple
        // when golden), a few times per fight. Its "a different minion" skips
        // another copy of itself: a summoned Kodo gets nothing from a Kodo.
        if self.gift_trigger {
            for j in 0..self.ms.len() {
                if j == pos || self.ms[j].m.health <= 0 {
                    continue;
                }
                let mj = self.ms[j].m;
                for (card, golden) in mj.parts() {
                    let Some(g) = db.get(card).after_summon_give_stats else { continue };
                    if self.ms[j].gifts >= g.times {
                        continue;
                    }
                    if g.different && self.ms[pos].m.parts().any(|(c, _)| c == card) {
                        continue;
                    }
                    self.ms[j].gifts += 1;
                    let mul = if golden { g.golden_multiply } else { g.multiply };
                    let max_health = mj.health + self.ms[j].taken;
                    self.gain_at(db, pos, mj.attack * mul, max_health * mul);
                }
            }
        }
        // The trinkets that listen for a summon (Mama Bear Sticker, Reinforced
        // Shield, Slamma Sticker) run on the minion as it now stands, and the
        // ones whose "after N Beasts summoned" counter carries across turns
        // count it for the shop. They run before the watcher sees it, so a
        // test watching the summon sees what the fight sees.
        if self.counters.trinkets.count_summons {
            self.out.friendly_summons = self.out.friendly_summons.saturating_add(1);
            let tribes = db.tribes_of(&m);
            for (i, t) in Tribe::EVERY.into_iter().enumerate() {
                if tribes.has(t) {
                    self.out.summon_tribes[i] = self.out.summon_tribes[i].saturating_add(1);
                }
            }
        }
        let new = self.ms[pos].m;
        self.trinket_hook(db, side, When::Summon, Some((self.ms[pos].uid, new)), uid, rng, w);
        w.summon(side, pos, &self.ms[pos].m);
        true
    }

    /// A Secret fires: it leaves the zone for good, and the outcome says
    /// which one so the player's zone loses it after the fight.
    fn fire_secret(&mut self, kind: u8) {
        if self.secrets.remove(kind) {
            self.out.secrets_fired |= 1 << kind;
        }
    }

    /// The Secrets that answer "when one of your minions is attacked", in the
    /// order they were got (docs/rules/heroes.md "Secrets"): Autodefense Matrix
    /// gives the minion Divine Shield before the hit lands, Pack Tactics
    /// summons a 3/3 copy of it to its right and Venomstrike Trap a 2/3
    /// Venomous Cobra there. A Secret that would summon stays in the zone,
    /// unfired, when the board is full.
    fn attack_secrets(
        &mut self,
        db: &CardDb,
        side: usize,
        def_uid: u16,
        uid: &mut u16,
        rng: &mut Rng,
        w: &mut impl Watch,
    ) {
        // Only a real fight: a Tavern attack and a shop destroy are not one.
        if self.outside || self.secrets.is_empty() {
            return;
        }
        let mut held: ArrayVec<u8, { crate::secrets::MAX_ZONE }> = ArrayVec::new();
        for k in self.secrets.iter() {
            let _ = held.try_push(k);
        }
        for kind in held {
            let Some(di) = self.index_of(def_uid) else { return };
            match kind {
                crate::secrets::AUTODEFENSE_MATRIX => {
                    self.ms[di].m.keywords.set(Keywords::DIVINE_SHIELD);
                    self.fire_secret(kind);
                }
                crate::secrets::PACK_TACTICS => {
                    if self.ms.is_full() {
                        continue;
                    }
                    // An exact copy (keywords, Deathrattles, gifts, the cards
                    // Magnetized into it) whose stats are set to 3/3: the
                    // base stats, so ordinary buffs are dropped. The bonuses
                    // that apply wherever the card is (the Undead bonus, an
                    // Eternal Knight's deaths, Goldrinn's Beasts, a trinket's
                    // "your minions have") go on top, from `summon`, exactly
                    // as they do for any newly summoned minion.
                    let mut copy = self.ms[di].m;
                    copy.attack = 3;
                    copy.health = 3;
                    copy.pool_copies = 0;
                    if self.summon(db, side, di + 1, copy, uid, rng, w) {
                        self.fire_secret(kind);
                    }
                }
                crate::secrets::VENOMSTRIKE_TRAP => {
                    let Some(cobra) = db.cobra_token else { continue };
                    if self.ms.is_full() {
                        continue;
                    }
                    let mut m = db.make(cobra, false);
                    m.pool_copies = 0;
                    if self.summon(db, side, di + 1, m, uid, rng, w) {
                        self.fire_secret(kind);
                    }
                }
                _ => {}
            }
        }
    }

    /// Redemption: a friendly minion died, so a fresh copy of it (its base
    /// card, Golden kept) comes back with 1 Health where it stood. With no
    /// room the copy waits for a slot and the Secret stays armed, so a
    /// Deathrattle that filled the board does not spend it.
    #[allow(clippy::too_many_arguments)]
    fn redemption(
        &mut self,
        db: &CardDb,
        side: usize,
        dead: &Minion,
        at: usize,
        uid: &mut u16,
        rng: &mut Rng,
        w: &mut impl Watch,
    ) {
        if self.outside || !self.secrets.has(crate::secrets::REDEMPTION) {
            return;
        }
        // The Secret already owes a copy to an earlier death: that one comes
        // back when there is room, not this minion's (one Secret, one save).
        if self.secret_wait.is_some() {
            return;
        }
        let mut m = db.make(dead.card, dead.golden);
        m.health = 1;
        let at = at.min(self.ms.len());
        if self.summon(db, side, at, m, uid, rng, w) {
            self.fire_secret(crate::secrets::REDEMPTION);
        } else {
            self.secret_wait = Some((at, m));
        }
    }

    fn spawn_boon_of_beetles(
        &mut self,
        db: &CardDb,
        side: usize,
        uid: &mut u16,
        rng: &mut Rng,
        w: &mut impl Watch,
    ) {
        if self.counters.boon_of_beetles == 0 || self.ms.is_full() {
            return;
        }
        let Some(tok) = db.beetle_token else { return };
        while self.counters.boon_of_beetles > 0 && !self.ms.is_full() {
            let mut m = db.make(tok, false);
            m.keywords.set(Keywords::TAUNT);
            let pos = self.ms.len();
            if self.summon(db, side, pos, m, uid, rng, w) {
                self.counters.boon_of_beetles -= 1;
                self.out.boon_of_beetles_used = self.out.boon_of_beetles_used.saturating_add(1);
            } else {
                break;
            }
        }
    }

    /// Pick a living friendly minion with the highest Attack (or Health when
    /// `by_health`), breaking ties at random.
    fn pick_highest(&self, by_health: bool, rng: &mut Rng) -> Option<usize> {
        let stat = |i: usize| if by_health { self.ms[i].m.health } else { self.ms[i].m.attack };
        let high = (0..self.ms.len()).filter(|&i| self.ms[i].m.health > 0).map(&stat).max()?;
        let ties: ArrayVec<usize, BOARD> = (0..self.ms.len())
            .filter(|&i| self.ms[i].m.health > 0 && stat(i) == high)
            .collect();
        Some(ties[rng.below(ties.len() as u32) as usize])
    }

    /// An exact copy of the friendly minion in slot `i` as it stands right
    /// now (docs/rules/design.md "Exact copies"): buffs, keywords and gifts included,
    /// the damage it has taken carried over and any keyword it has lost in
    /// this fight gone from the copy too, and no pool copy taken. The damage
    /// it has taken comes back beside the copy, so a copy summoned later
    /// keeps the original's maximum Health.
    fn exact_copy(&self, i: usize) -> (Minion, i32) {
        let mut copy = self.ms[i].m;
        copy.pool_copies = 0;
        (copy, self.ms[i].taken)
    }

    /// Summon an exact copy the fight made earlier, carrying the damage the
    /// original had taken: a 10/10 at 4 Health comes back as 10/4 whose
    /// maximum Health is still 10 (its `taken` is the original's, like the
    /// Reborn copy's). Rapid Reanimation and Stitched Salvager go through it
    /// too, so there is one exact-copy path in the fight.
    #[allow(clippy::too_many_arguments)]
    fn summon_saved_copy(
        &mut self,
        db: &CardDb,
        side: usize,
        pos: usize,
        copy: Minion,
        damage: i32,
        uid: &mut u16,
        rng: &mut Rng,
        w: &mut impl Watch,
    ) -> bool {
        if !self.summon_baked(db, side, pos, copy, uid, rng, w) {
            return false;
        }
        // `summon_baked` clamps `pos` to the end of the board, which is where
        // a copy from the last slot goes anyway.
        let at = pos.min(self.ms.len() - 1);
        self.ms[at].taken = damage;
        true
    }

    /// `summon_saved_copy` for a copy taken from the board right now.
    #[allow(clippy::too_many_arguments)]
    fn summon_exact_copy(
        &mut self,
        db: &CardDb,
        side: usize,
        pos: usize,
        i: usize,
        uid: &mut u16,
        rng: &mut Rng,
        w: &mut impl Watch,
    ) -> bool {
        let (copy, damage) = self.exact_copy(i);
        self.summon_saved_copy(db, side, pos, copy, damage, uid, rng, w)
    }

    /// Summon anything waiting for a board slot (`Rapid Reanimation`, `Lock
    /// and Load`, `Frostwolf Fervor`, `Stormpike Strength`, `Tentacular`).
    fn spawn_when_space(
        &mut self,
        db: &CardDb,
        side: usize,
        uid: &mut u16,
        rng: &mut Rng,
        w: &mut impl Watch,
    ) -> bool {
        if self.outside || self.ms.is_full() {
            return false;
        }
        let mut changed = false;
        while !self.ms.is_full() && !self.reanimate.is_empty() {
            let (slot, m, damage) = self.reanimate.remove(0);
            let at = slot.min(self.ms.len());
            if self.summon_baked(db, side, at, m, uid, rng, w) {
                self.ms[at].taken = damage;
                changed = true;
            }
        }
        if !self.ms.is_full()
            && let Some((card, golden, attack, health, keywords, mut left)) =
                self.locked_and_loaded.take()
        {
            while left > 0 && !self.ms.is_full() {
                left -= 1;
                let mut m = db.make(card, golden);
                m.pool_copies = 0;
                m.no_reward = true;
                m.attack = attack;
                m.health = health;
                m.keywords = keywords;
                let at = self.ms.len();
                if self.summon_baked(db, side, at, m, uid, rng, w) {
                    let u = self.ms[at].uid;
                    let _ = self.immediate.try_push(u);
                    changed = true;
                }
            }
            if left > 0 {
                self.locked_and_loaded = Some((card, golden, attack, health, keywords, left));
            }
        }
        // Redemption's copy, waiting for the Deathrattle that filled the
        // board to make room (docs/rules/heroes.md "Secrets").
        if !self.ms.is_full()
            && let Some((slot, m)) = self.secret_wait
        {
            let at = slot.min(self.ms.len());
            if self.summon(db, side, at, m, uid, rng, w) {
                self.secret_wait = None;
                self.fire_secret(crate::secrets::REDEMPTION);
                changed = true;
            }
        }
        while !self.ms.is_full() && self.frostwolf_left > 0 {
            let Some(pick) = self.pick_highest(false, rng) else { break };
            self.frostwolf_left -= 1;
            if self.summon_exact_copy(db, side, pick + 1, pick, uid, rng, w) {
                changed = true;
            }
        }
        while !self.ms.is_full() && self.stormpike_left > 0 {
            let Some(pick) = self.pick_highest(true, rng) else { break };
            self.stormpike_left -= 1;
            if self.summon_exact_copy(db, side, pick + 1, pick, uid, rng, w) {
                changed = true;
            }
        }
        if let Some((card, _)) = self.counters.tentacular {
            while !self.ms.is_full() && self.tentacular_left > 0 {
                self.tentacular_left -= 1;
                let mut m = db.make(card, false);
                let bonus = self.counters.minions_sold as i32;
                m.attack = m.attack.saturating_add(bonus);
                m.health = m.health.saturating_add(bonus);
                let at = self.ms.len();
                if self.summon(db, side, at, m, uid, rng, w) {
                    changed = true;
                }
            }
        }
        // Diremuck Forager's owed summons. Its own Start of Combat reaches into
        // the hand through `summon_from_hand`; a board that was full when it
        // ran carries the count in the file's `waits`, and each one comes out
        // as a slot opens.
        while !self.ms.is_full() && self.waits_left > 0 {
            let before = self.ms.len();
            let mut at = before;
            self.summon_from_hand(
                db,
                side,
                &mut at,
                1,
                Some(Tribe::Murloc),
                false,
                false,
                uid,
                rng,
                w,
            );
            if self.ms.len() == before {
                break;
            }
            self.waits_left -= 1;
            changed = true;
        }
        changed
    }

    /// How many times one Deathrattle fires for this side: once, plus one
    /// for each Titus Rivendare and two for a golden one ("an extra time"
    /// stacks). Every path that fires a Deathrattle asks here, so a new way
    /// to repeat them (trinkets, quest rewards) is added in this one place.
    /// Fish of N'Zoth: the minion with uid `dead_uid` died with these
    /// Deathrattles, so each friendly minion that gains Deathrattles this way
    /// takes a copy of each (twice when the Fish is golden). It fires them
    /// when it dies, and each one only once: the Fish takes only the dead
    /// minion's own Deathrattles, not the ones it had already gained.
    fn gain_deathrattles(&mut self, db: &CardDb, dead_uid: u16, dead: &Minion) {
        let mut got: ArrayVec<Attached, 8> = ArrayVec::new();
        for (card, golden) in dead.parts() {
            if db.get(card).deathrattle.is_some() {
                let _ = got.try_push(Attached { card, golden });
            }
        }
        let mut got_granted: ArrayVec<GrantedDr, 8> = ArrayVec::new();
        if !dead.parts().any(|(c, _)| db.get(c).gains_friendly_deathrattles) {
            for &(u, dr) in &self.granted {
                if u == dead_uid {
                    let _ = got_granted.try_push(dr);
                }
            }
        }
        if got.is_empty() && got_granted.is_empty() {
            return;
        }
        for i in 0..self.ms.len() {
            let uid = self.ms[i].uid;
            if uid == dead_uid || self.ms[i].m.health <= 0 {
                continue;
            }
            let times = self.ms[i]
                .m
                .parts()
                .filter(|&(c, _)| db.get(c).gains_friendly_deathrattles)
                .map(|(_, golden)| if golden { 2 } else { 1 })
                .max()
                .unwrap_or(0);
            for _ in 0..times {
                for &a in &got {
                    let _ = self.gained_dr.try_push((uid, a));
                }
                for &dr in &got_granted {
                    let _ = self.granted.try_push((uid, dr));
                }
            }
        }
    }

    fn deathrattle_fires(&mut self, db: &CardDb) -> u32 {
        let n = {
            let parts = self.ms.iter().filter(|f| f.m.health > 0).flat_map(|f| f.m.parts());
            how_many_times(db, parts, |a| match a {
                Aura::Deios => Some(TimesRole::Setter),
                Aura::Titus => Some(TimesRole::Adder),
                _ => None,
            }) as u32
        };
        // Deathly Phylactery: the first Deathrattle of the fight fires once
        // more than it otherwise would ("each combat": a shop destroy is not
        // one).
        n + u32::from(!self.outside && self.counters.trinkets.take_first_deathrattle())
    }

    /// How many Blood Gems a minion plays when one lands on it (Geomagus
    /// Roogug, either part of a multi-part card). Golden doubles it.
    fn gem_reaction(&self, db: &CardDb, m: &Minion) -> u8 {
        m.parts()
            .find_map(|(c, g)| {
                db.get(c).on_gem_played_on_self.map(|n| n * if g { 2 } else { 1 })
            })
            .unwrap_or(0)
    }

    /// Play `times` Blood Gems on the minion in slot `i`: its stats for this
    /// fight and, when `permanent` (Razorfen Vineweaver), stats and Blood Gem
    /// stats the game gives the board minion after the fight. Every Gem
    /// played on a Geomagus Roogug makes it play its own on a random
    /// different living friendly minion, the same as in the shop
    /// (`Game::gem_on_board`, docs/rules/design.md "Geomagus Roogug").
    /// `from_roogug` is the mark a Gem carries when a Roogug played it: a
    /// Roogug there reacts to every Gem but another Roogug's, so two Roogugs
    /// never loop.
    fn gem_at(
        &mut self,
        db: &CardDb,
        i: usize,
        times: i32,
        permanent: bool,
        from_roogug: bool,
        rng: &mut Rng,
    ) {
        let da = self.counters.gem_attack.saturating_mul(times);
        let dh = self.counters.gem_health.saturating_mul(times);
        if db.get(self.ms[i].m.card).fixed_stats {
            return;
        }
        let f = &mut self.ms[i];
        f.m.attack = f.m.attack.saturating_add(da);
        f.m.health = f.m.health.saturating_add(dh);
        if permanent
            && let Some(slot) = f.start_slot
        {
            let s = slot as usize;
            self.out.permanent_stats[s].0 = self.out.permanent_stats[s].0.saturating_add(da);
            self.out.permanent_stats[s].1 = self.out.permanent_stats[s].1.saturating_add(dh);
            self.out.permanent_gems[s].0 = self.out.permanent_gems[s].0.saturating_add(da);
            self.out.permanent_gems[s].1 = self.out.permanent_gems[s].1.saturating_add(dh);
        }
        if !from_roogug {
            self.roogug_on_gem(db, i, times, rng);
        }
    }

    /// Each of the `gems` Blood Gems just played on the minion in slot
    /// `i` makes a Geomagus Roogug there play its own Gems, on a random
    /// different living friendly minion. Those Gems carry the from-Roogug
    /// mark, so they never make another Roogug react and never carry on
    /// into the game (`permanent` is false).
    fn roogug_on_gem(&mut self, db: &CardDb, i: usize, gems: i32, rng: &mut Rng) {
        if gems <= 0 || self.ms[i].m.health <= 0 {
            return;
        }
        let times = self.gem_reaction(db, &self.ms[i].m) as i32;
        if times == 0 {
            return;
        }
        let mut others: ArrayVec<usize, BOARD> = ArrayVec::new();
        for j in 0..self.ms.len() {
            if j != i && self.ms[j].m.health > 0 {
                others.push(j);
            }
        }
        if others.is_empty() {
            return;
        }
        for _ in 0..gems.saturating_mul(times) {
            let at = others[rng.below(others.len() as u32) as usize];
            self.gem_at(db, at, 1, false, true, rng);
        }
    }

    /// Run one effect for the minion `fx.host`. `at` is where summons go
    /// and moves right with each one. Golden doubles the effect.
    #[allow(clippy::too_many_arguments)]
    fn run_effect(
        &mut self,
        db: &CardDb,
        e: Effect,
        golden: bool,
        fx: Fx,
        at: &mut usize,
        uid: &mut u16,
        rng: &mut Rng,
        w: &mut impl Watch,
    ) {
        let host_elemental = self.index_of(fx.host).is_some_and(|i| {
            db.tribes_of(&self.ms[i].m).has(Tribe::Elemental)
        });
        let e = if host_elemental
            && (self.counters.elemental_attack != 0 || self.counters.elemental_health != 0)
        {
            e.with_elemental_bonus((self.counters.elemental_attack, self.counters.elemental_health))
        } else {
            e
        };
        let mul: i32 = if golden { 2 } else { 1 };
        match e {
            Effect::BuffDeity { attack, health, per_spell } => {
                let n = self.counters.spells_cast as i32;
                self.buff_deity((attack + per_spell.0 * n) * mul, (health + per_spell.1 * n) * mul);
            }
            Effect::GetRandomSpellAtLeast { cost, count } => {
                for _ in 0..count as i32 * mul {
                    if let Some(c) = db.random_spell_at_least(
                        cost,
                        self.counters.tier,
                        self.counters.lobby,
                        rng,
                    ) {
                        self.out.push_card(c);
                    }
                }
            }
            Effect::GetRandomSpellUpTo { cost, count } => {
                for _ in 0..count as i32 * mul {
                    let card = match cost {
                        Some(cost) => db.random_spell_of_cost_up_to(cost, self.counters.tier, self.counters.lobby, rng),
                        None => db.random_spell(self.counters.tier, self.counters.lobby, rng),
                    };
                    if let Some(card) = card {
                        self.out.push_card(card);
                    }
                }
            }
            Effect::SummonAndBuffAll { card, count, attack, golden_doubles_count } => {
                let (n, g) = if golden_doubles_count {
                    (count * mul as u8, false)
                } else {
                    (count, golden)
                };
                for _ in 0..n {
                    if self.summon(db, fx.side, *at, db.make(card, g), uid, rng, w) {
                        *at += 1;
                    }
                }
                for i in 0..self.ms.len() {
                    if self.ms[i].m.health > 0 {
                        self.gain_at(db, i, attack * mul, 0);
                    }
                }
            }
            Effect::SummonDeadAberrations { count } => {
                let dead: ArrayVec<Minion, 4> = self.dead_aberrations.iter()
                    .take((count as i32 * mul) as usize).copied().collect();
                for dead in dead {
                    let (attack, health) = (dead.attack, dead.health);
                    if !self.summon(db, fx.side, *at, dead, uid, rng, w) {
                        break;
                    }
                    self.ms[*at].m.attack = attack;
                    self.ms[*at].m.health = health;
                    self.ms[*at].taken = 0;
                    *at += 1;
                }
            }
            Effect::Summon { card, count, golden_doubles_count } => {
                let (n, g) = if golden_doubles_count {
                    (count * mul as u8, false)
                } else {
                    (count, golden)
                };
                for _ in 0..n {
                    let m = db.make(card, g);
                    if self.summon(db, fx.side, *at, m, uid, rng, w) {
                        *at += 1;
                    }
                }
            }
            Effect::BuffOthers { tribe, attack, health, .. } => {
                for i in 0..self.ms.len() {
                    if self.ms[i].uid != fx.host
                        && self.ms[i].m.health > 0
                        && db.tribes_of(&self.ms[i].m).is(tribe)
                    {
                        self.gain_at(db, i, attack * mul, health * mul);
                    }
                }
            }
            Effect::BuffAll { tribe, attack, health } => {
                for i in 0..self.ms.len() {
                    if self.ms[i].m.health > 0 && db.tribes_of(&self.ms[i].m).is(tribe) {
                        self.gain_at(db, i, attack * mul, health * mul);
                    }
                }
            }
            Effect::BuffSelf { attack, health } => {
                if let Some(i) = self.index_of(fx.host) {
                    self.gain_at(db, i, attack * mul, health * mul);
                }
            }
            Effect::BuffRandomFriendly { tribe, attack, health, improve } => {
                let mut ok: ArrayVec<usize, BOARD> = ArrayVec::new();
                for (i, f) in self.ms.iter().enumerate() {
                    if f.m.health > 0 && db.tribes_of(&f.m).is(tribe) {
                        ok.push(i);
                    }
                }
                let scale = improve.map_or(1, |c| 1 + self.counters.improve[c.index()] as i32);
                if !ok.is_empty() {
                    let i = ok[rng.below(ok.len() as u32) as usize];
                    self.gain_at(db, i, attack * mul * scale, health * mul * scale);
                }
                if let Some(c) = improve {
                    let k = c.index();
                    self.counters.improve[k] = self.counters.improve[k].saturating_add(1);
                    self.out.improve_gain[k] = self.out.improve_gain[k].saturating_add(1);
                }
            }
            Effect::BuffKiller { attack, health } => {
                if let Some(k) = fx.killer {
                    let _ = self.killer_buffs.try_push((k, attack * mul, health * mul));
                }
            }
            Effect::GainKeyword { keyword } => {
                if let Some(i) = self.index_of(fx.host) {
                    self.gain_keyword_at(db, i, keyword);
                }
            }
            Effect::MaxGold(n) => {
                self.out.max_gold_gain = self
                    .out
                    .max_gold_gain
                    .saturating_add(n.saturating_mul(mul as u8));
            }
            Effect::GetRandomMinion { tribe } => {
                for _ in 0..mul {
                    if let Some(c) = db.random_pool_minion(tribe, self.counters.tier, self.counters.lobby, rng) {
                        self.out.push_card(c);
                    }
                }
            }
            Effect::GetMostCommonTypeMinion => {
                // Dark Gifts Charisma: the player's most common type among the
                // minions this side has in the fight (its board). Dark
                // Paradox t5's golden Rally gets two.
                let tribes: ArrayVec<crate::cards::Tribes, 64> =
                    self.ms.iter().map(|f| db.tribes_of(&f.m)).collect();
                for _ in 0..mul {
                    if let Some(t) = crate::darkgifts::most_common_type(&tribes, rng)
                        && let Some(c) =
                            db.random_pool_minion(t, self.counters.tier, self.counters.lobby, rng)
                    {
                        self.out.push_card(c);
                    }
                }
            }
            Effect::FreeRefreshes(n) => {
                // A Free Refresh a Deathrattle gives comes home with the
                // fight's gains (Dark Gifts Fresh Perspective, Herding Horn).
                self.out.free_refresh_gain = self
                    .out
                    .free_refresh_gain
                    .saturating_add(n.saturating_mul(mul as u16));
            }
            Effect::GetAnyTavernSpell => {
                // Dark Gifts Mystic Essence: any Tavern spell, cost and all.
                if let Some(c) = db.random_spell(crate::MAX_TIER, self.counters.lobby, rng) {
                    self.out.push_card(c);
                }
            }
            Effect::AddFodderRefreshes(n) => {
                self.out.push_fodder_refreshes(mul as u8, n as u32);
            }
            Effect::SummonRandomDeathrattle { count } => {
                for _ in 0..count as i32 * mul {
                    let Some(c) = db.random_deathrattle_minion(self.counters.tier, self.counters.lobby, rng) else { break };
                    let m = db.make(c, false);
                    if self.summon(db, fx.side, *at, m, uid, rng, w) {
                        *at += 1;
                    }
                }
            }
            Effect::TriggerDeathrattle { times } => {
                let deathstrider = self.ms.iter().find(|f| f.uid == fx.host).is_some_and(|f| {
                    f.m.parts().any(|(c, _)| {
                        db.get(c).after_friendly_attacks.is_some_and(|t| {
                            matches!(t.effect, Effect::TriggerDeathrattle { .. })
                        })
                    })
                });
                for _ in 0..times as i32 * mul {
                    if deathstrider {
                        self.out.deathstrider_fires = self.out.deathstrider_fires.saturating_add(1);
                    }
                    let Some(idx) = self.ms.iter().position(|f| {
                        f.m.health > 0 && f.m.parts().any(|(c, _)| db.get(c).deathrattle.is_some())
                    }) else {
                        break;
                    };
                    let (m, host) = (self.ms[idx].m, self.ms[idx].uid);
                    let runs = self.deathrattle_fires(db);
                    let n = self.fire_deathrattles(
                        db, fx.side, &m, m.health, host, None, idx + 1, runs, uid, rng, w,
                    );
                    if idx < *at {
                        *at += n;
                    }
                }
            }
            Effect::BeetleBuff { attack, health } => {
                self.add_beetle_buff(db, attack * mul, health * mul);
            }
            Effect::BeetleBuffAndSummon { attack, health, count } => {
                self.run_effect(
                    db,
                    Effect::BeetleBuff { attack, health },
                    golden,
                    fx,
                    at,
                    uid,
                    rng,
                    w,
                );
                if let Some(beetle) = db.beetle_token {
                    self.run_effect(
                        db,
                        Effect::Summon {
                            card: beetle,
                            count,
                            golden_doubles_count: true,
                        },
                        golden,
                        fx,
                        at,
                        uid,
                        rng,
                        w,
                    );
                }
            }
            Effect::RandomVolumizer => {
                for _ in 0..mul {
                    if let Some(v) = db.random_volumizer(self.counters.lobby, rng) {
                        self.out.push_card(v);
                    }
                }
            }
            Effect::GetCard { card, count } => {
                for _ in 0..count as i32 * mul {
                    self.out.push_card(card);
                }
            }
            Effect::SpellBonus { attack, health } => {
                self.counters.spell_attack += attack * mul;
                self.counters.spell_health += health * mul;
                self.out.spell_bonus_gain.0 += attack * mul;
                self.out.spell_bonus_gain.1 += health * mul;
            }
            Effect::RandomChromadrake => {
                for _ in 0..mul {
                    if let Some(c) = db.random_chromadrake(self.counters.lobby, rng) {
                        self.out.push_card(c);
                    }
                }
            }
            Effect::BuffAttacker { attack, health } => {
                if let Some(a) = fx.attacker
                    && let Some(i) = self.index_of(a)
                    && self.ms[i].m.health > 0
                {
                    self.gain_at(db, i, attack * mul, health * mul);
                }
            }
            Effect::BuffRandomOther {
                tribe,
                attack,
                health,
                keyword,
                count,
                permanent_keyword,
            } => {
                // A golden Amber Guardian picks two Dragons, not bigger buffs.
                let n = count as u32 * mul as u32;
                let mut ok: ArrayVec<usize, BOARD> = ArrayVec::new();
                for (i, f) in self.ms.iter().enumerate() {
                    if f.uid != fx.host && f.m.health > 0 && db.tribes_of(&f.m).is(tribe) {
                        ok.push(i);
                    }
                }
                for _ in 0..n {
                    if ok.is_empty() {
                        break;
                    }
                    let k = rng.below(ok.len() as u32) as usize;
                    let i = ok.remove(k);
                    self.gain_at(db, i, attack, health);
                    if keyword != 0 {
                        if permanent_keyword {
                            self.gain_keyword_permanent_at(i, keyword);
                        } else {
                            self.gain_keyword_at(db, i, keyword);
                        }
                    }
                }
            }
            Effect::SummonFromHand { tribe, count, by_health, shield } => {
                let n = count as u32 * mul as u32;
                self.summon_from_hand(db, fx.side, at, n, tribe, by_health, shield, uid, rng, w);
            }
            Effect::ShieldDragons { count } => {
                // Dragons without Divine Shield come first; once every one
                // has it, further picks are wasted on shielded Dragons.
                let n = count as i32 * mul;
                let mut without: ArrayVec<usize, BOARD> = ArrayVec::new();
                let mut with: ArrayVec<usize, BOARD> = ArrayVec::new();
                for (i, f) in self.ms.iter().enumerate() {
                    if f.m.health <= 0 || !db.tribes_of(&f.m).has(Tribe::Dragon) {
                        continue;
                    }
                    if f.m.keywords.has(Keywords::DIVINE_SHIELD) {
                        let _ = with.try_push(i);
                    } else {
                        let _ = without.try_push(i);
                    }
                }
                for _ in 0..n {
                    let pool = if without.is_empty() { &mut with } else { &mut without };
                    if pool.is_empty() {
                        break;
                    }
                    let k = rng.below(pool.len() as u32) as usize;
                    let i = pool.remove(k);
                    self.gain_keyword_at(db, i, Keywords::DIVINE_SHIELD);
                }
            }
            Effect::GetRandomTierMinion { tier } => {
                for _ in 0..mul {
                    if let Some(c) = db.random_tier_minion(tier, self.counters.lobby, rng) {
                        self.out.push_card(c);
                    }
                }
            }
            Effect::GetLockbox { sooner } => {
                self.out.push_lockbox(sooner.saturating_mul(mul as u8));
            }
            Effect::RefreshBuff { attack, health } => {
                self.out.refresh_buff_gain.0 += attack * mul;
                self.out.refresh_buff_gain.1 += health * mul;
            }
            Effect::BuffAllPerImprovement { attack, health } => {
                let Some(hi) = self.index_of(fx.host) else { return };
                let n = 1 + self.ms[hi].m.improvements as i32;
                for i in 0..self.ms.len() {
                    if self.ms[i].m.health > 0 {
                        self.gain_at(db, i, attack * mul * n, health * mul * n);
                    }
                }
            }
            Effect::BuffOthersPerImprovement { tribe, attack, health } => {
                // Times one plus the host's `improvements` (Ultraviolet
                // Ascendant counts the Elementals played while it was on the
                // board).
                let Some(hi) = self.index_of(fx.host) else { return };
                let k = 1 + self.ms[hi].m.improvements as i32;
                for i in 0..self.ms.len() {
                    if self.ms[i].uid != fx.host
                        && self.ms[i].m.health > 0
                        && db.tribes_of(&self.ms[i].m).is(tribe)
                    {
                        self.gain_at(db, i, attack * mul * k, health * mul * k);
                    }
                }
            }
            Effect::GetRandomBounty => {
                for _ in 0..mul {
                    if let Some(c) = db.random_bounty(self.counters.lobby, rng) {
                        self.out.push_card(c);
                    }
                }
            }
            Effect::BuffAllKeepGolden { attack, health } => {
                // Every living minion gets it for the fight; the Golden ones
                // keep it for good.
                let (da, dh) = (attack * mul, health * mul);
                for i in 0..self.ms.len() {
                    if self.ms[i].m.health <= 0 {
                        continue;
                    }
                    let keeps = self.ms[i].m.golden && self.keep_mul(db, i).is_none();
                    self.gain_at(db, i, da, dh);
                    if keeps
                        && !db.get(self.ms[i].m.card).fixed_stats
                        && let Some(slot) = self.ms[i].start_slot
                    {
                        let s = slot as usize;
                        self.out.permanent_stats[s].0 = self.out.permanent_stats[s].0.saturating_add(da);
                        self.out.permanent_stats[s].1 = self.out.permanent_stats[s].1.saturating_add(dh);
                    }
                }
            }
            Effect::BuffEachType { attack, health, improves_self } => {
                // A golden minion does the whole thing twice, with fresh
                // picks, so one minion can get two buffs from it.
                let host = if improves_self { self.index_of(fx.host) } else { None };
                for _ in 0..mul {
                    // "Improve this": the host's own counter grows the buff
                    // by its printed numbers each time it runs (The Last One
                    // Standing), and the count outlives the fight.
                    let (da, dh) = match host {
                        Some(hi) => {
                            let n = self.ms[hi].m.improvements as i32;
                            self.ms[hi].m.improvements = self.ms[hi].m.improvements.saturating_add(1);
                            if let Some(slot) = self.ms[hi].start_slot {
                                let s = slot as usize;
                                self.out.permanent_improvements[s] =
                                    self.out.permanent_improvements[s].saturating_add(1);
                            }
                            (attack * (1 + n), health * (1 + n))
                        }
                        None => (attack, health),
                    };
                    let mut tribes: ArrayVec<Tribes, BOARD> = ArrayVec::new();
                    for f in &self.ms {
                        let t = if f.m.health > 0 { db.tribes_of(&f.m) } else { Tribes::NONE };
                        tribes.push(t);
                    }
                    for i in pick_one_per_type(&tribes, rng) {
                        self.gain_permanent_at(db, i, da, dh);
                    }
                }
            }
            Effect::DestroyKiller => {
                if let Some(k) = fx.killer {
                    let _ = self.killer_destroy.try_push(k);
                }
            }
            Effect::BuffRandomHand { attack, health } => {
                self.buff_random_hand(db, attack * mul, health * mul, rng);
            }
            Effect::CastSpell { card, times } => {
                let Some(sp) = db.get(card).spell_effect else { return };
                for _ in 0..times as i32 * mul {
                    let cast = matches!(
                        sp,
                        SpellEffect::BuffBoard { .. } | SpellEffect::MightyDragonbreath { .. }
                    );
                    match sp {
                        SpellEffect::BuffBoard { attack, health, times, .. } => {
                            for _ in 0..times {
                                self.cast_board_buff(db, attack, health);
                            }
                        }
                        SpellEffect::MightyDragonbreath { attack, health } => {
                            self.cast_mighty_dragonbreath(db, attack, health);
                        }
                        _ => break,
                    }
                    if cast {
                        self.on_spell_cast(db);
                        // A trinket's in-fight cast trigger (Bewitched
                        // Ribbon's +2/+4 while in combat).
                        self.trinket_hook(db, fx.side, When::CastSpell, None, uid, rng, w);
                    }
                }
            }
            Effect::UndeadBonus { attack, health, outside_attack } => {
                let a = if self.outside && outside_attack != 0 { outside_attack } else { attack };
                self.gain_undead(db, a * mul, health * mul);
            }
            Effect::GiveRebornRandom { tribe, count } => {
                for _ in 0..count as i32 * mul {
                    let mut ok: ArrayVec<usize, BOARD> = ArrayVec::new();
                    for (i, f) in self.ms.iter().enumerate() {
                        if f.m.health > 0
                            && f.uid != fx.host
                            && db.tribes_of(&f.m).has(tribe)
                            && !f.m.keywords.has(Keywords::REBORN)
                            && Some(f.m.card) != self.dr_card
                        {
                            ok.push(i);
                        }
                    }
                    if ok.is_empty() {
                        break;
                    }
                    let i = ok[rng.below(ok.len() as u32) as usize];
                    self.gain_keyword_at(db, i, Keywords::REBORN);
                }
            }
            Effect::SummonAttacker { card } => {
                let m = db.make(card, golden);
                if self.summon(db, fx.side, *at, m, uid, rng, w) {
                    let new = *uid - 1;
                    let _ = self.immediate.try_push(new);
                    *at += 1;
                }
            }
            Effect::GetPlainCopy => {
                for _ in 0..mul {
                    if !self.start_cards.is_empty() {
                        let k = rng.below(self.start_cards.len() as u32) as usize;
                        self.out.push_card(self.start_cards[k]);
                    }
                }
            }
            Effect::GetRandomMinionHeld { tribe } => {
                for _ in 0..mul {
                    if let Some(c) = db.random_pool_minion(tribe, self.counters.tier, self.counters.lobby, rng) {
                        let before = self.out.n_cards;
                        self.out.push_card(c);
                        if self.out.n_cards > before {
                            let _ = self.held.try_push(c);
                        }
                    }
                }
            }
            Effect::SummonHeld => {
                while let Some(&c) = self.held.first() {
                    let m = db.make(c, false);
                    if !self.summon(db, fx.side, *at, m, uid, rng, w) {
                        break;
                    }
                    self.held.remove(0);
                    self.out.remove_card(c);
                    *at += 1;
                }
            }
            Effect::DestroyLeftNeighbor => {
                let Some(hi) = self.index_of(fx.host) else { return };
                let mut targets: ArrayVec<usize, 2> = ArrayVec::new();
                if let Some(l) = hi.checked_sub(1) {
                    targets.push(l);
                }
                if golden && hi + 1 < self.ms.len() {
                    targets.push(hi + 1);
                }
                for t in targets {
                    let f = self.ms[t];
                    if f.m.health <= 0 {
                        continue;
                    }
                    // Another Salvager is destroyed but never copied.
                    let is_salvager = db.get(f.m.card).deathrattle
                        == Some(Deathrattle::Effect(Effect::SummonSalvaged));
                    if !is_salvager {
                        let (copy, damage) = self.exact_copy(t);
                        let _ = self.salvage.try_push((fx.host, copy, damage));
                    }
                    let _ = self.self_destroyed.try_push(f.uid);
                    self.ms[t].m.health = 0;
                    self.ms[t].killer = Some(fx.host);
                }
            }
            Effect::SummonSalvaged => {
                for k in 0..self.salvage.len() {
                    let (host, copy, damage) = self.salvage[k];
                    if host != fx.host {
                        continue;
                    }
                    if self.summon_saved_copy(db, fx.side, *at, copy, damage, uid, rng, w) {
                        *at += 1;
                    }
                }
            }
            Effect::GetGems(n) => {
                if let Some(gem) = db.blood_gem {
                    for _ in 0..n as i32 * mul {
                        self.out.push_card(gem);
                    }
                }
            }
            Effect::GemSelf { count, permanent } => {
                if let Some(i) = self.index_of(fx.host) {
                    self.gem_at(db, i, count as i32 * mul, permanent, false, rng);
                }
            }
            Effect::GemTribe { tribe, count } => {
                let n = count as i32 * mul;
                for i in 0..self.ms.len() {
                    if self.ms[i].m.health > 0 && db.tribes_of(&self.ms[i].m).has(tribe) {
                        self.gem_at(db, i, n, false, false, rng);
                    }
                }
            }
            Effect::GemOthers(count) => {
                let n = count as i32 * mul;
                for i in 0..self.ms.len() {
                    if self.ms[i].uid != fx.host && self.ms[i].m.health > 0 {
                        self.gem_at(db, i, n, false, false, rng);
                    }
                }
            }
            Effect::GemOthersPermanent(count) => {
                let n = count as i32 * mul;
                for i in 0..self.ms.len() {
                    if self.ms[i].uid != fx.host && self.ms[i].m.health > 0 {
                        self.gem_at(db, i, n, true, false, rng);
                    }
                }
            }
            Effect::GemAttacker(count) => {
                if let Some(a) = fx.attacker
                    && let Some(i) = self.index_of(a)
                    && self.ms[i].m.health > 0
                {
                    self.gem_at(db, i, count as i32 * mul, false, false, rng);
                }
            }
            Effect::GemBonus { attack, health } => {
                // "This game": the fight's Gems grow now, the player's for
                // good through the outcome.
                self.counters.gem_attack += attack * mul;
                self.counters.gem_health += health * mul;
                self.out.gem_gain.0 += attack * mul;
                self.out.gem_gain.1 += health * mul;
            }
            Effect::GetGemAndBonus { attack, health } => {
                if let Some(gem) = db.blood_gem {
                    self.out.push_card(gem);
                }
                self.counters.gem_attack += attack * mul;
                self.counters.gem_health += health * mul;
                self.out.gem_gain.0 += attack * mul;
                self.out.gem_gain.1 += health * mul;
            }
            Effect::GetRandomChooseOne(n) => {
                for _ in 0..n as i32 * mul {
                    if let Some(c) =
                        db.random_choose_one_card(self.counters.tier, self.counters.lobby, rng)
                    {
                        self.out.push_card(c);
                    }
                }
            }
            // Jailbird Juggernaut's Rally is intercepted in `attack_triggers`,
            // where the target is known; this arm only summons the stat copy.
            Effect::SummonStatCopyAttacker { card } => {
                let Some(hi) = self.index_of(fx.host) else { return };
                let mut m = db.make(card, false);
                m.attack = self.ms[hi].m.attack.saturating_mul(mul);
                m.health = self.ms[hi].m.health.saturating_mul(mul);
                if self.summon(db, fx.side, *at, m, uid, rng, w) {
                    *at += 1;
                }
            }
            // Parsing keeps every other effect out of fights.
            _ => {}
        }
    }

    /// Run the Deathrattles of `dead`, a minion that died or whose
    /// Deathrattle was triggered. Summons go in from `at`; returns how many
    /// were inserted.
    #[allow(clippy::too_many_arguments)]
    fn fire_deathrattles(
        &mut self,
        db: &CardDb,
        side: usize,
        dead: &Minion,
        // The dead minion's maximum Health (its current Health plus the
        // damage it took this fight), for Defensive Sacrifice.
        dead_max: i32,
        host: u16,
        killer: Option<u16>,
        at: usize,
        runs: u32,
        uid: &mut u16,
        rng: &mut Rng,
        w: &mut impl Watch,
    ) -> usize {
        let mut pos = at;
        self.dr_card = Some(dead.card);
        // The Deathrattles this minion gained from the Fish of N'Zoth's text
        // fire with its own, once each: they are taken out of the side's
        // list here, so a Reborn Fish (a new uid) does not fire them again.
        let mut parts: ArrayVec<Attached, FISH_DR> = ArrayVec::new();
        for (card, golden) in dead.parts() {
            let _ = parts.try_push(Attached { card, golden });
        }
        let mut k = 0;
        while k < self.gained_dr.len() {
            if self.gained_dr[k].0 == host {
                let (_, a) = self.gained_dr.remove(k);
                let _ = parts.try_push(a);
            } else {
                k += 1;
            }
        }
        for _ in 0..runs {
            for &Attached { card, golden } in &parts {
                let Some(dr) = db.get(card).deathrattle else { continue };
                self.dr_card = Some(card);
                self.note_deathrattle(db);
                let mul = if golden { 2 } else { 1 };
                match dr {
                    Deathrattle::Summon { card: tok, count, golden_doubles_count, beetle_buff } => {
                        let (n, tok_golden) = if golden_doubles_count {
                            (count * mul as u8, false)
                        } else {
                            (count, golden)
                        };
                        for _ in 0..n {
                            let m = db.make(tok, tok_golden);
                            if self.summon(db, side, pos, m, uid, rng, w) {
                                pos += 1;
                            }
                        }
                        // Turquoise Skitterer summons its Beetle first, then
                        // gives all Beetles the buff: the log of 2026-11-11
                        // (Power.log 152571) has the new Beetle at the old
                        // bonus, and the buff lands on it after Banana
                        // Slamma's doubling. The buff still applies when a
                        // full board lost the summon.
                        if let Some((ba, bh)) = beetle_buff {
                            self.add_beetle_buff(db, ba * mul, bh * mul);
                        }
                    }
                    Deathrattle::OpponentWarband { count } => {
                        self.summon_opponent_warband(
                            db, side, pos, count as usize * mul as usize, uid, rng, w,
                        );
                    }
                    Deathrattle::SummonOverflowUndeadAttack {
                        card: tok,
                        count,
                        golden_doubles_count,
                        attack,
                    } => {
                        let (n, tok_golden) = if golden_doubles_count {
                            (count * mul as u8, false)
                        } else {
                            (count, golden)
                        };
                        let mut overflow = 0u8;
                        for _ in 0..n {
                            let m = db.make(tok, tok_golden);
                            if self.summon(db, side, pos, m, uid, rng, w) {
                                pos += 1;
                            } else {
                                overflow += 1;
                            }
                        }
                        if overflow > 0 {
                            self.gain_undead(db, attack * overflow as i32, 0);
                        }
                    }
                    Deathrattle::SummonByAttack { card: tok } => {
                        let n = dead.attack.max(0) as usize;
                        for _ in 0..n {
                            let m = db.make(tok, golden);
                            if self.summon(db, side, pos, m, uid, rng, w) {
                                pos += 1;
                            }
                        }
                    }
                    Deathrattle::CombatBeastBuff { attack, health } => {
                        self.add_combat_beast_buff(db, attack * mul, health * mul);
                    }
                    Deathrattle::RandomVolumizer => {
                        for _ in 0..mul {
                            if let Some(v) = db.random_volumizer(self.counters.lobby, rng) {
                                self.out.push_card(v);
                            }
                        }
                    }
                    Deathrattle::GetCard { card: tok, count } => {
                        for _ in 0..count as i32 * mul {
                            self.out.push_card(tok);
                        }
                    }
                    Deathrattle::TavernBuff { attack, health, per_improvement } => {
                        // "Improve this" stacks live in the dead minion's
                        // improvements (Sacrificial Wrathguard).
                        let n = dead.improvements as i32;
                        let da = (attack + per_improvement.0 * n) * mul;
                        let dh = (health + per_improvement.1 * n) * mul;
                        self.out.tavern_buff_gain.0 += da;
                        self.out.tavern_buff_gain.1 += dh;
                    }
                    Deathrattle::SummonShaped { card: tok } => {
                        // Dark Gifts Golemancy: a Golem with the dead
                        // minion's stats. A golden minion's own Deathrattle
                        // doubles them (Dark Paradox t9's golden: ONE Golem
                        // with double its stats); a gift part is never
                        // golden, so Golemancy never doubles.
                        let mut m = db.make(tok, false);
                        m.attack = dead.attack * mul;
                        // Its maximum Health, at least 1: the stats it had
                        // before the killing blow (the owner's reference
                        // engine reads the same).
                        m.health = dead_max.max(1) * mul;
                        if self.summon(db, side, pos, m, uid, rng, w) {
                            pos += 1;
                        }
                    }
                    Deathrattle::GiveOwnAttack => {
                        // Dark Gifts Offensive Sacrifice: the whole Attack to
                        // one other friendly minion, at random.
                        let alive: ArrayVec<usize, 32> = (0..self.ms.len())
                            .filter(|&i| self.ms[i].m.health > 0)
                            .collect();
                        if !alive.is_empty() {
                            let i = alive[rng.below(alive.len() as u32) as usize];
                            self.gain_at(db, i, dead.attack, 0);
                        }
                    }
                    Deathrattle::GiveOwnHealth => {
                        // Dark Gifts Defensive Sacrifice: the maximum Health,
                        // the damage it had taken included.
                        let alive: ArrayVec<usize, 32> = (0..self.ms.len())
                            .filter(|&i| self.ms[i].m.health > 0)
                            .collect();
                        if !alive.is_empty() {
                            let i = alive[rng.below(alive.len() as u32) as usize];
                            self.gain_at(db, i, 0, dead_max);
                        }
                    }
                    Deathrattle::Effect(e) => {
                        let fx = Fx { side, host, killer, attacker: None };
                        self.run_effect(db, e, golden, fx, &mut pos, uid, rng, w);
                    }
                }
            }
            // A Deathrattle a trinket or hero power gave this minion for the
            // fight (Hoggy Bank, Jarred Frostling, Sky Golem Portrait,
            // Fragrant Phylactery). It fires with the minion's own, once per
            // run, whether or not the card has one.
            let mut granted: ArrayVec<GrantedDr, 8> = ArrayVec::new();
            for &(u, dr) in &self.granted {
                if u == host {
                    let _ = granted.try_push(dr);
                }
            }
            for dr in granted {
                self.note_deathrattle(db);
                match dr {
                    GrantedDr::Gems(n) => {
                        if let Some(gem) = db.blood_gem {
                            for _ in 0..n {
                                self.out.push_card(gem);
                            }
                        }
                    }
                    GrantedDr::Summon { card: tok, count } => {
                        for _ in 0..count {
                            let m = db.make(tok, false);
                            if self.summon(db, side, pos, m, uid, rng, w) {
                                pos += 1;
                            }
                        }
                    }
                    GrantedDr::BuffAll { attack, health } => {
                        for i in 0..self.ms.len() {
                            if self.ms[i].m.health > 0 {
                                self.gain_permanent_at(db, i, attack, health);
                            }
                        }
                    }
                    GrantedDr::Phylactery => {
                        let da = dead.attack;
                        let dh = dead_max.max(1);
                        for i in 0..self.ms.len() {
                            if self.ms[i].m.health > 0 && self.ms[i].uid != host {
                                self.gain_at(db, i, da, dh);
                            }
                        }
                    }
                }
            }
        }
        pos - at
    }

    /// A friendly minion died: every living minion with Avenge counts it, and
    /// runs its effect when the count reaches its N.
    fn avenge_tick(&mut self, db: &CardDb, side: usize, uid: &mut u16, rng: &mut Rng, w: &mut impl Watch) {
        let mut fire: ArrayVec<(u16, Effect, bool), 32> = ArrayVec::new();
        for f in &mut self.ms {
            if f.m.health <= 0 || !f.m.parts().any(|(c, _)| db.get(c).avenge.is_some()) {
                continue;
            }
            f.avenge = f.avenge.saturating_add(1);
            for (card, golden) in f.m.parts() {
                if let Some(a) = db.get(card).avenge
                    && f.avenge % a.every as u16 == 0
                {
                    let _ = fire.try_push((f.uid, a.effect, golden));
                }
            }
        }
        for (host, e, golden) in fire {
            let Some(i) = self.index_of(host) else { continue };
            let fx = Fx { side, host, killer: None, attacker: None };
            let mut at = i + 1;
            self.run_effect(db, e, golden, fx, &mut at, uid, rng, w);
        }
    }

    /// Broodmother (Onyxia): Avenge (4) summons a 3/1 Onyxian Whelp with
    /// `+(base + broodmother_gain)` stats that attacks immediately, then
    /// improves future Whelps by +1/+1.
    fn broodmother_tick(
        &mut self,
        db: &CardDb,
        side: usize,
        uid: &mut u16,
        rng: &mut Rng,
        w: &mut impl Watch,
    ) {
        let Some((card, base_bonus, triggers)) = self.counters.broodmother else {
            return;
        };
        if triggers == 0 {
            return;
        }
        self.broodmother_avenge = self.broodmother_avenge.saturating_add(1);
        if !self.broodmother_avenge.is_multiple_of(4) {
            return;
        }
        for _ in 0..triggers {
            let bonus = base_bonus + self.out.broodmother_gain as i32;
            let mut m = db.make(card, false);
            m.attack += bonus;
            m.health += bonus;
            let at = self.ms.len();
            if self.summon(db, side, at, m, uid, rng, w) && at < self.ms.len() {
                let u = self.ms[at].uid;
                let _ = self.immediate.try_push(u);
            }
            self.out.broodmother_gain = self.out.broodmother_gain.saturating_add(1);
        }
    }

    /// The minion in slot `i` dealt `dealt` damage: count it towards a damage
    /// milestone (Treasure Parrot) and Bloodfury (`out.damage_dealt`).
    #[allow(clippy::too_many_arguments)]
    fn count_damage(&mut self, db: &CardDb, side: usize, i: usize, dealt: i32, uid: &mut u16, rng: &mut Rng, w: &mut impl Watch) {
        if dealt <= 0 {
            return;
        }
        if self.counters.count_damage {
            self.out.damage_dealt = self.out.damage_dealt.saturating_add(dealt as u32);
        }
        let m = self.ms[i].m;
        let host = self.ms[i].uid;
        for (card, golden) in m.parts() {
            let Some(ms) = db.get(card).damage_milestone else { continue };
            let n = dealt.min(u16::MAX as i32) as u16;
            let before = self.ms[i].m.improvements;
            let after = before.saturating_add(n);
            self.ms[i].m.improvements = after;
            if let Some(slot) = self.ms[i].start_slot {
                let s = slot as usize;
                self.out.permanent_improvements[s] = self.out.permanent_improvements[s].saturating_add(n);
            }
            if before < ms.total && after >= ms.total {
                let fx = Fx { side, host, killer: None, attacker: None };
                let mut at = i + 1;
                self.run_effect(db, ms.effect, golden, fx, &mut at, uid, rng, w);
            }
        }
    }

    fn buff_deity(&mut self, attack: i32, health: i32) {
        if let Some(deity) = &mut self.counters.deity {
            deity.attack += attack;
            deity.health += health;
            self.out.deity_gain.0 += attack;
            self.out.deity_gain.1 += health;
        }
    }

    fn try_awaken(
        &mut self,
        db: &CardDb,
        side: usize,
        uid: &mut u16,
        rng: &mut Rng,
        w: &mut impl Watch,
    ) {
        let Some(deity) = self.counters.deity else { return };
        if self.deity_awakened || self.deity_left > 0 || self.ms.is_full() {
            return;
        }
        let mut god = db.make(deity.card, deity.golden);
        god.attack = deity.attack;
        god.health = deity.health.max(1);
        let at = self.ms.len();
        if !self.summon(db, side, at, god, uid, rng, w) {
            return;
        }
        self.deity_awakened = true;
        let god = self.ms[at].m;
        self.trinket_hook(db, side, When::Awaken, Some((self.ms[at].uid, god)), uid, rng, w);
        if db.get(deity.card).deity != Some(DeityKind::Cthun) {
            return;
        }
        let Some(god_index) = self.ms.get(at).filter(|f| f.m.health > 0).map(|_| at) else { return };
        let god = self.ms[god_index].m;
        let mut others: ArrayVec<usize, BOARD> = (0..self.ms.len())
            .filter(|&i| i != god_index && self.ms[i].m.health > 0)
            .collect();
        if others.is_empty() {
            return;
        }
        let mul = if deity.golden { 2 } else { 1 };
        for (total, attack) in [(god.attack * mul, true), (god.health * mul, false)] {
            let share = total / others.len() as i32;
            let extra = total % others.len() as i32;
            rng.shuffle(&mut others);
            for (k, &i) in others.iter().enumerate() {
                let n = share + i32::from((k as i32) < extra);
                if attack {
                    self.gain_at(db, i, n, 0);
                } else {
                    self.gain_at(db, i, 0, n);
                }
            }
        }
    }

    fn note_deity_death(
        &mut self,
        db: &CardDb,
        side: usize,
        fm: Fm,
        uid: &mut u16,
        rng: &mut Rng,
        w: &mut impl Watch,
    ) {
        if self.outside || self.counters.deity.is_none() {
            return;
        }
        let card = db.get(fm.m.card);
        if db.tribes_of(&fm.m).has(Tribe::Aberration) {
            if card.deity.is_none() && self.dead_aberrations.len() < 4 {
                let mut dead = fm.m;
                dead.health += fm.taken;
                self.dead_aberrations.push(dead);
            }
            self.deity_left = self.deity_left.saturating_sub(1);
        }
        self.try_awaken(db, side, uid, rng, w);
    }

    /// Remove the dead in board order. Each one's Deathrattle runs, then
    /// Avenge counts it, then its Reborn, before the next dead minion is
    /// looked at.
    fn resolve(
        &mut self,
        db: &CardDb,
        side: usize,
        uid: &mut u16,
        rng: &mut Rng,
        w: &mut impl Watch,
    ) -> bool {
        let mut changed = false;
        // Dead minions that a summon took out of `ms` go first, where they
        // stood, so their Deathrattles and Reborn still happen.
        while !self.held_dead.is_empty() {
            let (i, fm) = self.held_dead.remove(0);
            changed = true;
            let i = i.min(self.ms.len());
            self.resolve_dead(db, side, i, fm, uid, rng, w);
        }
        let mut i = 0;
        while i < self.ms.len() {
            if self.ms[i].m.health > 0 {
                i += 1;
                continue;
            }
            changed = true;
            let fm = self.remove(i);
            i += self.resolve_dead(db, side, i, fm, uid, rng, w);
        }
        // Soul Fermenter: once this side's last minion is gone, the ones it
        // destroyed at Start of Combat come back exactly as they were.
        if self.ms.is_empty() && !self.fermenter.is_empty() {
            let held: ArrayVec<(usize, Minion), 3> = std::mem::take(&mut self.fermenter);
            for (slot, m) in held {
                let at = slot.min(self.ms.len());
                if self.summon(db, side, at, m, uid, rng, w) {
                    changed = true;
                }
            }
        }
        if self.spawn_when_space(db, side, uid, rng, w) {
            changed = true;
        }
        changed
    }

    /// Handle one dead minion at slot `i`: its Deathrattle, Avenge, Reborn
    /// and Redemption. Returns how many minions this inserted at `i`.
    #[allow(clippy::too_many_arguments)]
    fn resolve_dead(
        &mut self,
        db: &CardDb,
        side: usize,
        mut i: usize,
        fm: Fm,
        uid: &mut u16,
        rng: &mut Rng,
        w: &mut impl Watch,
    ) -> usize {
        let dead = fm.m;
        w.death(side, i, &dead);
        self.out.note_dead_card(dead.card);
        self.out.friendly_deaths = self.out.friendly_deaths.saturating_add(1);
        if self.first_killed_by_enemy.is_none() && !self.self_destroyed.contains(&fm.uid) {
            self.first_killed_by_enemy = Some(dead.card);
        }
        if self.counters.trinkets.count_deaths {
            self.out.note_death_card(dead.card);
            if self.counters.trinkets.count_death_tribes != 0 {
                let tribes = db.tribes_of(&dead);
                for (k, t) in Tribe::EVERY.into_iter().enumerate() {
                    if tribes.has(t) {
                        self.out.death_tribes[k] =
                            self.out.death_tribes[k].saturating_add(1);
                    }
                }
            }
        }
        // A death is what a "whenever a friendly minion dies" trinket
        // waits for, and what Avenge counts (its N runs of deaths).
        self.last_dead = Some(dead);
        self.dead_at = i;
        self.trinket_hook(db, side, When::Death, Some((fm.uid, dead)), uid, rng, w);
        self.trinket_hook(db, side, When::Avenge, Some((fm.uid, dead)), uid, rng, w);
        if db.get(dead.card).buff_per_kind_death.is_some() {
            self.note_knight_death(db);
        }
        // Fish of N'Zoth: a different friendly Deathrattle minion died,
        // so every minion with that text gains its Deathrattle for the
        // rest of the fight.
        self.gain_deathrattles(db, fm.uid, &dead);
        let runs = self.deathrattle_fires(db);
        let dead_max = dead.health + fm.taken;
        let mut added = self.fire_deathrattles(
            db, side, &dead, dead_max, fm.uid, fm.killer, i, runs, uid, rng, w,
        );
        i += added;
        self.note_deity_death(db, side, fm, uid, rng, w);
        if self.count_avenge {
            self.avenge_tick(db, side, uid, rng, w);
            self.broodmother_tick(db, side, uid, rng, w);
        }
        if dead.keywords.has(Keywords::REBORN) {
            let mut m = db.make(dead.card, dead.golden);
            // It counts as damaged down to 1 Health, so its maximum
            // Health is the full value (Stalwart Kodo reads it).
            let full = m.health;
            m.health = 1;
            m.keywords.clear(Keywords::REBORN);
            // Dark Gifts Persisting Horror: it comes back with full
            // stats and its Bonus Keywords, and with the gifts it had.
            if dead.has_gift(db, |c| c.reborn_full) {
                m.attack = dead.attack;
                m.health = dead.health.max(1);
                // Reborn itself is spent: the copy does not come back
                // a second time.
                let bonus = self.counters.bonus_keywords & !Keywords::REBORN;
                m.keywords.0 |= dead.keywords.0 & bonus;
                for &g in dead.gift_slice() {
                    m.push_gift(g);
                }
                m.shield_hits = dead.shield_hits;
            }
            self.no_summon_hooks = true;
            let reborn = self.summon(db, side, i, m, uid, rng, w);
            self.no_summon_hooks = false;
            if reborn {
                self.ms[i].taken = (full - 1).max(0);
                self.note_reborn(db, i, fm.start_slot);
                let plain = self.ms[i].m;
                let u = self.ms[i].uid;
                self.trinket_hook(db, side, When::Reborn, Some((u, plain)), uid, rng, w);
                i += 1;
                added += 1;
            }
        }
        // Redemption: a friendly minion died, so a fresh copy with 1
        // Health comes back where it stood. It comes after this minion's
        // own Deathrattles and its Reborn, so a board they filled makes
        // it wait for room (docs/rules/heroes.md "Secrets").
        if !self.secrets.is_empty() {
            self.redemption(db, side, &dead, i, uid, rng, w);
        }
        added
    }
}

/// Deal `dmg` non-combat damage (e.g. Boom-in-a-Box) to `target`. Returns
/// the damage its Health actually lost, 0 when a shield took it.
fn deal_damage(target: &mut Minion, dmg: i32) -> i32 {
    if dmg <= 0 {
        return 0;
    }
    if target.keywords.has(Keywords::DIVINE_SHIELD) {
        // A Toreth's Blessing shield takes several hits before it breaks
        // (Dark Gifts).
        if target.shield_hits > 1 {
            target.shield_hits -= 1;
            return 0;
        }
        target.keywords.clear(Keywords::DIVINE_SHIELD);
        target.shield_hits = 0;
        return 0;
    }
    target.health -= dmg;
    dmg
}

/// `source` deals `dmg` to `target`. Divine Shield takes any hit and keeps
/// Venomous and Poisonous unused; 0 damage does nothing at all. Returns the
/// damage dealt, 0 when the shield took it.
///
/// Poisonous kills what it damages and is never used up; Venomous kills and is
/// then gone (docs/rules/design.md "Keywords"). A minion with both kills with
/// its Poisonous, so the Venomous is not needed and stays: nothing settles
/// which of the two would be spent (Era asked for this reading, 2026-11-04).
fn hit(target: &mut Minion, dmg: i32, source: &mut Minion) -> i32 {
    if dmg <= 0 {
        return 0;
    }
    if target.keywords.has(Keywords::DIVINE_SHIELD) {
        if target.shield_hits > 1 {
            target.shield_hits -= 1;
            return 0;
        }
        target.keywords.clear(Keywords::DIVINE_SHIELD);
        target.shield_hits = 0;
        return 0;
    }
    target.health -= dmg;
    if source.keywords.has(Keywords::POISONOUS) {
        target.health = target.health.min(0);
    } else if source.keywords.has(Keywords::VENOMOUS) {
        target.health = target.health.min(0);
        source.keywords.clear(Keywords::VENOMOUS);
    }
    dmg
}

/// The uids of the enemies next to `target` when the attacker cleaves
/// (Blade Collector), as the board stands before the hit.
fn cleave_targets(db: &CardDb, attacker: &Minion, def: &Side, target: u16) -> ArrayVec<u16, 2> {
    let mut out = ArrayVec::new();
    if !attacker.parts().any(|(c, _)| db.get(c).cleave) {
        return out;
    }
    if let Some(di) = def.index_of(target) {
        if let Some(l) = di.checked_sub(1) {
            out.push(def.ms[l].uid);
        }
        if di + 1 < def.ms.len() {
            out.push(def.ms[di + 1].uid);
        }
    }
    out
}

fn targetable(f: &Fm) -> bool {
    !f.m.keywords.has(Keywords::STEALTH)
}

/// The defender is random among the enemies. Taunt minions come first and
/// Stealth minions can never be picked. None when every enemy has Stealth:
/// the attacking side then loses its turn.
fn pick_defender(db: &CardDb, def: &Side, rng: &mut Rng) -> Option<usize> {
    let eligible = |f: &&Fm| targetable(f);
    let taunts = def.ms.iter().filter(eligible).filter(|f| has_taunt(db, &f.m)).count();
    if taunts == 0 {
        let n = def.ms.iter().filter(eligible).count() as u32;
        if n == 0 {
            return None;
        }
        let k = rng.below(n) as usize;
        return Some(def.ms.iter().enumerate().filter(|(_, f)| eligible(f)).nth(k).unwrap().0);
    }
    let k = rng.below(taunts as u32) as usize;
    def.ms
        .iter()
        .enumerate()
        .filter(|(_, f)| eligible(f) && has_taunt(db, &f.m))
        .nth(k)
        .map(|(i, _)| i)
}

fn pair<'s, 'a>(
    sides: &'s mut [Side<'a>; 2],
    turn: usize,
) -> (&'s mut Side<'a>, &'s mut Side<'a>) {
    let [a, b] = sides;
    if turn == 0 { (a, b) } else { (b, a) }
}

/// Hand out the buffs Fishbait Deathrattles gave to their killers.
fn give_killer_buffs(sides: &mut [Side<'_>; 2], db: &CardDb) {
    for s in 0..2 {
        while let Some((killer, da, dh)) = sides[s].killer_buffs.pop() {
            for t in 0..2 {
                if let Some(i) = sides[t].ms.iter().position(|f| f.uid == killer && f.m.health > 0)
                {
                    sides[t].gain_at(db, i, da, dh);
                }
            }
        }
    }
}

/// Hand out the "destroy the minion that killed this" queue (Leeroy). A
/// destroy ignores Divine Shield, and does nothing when the killer is gone.
fn apply_killer_destroys(sides: &mut [Side<'_>; 2]) -> bool {
    let mut queued: ArrayVec<u16, 16> = ArrayVec::new();
    for s in 0..2 {
        while let Some(u) = sides[s].killer_destroy.pop() {
            let _ = queued.try_push(u);
        }
    }
    let mut any = false;
    for u in queued {
        for t in 0..2 {
            if let Some(i) = sides[t].ms.iter().position(|f| f.uid == u && f.m.health > 0) {
                sides[t].ms[i].m.health = 0;
                any = true;
            }
        }
    }
    any
}

fn resolve_deaths(
    sides: &mut [Side<'_>; 2],
    first_side: usize,
    db: &CardDb,
    uid: &mut u16,
    rng: &mut Rng,
    w: &mut impl Watch,
) {
    loop {
        let (att, def) = pair(sides, first_side);
        let c1 = att.resolve(db, first_side, uid, rng, w);
        let c2 = def.resolve(db, first_side ^ 1, uid, rng, w);
        give_killer_buffs(sides, db);
        let c3 = apply_killer_destroys(sides);
        if !(c1 || c2 || c3) {
            break;
        }
    }
    let (att, def) = pair(sides, first_side);
    att.spawn_boon_of_beetles(db, first_side, uid, rng, w);
    def.spawn_boon_of_beetles(db, first_side ^ 1, uid, rng, w);
    for s in 0..2 {
        let n = sides[s].take_first_killed_left;
        if n > 0
            && let Some(card) = sides[s ^ 1].first_killed_by_enemy
        {
            for _ in 0..n {
                sides[s].out.push_card(card);
            }
            sides[s].take_first_killed_left = 0;
        }
    }
}

fn run_start_of_combat(
    sides: &mut [Side<'_>; 2],
    db: &CardDb,
    uid: &mut u16,
    rng: &mut Rng,
    w: &mut impl Watch,
) {
    // Wingmen (Illidan Stormrage) runs before all other Start of Combat
    // effects (docs/rules/design.md "Order of events"), a trinket's included: the
    // left-most and right-most minions gain +2/+1 and attack immediately,
    // left then right.
    if sides[0].counters.wingmen > 0 || sides[1].counters.wingmen > 0 {
        let first = if sides[0].counters.wingmen > 0 && sides[1].counters.wingmen > 0 {
            rng.coin() as usize
        } else if sides[1].counters.wingmen > 0 {
            1
        } else {
            0
        };
        for s in [first, first ^ 1] {
            for _ in 0..sides[s].counters.wingmen {
                if sides[s].ms.is_empty() {
                    break;
                }
                let left_uid = sides[s].ms[0].uid;
                let right_idx = sides[s].ms.len() - 1;
                let right_uid = sides[s].ms[right_idx].uid;
                sides[s].gain_at(db, 0, 2, 1);
                if right_uid != left_uid {
                    sides[s].gain_at(db, right_idx, 2, 1);
                }
                if !sides[s ^ 1].ms.is_empty() {
                    attack_swings(sides, s, left_uid, db, uid, rng, w);
                }
                if right_uid != left_uid
                    && sides[s].index_of(right_uid).is_some()
                    && !sides[s ^ 1].ms.is_empty()
                {
                    attack_swings(sides, s, right_uid, db, uid, rng, w);
                }
            }
        }
    }
    // Boon of Beetles hands its Beetles over at Start of Combat, after
    // Wingmen's attacks.
    for s in 0..2 {
        sides[s].spawn_boon_of_beetles(db, s, uid, rng, w);
    }
    // A hero power's Start of Combat goes before any minion's (docs/rules/design.md
    // "Order of events"): Al'Akir's left-most minion gains its keywords,
    // Bru'kan's Water Invocation the right-most one's.
    for s in 0..2 {
        for _ in 0..sides[s].counters.all_will_burn {
            for t in 0..2 {
                for i in 0..sides[t].ms.len() {
                    if sides[t].ms[i].m.health > 0 {
                        sides[t].gain_permanent_at(db, i, 2, 0);
                    }
                }
            }
        }
        for _ in 0..sides[s].counters.wax_warband {
            let amt = 1 + (sides[s].counters.gold_spent / 10) as i32;
            let mut tribes: ArrayVec<Tribes, BOARD> = ArrayVec::new();
            for f in &sides[s].ms {
                let t = if f.m.health > 0 { db.tribes_of(&f.m) } else { Tribes::NONE };
                tribes.push(t);
            }
            for i in pick_one_per_type(&tribes, rng) {
                sides[s].gain_at(db, i, amt, amt);
            }
        }
        for _ in 0..sides[s].counters.phylactery {
            let picked = sides[s].pick(db, Pick::LowestAttack(1), None, rng);
            for i in picked {
                let u = sides[s].ms[i].uid;
                let _ = sides[s].granted.try_push((u, GrantedDr::Phylactery));
            }
        }
        for _ in 0..sides[s].counters.embrace_rage {
            if let Some(c) =
                db.random_tier_minion(sides[s].counters.tier, sides[s].counters.lobby, rng)
            {
                sides[s].out.push_card(c);
                let m = db.make(c, false);
                let at = sides[s].ms.len();
                sides[s].summon(db, s, at, m, uid, rng, w);
            }
        }
        let kw = sides[s].counters.soc_leftmost;
        if kw != 0
            && let Some(f) = sides[s].ms.first_mut()
        {
            f.m.keywords.0 |= kw;
        }
        let kw = sides[s].counters.soc_rightmost;
        if kw != 0
            && let Some(f) = sides[s].ms.last_mut()
        {
            f.m.keywords.0 |= kw;
        }
        // Fire Invocation: double the left-most minion's Attack, buffs
        // included.
        let n = sides[s].counters.soc_double_left;
        if n > 0
            && let Some(f) = sides[s].ms.first_mut()
        {
            for _ in 0..n {
                f.m.attack *= 2;
            }
        }
        // Earth Invocation: this many random friendly minions get a
        // Deathrattle that summons the 1/1 Elemental, each of them once.
        let n = sides[s].counters.soc_earth;
        let token = sides[s].counters.soc_earth_card;
        if n > 0 && token != 0 {
            let mut slots: ArrayVec<u16, BOARD> = sides[s].ms.iter().map(|f| f.uid).collect();
            for _ in 0..n {
                if slots.is_empty() {
                    break;
                }
                let k = rng.below(slots.len() as u32) as usize;
                let u = slots.remove(k);
                if sides[s].granted.len() < sides[s].granted.capacity() {
                    let _ = sides[s].granted.try_push((u, GrantedDr::Summon { card: token, count: 1 }));
                }
            }
        }
    }
    // Lightning Invocation: one damage to a different random enemy minion
    // each time, so no minion takes it twice unless there are fewer than the
    // hits. Both sides' hits are chosen first, then applied.
    let mut struck: [ArrayVec<u16, 8>; 2] = [ArrayVec::new(), ArrayVec::new()];
    for s in 0..2 {
        let n = sides[s].counters.soc_lightning;
        let dmg = sides[s].counters.soc_lightning_damage;
        if n == 0 || dmg == 0 {
            continue;
        }
        let opp = s ^ 1;
        let mut slots: ArrayVec<u16, BOARD> = sides[opp].ms.iter().map(|f| f.uid).collect();
        for _ in 0..n {
            if slots.is_empty() {
                break;
            }
            let k = rng.below(slots.len() as u32) as usize;
            let _ = struck[s].try_push(slots.remove(k));
        }
    }
    let mut hit: Option<usize> = None;
    for s in 0..2 {
        let opp = s ^ 1;
        let dmg = sides[s].counters.soc_lightning_damage;
        for &u in &struck[s] {
            let Some(i) = sides[opp].ms.iter().position(|f| f.uid == u) else { continue };
            // One point of damage each, Divine Shields and all, the way any
            // other damage in a fight lands.
            let had = sides[opp].ms[i].m.keywords.has(Keywords::DIVINE_SHIELD);
            let dealt = deal_damage(&mut sides[opp].ms[i].m, dmg);
            sides[opp].ms[i].taken += dealt;
            if had && !sides[opp].ms[i].m.keywords.has(Keywords::DIVINE_SHIELD) {
                sides[opp].note_shield_lost(db);
            }
            if dealt > 0 && sides[opp].damage_trigger && i < sides[opp].ms.len() {
                sides[opp].when_damaged(db, opp, i, uid, rng, w);
            }
            hit = Some(opp);
        }
    }
    // Rapid Reanimation (Teron Gorefiend): destroy the marked friendly minion
    // and queue an exact copy to summon once there is space.
    for s in 0..2 {
        let n = sides[s].counters.rapid_reanimation;
        if n == 0 {
            continue;
        }
        for i in 0..sides[s].ms.len() {
            if sides[s].ms[i].m.reanimate && sides[s].ms[i].m.health > 0 {
                let (mut copy, damage) = sides[s].exact_copy(i);
                copy.reanimate = false;
                for _ in 0..n {
                    let _ = sides[s].reanimate.try_push((i, copy, damage));
                }
                let u = sides[s].ms[i].uid;
                let _ = sides[s].self_destroyed.try_push(u);
                sides[s].ms[i].m.health = 0;
                if hit.is_none() {
                    hit = Some(s);
                }
            }
        }
    }
    // It kills at once, Deathrattles and all, like any other Start of Combat
    // damage: a 1-Health minion is gone before the minions' own Start of
    // Combat runs.
    if let Some(opp) = hit
        && sides.iter().any(|side| side.ms.iter().any(|f| f.m.health <= 0))
    {
        resolve_deaths(sides, opp, db, uid, rng, w);
        run_immediate(sides, db, uid, rng, w);
    }
    for s in 0..2 {
        sides[s].spawn_when_space(db, s, uid, rng, w);
    }
    run_immediate(sides, db, uid, rng, w);
    // A trinket's Start of Combat comes at the same step as the hero power's
    // (docs/rules/design.md: hero powers, then the lesser trinket, then the greater).
    // The bundle holds the slots in order, so a lesser trinket's effect runs
    // before a greater one's.
    for s in 0..2 {
        sides[s].trinket_hook(db, s, When::Soc, None, uid, rng, w);
    }
    // Flighty Scout triggers from the hand, so it comes before the board's
    // Start of Combat effects. One copy each, in hand order, if there is
    // board space; the card stays in hand.
    for s in 0..2 {
        for i in 0..sides[s].hand.len() {
            if sides[s].hand_taken(i)
                || sides[s].hand[i]
                    .locked_in_hand(sides[s].counters.tier, sides[s].counters.gold_spent)
                || !matches!(
                    db.get(sides[s].hand[i].card).start_of_combat,
                    Some(StartOfCombat::FlightyScout)
                )
            {
                continue;
            }
            let m = sides[s].hand_at(i);
            let at = sides[s].ms.len();
            sides[s].summon_baked(db, s, at, m, uid, rng, w);
        }
    }
    for s in 0..2 {
        for _ in 0..sides[s].counters.brood_of_nozdormu {
            if let Some(f) = sides[s].ms.first_mut() {
                f.m.attack *= 2;
            }
        }
        for _ in 0..sides[s].counters.upper_hand {
            let opp = s ^ 1;
            let n = sides[opp].ms.len();
            if n > 0 {
                let k = rng.below(n as u32) as usize;
                sides[opp].ms[k].m.health = 1;
            }
        }
    }
    // Sharing is Caring: the left-most minion gains the stats of the
    // nearest enemy minion (its left-most), as they are now. Both sides read
    // before either gains.
    let shared: [(i32, i32); 2] = std::array::from_fn(|s| {
        sides[s ^ 1].ms.first().map_or((0, 0), |f| (f.m.attack, f.m.health))
    });
    for s in 0..2 {
        let n = sides[s].counters.sharing_is_caring as i32;
        if let Some(f) = sides[s].ms.first_mut() {
            f.m.attack += shared[s].0 * n;
            f.m.health += shared[s].1 * n;
        }
    }
    let mut soc: [ArrayVec<u16, BOARD>; 2] = [ArrayVec::new(), ArrayVec::new()];
    for s in 0..2 {
        for f in &sides[s].ms {
            if f.m.parts().any(|(c, _)| db.get(c).start_of_combat.is_some()) {
                soc[s].push(f.uid);
            }
        }
    }
    if soc[0].is_empty() && soc[1].is_empty() {
        return;
    }
    let mut turn = rng.coin() as usize;
    let mut idx = [0usize; 2];
    while idx[0] < soc[0].len() || idx[1] < soc[1].len() {
        if sides[0].ms.is_empty() || sides[1].ms.is_empty() {
            break;
        }
        if idx[turn] >= soc[turn].len() {
            turn ^= 1;
        }
        let me = soc[turn][idx[turn]];
        idx[turn] += 1;
        let Some(mi) = sides[turn].ms.iter().position(|f| f.uid == me) else {
            if idx[turn ^ 1] < soc[turn ^ 1].len() {
                turn ^= 1;
            }
            continue;
        };
        let m = sides[turn].ms[mi].m;
        let mut shields_lost = [false; 2];
        for (card, golden) in m.parts() {
            let mul = if golden { 2 } else { 1 };
            match db.get(card).start_of_combat {
                Some(StartOfCombat::CombatBeastBuff { attack, health }) => {
                    sides[turn].add_combat_beast_buff(db, attack * mul, health * mul);
                }
                Some(StartOfCombat::DamageAllOthers(dmg)) => {
                    let waves = if golden { 2 } else { 1 };
                    let mut total_dealt = 0i32;
                    for _ in 0..waves {
                        for s in [turn, turn ^ 1] {
                            for i in 0..sides[s].ms.len() {
                                if sides[s].ms[i].uid == me {
                                    continue;
                                }
                                let had = sides[s].ms[i].m.keywords.has(Keywords::DIVINE_SHIELD);
                                let dealt = deal_damage(&mut sides[s].ms[i].m, dmg);
                                sides[s].ms[i].taken += dealt;
                                total_dealt += dealt;
                                if had && !sides[s].ms[i].m.keywords.has(Keywords::DIVINE_SHIELD)
                                {
                                    shields_lost[s] = true;
                                }
                                if dealt > 0 && sides[s].damage_trigger {
                                    sides[s].when_damaged(db, s, i, uid, rng, w);
                                }
                            }
                        }
                    }
                    if total_dealt > 0
                        && let Some(mi) = sides[turn].index_of(me)
                    {
                        sides[turn].count_damage(db, turn, mi, total_dealt, uid, rng, w);
                    }
                }
                Some(StartOfCombat::Effect(e)) => {
                    let fx = Fx { side: turn, host: me, killer: None, attacker: None };
                    let mut at = mi + 1;
                    sides[turn].run_effect(db, e, golden, fx, &mut at, uid, rng, w);
                }
                Some(StartOfCombat::AttackImmediately) => {
                    for _ in 0..mul {
                        attack_swings(sides, turn, me, db, uid, rng, w);
                    }
                }
                Some(StartOfCombat::GainEndsStats) => {
                    // All-Seeing Elder: the Attack of your left-most minion
                    // and the Health of your right-most one, as they stand
                    // before this lands. A lone minion gives its own, and a
                    // golden one doubles both.
                    let (la, rh) = if sides[turn].ms.is_empty() {
                        (0, 0)
                    } else {
                        let l = sides[turn].ms.first().map_or(0, |f| f.m.attack);
                        let r = sides[turn].ms.last().map_or(0, |f| f.m.health);
                        (l, r)
                    };
                    if let Some(mi) = sides[turn].index_of(me) {
                        sides[turn].gain_at(db, mi, la * mul, rh * mul);
                    }
                }
                Some(StartOfCombat::Hopebringer { attack, health, improve }) => {
                    let Some(mi) = sides[turn].index_of(me) else { break };
                    let imp = sides[turn].ms[mi].m.improvements as i32;
                    // The improvement is part of the buff, so a golden
                    // Hopebringer doubles it too (+8/+6 per lost shield).
                    let da = attack * mul + imp * improve.0 * mul;
                    let dh = health * mul + imp * improve.1 * mul;
                    for i in 0..sides[turn].ms.len() {
                        if sides[turn].ms[i].m.health > 0 {
                            sides[turn].gain_at(db, i, da, dh);
                        }
                    }
                }
                Some(StartOfCombat::FlightyScout) => {
                    // It triggers from the hand, handled before this loop.
                }
                Some(StartOfCombat::GainHighestHandAttack) => {
                    let Some(mi) = sides[turn].index_of(me) else { break };
                    let mut best = 0;
                    for (i, m) in sides[turn].hand.iter().enumerate() {
                        if !sides[turn].hand_taken(i) && !db.get(m.card).is_spell() {
                            best = best.max(m.attack + sides[turn].hand_bonus[i].0);
                        }
                    }
                    sides[turn].gain_at(db, mi, best * mul, 0);
                }
                Some(StartOfCombat::GainHandStats) => {
                    let Some(mi) = sides[turn].index_of(me) else { break };
                    let (mut da, mut dh) = (0, 0);
                    for (i, m) in sides[turn].hand.iter().enumerate() {
                        if !sides[turn].hand_taken(i) && !db.get(m.card).is_spell() {
                            da += m.attack + sides[turn].hand_bonus[i].0;
                            dh += m.health + sides[turn].hand_bonus[i].1;
                        }
                    }
                    sides[turn].gain_at(db, mi, da * mul, dh * mul);
                }
                // The Dark Gift ones. A gift never doubles, so `mul` is not
                // used here (its part is never golden).
                Some(StartOfCombat::DoubleHealth) => {
                    let Some(mi) = sides[turn].index_of(me) else { break };
                    let h = sides[turn].ms[mi].m.health;
                    // Through `gain_at`, so a keeper (Tarecgosa's Blessing)
                    // keeps what it gains.
                    sides[turn].gain_at(db, mi, 0, h);
                }
                Some(StartOfCombat::DoubleAttack) => {
                    let Some(mi) = sides[turn].index_of(me) else { break };
                    let a = sides[turn].ms[mi].m.attack;
                    sides[turn].gain_at(db, mi, a, 0);
                }
                Some(StartOfCombat::TripleStats) => {
                    let Some(mi) = sides[turn].index_of(me) else { break };
                    let (a, h) = (sides[turn].ms[mi].m.attack, sides[turn].ms[mi].m.health);
                    sides[turn].gain_at(db, mi, a * 2, h * 2);
                }
                Some(StartOfCombat::GainLeftAttack) => {
                    let Some(mi) = sides[turn].index_of(me) else { break };
                    let a = mi
                        .checked_sub(1)
                        .map(|j| sides[turn].ms[j].m.attack)
                        .unwrap_or(0);
                    sides[turn].gain_at(db, mi, a, 0);
                }
                Some(StartOfCombat::GainDeityStats) => {
                    let Some(mi) = sides[turn].index_of(me) else { break };
                    let (a, h) = sides[turn]
                        .counters
                        .deity
                        .map_or((0, 0), |d| (d.attack, d.health));
                    sides[turn].gain_at(db, mi, a, h);
                }
                Some(StartOfCombat::TriggerOwnDeathrattles) => {
                    // Treats the living minion as the source, the way a
                    // trinket's "trigger a friendly Deathrattle" does.
                    let Some(mi) = sides[turn].index_of(me) else { break };
                    let (m, host) = (sides[turn].ms[mi].m, me);
                    let runs = sides[turn].deathrattle_fires(db);
                    sides[turn].fire_deathrattles(
                        db, turn, &m, m.health, host, None, mi + 1, runs, uid, rng, w,
                    );
                }
                Some(StartOfCombat::TriggerAllFriendlyDeathrattles { times }) => {
                    // Snapshot the living board: a Deathrattle that summons
                    // another Deathrattle does not trigger it again here.
                    let uids: ArrayVec<u16, BOARD> =
                        sides[turn].ms.iter().filter(|f| f.m.health > 0).map(|f| f.uid).collect();
                    let reps = times.saturating_mul(mul as u8) as u32;
                    for u in uids {
                        let Some(mi) = sides[turn].index_of(u) else { continue };
                        if sides[turn].ms[mi].m.health <= 0 {
                            continue;
                        }
                        let (m, host) = (sides[turn].ms[mi].m, u);
                        let runs = sides[turn].deathrattle_fires(db).saturating_mul(reps);
                        sides[turn].fire_deathrattles(
                            db, turn, &m, m.health, host, None, mi + 1, runs, uid, rng, w,
                        );
                    }
                }
                Some(StartOfCombat::AdjacentMurlocsHandStats { times }) => {
                    let Some(mi) = sides[turn].index_of(me) else { break };
                    let (mut da, mut dh) = (0, 0);
                    for (i, m) in sides[turn].hand.iter().enumerate() {
                        if !sides[turn].hand_taken(i) && !db.get(m.card).is_spell() {
                            da += m.attack + sides[turn].hand_bonus[i].0;
                            dh += m.health + sides[turn].hand_bonus[i].1;
                        }
                    }
                    let reps = times.saturating_mul(mul as u8) as i32;
                    for j in [mi.checked_sub(1), Some(mi + 1)].into_iter().flatten() {
                        if j < sides[turn].ms.len()
                            && sides[turn].ms[j].m.health > 0
                            && db.tribes_of(&sides[turn].ms[j].m).has(Tribe::Murloc)
                        {
                            sides[turn].gain_at(db, j, da * reps, dh * reps);
                        }
                    }
                }
                Some(StartOfCombat::BecomeCopyLeft) => {
                    let Some(mi) = sides[turn].index_of(me) else { break };
                    if mi == 0 {
                        continue;
                    }
                    let mut copy = sides[turn].ms[mi - 1].m;
                    copy.pool_copies = 0;
                    if golden && !copy.golden {
                        let base = db.get(copy.card);
                        let (gold_a, gold_h) =
                            base.golden_stats.unwrap_or((base.attack * 2, base.health * 2));
                        copy.attack += (gold_a - base.attack).max(0);
                        copy.health += (gold_h - base.health).max(0);
                        copy.golden = true;
                    }
                    sides[turn].ms[mi].m = copy;
                    // The copy's own Start of Combat fires too, unless it is
                    // another Chameleon (which would chase its own left).
                    let new = sides[turn].ms[mi].m;
                    if new.parts().any(|(c, _)| {
                        let sc = db.get(c).start_of_combat;
                        sc.is_some() && !matches!(sc, Some(StartOfCombat::BecomeCopyLeft))
                    }) && !soc[turn].is_full()
                    {
                        let _ = soc[turn].try_push(me);
                    }
                }
                Some(StartOfCombat::CopyLeftDeathrattles { count, times }) => {
                    let mut found: ArrayVec<Attached, BOARD> = ArrayVec::new();
                    for f in &sides[turn].ms {
                        if f.m.health <= 0 || f.uid == me {
                            continue;
                        }
                        if f.m.parts().any(|(c, _)| db.get(c).whirl_o_tron) {
                            continue;
                        }
                        for (card, golden) in f.m.parts() {
                            if db.get(card).deathrattle.is_some() {
                                let _ = found.try_push(Attached { card, golden });
                            }
                        }
                        for &(uid, a) in &sides[turn].gained_dr {
                            if uid == f.uid {
                                let _ = found.try_push(a);
                            }
                        }
                    }
                    let reps = times.saturating_mul(mul as u8);
                    for a in found.iter().take(count as usize) {
                        for _ in 0..reps {
                            if !sides[turn].gained_dr.is_full() {
                                let _ = sides[turn].gained_dr.try_push((me, *a));
                            }
                        }
                    }
                }
                Some(StartOfCombat::SummonOwnCopy) => {
                    let Some(mi) = sides[turn].index_of(me) else { break };
                    let mut m = sides[turn].ms[mi].m;
                    // A summoned copy is generated: no pool copies travel.
                    m.pool_copies = 0;
                    sides[turn].summon(db, turn, mi + 1, m, uid, rng, w);
                }
                None => {}
            }
        }
        for s in 0..2 {
            if shields_lost[s] {
                sides[s].note_shield_lost(db);
            }
        }
        resolve_deaths(sides, turn, db, uid, rng, w);
        run_immediate(sides, db, uid, rng, w);
        if idx[turn ^ 1] < soc[turn ^ 1].len() {
            turn ^= 1;
        }
    }
}

/// Obsidian Ravager's Rally: the minion `host` of side `turn`, which is
/// attacking the minion `target`, deals its Attack to the target and to one
/// random neighbour of it (both when `golden`). It is damage, not an attack,
/// so only the target's side reacts to it, but Divine Shield, Venomous,
/// damage counters and "takes damage" triggers all see it. The deaths it
/// causes are resolved by the caller before the attack itself lands.
#[allow(clippy::too_many_arguments)]
fn rally_damage(
    sides: &mut [Side<'_>; 2],
    turn: usize,
    host: u16,
    golden: bool,
    target: u16,
    db: &CardDb,
    uid: &mut u16,
    rng: &mut Rng,
    w: &mut impl Watch,
) {
    let Some(di) = sides[turn ^ 1].index_of(target) else { return };
    let mut victims: ArrayVec<u16, 3> = ArrayVec::new();
    victims.push(target);
    let def = &sides[turn ^ 1];
    let mut near: ArrayVec<u16, 2> = ArrayVec::new();
    if let Some(l) = di.checked_sub(1) {
        near.push(def.ms[l].uid);
    }
    if di + 1 < def.ms.len() {
        near.push(def.ms[di + 1].uid);
    }
    if golden {
        victims.extend(near);
    } else if !near.is_empty() {
        victims.push(near[rng.below(near.len() as u32) as usize]);
    }
    for v in victims {
        let (att, def) = pair(sides, turn);
        let Some(hi) = att.index_of(host) else { return };
        let Some(vi) = def.index_of(v) else { continue };
        let (xf, yf) = (&mut att.ms[hi], &mut def.ms[vi]);
        let (was_alive, was_shielded) = (yf.m.health > 0, yf.m.keywords.has(Keywords::DIVINE_SHIELD));
        let dealt = hit(&mut yf.m, xf.m.attack, &mut xf.m);
        yf.taken += dealt;
        if was_alive && yf.m.health <= 0 && dealt > 0 {
            yf.killer = Some(xf.uid);
        }
        let shield_lost = was_shielded && !yf.m.keywords.has(Keywords::DIVINE_SHIELD);
        let demon = db.tribes_of(&xf.m).has(Tribe::Demon);
        att.count_damage(db, turn, hi, dealt, uid, rng, w);
        if dealt > 0 && demon {
            att.note_demon_damage(db, host);
        }
        if shield_lost {
            def.note_shield_lost(db);
        }
        if dealt > 0
            && def.damage_trigger
            && let Some(vi) = def.index_of(v)
        {
            def.when_damaged(db, turn ^ 1, vi, uid, rng, w);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn highest_health_damage(
    sides: &mut [Side<'_>; 2],
    turn: usize,
    host: u16,
    golden: bool,
    db: &CardDb,
    uid: &mut u16,
    rng: &mut Rng,
    w: &mut impl Watch,
) {
    let def = &sides[turn ^ 1];
    let highest = def.ms.iter().filter(|f| f.m.health > 0).map(|f| f.m.health).max();
    let Some(highest) = highest else { return };
    let targets: ArrayVec<u16, BOARD> = def.ms.iter()
        .filter(|f| f.m.health > 0 && f.m.health == highest)
        .map(|f| f.uid)
        .collect();
    let target = targets[rng.below(targets.len() as u32) as usize];
    let (att, def) = pair(sides, turn);
    let Some(hi) = att.index_of(host) else { return };
    let Some(vi) = def.index_of(target) else { return };
    let damage = att.ms[hi].m.attack * if golden { 2 } else { 1 };
    let (xf, yf) = (&mut att.ms[hi], &mut def.ms[vi]);
    let was_shielded = yf.m.keywords.has(Keywords::DIVINE_SHIELD);
    let dealt = deal_damage(&mut yf.m, damage);
    yf.taken += dealt;
    if yf.m.health <= 0 && dealt > 0 {
        yf.killer = Some(xf.uid);
    }
    att.count_damage(db, turn, hi, dealt, uid, rng, w);
    if was_shielded && !def.ms[vi].m.keywords.has(Keywords::DIVINE_SHIELD) {
        def.note_shield_lost(db);
    }
    if dealt > 0 && def.damage_trigger {
        def.when_damaged(db, turn ^ 1, vi, uid, rng, w);
    }
}

/// Run the attack triggers of one kind for the minion `me`, which attacks:
/// Rally (for `me` only) and "whenever a friendly minion attacks" before the
/// damage, or "after a friendly minion attacks" after it.
#[allow(clippy::too_many_arguments)]
fn attack_triggers(
    sides: &mut [Side<'_>; 2],
    turn: usize,
    me: u16,
    attacker: &Minion,
    target: Option<u16>,
    after: bool,
    db: &CardDb,
    uid: &mut u16,
    rng: &mut Rng,
    w: &mut impl Watch,
) {
    let trigger = |c: &crate::cards::Card| {
        if after { c.after_friendly_attacks } else { c.whenever_friendly_attacks }
    };
    let mut hosts: ArrayVec<u16, BOARD> = ArrayVec::new();
    for f in &sides[turn].ms {
        if !after && f.m.health <= 0 {
            continue;
        }
        let relevant = f.m.parts().any(|(c, _)| {
            let c = db.get(c);
            (!after && f.uid == me && c.rally.is_some())
                || trigger(c).is_some_and(|t| {
                    (!t.filter.exclude_self || f.uid != me) && filter_ok(db, t.filter, attacker)
                })
        });
        if relevant {
            hosts.push(f.uid);
        }
    }
    for host in hosts {
        let Some(hi) = sides[turn].index_of(host) else { continue };
        let m = sides[turn].ms[hi].m;
        for (card, golden) in m.parts() {
            let c = db.get(card);
            if !after
                && host == me
                && let Some(e) = c.rally
            {
                match e {
                    // These two read or change the minion being attacked,
                    // which is on the other side.
                    Effect::GainTargetAttack => {
                        if let Some(t) = target
                            && let Some(di) = sides[turn ^ 1].index_of(t)
                        {
                            let stat = sides[turn ^ 1].ms[di].m.attack;
                            let mul = if golden { 2 } else { 1 };
                            if let Some(hi) = sides[turn].index_of(host) {
                                sides[turn].gain_at(db, hi, stat * mul, 0);
                            }
                        }
                    }
                    Effect::StripTargetKeywords { mask } => {
                        if let Some(t) = target
                            && let Some(di) = sides[turn ^ 1].index_of(t)
                        {
                            sides[turn ^ 1].ms[di].m.keywords.0 &= !mask;
                        }
                    }
                    Effect::DamageTargetAndNeighbor => {
                        if let Some(t) = target {
                            rally_damage(sides, turn, host, golden, t, db, uid, rng, w);
                        }
                    }
                    Effect::SummonStatCopyAttacker { card } => {
                        // Jailbird Juggernaut: a Golem with this minion's
                        // stats (double when golden) is summoned to its right
                        // and attacks the target first. If it kills the
                        // target, the Juggernaut's own swing fizzles below,
                        // like a Rally that killed it. A full board summons
                        // nothing and the Juggernaut attacks normally.
                        let Some(hi) = sides[turn].index_of(host) else { break };
                        let (a, h) =
                            (sides[turn].ms[hi].m.attack, sides[turn].ms[hi].m.health);
                        let mut golem = db.make(card, false);
                        let k = if golden { 2 } else { 1 };
                        golem.attack = a.saturating_mul(k);
                        golem.health = h.saturating_mul(k);
                        let at = hi + 1;
                        if sides[turn].summon(db, turn, at, golem, uid, rng, w)
                            && let Some(&t) = target.as_ref()
                        {
                            let golem_uid = sides[turn].ms[at].uid;
                            attack_swings_at(sides, turn, golem_uid, Some(t), db, uid, rng, w);
                        }
                    }
                    _ => {
                        let times = {
                            let parts = sides[turn]
                                .ms
                                .iter()
                                .filter(|f| f.m.health > 0)
                                .flat_map(|f| f.m.parts());
                            how_many_times(db, parts, |a| {
                                (a == Aura::Deios).then_some(TimesRole::Setter)
                            })
                        };
                        for _ in 0..times {
                            let Some(hi) = sides[turn].index_of(host) else { break };
                            let fx = Fx {
                                side: turn,
                                host,
                                killer: None,
                                attacker: None,
                            };
                            let mut at = hi + 1;
                            sides[turn].run_effect(db, e, golden, fx, &mut at, uid, rng, w);
                        }
                    }
                }
            }
            if let Some(t) = trigger(c)
                && (!t.filter.exclude_self || host != me)
                && filter_ok(db, t.filter, attacker)
            {
                if t.effect == Effect::DamageHighestHealth {
                    highest_health_damage(sides, turn, host, golden, db, uid, rng, w);
                    continue;
                }
                let Some(hi) = sides[turn].index_of(host) else { break };
                let fx = Fx {
                    side: turn,
                    host,
                    killer: None,
                    attacker: Some(me),
                };
                let mut at = hi + 1;
                sides[turn].run_effect(db, t.effect, golden, fx, &mut at, uid, rng, w);
            }
        }
    }
    // A trinket's "whenever a friendly Beast attacks" comes after the
    // minions' own triggers for the same attack, once per swing.
    if !after {
        sides[turn].trinket_hook(db, turn, When::Attack, Some((me, *attacker)), uid, rng, w);
    }
}

/// Minions that were summoned to attack at once (Eternal Tycoon's Knight)
/// swing now, out of turn, each once, without moving the attack order.
fn run_immediate(
    sides: &mut [Side<'_>; 2],
    db: &CardDb,
    uid: &mut u16,
    rng: &mut Rng,
    w: &mut impl Watch,
) {
    for s in 0..2 {
        while !sides[s].immediate.is_empty() {
            let u = sides[s].immediate.remove(0);
            let ready = sides[s]
                .index_of(u)
                .is_some_and(|i| sides[s].ms[i].m.health > 0 && sides[s].ms[i].m.attack > 0);
            if ready && !sides[s ^ 1].ms.is_empty() {
                attack_swings(sides, s, u, db, uid, rng, w);
            }
        }
    }
}

/// The minion `me` of side `turn` attacks: one swing, or two with Windfury,
/// each ending in the deaths it causes.
fn attack_swings(
    sides: &mut [Side<'_>; 2],
    turn: usize,
    me: u16,
    db: &CardDb,
    uid: &mut u16,
    rng: &mut Rng,
    w: &mut impl Watch,
) {
    attack_swings_at(sides, turn, me, None, db, uid, rng, w);
}

/// `attack_swings` with the first swing's target forced (Jailbird
/// Juggernaut's Golem attacks the Juggernaut's target). A forced target that
/// is already gone means no swing; later Windfury swings pick freely.
#[allow(clippy::too_many_arguments)]
fn attack_swings_at(
    sides: &mut [Side<'_>; 2],
    turn: usize,
    me: u16,
    forced: Option<u16>,
    db: &CardDb,
    uid: &mut u16,
    rng: &mut Rng,
    w: &mut impl Watch,
) {
    let windfury = sides[turn]
        .index_of(me)
        .is_some_and(|i| sides[turn].ms[i].m.keywords.has(Keywords::WINDFURY));
    let mut forced = forced;
    for _ in 0..(if windfury { 2 } else { 1 }) {
        let Some(ai) = sides[turn].index_of(me) else { break };
        if sides[turn].ms[ai].m.attack <= 0 || sides[turn ^ 1].ms.is_empty() {
            break;
        }
        let di = match forced.take() {
            Some(u) => sides[turn ^ 1].index_of(u),
            None => pick_defender(db, &sides[turn ^ 1], rng),
        };
        let Some(di) = di else { break };
        let def_uid = sides[turn ^ 1].ms[di].uid;

        // Attacking ends Stealth.
        sides[turn].ms[ai].m.keywords.clear(Keywords::STEALTH);

        // Rally and "whenever a friendly minion attacks" trigger when the
        // minion attacks, before damage.
        let attacker = sides[turn].ms[ai].m;
        attack_triggers(sides, turn, me, &attacker, Some(def_uid), false, db, uid, rng, w);

        // Rally damage (Obsidian Ravager) can kill before the attack lands.
        // The deaths resolve now; when the target is among them the swing
        // fizzles, but "after a friendly minion attacks" still happens.
        if sides.iter().any(|s| s.ms.iter().any(|f| f.m.health <= 0)) {
            resolve_deaths(sides, turn, db, uid, rng, w);
            let Some(ai) = sides[turn].index_of(me) else { break };
            if sides[turn ^ 1].ms.is_empty() {
                break;
            }
            if sides[turn ^ 1].index_of(def_uid).is_none() {
                let attacker = sides[turn].ms[ai].m;
                attack_triggers(sides, turn, me, &attacker, Some(def_uid), true, db, uid, rng, w);
                resolve_deaths(sides, turn, db, uid, rng, w);
                continue;
            }
        }

        // "When one of your minions is attacked": the defender's Secrets, in
        // the order they were got, before the hit lands (so Autodefense
        // Matrix's Divine Shield blocks it). Nothing here damages anything,
        // and a defender with no Secret pays one field test.
        if !sides[turn ^ 1].secrets.is_empty() {
            sides[turn ^ 1].attack_secrets(db, turn ^ 1, def_uid, uid, rng, w);
        }

        if sides[turn ^ 1].attacked_trigger {
            sides[turn ^ 1].when_friendly_attacked(db, def_uid);
        }
        let Some(ai) = sides[turn].index_of(me) else { break };
        let Some(di) = sides[turn ^ 1].index_of(def_uid) else { break };
        let attacker = sides[turn].ms[ai].m;
        let (att, def) = pair(sides, turn);
        let (x_alive, y_alive) = (att.ms[ai].m.health > 0, def.ms[di].m.health > 0);
        let (x_shielded, y_shielded) = (
            att.ms[ai].m.keywords.has(Keywords::DIVINE_SHIELD),
            def.ms[di].m.keywords.has(Keywords::DIVINE_SHIELD),
        );
        w.attack(turn, &att.ms[ai].m, &def.ms[di].m);
        // One count per swing: a Windfury minion's two attacks count 2
        // (Dragonhead Staff, docs/rules/trinkets.md).
        att.out.friendly_attacks = att.out.friendly_attacks.saturating_add(1);
        let (dx, dy) = (att.ms[ai].m.attack, def.ms[di].m.attack);
        // Dark Gifts Invulnerability: the attacker takes nothing back. The
        // check is one flag test on a minion without gifts.
        let immune = attacker.has_gift(db, |c| c.immune_attacking);
        let def_health_before = def.ms[di].m.health;
        let (xf, yf) = (&mut att.ms[ai], &mut def.ms[di]);
        let dealt_x = hit(&mut yf.m, dx, &mut xf.m);
        let dealt_y = if immune { 0 } else { hit(&mut xf.m, dy, &mut yf.m) };
        yf.taken += dealt_x;
        xf.taken += dealt_y;
        let x_lost = x_shielded && !xf.m.keywords.has(Keywords::DIVINE_SHIELD);
        let y_lost = y_shielded && !yf.m.keywords.has(Keywords::DIVINE_SHIELD);
        let mut kills = 0i32;
        if y_alive && yf.m.health <= 0 && dealt_x > 0 {
            yf.killer = Some(xf.uid);
            kills += 1;
        }
        if x_alive && xf.m.health <= 0 && dealt_y > 0 {
            xf.killer = Some(yf.uid);
        }
        att.count_damage(db, turn, ai, dealt_x, uid, rng, w);
        // Cleave (Blade Collector): the enemies next to the one attacked take
        // the same damage, shields pop, Venomous kills them, and they hit
        // nothing back. The attacker's Venomous is used up by the first kill.
        let cleave_victims = cleave_targets(db, &attacker, def, def_uid);
        let mut cleave_dealt = 0;
        if !cleave_victims.is_empty() {
            let mut src = attacker;
            let mut killed = false;
            for v in cleave_victims {
                let Some(vi) = def.index_of(v) else { continue };
                let (was_alive, was_shielded) =
                    (def.ms[vi].m.health > 0, def.ms[vi].m.keywords.has(Keywords::DIVINE_SHIELD));
                let mut one = src;
                let dealt = hit(&mut def.ms[vi].m, dx, &mut one);
                def.ms[vi].taken += dealt;
                cleave_dealt += dealt;
                if was_alive && def.ms[vi].m.health <= 0 && dealt > 0 {
                    def.ms[vi].killer = Some(me);
                    killed = true;
                    kills += 1;
                }
                if dealt > 0 && def.damage_trigger {
                    def.when_damaged(db, turn ^ 1, vi, uid, rng, w);
                }
                if was_shielded && !def.ms[vi].m.keywords.has(Keywords::DIVINE_SHIELD) {
                    def.note_shield_lost(db);
                }
            }
            // Poisonous did the killing, so a Venomous alongside it is not
            // spent (see `hit`).
            if killed
                && src.keywords.has(Keywords::VENOMOUS)
                && !src.keywords.has(Keywords::POISONOUS)
            {
                src.keywords.clear(Keywords::VENOMOUS);
                if let Some(ai) = att.index_of(me) {
                    att.ms[ai].m.keywords.clear(Keywords::VENOMOUS);
                }
            }
            if cleave_dealt > 0
                && let Some(ai) = att.index_of(me)
            {
                att.count_damage(db, turn, ai, cleave_dealt, uid, rng, w);
            }
        }
        // Wildfire Elemental: when the hit killed the minion, the excess
        // damage goes to one random adjacent enemy (both when golden). The
        // board is still intact, so the neighbours are where they stood.
        if dealt_x > 0 && y_alive && def_health_before <= dx {
            let both = attacker
                .parts()
                .filter(|&(c, _)| db.get(c).overkill)
                .map(|(_, golden)| golden)
                .reduce(|a, b| a || b);
            let excess = dx - def_health_before;
            if let Some(both) = both
                && excess > 0
                && let Some(di) = def.index_of(def_uid)
            {
                let mut near: ArrayVec<u16, 2> = ArrayVec::new();
                if let Some(l) = di.checked_sub(1) {
                    near.push(def.ms[l].uid);
                }
                if di + 1 < def.ms.len() {
                    near.push(def.ms[di + 1].uid);
                }
                if !both && near.len() > 1 {
                    let keep = near[rng.below(near.len() as u32) as usize];
                    near.retain(|u| *u == keep);
                }
                let mut overkill_dealt = 0;
                for v in near {
                    let Some(vi) = def.index_of(v) else { continue };
                    let (was_alive, was_shielded) = (
                        def.ms[vi].m.health > 0,
                        def.ms[vi].m.keywords.has(Keywords::DIVINE_SHIELD),
                    );
                    let dealt = deal_damage(&mut def.ms[vi].m, excess);
                    def.ms[vi].taken += dealt;
                    overkill_dealt += dealt;
                    if was_alive && def.ms[vi].m.health <= 0 && dealt > 0 {
                        def.ms[vi].killer = Some(me);
                        kills += 1;
                    }
                    if dealt > 0 && def.damage_trigger {
                        def.when_damaged(db, turn ^ 1, vi, uid, rng, w);
                    }
                    if was_shielded && !def.ms[vi].m.keywords.has(Keywords::DIVINE_SHIELD) {
                        def.note_shield_lost(db);
                    }
                }
                if overkill_dealt > 0
                    && let Some(ai) = att.index_of(me)
                {
                    att.count_damage(db, turn, ai, overkill_dealt, uid, rng, w);
                }
            }
        }
        // Glory of Combat (Rokara): after a friendly minion attacks and kills
        // an enemy, it gains +1 Attack permanently per kill.
        if kills > 0
            && att.counters.glory_of_combat > 0
            && let Some(ai) = att.index_of(me)
        {
            att.gain_permanent_at(db, ai, kills * att.counters.glory_of_combat as i32, 0);
        }
        // An on-kill reward (Tide Oracle Morgl, Murcules): after it attacks
        // and kills a minion, the reward pays out to a random card in its
        // owner's hand. A golden one gives double.
        if dealt_x > 0
            && y_alive
            && let Some(di) = def.index_of(def_uid)
            && def.ms[di].m.health <= 0
        {
            let reward = attacker.parts().find_map(|(c, golden)| {
                db.get(c).on_kill.map(|r| (r, if golden { 2 } else { 1 }))
            });
            if let Some((reward, mult)) = reward {
                match reward {
                    KillReward::MaxStatsToHand => {
                        // Maximum Health: the Health it had before the hit
                        // plus the damage it had already taken. (Its `taken`
                        // after the hit counts only the damage dealt, so a
                        // Venomous kill, which does not need to deal its
                        // Health, would undercount.)
                        let killed = def.ms[di].m;
                        let taken_before = def.ms[di].taken - dealt_x;
                        let max_health = def_health_before + taken_before;
                        att.buff_random_hand(db, killed.attack * mult, max_health * mult, rng);
                    }
                    KillReward::BuffHand { attack, health } => {
                        att.buff_random_hand(db, attack * mult, health * mult, rng);
                    }
                    KillReward::TripleReward => {
                        if !att.ms[ai].triple_reward_used {
                            att.ms[ai].triple_reward_used = true;
                            if let Some(tok) = db.triple_reward {
                                for _ in 0..mult {
                                    att.out.push_card(tok);
                                }
                            }
                        }
                    }
                }
            }
        }
        let Some(di) = def.index_of(def_uid) else { continue };
        def.count_damage(db, turn ^ 1, di, dealt_y, uid, rng, w);
        // Devout Hellcaller: another friendly Demon dealt damage. The
        // attacker's tribe is a cheap check, so a side without Demons pays
        // nothing extra here.
        if dealt_x > 0 && db.tribes_of(&attacker).has(Tribe::Demon) {
            sides[turn].note_demon_damage(db, me);
        }
        // "Whenever this takes damage" (Very Hungry Winterfinner). It fires
        // even when the damage is lethal.
        if dealt_x > 0 && sides[turn ^ 1].damage_trigger {
            sides[turn ^ 1].when_damaged(db, turn ^ 1, di, uid, rng, w);
        }
        if dealt_y > 0
            && sides[turn].damage_trigger
            && let Some(ai) = sides[turn].index_of(me)
        {
            sides[turn].when_damaged(db, turn, ai, uid, rng, w);
        }

        // "After a friendly minion attacks" triggers (Ravaging Scorpid,
        // Deathstrider).
        attack_triggers(sides, turn, me, &attacker, Some(def_uid), true, db, uid, rng, w);

        // Hopebringer improves when a friendly minion loses Divine Shield.
        if x_lost {
            sides[turn].note_shield_lost(db);
        }
        if y_lost {
            sides[turn ^ 1].note_shield_lost(db);
        }

        // The attacking side's dead are handled first.
        resolve_deaths(sides, turn, db, uid, rng, w);
    }
    run_immediate(sides, db, uid, rng, w);
}

pub fn fight(db: &CardDb, a: &[Minion], b: &[Minion], max_attacks: u32, rng: &mut Rng) -> Outcome {
    fight_with(db, a, b, [Counters::default(); 2], max_attacks, rng, &mut ())
}

pub fn fight_watched(
    db: &CardDb,
    a: &[Minion],
    b: &[Minion],
    max_attacks: u32,
    rng: &mut Rng,
    w: &mut impl Watch,
) -> Outcome {
    fight_with(db, a, b, [Counters::default(); 2], max_attacks, rng, w)
}

pub fn fight_with(
    db: &CardDb,
    a: &[Minion],
    b: &[Minion],
    counters: [Counters; 2],
    max_attacks: u32,
    rng: &mut Rng,
    w: &mut impl Watch,
) -> Outcome {
    fight_with_hands(db, a, b, [&[], &[]], counters, max_attacks, rng, w)
}

/// A fight where each side also brings a hand. Only hand-reading cards look
/// at it; passing empty hands is the same as `fight_with`.
#[allow(clippy::too_many_arguments)]
pub fn fight_with_hands(
    db: &CardDb,
    a: &[Minion],
    b: &[Minion],
    hands: [&[Minion]; 2],
    counters: [Counters; 2],
    max_attacks: u32,
    rng: &mut Rng,
    w: &mut impl Watch,
) -> Outcome {
    let mut uid = 0u16;
    let mut sides = [
        Side::new(db, a, hands[0], counters[0], &mut uid),
        Side::new(db, b, hands[1], counters[1], &mut uid),
    ];
    sides[0].enemy = b;
    sides[1].enemy = a;
    run_start_of_combat(&mut sides, db, &mut uid, rng, w);
    // Who attacks first is counted after Start of Combat: in Power.log
    // 2026_11_11_02_43_24 (game 1, turn 9) a 6 minion side whose Herald
    // Sticker made a Turquoise Skitterer summon a Beetle met 7 with 7 and
    // won the coin flip.
    let (na, nb) = (sides[0].ms.len(), sides[1].ms.len());
    let mut turn = if na > nb {
        0
    } else if nb > na {
        1
    } else {
        rng.coin() as usize
    };
    // What the client shows during combat: the counters at the first attack,
    // after Start of Combat effects (the tracker snapshots them then).
    for s in &mut sides {
        s.out.seen = SeenCounters {
            deity: s.counters.deity,
            gem: (s.counters.gem_attack, s.counters.gem_health),
            spell: (s.counters.spell_attack, s.counters.spell_health),
        };
    }

    let mut attacks = 0;
    let mut skips = 0;
    while !sides[0].ms.is_empty() && !sides[1].ms.is_empty() && attacks < max_attacks {
        // nothing to attack when every enemy has Stealth: the turn passes
        let can_target = sides[turn ^ 1].ms.iter().any(targetable);
        let att = &mut sides[turn];
        let Some(i) = att.next_attacker().filter(|_| can_target) else {
            skips += 1;
            if skips >= 2 {
                break; // neither side can attack
            }
            turn ^= 1;
            continue;
        };
        skips = 0;
        att.ptr = i + 1;
        let me = att.ms[i].uid;
        attack_swings(&mut sides, turn, me, db, &mut uid, rng, w);
        attacks += 1;
        turn ^= 1;
    }
    let (ea, eb) = (sides[0].ms.is_empty(), sides[1].ms.is_empty());
    let winner = match (ea, eb) {
        (false, true) => Some(0),
        (true, false) => Some(1),
        _ => None,
    };
    let survivor_tiers =
        winner.map_or(0, |s| sides[s].ms.iter().map(|f| db.get(f.m.card).tier as i32).sum());
    Outcome { winner, survivor_tiers, attacks, sides: [sides[0].out, sides[1].out] }
}

/// What a Tavern attack leaves behind.
#[derive(Clone, Debug)]
pub struct TavernAttack {
    /// The attacking player's board afterwards, with everything the attack
    /// changed on it (Rally summons and buffs, stats taken from the target).
    pub board: ArrayVec<Minion, BOARD>,
    /// The target if it survived.
    pub target: Option<Minion>,
    /// Cards, Beetle and "future" bonuses and Deathrattles that carry over.
    /// Its `permanent_improvements` are already in `board`.
    pub side: SideOutcome,
}

/// The minion in slot `attacker` of `board` attacks `target`, a minion in the
/// Tavern (Fishbait), once. Rally, "whenever" and "after" triggers, the
/// target's Deathrattle and everything they cause run as in a fight.
pub fn tavern_attack(
    db: &CardDb,
    board: &[Minion],
    attacker: usize,
    target: Minion,
    counters: Counters,
    rng: &mut Rng,
) -> TavernAttack {
    let mut uid = 0u16;
    let counters = Counters {
        boon_of_beetles: 0,
        brood_of_nozdormu: 0,
        upper_hand: 0,
        sharing_is_caring: 0,
        ..counters
    };
    let mut sides = [
        Side::new(db, board, &[], counters, &mut uid),
        Side::new(db, &[target], &[], Counters::default(), &mut uid),
    ];
    sides[0].outside = true;
    let me = sides[0].ms[attacker].uid;
    let target_card = target.card;
    attack_swings(&mut sides, 0, me, db, &mut uid, rng, &mut ());
    // A Tavern card has no side of its own, so the death of the Fishbait has
    // to be reported with the attacker's: a trinket that counts one card's
    // deaths (Snarky Portrait: "Whenever a Fishbait dies") counts this one
    // like a friendly death in a fight.
    if sides[1].ms.is_empty() && counters.trinkets.count_deaths {
        sides[0].out.note_death_card(target_card);
    }
    TavernAttack {
        board: sides[0].ms.iter().map(|f| f.m).collect(),
        target: sides[1].ms.first().map(|f| f.m),
        side: sides[0].out,
    }
}

/// What destroying minions in the shop phase leaves behind.
#[derive(Clone, Debug)]
pub struct ShopDestroy {
    /// The board afterwards: Deathrattle summons, Reborn minions and every
    /// gain of the deaths are in it.
    pub board: ArrayVec<Minion, BOARD>,
    /// Where the tracked minion ended up, if it is still there.
    pub tracked: Option<usize>,
    /// Cards, Undead bonus and other gains that carry over. The board
    /// already has what it gave the minions on it.
    pub side: SideOutcome,
    /// Every minion that died, as it was when it died.
    pub dead: ArrayVec<Minion, 32>,
}

struct DeadWatch(ArrayVec<Minion, 32>);

impl Watch for DeadWatch {
    fn death(&mut self, _side: usize, _pos: usize, m: &Minion) {
        let _ = self.0.try_push(*m);
    }
}

/// Destroy the minions in `victims` (board slots) in the shop phase. They
/// die for real: Deathrattles fire (Titus included), Eternal Knight deaths
/// count, Reborn brings them back in the same slot, and Banshee and Phantom
/// react. The `track`ed board slot is followed through the changes. Avenge
/// does not count shop deaths (its counts start from 0 in every fight).
/// The counters a death outside combat resolves with: everything that applies
/// "wherever the card is" (the Undead bonus, Goldrinn's Beasts, the Beetles)
/// stays, and everything that only a fight sets goes: the trinkets' `fight`
/// acts and the Start of Combat powers. A shop-phase destroy is not a fight
/// (docs/rules/trinkets.md: a `fight` act runs in a fight), so a Funeral Wreath must
/// not copy a minion that is reborn in the shop.
fn shop_counters(counters: Counters) -> Counters {
    Counters {
        health: None,
        boon_of_beetles: 0,
        brood_of_nozdormu: 0,
        upper_hand: 0,
        sharing_is_caring: 0,
        soc_leftmost: 0,
        soc_rightmost: 0,
        soc_double_left: 0,
        soc_earth: 0,
        soc_earth_card: 0,
        soc_lightning: 0,
        soc_lightning_damage: 0,
        trinkets: FightTrinkets::default(),
        ..counters
    }
}

pub fn shop_destroy(
    db: &CardDb,
    board: &[Minion],
    victims: &[usize],
    track: Option<usize>,
    counters: Counters,
    rng: &mut Rng,
) -> ShopDestroy {
    let mut uid = 0u16;
    let counters = shop_counters(counters);
    let mut side = Side::new(db, board, &[], counters, &mut uid);
    side.outside = true;
    side.count_avenge = false;
    let track_uid = track.and_then(|t| side.ms.get(t).map(|f| f.uid));
    for &v in victims {
        if let Some(f) = side.ms.get_mut(v) {
            f.m.health = 0;
        }
    }
    let mut w = DeadWatch(ArrayVec::new());
    side.resolve(db, 0, &mut uid, rng, &mut w);
    ShopDestroy {
        board: side.ms.iter().map(|f| f.m).collect(),
        tracked: track_uid.and_then(|u| side.index_of(u)),
        side: side.out,
        dead: w.0,
    }
}

/// Trigger the Deathrattles of the friendly minion in slot `at` outside
/// combat (Floating Candle Set, Wolfhead Flail): a summon lands where it
/// stands, and whatever the Deathrattles give the player comes back in the
/// outcome. The minion itself does not die.
pub fn shop_trigger_deathrattles(
    db: &CardDb,
    board: &[Minion],
    at: usize,
    counters: Counters,
    rng: &mut Rng,
) -> ShopDestroy {
    let mut uid = 0u16;
    let counters = shop_counters(counters);
    let mut side = Side::new(db, board, &[], counters, &mut uid);
    side.outside = true;
    side.count_avenge = false;
    let mut w = DeadWatch(ArrayVec::new());
    let host = side.ms.get(at).map(|f| (f.m, f.uid));
    if let Some((m, host)) = host {
        let runs = side.deathrattle_fires(db);
        side.fire_deathrattles(db, 0, &m, m.health, host, None, at + 1, runs, &mut uid, rng, &mut w);
        // A Deathrattle can destroy a minion (or summon one that comes in
        // with no Health): resolve those deaths like a shop destroy does.
        side.resolve(db, 0, &mut uid, rng, &mut w);
    }
    ShopDestroy {
        board: side.ms.iter().map(|f| f.m).collect(),
        tracked: None,
        side: side.out,
        dead: w.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> CardDb {
        crate::Data::bundled().cards
    }

    fn side(db: &CardDb, names: &[&str]) -> (Side<'static>, u16) {
        let mut uid = 0;
        let board: Vec<Minion> =
            names.iter().map(|n| db.make(db.by_name(n).unwrap(), false)).collect();
        (Side::new(db, &board, &[], Counters::default(), &mut uid), uid)
    }

    fn names(db: &CardDb, s: &Side) -> Vec<String> {
        s.ms.iter().map(|f| db.get(f.m.card).name.clone()).collect()
    }

    #[test]
    fn deathrattle_summons_where_the_minion_stood() {
        let db = db();
        let (mut s, mut uid) = side(&db, &["Cord Puller", "Buzzing Vermin", "Amber Guardian"]);
        let mut rng = Rng::new(1);
        s.ms[1].m.health = 0;
        s.resolve(&db, 0, &mut uid, &mut rng, &mut ());
        assert_eq!(names(&db, &s), ["Cord Puller", "Beetle", "Amber Guardian"]);
    }

    #[test]
    fn summons_past_seven_are_lost() {
        let db = db();
        let (mut s, mut uid) = side(
            &db,
            &[
                "Amber Guardian",
                "Amber Guardian",
                "Sewer Lord",
                "Amber Guardian",
                "Amber Guardian",
                "Amber Guardian",
                "Amber Guardian",
            ],
        );
        let mut rng = Rng::new(1);
        s.ms[2].m.health = 0;
        s.resolve(&db, 0, &mut uid, &mut rng, &mut ());
        assert_eq!(
            names(&db, &s),
            ["Amber Guardian", "Amber Guardian", "Sewer Rat", "Amber Guardian", "Amber Guardian", "Amber Guardian", "Amber Guardian"]
        );
    }

    #[test]
    fn a_skitterer_summons_then_buffs_its_beetle() {
        // Power.log 2026_11_11_02_43_24 line 152571: the new Beetle stands
        // at the old bonus (2 + 56 + Goldrinn's 7 = 65/62), Banana Slamma
        // makes it 193/62, and only then does the Skitterer's +5/+5 land.
        let db = db();
        let mut uid = 0;
        let mut rng = Rng::new(1);
        let mk = |name: &str| db.make(db.by_name(name).unwrap(), false);
        let counters = Counters { beetle_attack: 5, beetle_health: 5, ..Counters::default() };
        let mut s = Side::new(
            &db,
            &[mk("Turquoise Skitterer"), mk("Banana Slamma")],
            &[],
            counters,
            &mut uid,
        );
        s.ms[0].m.health = 0;
        s.resolve(&db, 0, &mut uid, &mut rng, &mut ());
        let beetle = s.ms.iter().find(|f| db.get(f.m.card).is_beetle).expect("a Beetle");
        // Summoned 7/7 (2 printed + 5 of bonus), base doubled to 14 with the
        // +5 kept on top (19/7), then +5/+5 makes 24/12.
        assert_eq!((beetle.m.attack, beetle.m.health), (24, 12));
    }

    #[test]
    fn a_dead_minion_does_not_hold_its_slot_against_a_summon() {
        // Power.log 274478, Era vs Tickatus 2026-11-17: a Headhunter Gryphon
        // died on its attack, Deathstrider then fired the golden Ravaging
        // Scorpid's Deathrattle, and the game used the dead Gryphon's slot
        // for one Beetle (the second was lost). The deaths block ran after.
        let db = db();
        let (mut s, mut uid) = side(
            &db,
            &[
                "Buzzing Vermin",
                "Amber Guardian",
                "Amber Guardian",
                "Amber Guardian",
                "Amber Guardian",
                "Amber Guardian",
                "Ravaging Scorpid",
            ],
        );
        s.ms[0].m.health = 0;
        s.ms[6].m = db.make(db.by_name("Ravaging Scorpid").unwrap(), true);
        let host = s.ms[6].uid;
        let fx = Fx { side: 0, host, killer: None, attacker: None };
        let mut at = s.ms.len();
        let mut rng = Rng::new(1);
        s.run_effect(
            &db,
            Effect::TriggerDeathrattle { times: 1 },
            false,
            fx,
            &mut at,
            &mut uid,
            &mut rng,
            &mut (),
        );
        // Six living minions leave one slot: one of the two Beetles lands.
        assert_eq!(names(&db, &s).iter().filter(|n| *n == "Beetle").count(), 1);
        // The dead minion's own death was put aside, not skipped: its
        // Deathrattle still runs when the fight resolves deaths.
        assert_eq!(s.out.friendly_deaths, 0);
        let before = s.out.deathrattles_triggered;
        s.resolve(&db, 0, &mut uid, &mut rng, &mut ());
        assert_eq!(s.out.friendly_deaths, 1);
        assert_eq!(s.out.deathrattles_triggered, before + 1);
    }

    #[test]
    fn golden_deathrattle_summons_golden_tokens() {
        let db = db();
        let mut uid = 0;
        let mut rng = Rng::new(1);
        let lord = db.make(db.by_name("Sewer Lord").unwrap(), true);
        let mut s = Side::new(&db, &[lord], &[], Counters::default(), &mut uid);
        s.ms[0].m.health = 0;
        s.resolve(&db, 0, &mut uid, &mut rng, &mut ());
        assert_eq!(s.ms.len(), 2);
        assert!(s.ms.iter().all(|f| f.m.golden && f.m.attack == 6 && f.m.health == 4));
    }

    #[test]
    fn removing_before_the_pointer_keeps_the_next_attacker() {
        let db = db();
        let (mut s, mut uid) = side(&db, &["Cord Puller", "Amber Guardian", "Deadly Spore"]);
        let mut rng = Rng::new(1);
        s.ptr = 2; // Deadly Spore is next
        s.ms[0].m.health = 0;
        s.resolve(&db, 0, &mut uid, &mut rng, &mut ());
        assert_eq!(db.get(s.ms[s.next_attacker().unwrap()].m.card).name, "Deadly Spore");
    }

    fn s2_db() -> CardDb {
        crate::Data::bundled().cards
    }

    fn side2(db: &CardDb, names: &[&str]) -> (Side<'static>, u16) {
        side(db, names)
    }

    #[test]
    fn tasty_lobster_buff_grows_with_each_trigger() {
        let db = s2_db();
        let (mut s, mut uid) = side2(&db, &["Tasty Lobster", "Cage Gnawer", "Cord Puller"]);
        let mut rng = Rng::new(1);
        s.ms[0].m.health = 0;
        s.resolve(&db, 0, &mut uid, &mut rng, &mut ());
        // Cage Gnawer is the only other Beast: 2/7 becomes 4/8, the Mech is left alone
        assert_eq!((s.ms[0].m.attack, s.ms[0].m.health), (4, 8));
        assert_eq!((s.ms[1].m.attack, s.ms[1].m.health), (1, 1));
        assert_eq!(s.counters.improve, [1, 0]);
        assert_eq!(s.out.improve_gain, [1, 0]);

        // a second Lobster, made plain, now gives +4/+2
        let lobster = db.make(db.by_name("Tasty Lobster").unwrap(), false);
        s.summon(&db, 0, 0, lobster, &mut uid, &mut rng, &mut ());
        assert_eq!((s.ms[0].m.attack, s.ms[0].m.health), (2, 1));
        s.ms[0].m.health = 0;
        s.resolve(&db, 0, &mut uid, &mut rng, &mut ());
        assert_eq!((s.ms[0].m.attack, s.ms[0].m.health), (8, 10));
        assert_eq!(s.out.improve_gain, [2, 0]);

        // golden at count 0: twice the buff, and the count still goes up by one
        let gold = db.make(db.by_name("Tasty Lobster").unwrap(), true);
        let (mut s, mut uid) = side2(&db, &["Cage Gnawer"]);
        s.ms.insert(
            0,
            Fm { m: gold, uid: 99, start_slot: None, avenge: 0, damaged_fires: 0, triple_reward_used: false, killer: None, taken: 0, gifts: 0 },
        );
        s.ms[0].m.health = 0;
        s.resolve(&db, 0, &mut uid, &mut rng, &mut ());
        assert_eq!((s.ms[0].m.attack, s.ms[0].m.health), (2 + 4, 7 + 2));
        assert_eq!(s.out.improve_gain, [1, 0]);
    }

    #[test]
    fn lobster_count_is_live_with_titus_and_with_deathstrider() {
        let db = s2_db();
        let mut rng = Rng::new(1);
        // Titus makes the Lobster's Deathrattle trigger twice: +2/+1 then +4/+2
        let (mut s, mut uid) = side2(&db, &["Titus Rivendare", "Tasty Lobster", "Cage Gnawer"]);
        s.ms[1].m.health = 0;
        s.resolve(&db, 0, &mut uid, &mut rng, &mut ());
        assert_eq!((s.ms[1].m.attack, s.ms[1].m.health), (2 + 2 + 4, 7 + 1 + 2));
        assert_eq!(s.out.improve_gain, [2, 0]);

        // Deathstrider's trigger, then the same Lobster dying: +2/+1 then +4/+2. The
        // living Lobster is a Beast too, so the first buff may land on it.
        for seed in 0..8 {
            let mut rng = Rng::new(seed);
            let (mut s, mut uid) = side2(&db, &["Tasty Lobster", "Cage Gnawer"]);
            let fx = Fx { side: 0, host: s.ms[1].uid, killer: None, attacker: None };
            let mut at = 0;
            let e = Effect::TriggerDeathrattle { times: 1 };
            s.run_effect(&db, e, false, fx, &mut at, &mut uid, &mut rng, &mut ());
            let first_on_gnawer = s.ms[1].m.attack == 4;
            assert_eq!(s.ms[0].m.attack + s.ms[1].m.attack, 2 + 2 + 2);
            assert_eq!(s.counters.improve, [1, 0]);
            s.ms[0].m.health = 0;
            s.resolve(&db, 0, &mut uid, &mut rng, &mut ());
            let (a, h) = if first_on_gnawer { (2 + 2 + 4, 7 + 1 + 2) } else { (2 + 4, 7 + 2) };
            assert_eq!((s.ms[0].m.attack, s.ms[0].m.health), (a, h));
            assert_eq!(s.out.improve_gain, [2, 0]);
        }

        // a count carried in from an earlier fight: base +4/+2, golden +8/+4
        let mut uid = 0;
        let counters = Counters { improve: [1, 0], ..Counters::default() };
        let gold = db.make(db.by_name("Tasty Lobster").unwrap(), true);
        let gnawer = db.make(db.by_name("Cage Gnawer").unwrap(), false);
        let mut s = Side::new(&db, &[gold, gnawer], &[], counters, &mut uid);
        s.ms[0].m.health = 0;
        s.resolve(&db, 0, &mut uid, &mut rng, &mut ());
        assert_eq!((s.ms[0].m.attack, s.ms[0].m.health), (2 + 8, 7 + 4));
        assert_eq!(s.counters.improve, [2, 0]);
    }

    #[test]
    fn relentless_deflector_avenges_every_third_friendly_death() {
        let db = s2_db();
        let (mut s, mut uid) = side2(&db, &["Relentless Deflector", "Microbot", "Microbot", "Microbot"]);
        let mut rng = Rng::new(1);
        let deflector = |s: &Side| s.ms[0].m;
        // no Divine Shield, so no Taunt either
        assert!(!has_taunt(&db, &deflector(&s)));
        for k in 1..=3 {
            s.ms[1].m.health = 0;
            s.resolve(&db, 0, &mut uid, &mut rng, &mut ());
            assert_eq!(deflector(&s).keywords.has(Keywords::DIVINE_SHIELD), k == 3, "after death {k}");
        }
        // while it has Divine Shield it has Taunt
        assert!(has_taunt(&db, &deflector(&s)));
        // the count goes on: three more deaths give the shield back
        s.ms[0].m.keywords.clear(Keywords::DIVINE_SHIELD);
        assert!(!has_taunt(&db, &deflector(&s)));
        for k in 1..=3 {
            let m = db.make(db.by_name("Microbot").unwrap(), false);
            s.summon(&db, 0, 1, m, &mut uid, &mut rng, &mut ());
            s.ms[1].m.health = 0;
            s.resolve(&db, 0, &mut uid, &mut rng, &mut ());
            assert_eq!(deflector(&s).keywords.has(Keywords::DIVINE_SHIELD), k == 3, "after death {}", k + 3);
        }
        // its own death, or an enemy's, is not counted: only friendly deaths
        let (mut s, mut uid) = side2(&db, &["Relentless Deflector"]);
        s.ms[0].m.health = 0;
        s.resolve(&db, 0, &mut uid, &mut rng, &mut ());
        assert!(s.ms.is_empty());
    }

    #[test]
    fn ghastcoiler_summons_random_deathrattle_minions() {
        let db = s2_db();
        for seed in 0..40 {
            let mut rng = Rng::new(seed);
            let (mut s, mut uid) = side2(&db, &["Ghastcoiler"]);
            s.ms[0].m.health = 0;
            s.resolve(&db, 0, &mut uid, &mut rng, &mut ());
            assert_eq!(s.ms.len(), 2);
            assert!(s.ms.iter().all(|f| db.get(f.m.card).deathrattle.is_some() && !f.m.golden));

            let gold = db.make(db.by_name("Ghastcoiler").unwrap(), true);
            let mut uid = 0;
            let mut s = Side::new(&db, &[gold], &[], Counters::default(), &mut uid);
            s.ms[0].m.health = 0;
            s.resolve(&db, 0, &mut uid, &mut rng, &mut ());
            assert_eq!(s.ms.len(), 4);
        }
        // only Deathrattle minions up to the owner's Tavern tier
        for tier in 1..=3u8 {
            for seed in 0..40 {
                let mut uid = 0;
                let gold = db.make(db.by_name("Ghastcoiler").unwrap(), true);
                let counters = Counters { tier, ..Counters::default() };
                let mut s = Side::new(&db, &[gold], &[], counters, &mut uid);
                s.ms[0].m.health = 0;
                s.resolve(&db, 0, &mut uid, &mut Rng::new(seed), &mut ());
                assert_eq!(s.ms.len(), 4);
                assert!(s.ms.iter().all(|f| db.get(f.m.card).tier <= tier), "tier {tier}");
            }
        }
        // a full board loses the extra summons
        let (mut s, mut uid) = side2(&db, &["Ghastcoiler", "Cord Puller", "Cord Puller", "Cord Puller", "Cord Puller", "Cord Puller", "Cord Puller"]);
        s.ms[0].m.health = 0;
        s.resolve(&db, 0, &mut uid, &mut Rng::new(3), &mut ());
        assert_eq!(s.ms.len(), 7);
    }

    #[test]
    fn falling_sky_golem_grows_with_every_deathrattle() {
        let db = s2_db();
        let (mut s, mut uid) = side2(&db, &["Falling Sky Golem", "Cord Puller", "Buzzing Vermin"]);
        let mut rng = Rng::new(1);
        s.ms[1].m.health = 0;
        s.resolve(&db, 0, &mut uid, &mut rng, &mut ());
        // Cord Puller's Deathrattle: the Golem is 4/2 + 4/2
        assert_eq!((s.ms[0].m.attack, s.ms[0].m.health), (8, 4));
        assert_eq!(s.out.deathrattles_triggered, 1);
        // a minion without a Deathrattle changes nothing
        let (mut s, mut uid) = side2(&db, &["Falling Sky Golem", "Annoy-o-Module"]);
        s.ms[1].m.health = 0;
        s.resolve(&db, 0, &mut uid, &mut rng, &mut ());
        assert_eq!((s.ms[0].m.attack, s.ms[0].m.health), (4, 2));
        assert_eq!(s.out.deathrattles_triggered, 0);
        // Titus makes each Deathrattle trigger twice, and each one counts
        let (mut s, mut uid) = side2(&db, &["Falling Sky Golem", "Titus Rivendare", "Cord Puller"]);
        s.ms[2].m.health = 0;
        s.resolve(&db, 0, &mut uid, &mut rng, &mut ());
        assert_eq!((s.ms[0].m.attack, s.ms[0].m.health), (12, 6));
        assert_eq!(s.out.deathrattles_triggered, 2);
    }

    #[test]
    fn fishbait_deathrattle_gives_its_killer_plus_five() {
        let db = s2_db();
        let fishbait = db.make(db.by_name("Fishbait").unwrap(), false);
        let beast = db.make(db.by_name("Buzzing Vermin").unwrap(), false);
        let counters = Counters::default();
        // the Beast kills it and gets +5/+5
        let out = tavern_attack(&db, &[beast], 0, fishbait, counters, &mut Rng::new(1));
        assert!(out.target.is_none());
        assert_eq!((out.board[0].attack, out.board[0].health), (1 + 5, 1 + 5));
        // golden Fishbait has 2 Health: one point of damage leaves it alive and gives nothing
        let gold = db.make(db.by_name("Fishbait").unwrap(), true);
        assert_eq!((gold.attack, gold.health), (0, 2));
        let out = tavern_attack(&db, &[beast], 0, gold, counters, &mut Rng::new(1));
        assert_eq!(out.target.map(|t| t.health), Some(1));
        assert_eq!((out.board[0].attack, out.board[0].health), (1, 1));
        // killing the golden one gives +10/+10
        let strong = db.make(db.by_name("Turquoise Skitterer").unwrap(), false);
        let out = tavern_attack(&db, &[strong], 0, gold, counters, &mut Rng::new(1));
        assert!(out.target.is_none());
        assert_eq!((out.board[0].attack, out.board[0].health), (5 + 10, 5 + 10));
        // a Beast with 0 Attack can't attack
        let mut weak = beast;
        weak.attack = 0;
        let out = tavern_attack(&db, &[weak], 0, fishbait, counters, &mut Rng::new(1));
        assert!(out.target.is_some());
        // A trinket that counts one card's deaths takes this one where the
        // dead card would report it: the attacker's side (Snarky Portrait
        // counts Fishbaits that die in the Tavern).
        let counting = Counters {
            trinkets: FightTrinkets { count_deaths: true, ..FightTrinkets::default() },
            ..Counters::default()
        };
        let out = tavern_attack(&db, &[beast], 0, fishbait, counting, &mut Rng::new(1));
        assert_eq!(out.side.n_death_cards, 1);
        assert_eq!(out.side.death_cards[0], (fishbait.card, 1));
        // One that survives its attack is no death, and with no trinket
        // counting deaths nothing is recorded either.
        let out = tavern_attack(&db, &[beast], 0, gold, counting, &mut Rng::new(1));
        assert_eq!(out.side.n_death_cards, 0);
        let out = tavern_attack(&db, &[beast], 0, fishbait, counters, &mut Rng::new(1));
        assert_eq!(out.side.n_death_cards, 0);
    }

    /// A Side built from card names, golden marked with a trailing '!'.
    fn gem_side(db: &CardDb, names: &[&str]) -> (Side<'static>, u16) {
        let mut uid = 0;
        let board: Vec<Minion> = names
            .iter()
            .map(|n| {
                let (n, golden) = match n.strip_suffix('!') {
                    Some(plain) => (plain, true),
                    None => (*n, false),
                };
                db.make(db.by_name(n).unwrap(), golden)
            })
            .collect();
        (Side::new(db, &board, &[], Counters::default(), &mut uid), uid)
    }

    /// A fight's Gems on every other friendly minion (Bonker's Rally, a
    /// trinket's act), played the way `run_effect` plays them.
    fn gem_others(
        db: &CardDb,
        s: &mut Side,
        host: usize,
        count: u8,
        uid: &mut u16,
        rng: &mut Rng,
    ) {
        let h = s.ms[host].uid;
        let fx = Fx { side: 0, host: h, killer: None, attacker: None };
        let mut at = s.ms.len();
        s.run_effect(db, Effect::GemOthers(count), false, fx, &mut at, uid, rng, &mut ());
    }

    /// The Gems each minion has taken, counted as the stats it gained: a
    /// fight does not keep `Minion::gem_attack` (that is the board's, from
    /// the shop), it adds the Gem's stats straight away.
    fn gems_taken(before: &[(i32, i32)], s: &Side) -> Vec<i32> {
        s.ms.iter()
            .zip(before)
            .map(|(f, &(a, h))| (f.m.attack - a).min(f.m.health - h))
            .collect()
    }

    fn stats(s: &Side) -> Vec<(i32, i32)> {
        s.ms.iter().map(|f| (f.m.attack, f.m.health)).collect()
    }

    #[test]
    fn a_fight_gem_on_a_roogug_plays_one_on_another_minion() {
        // docs/rules/design.md "Geomagus Roogug": every Blood Gem played on it
        // makes it play its own Gems on a random different friendly minion,
        // in a fight as in the shop (`Game::gem_on_board`). Bonker's Gem
        // others puts one Gem on each of the other two.
        let db = db();
        let (mut s, mut uid) = gem_side(&db, &["Bonker", "Geomagus Roogug", "Tusked Camper"]);
        let before = stats(&s);
        let mut rng = Rng::new(1);
        gem_others(&db, &mut s, 0, 1, &mut uid, &mut rng);
        let gems = gems_taken(&before, &s);
        assert_eq!(gems[1], 1, "the Gem played on the Roogug");
        assert_eq!(gems[0] + gems[2], 2, "Bonker's own Gem plus the Roogug's one");
        assert_eq!(gems[0].max(gems[2]), 2, "the Roogug's Gem landed on one of them");
    }

    #[test]
    fn a_fight_gem_on_a_golden_roogug_plays_two() {
        let db = db();
        let (mut s, mut uid) = gem_side(&db, &["Bonker", "Geomagus Roogug!", "Tusked Camper"]);
        let before = stats(&s);
        let mut rng = Rng::new(1);
        gem_others(&db, &mut s, 0, 1, &mut uid, &mut rng);
        let gems = gems_taken(&before, &s);
        assert_eq!(gems[1], 1, "the Gem played on the golden Roogug");
        assert_eq!(gems[0] + gems[2], 3, "Bonker's own Gem plus the golden Roogug's two");
    }

    #[test]
    fn two_roogugs_in_a_fight_never_loop() {
        // Each Roogug reacts to Bonker's Gem, and each Gem it plays carries
        // the from-Roogug mark: the other Roogug it lands on does not react,
        // so the board takes exactly 2 + 2 Gems and never one more.
        let db = db();
        let (mut s, mut uid) = gem_side(&db, &["Bonker", "Geomagus Roogug", "Geomagus Roogug"]);
        let before = stats(&s);
        let mut rng = Rng::new(1);
        gem_others(&db, &mut s, 0, 1, &mut uid, &mut rng);
        let gems = gems_taken(&before, &s);
        assert_eq!(gems.iter().sum::<i32>(), 4, "Bonker's two Gems and one reaction each");
        assert!(gems[1].min(gems[2]) >= 1, "each Roogug kept its own Gem");
    }

    #[test]
    fn a_roogugs_own_gem_on_a_roogug_does_not_trigger_it() {
        // The Gem a Roogug plays carries the from-Roogug mark, so on a
        // Roogug it is stats and nothing else.
        let db = db();
        let (mut s, _uid) = gem_side(&db, &["Geomagus Roogug", "Tusked Camper"]);
        let before = stats(&s);
        let mut rng = Rng::new(1);
        s.gem_at(&db, 0, 1, false, true, &mut rng);
        assert_eq!(gems_taken(&before, &s), [1, 0], "stats only, no reaction");
        // The same Gem without the mark makes the Roogug play its own on the
        // Camper.
        let before = stats(&s);
        s.gem_at(&db, 0, 1, false, false, &mut rng);
        assert_eq!(gems_taken(&before, &s), [1, 1]);
    }

    #[derive(Default)]
    struct Attacks(Vec<(usize, CardId)>);

    impl Watch for Attacks {
        fn attack(&mut self, side: usize, attacker: &Minion, _defender: &Minion) {
            self.0.push((side, attacker.card));
        }
    }

    fn attack_names(db: &CardDb, rec: &Attacks, side: usize) -> Vec<String> {
        rec.0
            .iter()
            .filter(|(s, _)| *s == side)
            .map(|(_, c)| db.get(*c).name.clone())
            .collect()
    }

    #[test]
    fn a_rally_summon_attacks_before_an_older_right_neighbour() {
        // Hearthstone_2026_11_13_14_50_52 lines 80561 to 80960: Hoarding
        // Hyena rallies a Tasty Lobster to its right, and the token attacks
        // next, before the Hyena's older right neighbours.
        let db = db();
        let a = vec![
            db.make(db.by_name("Flittering Bat").unwrap(), false),
            db.make(db.by_name("Cord Puller").unwrap(), false),
        ];
        let mut b = vec![db.make(db.by_name("Aberrant Tentacle").unwrap(), false)];
        b[0].health = 100;
        let mut rng = Rng::new(1);
        let mut rec = Attacks::default();
        fight_watched(&db, &a, &b, 20, &mut rng, &mut rec);
        let names = attack_names(&db, &rec, 0);
        assert_eq!(names[0], "Flittering Bat");
        assert_eq!(names[1], "Foraging Bat");
    }

    #[test]
    fn a_rally_summon_at_the_right_edge_takes_the_next_attack() {
        // Hearthstone_2026_11_13_14_50_52 lines 491415 and 491779: their
        // only Flittering Bat is right-most, its Foraging Bat is summoned
        // past it, and the token attacks next.
        let db = db();
        let a = vec![db.make(db.by_name("Flittering Bat").unwrap(), false)];
        let mut b = vec![db.make(db.by_name("Aberrant Tentacle").unwrap(), false)];
        b[0].health = 100;
        let mut rng = Rng::new(1);
        let mut rec = Attacks::default();
        fight_watched(&db, &a, &b, 20, &mut rng, &mut rec);
        let names = attack_names(&db, &rec, 0);
        assert_eq!(names[0], "Flittering Bat");
        assert_eq!(names[1], "Foraging Bat");
    }

    #[test]
    fn an_attackers_deathrattle_summon_takes_its_slot() {
        // No pure Deathrattle of the attacker was found in the logs. Helping
        // Hand's Reborn copy at Hearthstone_2026_11_13_14_50_52 line 948920
        // shows the same slot and the same order; Buzzing Vermin is the
        // Deathrattle version of that case.
        let db = db();
        let mut a = vec![
            db.make(db.by_name("Buzzing Vermin").unwrap(), false),
            db.make(db.by_name("Cord Puller").unwrap(), false),
        ];
        a[0].attack = 1;
        a[0].health = 1;
        let mut b = vec![db.make(db.by_name("Aberrant Tentacle").unwrap(), false)];
        b[0].attack = 1;
        b[0].health = 100;
        let mut rng = Rng::new(1);
        let mut rec = Attacks::default();
        fight_watched(&db, &a, &b, 20, &mut rng, &mut rec);
        let names = attack_names(&db, &rec, 0);
        assert_eq!(names[0], "Buzzing Vermin");
        assert_eq!(names[1], "Beetle");
    }

    #[test]
    fn a_summon_left_of_the_pointer_keeps_the_next_attacker() {
        // Hearthstone_2026_11_08_03_26_06 lines 1064395 and 1064994: a
        // friendly minted at position 3 while the pointer was past it did
        // not jump the minion already next in line.
        let db = db();
        let (mut s, mut uid) = side(&db, &["Cord Puller", "Amber Guardian", "Deadly Spore"]);
        let mut rng = Rng::new(1);
        s.ptr = 2; // Deadly Spore is next
        let m = db.make(db.by_name("Foraging Bat").unwrap(), false);
        assert!(s.summon(&db, 0, 0, m, &mut uid, &mut rng, &mut ()));
        assert_eq!(db.get(s.ms[s.next_attacker().unwrap()].m.card).name, "Deadly Spore");
    }

    #[test]
    fn the_bg_odds_flittering_bat_fight_can_tie() {
        // train/runs/bg-odds/report.md section 3: mine has Flittering Bat
        // 2/2 and Aberrant Tentacle 0/2, theirs has Flittering Bat 1/4. The
        // real fight was a tie.
        let db = db();
        let mut mine = vec![
            db.make(db.by_name("Flittering Bat").unwrap(), false),
            db.make(db.by_name("Aberrant Tentacle").unwrap(), false),
        ];
        mine[0].attack = 2;
        mine[0].health = 2;
        let theirs = vec![db.make(db.by_name("Flittering Bat").unwrap(), false)];
        let mut ties = 0;
        for seed in 0..1000 {
            let mut rng = Rng::new(seed);
            let o = fight(&db, &mine, &theirs, 500, &mut rng);
            if o.winner.is_none() && o.attacks < 500 {
                ties += 1;
            }
        }
        assert!(ties > 0, "the boards tie in at least one seeded fight");
    }
}
