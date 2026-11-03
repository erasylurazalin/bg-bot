use arrayvec::ArrayVec;
use serde::Deserialize;

use crate::minion::{Attached, CardId, GiftOn, Keywords, MAX_ATTACHED, Minion};
use crate::rng::Rng;
use crate::BOARD;

/// A per-player count that an "improve your future ..." effect raises and
/// scales by. Each variant is one slot in `Counters::improve`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Counter {
    TastyLobster,
    /// Ballers sold this game, shared by Fire, Snow and Air Baller.
    Baller,
}

pub const N_COUNTERS: usize = 2;

impl Counter {
    pub fn index(self) -> usize {
        self as usize
    }
}

fn counter(name: &str) -> Result<Counter, String> {
    match name {
        "tasty_lobster" => Ok(Counter::TastyLobster),
        "baller" => Ok(Counter::Baller),
        other => Err(format!("unknown counter {other:?}")),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tribe {
    Beast,
    Mech,
    Dragon,
    Murloc,
    Demon,
    Pirate,
    Elemental,
    Undead,
    Quilboar,
    Aberration,
}

impl Tribe {
    /// Every tribe a card can have.
    pub const EVERY: [Tribe; 10] = [
        Tribe::Beast,
        Tribe::Mech,
        Tribe::Dragon,
        Tribe::Murloc,
        Tribe::Demon,
        Tribe::Pirate,
        Tribe::Elemental,
        Tribe::Undead,
        Tribe::Quilboar,
        Tribe::Aberration,
    ];

    fn bit(self) -> u16 {
        1 << self as u16
    }
}

/// The tribes of a card: none (neutral), one, two, or all ("all").
/// Every tribe filter in the game asks `has`, so a two-tribe card counts for
/// both and an "all" card counts for every tribe.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Tribes(u16);

impl Tribes {
    pub const NONE: Tribes = Tribes(0);
    pub const ALL: Tribes = Tribes(0b11_1111_1111);

    pub fn of(t: Tribe) -> Tribes {
        Tribes(t.bit())
    }

    /// This set plus `t`.
    pub fn with(self, t: Tribe) -> Tribes {
        Tribes(self.0 | t.bit())
    }

    pub fn has(self, t: Tribe) -> bool {
        self.0 & t.bit() != 0
    }

    /// The raw bit mask (ten bits, `Tribe::EVERY` order).
    pub fn mask(self) -> u16 {
        self.0
    }

    /// This set plus every tribe in `other`.
    pub fn with_all(self, other: Tribes) -> Tribes {
        Tribes(self.0 | other.0)
    }

    /// Neutral: no tribe at all.
    pub fn is_neutral(self) -> bool {
        self.0 == 0
    }

    /// The test a tribe filter asks for, on a minion's tribes: this set has
    /// `tribe`, or `None` asks for any minion. It goes with
    /// `CardDb::tribes_of`, so an Amalgamation and a portrait's extra tribes
    /// count (docs/rules/trinkets.md). A card definition (a pool draw, an
    /// offer) has no minion: it tests its own `tribes` directly.
    pub fn is(self, tribe: Option<Tribe>) -> bool {
        tribe.is_none_or(|t| self.has(t))
    }

    /// Whether the two sets share a tribe ("a minion of its type").
    pub fn overlaps(self, other: Tribes) -> bool {
        self.0 & other.0 != 0
    }

    /// One of the card's tribes picked uniformly at random; None when it is
    /// neutral (docs/rules/design.md "Types": a dual or all-type minion uses one of its
    /// types at random).
    pub fn random_type(self, rng: &mut Rng) -> Option<Tribe> {
        let n = self.0.count_ones();
        if n == 0 {
            return None;
        }
        let mut k = rng.below(n);
        for t in Tribe::EVERY {
            if self.has(t) {
                if k == 0 {
                    return Some(t);
                }
                k -= 1;
            }
        }
        None
    }

    /// A lobby: `n` of the ten tribes, distinct, uniformly at random
    /// (docs/rules/design.md "The lobby's tribes": every game has 5).
    pub fn random_lobby(n: usize, rng: &mut Rng) -> Tribes {
        let mut left: ArrayVec<Tribe, { Tribe::EVERY.len() }> = Tribe::EVERY.into_iter().collect();
        let mut lobby = Tribes::NONE;
        for _ in 0..n {
            let k = rng.below(left.len() as u32) as usize;
            lobby = lobby.with(left[k]);
            left.swap_remove(k);
        }
        lobby
    }

    /// How many tribes the set holds.
    pub fn len(self) -> u32 {
        self.0.count_ones()
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
}

/// "Give a friendly minion of each type ..." (Misplaced Tea Set, The Last One
/// Standing): each type on the board goes to a different minion that has it,
/// so every minion gets at most one buff, even a dual or all-type one, and as
/// many minions as possible get one: a type is never wasted while another
/// minion could take it (Era 2026-11-03, docs/rules/design.md "Types"). With
/// a Demon/Dragon and a Dragon, the Demon/Dragon always takes the Demon buff
/// and the Dragon the Dragon buff. Among equally good sharings (two Demons)
/// the pick is random. `tribes[i]` is the types of board minion `i`,
/// `Tribes::NONE` for one that can't be picked (dead). Returns the picked
/// indexes.
///
/// It is a maximum matching of minions to types (augmenting paths, minions
/// and types tried in random order). An all-type minion (Gatekeeper Amalgam)
/// needs no special case: with ten types and at most seven minions there is
/// always a type left for it, so it always gets exactly one buff. Era's log
/// backs this: all 49 Tea Sets in `15_25_26-g1` turns 10 and 11 gave every
/// typed minion exactly one buff.
pub fn pick_one_per_type(tribes: &[Tribes], rng: &mut Rng) -> ArrayVec<usize, BOARD> {
    const T: usize = Tribe::EVERY.len();
    let n = tribes.len().min(BOARD);
    let mut minions: ArrayVec<usize, BOARD> = (0..n).collect();
    rng.shuffle(&mut minions);
    // owner[t]: the minion holding type `t`, if any.
    let mut owner: [Option<usize>; T] = [None; T];
    for &m in &minions {
        let mut types: ArrayVec<usize, T> =
            (0..T).filter(|&t| tribes[m].has(Tribe::EVERY[t])).collect();
        if types.is_empty() {
            continue;
        }
        rng.shuffle(&mut types);
        let mut seen = [false; T];
        augment(m, tribes, &types, &mut owner, &mut seen);
    }
    let mut picked: ArrayVec<usize, BOARD> = ArrayVec::new();
    for &m in &minions {
        if owner.contains(&Some(m)) {
            picked.push(m);
        }
    }
    picked
}

/// One augmenting-path step for `pick_one_per_type`: give minion `m` one of
/// `types` (its own types, in its random order), moving a holder to another
/// of its types if needed. True if `m` got a type.
fn augment(
    m: usize,
    tribes: &[Tribes],
    types: &[usize],
    owner: &mut [Option<usize>; Tribe::EVERY.len()],
    seen: &mut [bool; Tribe::EVERY.len()],
) -> bool {
    for &t in types {
        if seen[t] {
            continue;
        }
        seen[t] = true;
        let free = match owner[t] {
            None => true,
            Some(o) => {
                let theirs: ArrayVec<usize, { Tribe::EVERY.len() }> = (0..Tribe::EVERY.len())
                    .filter(|&u| tribes[o].has(Tribe::EVERY[u]))
                    .collect();
                augment(o, tribes, &theirs, owner, seen)
            }
        };
        if free {
            owner[t] = Some(m);
            return true;
        }
    }
    false
}

/// Which Tavern minion a consuming minion takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ConsumePick {
    /// The player picks one in the Tavern.
    Chosen,
    Random,
    HighestHealth,
}

/// Who does the consuming.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ConsumeBy {
    /// The minion with the effect.
    This,
    /// A friendly minion the player picks, of this tribe if one is given.
    Chosen(Option<Tribe>),
}

/// Battlecry and general minion card text effects. Also what Activate, event
/// triggers, "after your hero takes damage" and some end of turn effects run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Effect {
    BuffDeity { attack: i32, health: i32, per_spell: (i32, i32) },
    BuffSelfAndDeity { attack: i32, health: i32 },
    BuffDeityAndImprove { attack: i32, health: i32 },
    BuffLeftmostPerDiscard { attack: i32, health: i32 },
    GetRandomSpellUpTo { cost: Option<u8>, count: u8 },
    GetLinkedSpells { count: u8 },
    GetLinkedMinions { tribe: Tribe, count: u8 },
    DiscardLeftmostSpells { count: u8, attack: i32, health: i32 },
    SummonAndBuffAll { card: CardId, count: u8, attack: i32, golden_doubles_count: bool },
    SummonDeadAberrations { count: u8 },
    DamageHighestHealth,
    /// Summon `count` copies of a card (a token or a pool minion) next to the
    /// minion. A golden minion summons `count` golden copies, or, with
    /// `golden_doubles_count`, twice as many plain ones. Fights only.
    Summon { card: CardId, count: u8, golden_doubles_count: bool },
    /// Buff one other friendly minion the player picks. `per_keyword` is
    /// added once more for each different Bonus Keyword in the warband.
    BuffChosen {
        tribe: Option<Tribe>,
        attack: i32,
        health: i32,
        per_keyword: (i32, i32),
        random_keyword: bool,
        /// The pick may be a Tavern minion (a text that says "another
        /// Murloc" or "a Beast", not "a friendly minion").
        in_shop: bool,
    },
    /// Buff one minion the player picks on the board or in the Tavern
    /// ("another minion", Suspicious Prisonguard).
    BuffAnyMinion { attack: i32, health: i32 },
    /// Set one minion's (board or Tavern) stats (Tyrael).
    SetStatsAnyMinion { attack: i32, health: i32 },
    /// Buff every other friendly minion.
    BuffOthers { tribe: Option<Tribe>, attack: i32, health: i32, per_keyword: (i32, i32) },
    /// Buff every friendly minion, the one with the effect included.
    BuffAll { tribe: Option<Tribe>, attack: i32, health: i32 },
    /// Buff the minion itself.
    BuffSelf { attack: i32, health: i32 },
    /// Buff a random friendly minion (of a tribe, if given). With `improve`
    /// set the buff is multiplied by 1 plus the player's count for that
    /// counter, and the count then goes up by one ("Improve your future ...").
    BuffRandomFriendly { tribe: Option<Tribe>, attack: i32, health: i32, improve: Option<Counter> },
    /// Buff the minion that killed the one with the effect (Fishbait).
    BuffKiller { attack: i32, health: i32 },
    /// The minion gains a keyword.
    GainKeyword { keyword: u8 },
    /// Get a random pool minion of a tribe.
    GetRandomMinion { tribe: Tribe },
    /// Get `count` random Tavern spells that cost `cost`.
    GetRandomSpell { cost: u8, count: u8 },
    /// Summon `count` random pool minions that have a Deathrattle. Fights only.
    SummonRandomDeathrattle { count: u8 },
    /// Trigger your left-most Deathrattle `times` times. Fights only.
    TriggerDeathrattle { times: u8 },
    /// Pick a Tavern card, replace it with a Fishbait and have your left-most
    /// Beast attack it.
    ReplaceWithFishbait { card: CardId },
    /// Refresh the Tavern for free, put a Fishbait in the left-most slot of it and
    /// have your left-most Beast attack it.
    RefreshWithFishbait { card: CardId },
    /// The next Magnetization to this minion this turn happens `n` extra
    /// times.
    ExtraMagnetization(u8),
    /// The minion gains a random Bonus Keyword it doesn't have.
    RandomKeyword,
    /// A minion consumes minions from the Tavern and gains their stats, and
    /// their Bonus Keywords when `keywords` is set.
    Consume { by: ConsumeBy, pick: ConsumePick, keywords: bool, count: u8 },
    /// Buff the Tavern minions (of a tribe, if given), now and, unless
    /// `this_turn`, for the rest of the game.
    TavernBuff { tribe: Option<Tribe>, attack: i32, health: i32, this_turn: bool },
    /// Deal damage to your own hero (never below the self damage floor).
    DamageHero(i32),
    /// Undo the damage your hero just took.
    RewindDamage,
    /// Magnetize a random Volumizer to this minion and get a copy of it.
    MagnetizeVolumizerAndCopy,
    /// Give your Beetles +attack/+health this game.
    BeetleBuff { attack: i32, health: i32 },
    /// Gain extra Gold next turn.
    GoldNextTurn(u8),
    /// Get a random Magnetic Volumizer (2 when golden).
    RandomVolumizer,
    /// The next Tavern spell you buy costs `n` less.
    SpellDiscount(u8),
    /// Get `count` copies of a generated card (`count * 2` when golden).
    GetCard { card: CardId, count: u8 },
    /// Raise the player's Tavern spell bonus for the rest of the game.
    SpellBonus { attack: i32, health: i32 },
    /// Get a random Chromadrake (`2` when golden).
    RandomChromadrake,
    /// Cast a named Tavern spell `times` times for the minion's side.
    /// Fights only run the spell effects the fight knows (Shiny Ring's
    /// board buff and Mighty Dragonbreath).
    CastSpell { card: CardId, times: u8 },
    /// Activate only: trigger a friendly minion's Rally, chosen by the
    /// player, in the shop.
    TriggerRally,
    /// Buff the minion that just attacked (Roaring Recruiter).
    BuffAttacker { attack: i32, health: i32 },
    /// Give `count` random other friendly minions of a tribe stats and a
    /// keyword (Amber Guardian). A golden minion picks `count` twice as many.
    BuffRandomOther { tribe: Option<Tribe>, attack: i32, health: i32, keyword: u8, count: u8 },
    /// Summon the `count` highest-Attack minions of a tribe (or any minion)
    /// from hand, for this combat only (Expert Aviator, Diremuck Forager,
    /// Sneed's New Shredder). The copies leave the hand for the rest of the
    /// fight and are back in it afterwards.
    SummonFromHand { tribe: Option<Tribe>, count: u8, by_health: bool, shield: bool },
    /// Give a random minion in hand stats, permanently (Very Hungry
    /// Winterfinner). Fights only.
    BuffRandomHand { attack: i32, health: i32 },
    /// Give the left-most minion in hand stats (Twilight Tidehunter).
    BuffLeftmostHand { attack: i32, health: i32 },
    /// Give the minions of a tribe on the board and in hand stats (Shamanic
    /// Tidecaller).
    BuffTribeBoardAndHand { tribe: Tribe, attack: i32, health: i32 },
    /// Trigger a friendly minion's Battlecry, chosen by the player, in the
    /// shop (Kelp Keeper).
    TriggerBattlecry,
    /// If you control another minion of a tribe, Discover `count` minions of
    /// it (Primalfin Lookout).
    DiscoverTribeIfAnother { tribe: Tribe, count: u8 },
    /// Cast the Tavern spell this minion was taught (Magicfin Apprentice).
    CastTaughtSpell,
    /// Trigger the Battlecries of the adjacent minions this many times
    /// (Young Murk-Eye).
    TriggerAdjacentBattlecries(u8),
    /// Add `count` upcoming Refreshes that each include a Fodder (two when
    /// golden). Only the current turn's coming Refreshes, not the past ones.
    AddFodderRefreshes(u8),
    /// Add Fodders to the very next Refresh (Twisted Wrathguard). Golden adds
    /// twice as many to it.
    AddFodderToNextRefresh(u8),
    /// Every friendly Demon consumes `count` random Tavern minions (golden
    /// doubles the stats taken). Soulkeeping Jailer.
    DemonsConsume { count: u8 },
    /// Raise this minion's `improvements` by `n` (Sacrificial Wrathguard's
    /// "Improve this"). The count is Activates, so golden does not double
    /// it; the Deathrattle's per-improvement number is doubled instead.
    ImproveSelf(u8),
    /// Give every minion in the Tavern stats and one random keyword from
    /// `mask`, the same keyword for all of them (Deft Deserter).
    TavernBuffRandomKeyword { attack: i32, health: i32, mask: u8 },
    /// Discover `count` Demons up to the player's tier; the hero then takes
    /// damage equal to the tier of each one chosen (Imposing Percussionist).
    DiscoverDemonTierDamage(u8),
    /// Steal the `count` highest-Attack minions from the Tavern into hand
    /// (Decoy Conjurer). Golden steals two.
    StealTavern { count: u8 },
    /// Discover `count` Tavern spells up to the player's tier (Rodeo
    /// Performer). Golden discovers two.
    DiscoverSpell { count: u8 },
    /// Rally: gain the Attack of the minion this one attacks, for this fight
    /// (Heroic Underdog). A golden one gains double.
    GainTargetAttack,
    /// Rally: remove these keywords from the minion this one attacks, for
    /// this fight (Sin'dorei Straight Shot).
    StripTargetKeywords { mask: u8 },
    /// Deathrattle: destroy the minion that killed this one, ignoring Divine
    /// Shield (Leeroy the Reckless).
    DestroyKiller,
    /// Deathrattle: give `count` friendly Dragons Divine Shield, picking
    /// Dragons without it first (Nadina the Red).
    ShieldDragons { count: u8 },
    /// Get a random minion of exactly this tier (Highkeeper Ra gets Tier 6).
    /// Golden gets two.
    GetRandomTierMinion { tier: u8 },
    /// Give a friendly minion of each type the stats permanently, one random
    /// minion per type and at most one buff per minion (The Last One
    /// Standing). A golden minion does the whole thing twice.
    BuffEachType { attack: i32, health: i32 },
    /// Rally: deal this minion's Attack to the minion it attacks and to one
    /// random minion next to it (both neighbours when golden). Obsidian
    /// Ravager.
    DamageTargetAndNeighbor,
    /// Give this minion's current stats to the left-most minion in hand
    /// (Futurefin). A golden minion gives double.
    GiveOwnStatsToLeftmostHand,
    /// Get a Lockbox, or open the one in hand `sooner` turns sooner (golden
    /// doubles the number of turns, it is still one Lockbox).
    GetLockbox { sooner: u8 },
    /// Get a random Bounty (two when golden).
    GetRandomBounty,
    /// Give all friendly minions stats; golden minions keep them after the
    /// fight (Ship Master Eudora).
    BuffAllKeepGolden { attack: i32, health: i32 },
    /// Give the minions next to this one +attack, once and again for each
    /// friendly Golden minion (Surfing Sylvar).
    BuffAdjacentPerGolden { attack: i32 },
    /// Buff one other friendly minion the player picks (of a tribe), by
    /// `attack`/`health` plus `per_gold` for each Gold spent this turn
    /// (Lovesick Balladist).
    BuffChosenPerGold {
        tribe: Option<Tribe>,
        attack: i32,
        health: i32,
        per_gold: (i32, i32),
        in_shop: bool,
    },
    /// Make `count` friendly minions of a tribe (if given) of Tier `max_tier`
    /// or lower Golden on the board, picked by the player (Elite Navigator,
    /// Captain Sanders). Golden doubles `count`.
    MakeGoldenChosen { tribe: Option<Tribe>, max_tier: u8, count: u8 },
    /// Get `count` random Golden minions of exactly this tier from the pool;
    /// they give no Triple Reward (Silent Deliverer). Golden doubles it.
    GetGoldenMinion { tier: u8, count: u8 },
    /// Raise the player's maximum Gold for good (Doubloon Grifter: "increase
    /// your maximum Gold by 1"). Golden doubles it.
    MaxGold(u8),
    /// Become an exact copy of a minion the player picks, on the board or in
    /// the Tavern (Faceless Manipulator). The copy takes no pool copy.
    BecomeCopyChosen,
    /// Get a plain copy of a random friendly minion (Reflective Pendant): the
    /// card at its printed stats, its buffs left behind.
    PlainCopyRandomFriendly,
    /// Get a copy of the last card the player discarded, with double stats
    /// (Writhing Tentacles). Nothing when nothing was discarded.
    CopyLastDiscarded,
    /// Give the other friendly minions of a tribe +attack/+health, times one
    /// plus the Golden minions the player played this game (Hooktusk).
    BuffOthersPerGolden { tribe: Option<Tribe>, attack: i32, health: i32 },
    /// Add to the player's "after the Tavern is Refreshed this game" amount
    /// (Waveling, En-Djinn Blazer, Easterly Winds). Every Refresh gives the
    /// whole amount to one random Tavern minion, the turn-start roll
    /// included when the Tavern changed. Golden doubles it.
    RefreshBuff { attack: i32, health: i32 },
    /// Gain free Refreshes, which are used before Gold and carry over turns
    /// (Refreshing Anomaly). Golden doubles it.
    FreeRefreshes(u8),
    /// Give friendly minions stats multiplied by one plus the player's count
    /// of `counter`, then raise the count (the Ballers: "Improve your future
    /// Ballers"). Golden doubles the stats, not the count.
    BuffAllImproving { attack: i32, health: i32, counter: Counter },
    /// Activate: this minion gains the stats of the next minion its owner
    /// buys this turn (Living Prison). Armed in `improvements`.
    ArmNextBuy,
    /// Gain the stats of the highest-Health minion in the Tavern (Unbound
    /// Tempest). Golden gains double.
    GainHighestTavernStats,
    /// Give the other friendly minions of a tribe the stats, times one plus
    /// this minion's `improvements` (Ultraviolet Ascendant). Golden doubles.
    BuffOthersPerImprovement { tribe: Option<Tribe>, attack: i32, health: i32 },
    /// Your Undead have +attack/+health this game, wherever they are
    /// (Nerubian Deathswarmer, Plaguerunner, Butchering, Forsaken Weaver).
    /// Golden doubles it. `outside_attack` replaces `attack` when it runs
    /// outside combat (Plaguerunner's +4 against +2 in a fight), 0 for no
    /// difference.
    UndeadBonus { attack: i32, health: i32, outside_attack: i32 },
    /// Give `count` random friendly minions of a tribe without Reborn that
    /// are not a copy of this card Reborn (Mummifier). Golden doubles the
    /// count.
    GiveRebornRandom { tribe: Tribe, count: u8 },
    /// Summon a card that attacks right after the current attack is over
    /// (Eternal Tycoon). Fights only.
    SummonAttacker { card: CardId },
    /// Get a plain copy of a random minion that started this fight on your
    /// board (Lichling Hoarder). Golden gets two.
    GetPlainCopy,
    /// Get a random pool minion of a tribe, up to your tier, that this
    /// minion's Deathrattle can summon from hand for this fight (Deathly
    /// Striker). Golden gets two. Fights only.
    GetRandomMinionHeld { tribe: Tribe },
    /// Summon the minions `GetRandomMinionHeld` got this fight. They leave
    /// the hand for good. Fights only.
    SummonHeld,
    /// Start of Combat: destroy the minion to the left (both neighbours when
    /// golden), remembering a copy of each (Stitched Salvager).
    DestroyLeftNeighbor,
    /// Deathrattle: summon the exact copies `DestroyLeftNeighbor` kept.
    SummonSalvaged,
    /// Battlecry: destroy a friendly minion of a tribe you pick, then
    /// Discover `count` of that tribe (Maw Caster). Golden Discovers twice.
    DestroyChosenDiscover { tribe: Tribe, count: u8 },
    /// Battlecry: destroy a friendly minion of a tribe you pick and get
    /// `copies` plain copies of it (Disguised Graverobber; golden doubles).
    DestroyChosenGetCopy { tribe: Tribe, copies: u8 },
    /// Activate: give a different friendly minion of a tribe Reborn, then
    /// destroy it, and this minion gains the stats (Dead Bellringer).
    GiveRebornDestroy { tribe: Tribe, attack: i32, health: i32 },
    /// Get `count` Blood Gems in hand (Razorfen Geomancer, Roadboar). A
    /// golden minion gets twice as many. Gems past a full hand are lost.
    GetGems(u8),
    /// Get `count` Blood Gems in hand; the ones that do not fit are cast on
    /// the left-most friendly minion instead (Victorious Geomant). Shop only.
    GetGemsCastOverflow(u8),
    /// Play `count` Blood Gems on the minion with the effect (Tusked Camper,
    /// Razorfen Vineweaver). `permanent` keeps the stats after a fight
    /// (Vineweaver); in the shop every gain is permanent anyway.
    GemSelf { count: u8, permanent: bool },
    /// Play `count` Blood Gems on every other friendly minion (Bonker).
    GemOthers(u8),
    /// Play `count` Blood Gems on the minion that just attacked (Prodigious
    /// Tusker). Fights only.
    GemAttacker(u8),
    /// Your Blood Gems give an extra +attack/+health this game (Sanguine
    /// Refiner's Rally, Sanguine Champion, Fearless Foodie, Gem Day).
    GemBonus { attack: i32, health: i32 },
    /// Get `count` random Choose One cards, any tribe, up to your tier
    /// (Bramble Tunneler). Golden gets twice as many.
    GetRandomChooseOne(u8),
    /// Rally: summon a copy of `card` with this minion's current stats
    /// (double when golden) to its right, which attacks the target first,
    /// before this minion's own swing (Jailbird Juggernaut). Fights only;
    /// the shop copy just summons it.
    SummonStatCopyAttacker { card: CardId },
    /// Get one random Tavern spell of any cost (Dark Gifts Mystic Essence).
    GetAnyTavernSpell,
    /// Rally (Dark Gifts Charisma): get a random minion of the player's most
    /// common type on the board. Nothing when the board has no type.
    GetMostCommonTypeMinion,
}

impl Effect {
    /// Whether the player has to pick something to finish this effect. Event
    /// triggers and end of turn effects can't ask, so they can't use these.
    pub fn needs_choice(self) -> bool {
        matches!(
            self,
            Effect::BuffChosen { .. }
                | Effect::ReplaceWithFishbait { .. }
                | Effect::Consume { by: ConsumeBy::Chosen(_), .. }
                | Effect::Consume { pick: ConsumePick::Chosen, .. }
                | Effect::TriggerRally
                | Effect::TriggerBattlecry
                | Effect::DiscoverDemonTierDamage(_)
                | Effect::BuffAnyMinion { .. }
                | Effect::SetStatsAnyMinion { .. }
                | Effect::BuffChosenPerGold { .. }
                | Effect::MakeGoldenChosen { .. }
                | Effect::DestroyChosenDiscover { .. }
                | Effect::DestroyChosenGetCopy { .. }
                | Effect::GiveRebornDestroy { .. }
                | Effect::BecomeCopyChosen
        )
    }

    /// Whether the effect needs another friendly minion of a tribe that is
    /// not a copy of the host (Dead Bellringer's Activate).
    pub fn needs_different_of_tribe(self) -> Option<Tribe> {
        match self {
            Effect::GiveRebornDestroy { tribe, .. } => Some(tribe),
            _ => None,
        }
    }

    /// Whether the effect needs the player to pick a minion on the board or
    /// in the Tavern ("another minion").
    pub fn needs_any_minion(self) -> bool {
        matches!(self, Effect::BuffAnyMinion { .. } | Effect::SetStatsAnyMinion { .. })
    }

    /// Whether the effect takes a Tavern minion into hand, so it needs one
    /// in the Tavern and room in hand.
    pub fn needs_shop_minion(self) -> bool {
        matches!(self, Effect::StealTavern { .. })
    }

    /// Whether the effect needs a friendly minion with a Rally to be legal
    /// (Sky-hatch Runaway).
    pub fn needs_rally_target(self) -> bool {
        matches!(self, Effect::TriggerRally)
    }

    /// Whether the effect needs another friendly minion with a Battlecry to
    /// be legal (Kelp Keeper).
    pub fn needs_battlecry_target(self) -> bool {
        matches!(self, Effect::TriggerBattlecry)
    }
}

impl Effect {
    /// Whether a fight can run this effect: Rally, "whenever a friendly minion
    /// attacks", Deathrattle, Avenge and damage milestone effects must be.
    pub fn runs_in_fights(self) -> bool {
        match self {
            Effect::BuffOthers { per_keyword, .. } => per_keyword == (0, 0),
            Effect::BuffDeity { .. }
            | Effect::GetRandomSpellUpTo { .. }
            | Effect::SummonAndBuffAll { .. }
            | Effect::SummonDeadAberrations { .. }
            | Effect::DamageHighestHealth
            | Effect::Summon { .. }
            | Effect::BuffAll { .. }
            | Effect::BuffSelf { .. }
            | Effect::BuffRandomFriendly { .. }
            | Effect::BuffKiller { .. }
            | Effect::GainKeyword { .. }
            | Effect::GetRandomMinion { .. }
            | Effect::SummonRandomDeathrattle { .. }
            | Effect::TriggerDeathrattle { .. }
            | Effect::BeetleBuff { .. }
            | Effect::RandomVolumizer
            | Effect::GetCard { .. }
            | Effect::SpellBonus { .. }
            | Effect::RandomChromadrake
            | Effect::CastSpell { .. }
            | Effect::BuffAttacker { .. }
            | Effect::BuffRandomOther { .. }
            | Effect::SummonFromHand { .. }
            | Effect::BuffRandomHand { .. }
            | Effect::AddFodderRefreshes(_)
            | Effect::GainTargetAttack
            | Effect::StripTargetKeywords { .. }
            | Effect::DestroyKiller
            | Effect::ShieldDragons { .. }
            | Effect::GetRandomTierMinion { .. }
            | Effect::BuffEachType { .. }
            | Effect::GetLockbox { .. }
            | Effect::GetRandomBounty
            | Effect::BuffAllKeepGolden { .. }
            | Effect::RefreshBuff { .. }
            | Effect::BuffOthersPerImprovement { .. }
            | Effect::UndeadBonus { .. }
            | Effect::GiveRebornRandom { .. }
            | Effect::SummonAttacker { .. }
            | Effect::GetPlainCopy
            | Effect::GetRandomMinionHeld { .. }
            | Effect::SummonHeld
            | Effect::DestroyLeftNeighbor
            | Effect::SummonSalvaged
            | Effect::GetGems(_)
            | Effect::GemSelf { .. }
            | Effect::GemOthers(_)
            | Effect::GemAttacker(_)
            | Effect::GemBonus { .. }
            | Effect::GetRandomChooseOne(_)
            | Effect::SummonStatCopyAttacker { .. }
            | Effect::GetAnyTavernSpell
            | Effect::GetMostCommonTypeMinion
            | Effect::FreeRefreshes(_)
            | Effect::DamageTargetAndNeighbor => true,
            _ => false,
        }
    }

    /// Whether the effect picks one of the player's Tavern cards, so it can
    /// only be activated while there is one.
    pub fn needs_shop_slot(self) -> bool {
        matches!(self, Effect::ReplaceWithFishbait { .. } | Effect::StealTavern { .. })
    }
}

/// An Activate ability: pay gold, run the effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Activate {
    pub cost: u8,
    pub effect: Effect,
    /// The player discards a card in hand first; with nothing discardable the
    /// Activate can't be used (Mangled Bandit).
    pub discard: bool,
}

/// Things a player does that event triggers count.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EventKind {
    /// Tavern spells cast: `tavern_spell = true` only. This is the
    /// observation's "Tavern spells cast this game" (`Player.spells_cast`)
    /// and the counter for "improved by each Tavern spell you've cast"
    /// (Faceless Converter), Living Azerite's trigger and Heart of the
    /// Forest's.
    SpellsCast,
    /// Every spell cast: Tavern spells, Blood Gems, Gem Days, Bananas,
    /// Spellcraft spells (docs/rules/design.md "Read the card's exact words"). Felboar
    /// counts these. A Triple Reward is not a spell you cast.
    AllSpellsCast,
    /// Counts gold, so spending 5 adds 5.
    GoldSpent,
    /// Counts damage, so taking 3 adds 3.
    HeroDamage,
    MinionsSold,
    CardsBought,
    /// Elementals played from hand (Unbound Tempest).
    ElementalsPlayed,
    /// Cards discarded (Mangled Bandit's Activate). The hook "when you
    /// discard" triggers read.
    Discarded,
    /// A card of this tribe entered the player's hand, from anywhere: bought,
    /// generated, Discovered or copied (Doubloon Grifter, Trusty Crowbar).
    CardGot(Tribe),
}

/// "After N events, do the effect (N left!)". The count lives in the
/// minion's `improvements`. Going past N keeps the remainder, so one big
/// event can fire it more than once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EventTrigger {
    pub kind: EventKind,
    pub every: u8,
    pub effect: Effect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DiscoverKind {
    Tier1Minion,
    MyTierMinion,
    MyTierSpell,
    DeathrattleMinion,
    BattlecryMinion,
    /// A Tier 7 minion (Hallowed Ritual).
    Tier7Minion,
    /// A Choose One minion with both effects combined (Fandral's Fortune).
    ChooseOneMinion,
    /// A minion of exactly this tier, whatever its type (Azeroth Model
    /// Globe).
    TierMinion(u8),
    /// A minion of exactly this tier with a type, so no neutral (Portable
    /// Factory).
    TierMinionWithType(u8),
    /// A minion of one Tier higher than the player's, up to Tier 7 (Top
    /// Shelf).
    HigherTierMinion,
    /// A plain copy of a minion from the last opponent's warband, as it was
    /// last seen (Time Thief).
    LastOpponentMinion,
}

/// Which stat a "then double it" spell doubles (I'm Still Just a Rat in a
/// Cage doubles Attack, The Bouncer doubles Health).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DoubleStat {
    Attack,
    Health,
}

/// A Tavern spell (or Choose One spell option) that targets a friendly minion.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SpellTarget {
    Buff {
        attack: i32,
        health: i32,
        keyword: u8,
        times: u8,
        if_win_next_turn_buff: Option<(i32, i32)>,
        /// The target must be of this tribe ("Give a Murloc...", "Give a
        /// Beast...": the Spellcraft spells).
        tribe: Option<Tribe>,
        /// Give it a random Bonus Keyword it does not have (Vibrant Bubble).
        random_keyword: bool,
    },
    BuffSameTribe {
        attack: i32,
        health: i32,
    },
    SameTribeMinion,
    SetStats {
        attack: i32,
        health: i32,
    },
    MakeGolden {
        max_tier: u8,
    },
    /// Give the minion stats, once more for each different Bonus Keyword in
    /// the warband (Seafood Stew).
    BuffPerBonusKeyword {
        attack: i32,
        health: i32,
    },
    /// The chosen minion (of a tribe, if given) consumes Tavern minions
    /// (Methodical Madness, Corrupted Cupcakes).
    Consume {
        tribe: Option<Tribe>,
        pick: ConsumePick,
        keywords: bool,
        count: u8,
    },
    /// +attack/+health and Taunt, or remove Taunt if it already has it
    /// (Tricky Trousers).
    TrickyTrousers { attack: i32, health: i32 },
    /// Transform the minion into a random minion exactly one tier higher,
    /// keeping only its Attack and Health (Robust Evolution).
    RobustEvolution,
    /// Tavern minions of the chosen minion's type get stats this game
    /// (Eonar's Favor). A dual or all-type one picks one type at random.
    TavernBuffType { attack: i32, health: i32 },
    /// Refresh the Tavern with minions of the chosen minion's type (Lost
    /// Staff of Hamuul).
    RefreshWithType,
    /// Sell the chosen friendly minion and give its stats to a random other
    /// friendly minion (Channel the Devourer).
    Devourer,
    /// Give the chosen friendly minion of a tribe half (rounded up) the stats
    /// of the highest-Health minion in the Tavern (Arcane Absorption).
    HalfOfHighestTavern { tribe: Tribe },
    /// Give a minion (board or Tavern) stats, plus `per` for each minion of
    /// `tribe` its owner played this turn (Conflagration).
    BuffPerTribePlayed { tribe: Tribe, attack: i32, health: i32, per: (i32, i32) },
    /// Destroy the chosen friendly Undead, then your Undead have +attack this
    /// game, wherever they are (Butchering). The Tavern spell bonus adds to
    /// both stats.
    Butchering { attack: i32 },
    /// Play `gems` Blood Gems on the chosen minion (board or Tavern), then
    /// move all its neighbours' Blood Gem stats onto it: they lose them, it
    /// gains them (Gem Confiscation).
    Confiscation { gems: u8 },
    /// A Blood Gem that also gives this keyword when its target is a
    /// Quilboar (Jewelry Box's three special Gems). The stats are the
    /// player's Gem value.
    GemKeyword { keyword: u8 },
    /// Trigger the chosen friendly minion's Deathrattle outside combat
    /// (Floating Candle Set's Spellcraft spell).
    TriggerDeathrattle,
    /// Get a plain copy of the chosen minion (board or Tavern) of Tier
    /// `max_tier` or below (Duplicating Lens).
    PlainCopy { max_tier: u8 },
    /// Destroy the chosen friendly minion of a tribe, then get `count`
    /// random minions of it (Jailer Sticker's Spellcraft spell).
    DestroyGetRandom { tribe: Tribe, count: u8 },
    /// Transform the chosen minion into one from a Tier higher, which the
    /// player picks from three (Token of the Old Gods).
    TransformHigher,
    /// Watch the chosen minion, the Tavern's too: whatever stats it gains
    /// outside combat this turn, the Deity gains too, and the watch follows
    /// the card when it is bought (Makeshift Master).
    WatchStats,
    /// Give the minion stats and a keyword, then multiply one of its stats
    /// (I'm Still Just a Rat in a Cage, The Bouncer).
    BuffThenDouble { attack: i32, health: i32, keyword: u8, stat: DoubleStat },
    /// Return the minion to hand, with these stats added (Repeat Customer).
    ReturnToHand { attack: i32, health: i32 },
    /// Make the minion Golden and return it to hand (Raise the Stakes).
    MakeGoldenReturn,
    /// Give the minion "At the end of your turn, gain this much" for the
    /// rest of the game (Gruul Rules).
    GrantEndOfTurn { attack: i32, health: i32 },
}

impl SpellTarget {
    /// Whether this target may also be a minion in the Tavern. Spells that
    /// say "a minion" (no "friendly") reach both, per docs/rules/design.md; "friendly"
    /// means the board only.
    pub fn can_target_shop(self) -> bool {
        matches!(
            self,
            SpellTarget::Buff { .. }
                | SpellTarget::BuffSameTribe { .. }
                | SpellTarget::SameTribeMinion
                | SpellTarget::SetStats { .. }
                | SpellTarget::BuffPerBonusKeyword { .. }
                | SpellTarget::TrickyTrousers { .. }
                | SpellTarget::RobustEvolution
                | SpellTarget::TavernBuffType { .. }
                | SpellTarget::RefreshWithType
                | SpellTarget::BuffPerTribePlayed { .. }
                | SpellTarget::Confiscation { .. }
                | SpellTarget::WatchStats
                | SpellTarget::PlainCopy { .. }
                | SpellTarget::TransformHigher
                | SpellTarget::GemKeyword { .. }
                | SpellTarget::BuffThenDouble { .. }
            )
    }

    /// Whether the chosen minion must have a type (a neutral can't be
    /// picked).
    pub fn needs_type(self) -> bool {
        matches!(
            self,
            SpellTarget::BuffSameTribe { .. }
                | SpellTarget::SameTribeMinion
                | SpellTarget::TavernBuffType { .. }
                | SpellTarget::RefreshWithType
        )
    }
}

/// A Tavern spell (or Choose One spell option) that does not target a minion.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SpellEffect {
    BuffDeity { attack: i32, health: i32 },
    GainGold(u8),
    GoldNextTurn(u8),
    MaxGold(u8),
    FreeRolls(u8),
    Overconfidence { win: u8, tie: u8 },
    BuffShop { attack: i32, health: i32 },
    BuffBoard { attack: i32, health: i32, times: u8, golden_extra: bool },
    BuffRandomBoard { count: u8, attack: i32, health: i32 },
    NextTurnBoardBuff { attack: i32, health: i32, times: u8 },
    RandomTierMinion(u8),
    Discover(DiscoverKind),
    BoonOfBeetles(u8),
    /// Buff every friendly minion, again for Dragons, again for minions
    /// with Divine Shield.
    MightyDragonbreath { attack: i32, health: i32 },
    BroodOfNozdormu,
    UpperHand,
    /// Start of Combat: the left-most minion gains the nearest enemy
    /// minion's stats (Sharing is Caring).
    SharingIsCaring,
    /// Give your minions stats, again for each different friendly minion
    /// type (Menagerie Tableware).
    MenagerieTableware { attack: i32, health: i32 },
    /// Make a random minion in the Tavern golden.
    GoldenTouch,
    /// Get a random Murloc and a copy of it (Cloning Conch).
    CloningConch,
    /// Give minions in the Tavern stats for the rest of the game (Staff of
    /// Enrichment).
    BuffTavernPermanent { attack: i32, health: i32 },
    /// For each type on the board, give one random friendly minion of it
    /// +attack/+health, at most one buff per minion (Misplaced Tea Set).
    MisplacedTeaSet { attack: i32, health: i32 },
    /// Discover a minion of the most common type on the board (Planar
    /// Telescope).
    PlanarTelescope,
    /// Steal a random minion from the Tavern into hand (Enchanted Lasso).
    StealRandomTavern,
    /// Discover a minion of exactly the player's tier and lock it in hand
    /// for one turn (Search Through Time).
    DiscoverLockedMinion,
    /// Refresh the Tavern with Tavern spells (Saloon's Finest).
    SaloonFinest,
    /// Refresh the Tavern with Battlecry minions, which cost this much while
    /// they stay (Chillmere Mosaic's Spellcraft spell).
    RefreshWithBattlecries(u8),
    /// Get `count` copies of a card (Weapons Forge).
    GetCard { card: CardId, count: u8 },
    /// An unplayable Lockbox that counts down from this many turns (its
    /// count lives in the hand minion's `improvements`).
    Lockbox(u8),
    /// Give the left-most minion on the board stats (Selfish Bounty).
    BuffLeftmost { attack: i32, health: i32 },
    /// Get a random minion of the most common type on the board (Friendly
    /// Bounty).
    FriendlyBounty,
    /// Get `count` Blood Gems in hand (a Choose One option).
    GetGems(u8),
    /// Your Blood Gems give an extra +attack/+health this game (Gem Day, a
    /// Choose One option).
    GemBonus { attack: i32, health: i32 },
    /// Your Tavern spells give an extra +attack/+health this game
    /// (Crystallization): the same player field a minion's `spell_bonus`
    /// raises.
    SpellBonus { attack: i32, health: i32 },
    /// Play `count` Blood Gems on every friendly minion (Veteran Brigand's
    /// Choose One option).
    GemsOnBoard(u8),
    /// "After the Tavern is Refreshed this game", every Tavern minion gets
    /// this much worth of Blood Gems, plus the Gem bonus and the Tavern spell
    /// bonus once per cast. Each cast adds on top (Blood Gem Barrage).
    Barrage { attack: i32, health: i32 },
    /// Cast a named Tavern spell `times` times, each cast a real one (Veteran
    /// Brigand's Blood Gem Barrage option).
    CastSpell { card: CardId, times: u8 },
    /// Get `count` random minions of this tribe, up to your tier, generated
    /// (Snare Trapper's Choose One option).
    RandomTribeMinion { tribe: Tribe, count: u8 },
    /// Discover a minion of the tier stored on this spell copy when it was
    /// given (Triple Reward).
    TripleReward,
    /// Add to the "after the Tavern is Refreshed this game" amount (Easterly
    /// Winds).
    RefreshBuff { attack: i32, health: i32 },
    /// Get one of each of two cards (Temperature Shift).
    GetTwo { a: CardId, b: CardId },
    /// Discover an Undead that dies if you play it this turn (Tomb Turning).
    TombTurning,
    /// Set your Armor to this much (Armor Stash). Sets, doesn't add.
    SetArmor(u8),
    /// Discover a new Hero Power (Unmasked Identity). `exclude_heroes` (on
    /// the Card) lists heroes whose powers are never offered.
    DiscoverHeroPower,
    // ---- the Darkmoon Prizes (block 5) ----
    /// Discover a Darkmoon Prize of exactly this Prize tier (the Triple
    /// Prize, Tickatus Sticker).
    DiscoverPrize(u8),
    /// Discover a Darkmoon Prize from each Prize tier below this one: one
    /// from every tier up to and including this number (Big Winner!).
    DiscoverPrizesBelow(u8),
    /// Get this many random Darkmoon Prizes, any tier (Hand of Fate).
    RandomPrize(u8),
    /// Fill the hand with copies of a card (B.A.N.A.N.A.S.).
    FillHand { card: CardId },
    /// Fill the hand with random Tavern spells (Gorgeous Goblet).
    FillHandSpells,
    /// Gain 1 Gold, and get this card back at the end of every turn (The
    /// Unlimited Coin). One flag: the card it gives back.
    ReturnEachTurn { card: CardId },
    /// Replace every card in the Tavern with a random one from the Tier
    /// above (Evolving Tavern).
    EvolvingTavern,
    /// Steal every card in the Tavern, then Refresh it (Mindflayer Goggles).
    StealAllTavern,
    /// The Tavern offers one more minion, with these stats, for the rest of
    /// the game (New Recruit).
    ExtraShopMinion { attack: i32, health: i32 },
    /// Gain this many free Refreshes now and this many at the start of every
    /// turn for the rest of the game (Rocking and Rolling: `[0, 1]`, Open
    /// Bar: `[5, 5]`).
    FreeRollsPerTurn { now: u8, per_turn: u8 },
    /// Get a random Tavern spell costing at least this much (Unfurled
    /// Codex).
    RandomSpellAtLeast(u8),
    /// Tavern spells cost this much less this turn (Reserve Prices).
    SpellDiscountThisTurn(u8),
    /// Minions in the Tavern cost this much for the rest of the game
    /// (Friends and Family Discount).
    MinionPrice(u8),
    /// Your Battlecries trigger this many extra times this turn (Big Brann
    /// Play).
    BattlecryExtraThisTurn(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChooseOption {
    MinionTarget {
        tribe: Option<Tribe>,
        attack: i32,
        health: i32,
        keyword: u8,
        /// The pick may be a Tavern minion too.
        in_shop: bool,
    },
    SpellBonus {
        attack: i32,
        health: i32,
    },
    SpellTarget(SpellTarget),
    SpellEffect(SpellEffect),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Deathrattle {
    Summon {
        card: CardId,
        count: u8,
        /// When true, a golden minion summons twice as many plain tokens
        /// (Beetles); when false, it summons `count` golden tokens.
        golden_doubles_count: bool,
        beetle_buff: Option<(i32, i32)>,
    },
    SummonByAttack {
        card: CardId,
    },
    CombatBeastBuff {
        attack: i32,
        health: i32,
    },
    RandomVolumizer,
    GetCard {
        card: CardId,
        count: u8,
    },
    /// Any effect a fight can run.
    Effect(Effect),
    /// Give the Tavern minions (of the fight's owner) +attack/+health for the
    /// rest of the game, once more for each `improvements` the dead minion
    /// has (Sacrificial Wrathguard). Golden doubles it.
    TavernBuff {
        attack: i32,
        health: i32,
        per_improvement: (i32, i32),
    },
    /// Dark Gifts Golemancy: summon a `card` token with the dead minion's
    /// current stats, next to where it stood.
    SummonShaped { card: CardId },
    /// Dark Gifts Offensive Sacrifice: give the dead minion's Attack to one
    /// other friendly minion picked at random. The gift never doubles, so a
    /// golden host gives the same.
    GiveOwnAttack,
    /// Dark Gifts Defensive Sacrifice: give the dead minion's maximum Health
    /// to one other friendly minion picked at random.
    GiveOwnHealth,
}

/// "After you play a minion of `tribe`, ..." for the minion that has it.
/// Every effect runs for it; a golden one doubles each.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AfterPlayTribe {
    pub tribe: Tribe,
    pub effects: Vec<Effect>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartOfTurn {
    GainGold(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndOfTurn {
    BuffSelf { attack: i32, health: i32 },
    ResourcefulRobot,
    BuffPerMagnetization { attack: i32, health: i32 },
    ImproveTier,
    GetCard { card: CardId, count: u8 },
    CopyLastSpell,
    /// Any generic effect (one that needs no choice).
    Effect(Effect),
    /// Dark Gifts Echoing Voice: trigger this minion's own Battlecries now.
    TriggerOwnBattlecry,
    /// Dark Gifts Polarization: Magnetize a random Mech from the pool to it.
    MagnetizeRandomMech,
    /// Dark Gifts Steady Growth: gain the +attack/+health fixed when the gift
    /// landed, which lives in the gift's own counter on the minion as two
    /// nibbles (attack low, health high). The card data names the numbers it
    /// starts from; the counter is what the landing wrote.
    GiftBuffSelf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartOfCombat {
    CombatBeastBuff { attack: i32, health: i32 },
    /// Gain the Attack of your left-most minion and the Health of your
    /// right-most minion (All-Seeing Elder). Golden doubles both.
    GainEndsStats,
    DamageAllOthers(i32),
    /// Any effect a fight can run (Electric Synthesizer, Amber Guardian,
    /// Runic Arcanist).
    Effect(Effect),
    /// Attack at once, before the normal attack rotation (Heroic Broodmother).
    AttackImmediately,
    /// Give your minions stats, improved by a per-minion count that goes up
    /// every time a friendly minion loses Divine Shield (Hopebringer).
    Hopebringer { attack: i32, health: i32, improve: (i32, i32) },
    /// If this minion is in your hand, summon a copy of it (Flighty Scout).
    /// A golden one summons a golden copy, which has double its stats.
    FlightyScout,
    /// Gain the Attack of the highest-Attack minion in your hand, for this
    /// combat (Costume Enthusiast). A golden one gains double.
    GainHighestHandAttack,
    /// Gain the stats of every minion in your hand, for this combat (Choral
    /// Mrrrglr). A golden one counts the hand twice.
    GainHandStats,
    /// Dark Gifts Resistance: double this minion's Health.
    DoubleHealth,
    /// Dark Gifts Hostility: double this minion's Attack.
    DoubleAttack,
    /// Dark Gifts Transcendence: triple this minion's Attack and Health.
    TripleStats,
    /// Dark Gifts Admiration: gain the Attack of the minion to its left (of
    /// its own side, as it stands now). Nothing when it is left-most.
    GainLeftAttack,
    /// Dark Gifts Worship: gain the stats of your Deity. Nothing when you
    /// have none.
    GainDeityStats,
    /// Dark Gifts Jaws of Death: trigger this minion's own Deathrattles.
    TriggerOwnDeathrattles,
    /// Dark Gifts Doppelganger: summon a copy of this minion (no gift
    /// doubling; the copy is plain stats and keeps the keywords it has).
    SummonOwnCopy,
}

/// "While this is in your hand, after you play a minion of a tribe, ...".
/// The buff is on the hand card itself and is permanent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WhileInHand {
    pub tribe: Tribe,
    pub attack: i32,
    pub health: i32,
}

/// "Once this reaches `total` Attack, gain `keyword`" (Scarlet Survivor).
/// The minion's `improvements` doubles as the "already fired" flag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AttackThreshold {
    pub total: u16,
    pub keyword: u8,
}

/// What a minion keeps from combat, permanently.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermanentKeep {
    /// This minion keeps its own stat and Bonus Keyword gains (Tarecgosa).
    SelfKeep,
    /// Adjacent Dragons keep theirs (Persistent Poet).
    AdjacentDragons,
    /// Every friendly Dragon keeps its own, the keeper included (Timewarped
    /// Poet, the Poet Portrait's token).
    AllDragons,
}

/// Which attacking minions an attack trigger reacts to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AttackFilter {
    /// Only attackers of this tribe.
    pub tribe: Option<Tribe>,
    /// Only attackers with a Rally effect.
    pub rally: bool,
    /// Another minion: the minion with the trigger doesn't count (Roaring
    /// Recruiter).
    pub exclude_self: bool,
}

/// "Whenever" or "after a friendly minion attacks" (of a kind): run the
/// effect for the minion that has it. Any effect a fight can run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AttackTrigger {
    pub filter: AttackFilter,
    pub effect: Effect,
}

/// "Avenge (N)": after every N friendly minions die in a fight, run the
/// effect. The count starts at 0 in every fight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Avenge {
    pub every: u8,
    pub effect: Effect,
}

/// "Once this deals N damage, ...": the damage dealt so far lives in the
/// minion's `improvements`, and the effect runs once, when it passes `total`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DamageMilestone {
    pub total: u16,
    pub effect: Effect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnSummonBeast {
    Buff { attack: i32, health: i32, improve_permanently: bool },
    MultiplyAttackInCombat,
}

/// "After a friendly minion is Reborn" (never this minion's own Reborn). A
/// golden minion doubles the numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnReborn {
    /// Gain Divine Shield and stats (Barrier Banshee).
    Banshee { attack: i32, health: i32 },
    /// Give your right-most Undead stats equal to the Attack the Reborn
    /// minion came back with (Snazzy Phantom).
    Phantom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnPlayMech {
    MagnetizeSatellite { card: CardId, attack: i32, health: i32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnPlayOrMagnetizeMech {
    Buff { attack: i32, health: i32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeityKind {
    Cthun,
    Yshaarj,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnDiscard {
    CastSpell(u8),
    MaxGold(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnCastSpellOnMech {
    MagnetizeSatellite { card: CardId, attack: i32, health: i32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnCastSpell {
    BuffSelfAndDeity { attack: i32, health: i32 },
    BuffAllAndDeity { attack: i32, health: i32 },
    BuffDivineShield { attack: i32, health: i32 },
    /// Buff every friendly minion. A golden minion does it twice rather
    /// than twice as much (Timecap'n Hooktail).
    BuffAll { attack: i32, health: i32 },
    /// Your Undead have +attack/+health this game, wherever they are (Forsaken
    /// Weaver). Golden doubles it.
    UndeadBonus { attack: i32, health: i32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnSell {
    GetCard { card: CardId, count: u8 },
    Effect(Effect),
    DiscoverByTier,
    GoldIfLost(u8),
    /// If the minion is Golden when sold, Discover a Tier 7 minion (Greedy
    /// Conniver).
    DiscoverTier7IfGolden,
    /// Gain the stats of the minion sold when it is of `tribe` (Arcane
    /// Behemoth). Golden doubles them.
    GainSoldStats { tribe: Option<Tribe> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Aura {
    Brann,
    Titus,
    Drakkari,
    Balinda,
    /// Has Taunt while it has Divine Shield.
    TauntWhileShielded,
    /// Has +30/+30 while its owner's hero has 15 or less Health (Holy
    /// Vanguard). A golden one has +60/+60.
    HolyVanguard,
    /// Whenever its owner Magnetizes to a different minion, the same
    /// Magnetic also Magnetizes to this one (twice when golden). Polarizing
    /// Beatboxer.
    PolarizingBeatboxer,
    /// Your Bounties cast twice (three times when golden). Proud Privateer.
    ProudPrivateer,
    /// Your Blood Gems played from your hand cast an extra time (two when
    /// golden). Hot-Air Surveyor.
    HotAirSurveyor,
}

/// What a minion does after it attacks and kills a minion (Tide Oracle
/// Morgl). A golden minion doubles the numbers the reward carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KillReward {
    /// Give the killed minion's Attack and maximum Health to a random card in
    /// the owner's hand.
    MaxStatsToHand,
}

/// A card's role in a "how many times" count (docs/rules/design.md "Set versus add, as
/// one mechanic").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimesRole {
    /// "Trigger twice" / "cast twice" (golden "three times"): a fixed count,
    /// the highest one wins, copies never stack.
    Setter,
    /// "An extra time" (golden "two extra times"): adds on top, copies stack.
    Adder,
}

impl Aura {
    /// The count every setter aura names: twice, three times when golden.
    /// An adder adds one less than this ("an extra time", "two extra times").
    pub fn times(self, golden: bool) -> u8 {
        if golden { 3 } else { 2 }
    }
}

/// The shared "how many times" count of docs/rules/design.md's "Set versus add": the
/// highest value any setter on the board gives (1 when none), plus what each
/// adder adds. `role` classifies an aura; auras it ignores are not counted.
/// `parts` yields every (card, golden) of the board, attachments included.
pub fn how_many_times(
    cards: &CardDb,
    parts: impl Iterator<Item = (CardId, bool)>,
    role: impl Fn(Aura) -> Option<TimesRole>,
) -> u8 {
    let mut set = 1u8;
    let mut add = 0u8;
    for (card, golden) in parts {
        let Some(a) = cards.get(card).aura else { continue };
        match role(a) {
            Some(TimesRole::Setter) => set = set.max(a.times(golden)),
            Some(TimesRole::Adder) => add = add.saturating_add(a.times(golden) - 1),
            None => {}
        }
    }
    set.saturating_add(add)
}

/// "After you buy a minion, give it +attack/+health and multiply its stats,
/// once per turn" (Stone Age Slab). A golden minion multiplies by
/// `golden_multiply`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AfterBuy {
    pub attack: i32,
    pub health: i32,
    pub multiply: i32,
    pub golden_multiply: i32,
}

/// "After you summon a minion in combat, give it this minion's current
/// Attack and maximum Health, `times` times per combat" (Stalwart Kodo).
/// A golden minion gives double.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AfterSummonGiveStats {
    pub times: u8,
}

#[derive(Clone, Debug)]
pub struct Card {
    pub name: String,
    pub tier: u8,
    pub tribes: Tribes,
    /// The tribe this neutral or spell is tied to (cards.toml `lobby`, the
    /// game data's `lobby_subsets`): it only exists in lobbies with that
    /// tribe. None when no tie.
    pub lobby: Option<Tribe>,
    pub attack: i32,
    pub health: i32,
    /// Golden stats, when the data doesn't double the printed ones.
    pub golden_stats: Option<(i32, i32)>,
    pub keywords: Keywords,
    /// A Darkmoon Prize: its Prize tier, 1 to 4 (the game data's
    /// `battlegroundsDarkmoonPrizeTurn`). Prizes are never in the pool; a
    /// Prize Discover offers three of one tier (`CardDb.prizes`).
    pub prize: Option<u8>,
    /// The card is an Amalgam, whose stat gains Mishmash copies.
    pub amalgam: bool,
    /// Whenever a friendly Amalgam gains stats, this gains them this many
    /// times (Mishmash: 1, twice when golden). 0 for every other card.
    pub amalgam_gain_copy: u8,
    /// The tribes a Magnetic card can Magnetize onto: Mech for all but the
    /// Mecha-Demons, which take Mechs and Demons.
    pub magnetize_tribes: Tribes,
    /// One of the four Magnetic Mecha-Demons the Mecha-Jaraxxus Sticker hands
    /// out (`CardDb.mecha_demons`).
    pub mecha_demon: bool,
    /// A buddy card (the game data's `isBattlegroundsBuddy`), tagged for when
    /// buddies come: Mishmash, which the Curator Sticker hands out. Nothing
    /// reads it yet.
    pub buddy: bool,
    pub token: bool,
    pub spell: bool,
    /// Bought with Health instead of Gold (Hasty Excavation).
    pub pay_health: bool,
    pub cost: u8,
    pub is_beetle: bool,
    pub is_volumizer: bool,
    /// Index in `Game::pool` that holds copies of this card. Shared pool
    /// groups (such as the three Volumizers) all point to their first card's
    /// slot.
    pub pool_slot: CardId,
    pub volumizer_buff: Option<(i32, i32)>,
    pub spell_buff: Option<(i32, i32)>,
    pub battlecry: Option<Effect>,
    pub choose_one: Option<[ChooseOption; 2]>,
    pub deathrattle: Option<Deathrattle>,
    pub start_of_turn: Option<StartOfTurn>,
    pub end_of_turn: Option<EndOfTurn>,
    pub start_of_combat: Option<StartOfCombat>,
    pub rally: Option<Effect>,
    pub on_summon_beast: Option<OnSummonBeast>,
    pub on_play_mech: Option<OnPlayMech>,
    pub on_play_or_magnetize_mech: Option<OnPlayOrMagnetizeMech>,
    pub on_cast_spell_on_mech: Option<OnCastSpellOnMech>,
    pub on_cast_spell: Option<OnCastSpell>,
    /// Whenever the owner casts a spell on this minion (Twilight Tidehunter).
    pub on_cast_spell_on_self: Option<Effect>,
    /// Whenever the owner casts a spell on a minion of `tribe` (Shamanic
    /// Tidecaller).
    pub on_cast_spell_on_tribe: Option<(Tribe, Effect)>,
    /// Whenever this minion takes damage in a fight (Very Hungry
    /// Winterfinner).
    pub when_damaged: Option<Effect>,
    /// While this is in its owner's hand, after they play a minion of a
    /// tribe (Bream Counter).
    pub while_in_hand: Option<WhileInHand>,
    /// After the owner buys a Tavern spell, get a taught 1/1 Murloc token
    /// (Magicfin Mycologist).
    pub teaches_spell: bool,
    /// Tokens that can't be tripled (Magicfin Apprentice).
    pub no_triple: bool,
    /// The 1/1 Murloc token Magicfin Mycologist teaches a spell to.
    pub taught_token: bool,
    pub whenever_friendly_attacks: Option<AttackTrigger>,
    pub after_friendly_attacks: Option<AttackTrigger>,
    pub avenge: Option<Avenge>,
    pub damage_milestone: Option<DamageMilestone>,
    /// Gets this much for every Deathrattle its owner has triggered this
    /// game, wherever it is.
    pub buff_per_deathrattle: Option<(i32, i32)>,
    /// Tavern buffs don't change its stats.
    pub fixed_stats: bool,
    pub activate: Option<Activate>,
    pub on_event: Option<EventTrigger>,
    /// What happens after you trigger a Battlecry (Kalecgos).
    pub after_battlecry: Option<Effect>,
    /// Keeps combat gains permanently (Tarecgosa, Persistent Poet).
    pub permanent_keep: Option<PermanentKeep>,
    /// Gains a keyword once its Attack reaches a threshold (Scarlet
    /// Survivor).
    pub attack_threshold: Option<AttackThreshold>,
    /// Effects that run in order after your hero takes damage.
    pub after_hero_damage: Vec<Effect>,
    /// Effects that run after the owner plays a minion of a tribe (Wrath
    /// Weaver, Insatiable Ur'zul).
    pub after_play_tribe: Option<AfterPlayTribe>,
    /// Stats this minion gains permanently when another friendly Demon deals
    /// damage in a fight (Devout Hellcaller).
    pub after_demon_damage: Option<(i32, i32)>,
    /// A Demon Fodder: on the Refresh it appears in, it feeds itself to a
    /// random friendly Demon and its slot is refilled, if the player has a
    /// Demon then. Otherwise it waits in the Tavern as a plain 2/2 Demon.
    pub is_fodder: bool,
    /// Refreshes each turn that cost Health instead of Gold.
    pub health_refreshes: u8,
    pub on_sell: Option<OnSell>,
    pub aura: Option<Aura>,
    /// Gives each minion its owner summons in combat this minion's stats
    /// (Stalwart Kodo).
    pub after_summon_give_stats: Option<AfterSummonGiveStats>,
    pub spell_target: Option<SpellTarget>,
    pub spell_effect: Option<SpellEffect>,
    /// Always Golden, with its printed stats (Aureate Laureate): it is made
    /// Golden and gives no Triple Reward.
    pub always_golden: bool,
    /// Also damages the enemies next to the one it attacks (Blade Collector).
    pub cleave: bool,
    /// Gets this much for every Golden minion its owner played this game,
    /// wherever it is (Maritime Extortionist).
    pub buff_per_golden_played: Option<(i32, i32)>,
    /// Gets this much for every Elemental its owner played this game,
    /// wherever it is (Flourishing Frostling).
    pub buff_per_elemental_played: Option<(i32, i32)>,
    /// Gets this much for every time its owner Magnetized this game, wherever
    /// it is (Dr. Boom's Monster).
    pub buff_per_magnetize: Option<(i32, i32)>,
    /// After this attacks and kills a minion, the reward (Tide Oracle Morgl).
    pub on_kill: Option<KillReward>,
    /// After a different friendly Deathrattle minion dies in combat, this
    /// gains its Deathrattle for the rest of the fight (Fish of N'Zoth).
    pub gains_friendly_deathrattles: bool,
    /// After its owner Discovers a card (Hooktusk).
    pub after_discover: Option<Effect>,
    /// One of the five Bounties, which share one slot of the spell draw.
    pub is_bounty: bool,
    /// After this attacks and kills a minion, it deals the excess damage to an
    /// adjacent enemy, both when golden (Wildfire Elemental).
    pub overkill: bool,
    /// Can stand in for any minion of this tribe in a triple (Elemental of
    /// Surprise).
    pub triple_wildcard: Option<Tribe>,
    /// Whenever its owner plays a minion of this tribe from hand, it gets
    /// Divine Shield until the next shop phase, for good when this is golden
    /// (Ichoron the Protector).
    pub play_shield: Option<Tribe>,
    /// After its owner buys a minion (Stone Age Slab).
    pub after_buy: Option<AfterBuy>,
    /// Gets this much for every friendly minion with this same flag that died
    /// this game, wherever it is (Eternal Knight). Golden doubles it.
    pub buff_per_kind_death: Option<(i32, i32)>,
    /// After another friendly minion is Reborn (Barrier Banshee, Snazzy
    /// Phantom).
    pub on_reborn: Option<OnReborn>,
    pub deity: Option<DeityKind>,
    pub on_card_added: Option<(i32, i32)>,
    pub on_discard: Option<OnDiscard>,
    /// A Tavern spell (BATTLEGROUND_SPELL in the game data): only these fire
    /// "Tavern spell" triggers and spell counters, and get spell power. Plain
    /// spells (Blood Gem, Gem Day, Triple Reward, Lockbox) do not.
    pub tavern_spell: bool,
    /// The Blood Gem itself (BG20_GEM): a plain 0-cost spell whose buff is
    /// the player's current Blood Gem value.
    pub blood_gem: bool,
    /// A Spellcraft spell: it leaves the hand at the end of the turn if it
    /// was not cast (docs/rules/design.md "Spellcraft").
    pub spellcraft: bool,
    /// Whenever a Blood Gem is cast from hand on this minion, this plays this
    /// many Blood Gems on a random different friendly minion (Geomagus
    /// Roogug). Golden doubles the count. The Gems it plays are not from
    /// hand, so they never chain.
    pub on_gem_played_on_self: Option<u8>,
    /// How many Choose One cards played this turn get both effects combined
    /// (Thorned Trailblazer). Golden doubles it. Left to right, each
    /// Trailblazer covers one more card.
    pub choose_one_cover: u8,
    /// After you play a Choose One card, this plays this many Blood Gems on
    /// all your Quilboar (Turbo Hogrider). Golden doubles it.
    pub after_choose_one_play: Option<u8>,
    /// Hero card_ids whose starting powers a "Discover a new Hero Power"
    /// spell never offers (Unmasked Identity's exclusion list). Data only;
    /// the game resolves them against the hero table.
    pub exclude_heroes: Vec<String>,
    /// Whenever its owner plays a card (a Blood Gem and a Banana included),
    /// this minion on the board gains this much (Dark Gifts Dexterity,
    /// Sharpened Sword, Toughened Shield). A gift, never on a pool card.
    pub on_card_play: Option<(i32, i32)>,
    /// Gets this much for every Battlecry its owner has triggered this game,
    /// wherever it is (Dark Gifts Battle Scars).
    pub buff_per_battlecry: Option<(i32, i32)>,
    /// Gets this much for every Tavern spell its owner has cast this game
    /// (Dark Gifts Spell Siphon).
    pub buff_per_tavern_spell: Option<(i32, i32)>,
    /// Gets this much for every card its owner has discarded this game
    /// (Dark Gifts Desecration).
    pub buff_per_discard: Option<(i32, i32)>,
    /// Its owner gets this many extra plain copies of it when it enters the
    /// hand (Dark Gifts Double Vision). The copies carry no gifts.
    pub acquire_copies: u8,
    /// While its owner plays an Activate, the effect runs this many times in
    /// all (Dark Gifts Efficiency: 2).
    pub activate_twice: bool,
    /// Its Divine Shield takes this many hits to break (Dark Gifts Toreth's
    /// Blessing: 3; Dark Paradox t4: 3). The landing sets the host's
    /// `shield_hits` for a gift; a pool card sets it in `CardDb::make`.
    pub shield_hits: u8,
    /// The hits a GOLDEN version of this card's shield takes, when that is
    /// not double `shield_hits` (Dark Paradox's Tier 5 golden: 4, not 6). 0
    /// means the double.
    pub golden_shield_hits: u8,
    /// One of the Dark Paradox versions (`BG36_360t3` and its kin): a pool
    /// card, but only ONE of them is in a game's pool, the same for every
    /// player (docs/rules/darkgifts.md "Dark Paradox").
    pub paradox: bool,
    /// Immune while attacking, in a fight (Dark Gifts Invulnerability).
    pub immune_attacking: bool,
    /// Has all ten minion types (Dark Gifts Amalgamation). Every tribe check
    /// on the minion reads it through `CardDb::tribes_of`.
    pub all_types: bool,
    /// Reborn with full stats and its Bonus Keywords (Dark Gifts Persisting
    /// Horror). Fights only.
    pub reborn_full: bool,
    /// Land on the host as a Golden minion (Dark Gifts Gilding).
    pub makes_golden: bool,
    /// A gift that does not stack: a second copy of it on the same minion
    /// changes nothing (docs/rules/darkgifts.md "Gifts stack": the keyword gifts,
    /// Amalgamation, Toreth's Blessing's hits and Gilding, which set a state
    /// instead of firing again). Every other gift stacks.
    pub gift_unique: bool,
    /// Its end of turn effects also trigger at the start of its owner's turn
    /// (Dark Gifts Time Turning).
    pub end_of_turn_at_start: bool,
    /// A countdown the gift runs at the end of its owner's turn (Dark Gifts
    /// Incubation, Replication, Affinity).
    pub countdown: Option<Countdown>,
}

/// "Count down N turns, then do this" (Dark Gifts Incubation, Replication,
/// Affinity). The turns left live in the gift's own counter on the minion
/// (`GiftOn::state`); whether the gift starts the count again after paying
/// out is darkgifts.toml's `repeats`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Countdown {
    pub turns: u8,
    pub effect: CountdownEffect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CountdownEffect {
    /// Double the minion's Attack and Health (Incubation).
    DoubleStats,
    /// Get a plain copy of the minion (Replication).
    PlainCopy,
    /// Get a random minion of the minion's own type (Affinity). None when it
    /// has no type.
    RandomMinionOfType,
}

impl Card {
    pub fn is_spell(&self) -> bool {
        self.spell
    }

    /// Whether the card may exist in a lobby of these tribes, which is also
    /// the filter every card draw applies: a minion with a lobby tribe, or a
    /// neutral or spell not tied to a tribe outside it (`Card.lobby`, the
    /// game data's `lobby_subsets`). An all-type minion is always in. A
    /// wider mask widens the draw (a future trinket), `Tribes::ALL` is no
    /// filter.
    pub fn lobby_ok(&self, lobby: Tribes) -> bool {
        match self.lobby {
            Some(t) => lobby.has(t),
            None => self.tribes.is_neutral() || self.tribes.overlaps(lobby),
        }
    }

    /// Whether this spell gives stats to minions (used by the rule bot).
    pub fn gives_stats(&self) -> bool {
        matches!(
            self.spell_target,
            Some(
                SpellTarget::Buff { .. }
                    | SpellTarget::BuffSameTribe { .. }
                    | SpellTarget::BuffPerBonusKeyword { .. }
                    | SpellTarget::BuffPerTribePlayed { .. }
                    | SpellTarget::HalfOfHighestTavern { .. }
            )
        )
            || matches!(
                self.spell_effect,
                Some(
                    SpellEffect::BuffBoard { .. }
                        | SpellEffect::BuffRandomBoard { .. }
                        | SpellEffect::BuffShop { .. }
                        | SpellEffect::NextTurnBoardBuff { .. }
                )
            )
            || self.choose_one.is_some_and(|opts| {
                opts.iter().any(|o| {
                    matches!(
                        o,
                        ChooseOption::SpellTarget(SpellTarget::Buff { .. })
                            | ChooseOption::SpellEffect(SpellEffect::BuffBoard { .. })
                    )
                })
            })
    }

    /// Whether this card reads its owner's hand during a fight, so the
    /// fight needs the hand passed in (Expert Aviator, Flighty Scout,
    /// Diremuck Forager, Costume Enthusiast, Choral Mrrrglr, Very Hungry
    /// Winterfinner).
    pub fn reads_hand(&self) -> bool {
        self.when_damaged.is_some()
            || matches!(
                self.deathrattle,
                Some(Deathrattle::Effect(Effect::SummonFromHand { .. }))
            )
            || matches!(
                self.start_of_combat,
                Some(
                    StartOfCombat::FlightyScout
                        | StartOfCombat::GainHighestHandAttack
                        | StartOfCombat::GainHandStats
                        | StartOfCombat::Effect(Effect::SummonFromHand { .. })
                )
            )
            || matches!(self.rally, Some(Effect::SummonFromHand { .. }))
    }

    /// Whether a host Mech needs to store this card in `Minion::attached`
    /// when Magnetized. Satellites and Volumizers have no persistent board
    /// triggers after being Magnetized, so they take no slot.
    pub fn carries_effect(&self) -> bool {
        self.deathrattle.is_some()
            || self.start_of_turn.is_some()
            || self.end_of_turn.is_some()
            || self.start_of_combat.is_some()
            || self.rally.is_some()
            || self.on_summon_beast.is_some()
            || self.on_play_mech.is_some()
            || self.on_play_or_magnetize_mech.is_some()
            || self.on_cast_spell_on_mech.is_some()
            || self.on_cast_spell.is_some()
            || self.on_cast_spell_on_self.is_some()
            || self.on_cast_spell_on_tribe.is_some()
            || self.when_damaged.is_some()
            || self.while_in_hand.is_some()
            || self.whenever_friendly_attacks.is_some()
            || self.after_friendly_attacks.is_some()
            || self.avenge.is_some()
            || self.damage_milestone.is_some()
            || self.activate.is_some()
            || self.on_event.is_some()
            || !self.after_hero_damage.is_empty()
            || self.after_play_tribe.is_some()
            || self.after_demon_damage.is_some()
            || self.health_refreshes > 0
            || self.on_sell.is_some()
            || self.after_discover.is_some()
            || self.play_shield.is_some()
            || self.after_buy.is_some()
            || self.aura.is_some()
            || self.after_summon_give_stats.is_some()
            || self.spell_buff.is_some()
            || self.on_reborn.is_some()
            || self.on_gem_played_on_self.is_some()
            || self.choose_one_cover > 0
            || self.after_choose_one_play.is_some()
            || self.gains_friendly_deathrattles
    }
}

/// All cards. Ids `0..pool_cards` are the pool minions, tokens and spells
/// come after.
#[derive(Clone, Debug)]
pub struct CardDb {
    pub cards: Vec<Card>,
    pub pool_cards: usize,
    /// Tier of every card by id, kept compact for the pool draws.
    pub tiers: Vec<u8>,
    /// Pool card ids (ascending) of tier at most the index, for the pool draws.
    pub ids_up_to_tier: Vec<Vec<u8>>,
    pub volumizers: Vec<CardId>,
    pub spells: Vec<CardId>,
    pub beetle_token: Option<CardId>,
    /// The 2/3 Venomous Cobra Venomstrike Trap summons (a Beast token).
    pub cobra_token: Option<CardId>,
    /// The five Chromadrakes "a random Chromadrake" picks from.
    pub chromadrakes: Vec<CardId>,
    /// The 1/1 Murloc Magicfin Mycologist teaches a spell to.
    pub taught_token: Option<CardId>,
    /// The generated Demon Fodder token.
    pub fodder_token: Option<CardId>,
    /// The five Bounties. Only the first is in `spells`: the five are one
    /// slot of every spell draw, and a drawn Bounty is one of them at random.
    pub bounties: Vec<CardId>,
    /// The generated Lockbox spell.
    pub lockbox: Option<CardId>,
    /// The generated Blood Gem spell.
    pub blood_gem: Option<CardId>,
    /// The generated Triple Reward spell.
    pub triple_reward: Option<CardId>,
    /// Every hero a card says a Hero Power Discover never offers (cards.toml
    /// `exclude_heroes`, by hero card_id, deduplicated). A Hero Power Discover
    /// that no card opens (Genn's King of Duality, the Reinvigorating Light
    /// trinket) uses the same list: one place for Era to add to.
    pub hero_power_excludes: Vec<String>,
    /// The 32 Darkmoon Prizes by Prize tier (index 0 is tier 1): the list a
    /// Prize Discover offers three different cards from, and the one a random
    /// Prize is drawn from. Nothing else ever draws them: they are not in the
    /// Tavern's spell pool.
    pub prizes: [Vec<CardId>; 4],
    /// The four Magnetic Mecha-Demons the Mecha-Jaraxxus Sticker hands out,
    /// in card order so a seed replays.
    pub mecha_demons: Vec<CardId>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    minion: Vec<RawCard>,
    #[serde(default)]
    token: Vec<RawCard>,
    #[serde(default)]
    spell: Vec<RawSpell>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCard {
    name: String,
    tier: u8,
    /// One tribe, "neutral" or "all".
    tribe: Option<String>,
    /// Two or more tribes, for multi-tribe cards.
    tribes: Option<Vec<String>>,
    /// A tribe this neutral is tied to (lobby_subsets in the export): it
    /// only exists in lobbies with that tribe.
    lobby: Option<String>,
    stats: [i32; 2],
    /// Golden stats when they are not exactly double the printed ones (the
    /// data has one such card).
    golden_stats: Option<[i32; 2]>,
    #[serde(default)]
    keywords: Vec<String>,
    #[serde(default)]
    beetle: bool,
    /// The token Venomstrike Trap summons (cards.toml `secret_cobra`).
    #[serde(default)]
    secret_cobra: bool,
    volumizer_buff: Option<[i32; 2]>,
    spell_buff: Option<[i32; 2]>,
    battlecry: Option<RawEffect>,
    /// A Choose One option: a minion-target buff (the `RawMinionChooseOption`
    /// fields) or a whole spell effect (the `RawSpellEffect` fields), the way
    /// the Quilboar Choose One cards mix both.
    choose_one: Option<[RawChooseOption; 2]>,
    deathrattle: Option<RawDeathrattle>,
    start_of_turn: Option<RawStartOfTurn>,
    end_of_turn: Option<RawEndOfTurn>,
    start_of_combat: Option<RawStartOfCombat>,
    rally: Option<RawEffect>,
    on_summon_beast: Option<RawOnSummonBeast>,
    on_play_mech: Option<RawOnPlayMech>,
    on_play_or_magnetize_mech: Option<RawBuff>,
    on_cast_spell_on_mech: Option<RawOnCastSpellOnMech>,
    on_cast_spell: Option<RawOnCastSpell>,
    on_cast_spell_on_self: Option<RawEffect>,
    on_cast_spell_on_tribe: Option<RawOnCastSpellOnTribe>,
    when_damaged: Option<RawEffect>,
    while_in_hand: Option<RawWhileInHand>,
    #[serde(default)]
    teaches_spell: bool,
    #[serde(default)]
    no_triple: bool,
    #[serde(default)]
    taught_token: bool,
    whenever_friendly_attack: Option<RawAttackTrigger>,
    after_friendly_attack: Option<RawAttackTrigger>,
    avenge: Option<RawAvenge>,
    damage_milestone: Option<RawMilestone>,
    buff_per_deathrattle: Option<[i32; 2]>,
    #[serde(default)]
    fixed_stats: bool,
    activate: Option<RawActivate>,
    on_event: Option<RawOnEvent>,
    after_battlecry: Option<RawEffect>,
    /// "self" or "adjacent_dragons".
    permanent_keep: Option<String>,
    attack_threshold: Option<RawAttackThreshold>,
    /// One of the five generated Chromadrakes.
    #[serde(default)]
    chromadrake: bool,
    #[serde(default)]
    after_hero_damage: Vec<RawEffect>,
    after_play_tribe: Option<RawAfterPlayTribe>,
    after_demon_damage: Option<[i32; 2]>,
    /// A generated Demon Fodder token.
    #[serde(default)]
    fodder: bool,
    health_refreshes: Option<u8>,
    on_sell: Option<RawOnSell>,
    aura: Option<String>,
    after_summon_give_stats: Option<RawAfterSummonGiveStats>,
    #[serde(default)]
    always_golden: bool,
    #[serde(default)]
    cleave: bool,
    buff_per_golden_played: Option<[i32; 2]>,
    buff_per_elemental_played: Option<[i32; 2]>,
    buff_per_magnetize: Option<[i32; 2]>,
    /// What this minion gets after it attacks and kills a minion.
    on_kill: Option<String>,
    /// Gains the Deathrattle of a friendly Deathrattle minion that dies in
    /// combat (Fish of N'Zoth).
    #[serde(default)]
    gains_friendly_deathrattles: bool,
    after_discover: Option<RawEffect>,
    #[serde(default)]
    overkill: bool,
    /// Tribe a triple wildcard stands in for.
    triple_wildcard: Option<String>,
    /// Tribe whose members get Divine Shield when played.
    play_shield: Option<String>,
    after_buy: Option<RawAfterBuy>,
    buff_per_kind_death: Option<[i32; 2]>,
    on_reborn: Option<RawOnReborn>,
    /// A Magnetic minion that can Magnetize to an Undead as well as a Mech
    /// (Prosthetic Hand).
    #[serde(default)]
    magnetize_to_undead: bool,
    /// The tribes this Magnetic card Magnetizes onto, when they are not just
    /// Mech (Mecha-Jaraxxus's Mecha-Demons).
    #[serde(default)]
    magnetize_to_tribes: Vec<String>,
    /// A Darkmoon Prize's tier, 1 to 4 (All-Seeing Elder).
    prize: Option<u8>,
    /// One of the four Magnetic Mecha-Demons (Mecha-Jaraxxus Sticker).
    #[serde(default)]
    mecha_demon: bool,
    /// A buddy card, tagged for when buddies come (Mishmash).
    #[serde(default)]
    buddy: bool,
    /// The card is an Amalgam (the Curator's token).
    #[serde(default)]
    amalgam: bool,
    /// Whenever a friendly Amalgam gains stats, this gains them this many
    /// times (Mishmash). Golden doubles it.
    amalgam_gain_copy: Option<u8>,
    /// Blood Gems this plays on a random different friendly minion when a
    /// Gem lands on it (Roogug).
    on_gem_played_on_self: Option<u8>,
    /// Choose One cards each turn that get both effects (Trailblazer).
    choose_one_cover: Option<u8>,
    /// Blood Gems played on all your Quilboar after a Choose One card
    /// (Turbo Hogrider).
    after_choose_one_play: Option<u8>,
    deity: Option<String>,
    on_card_added: Option<RawBuff>,
    /// Dark Gifts: see the same names on `Card`.
    on_card_play: Option<[i32; 2]>,
    buff_per_battlecry: Option<[i32; 2]>,
    buff_per_tavern_spell: Option<[i32; 2]>,
    buff_per_discard: Option<[i32; 2]>,
    #[serde(default)]
    acquire_copies: u8,
    #[serde(default)]
    activate_twice: bool,
    #[serde(default)]
    shield_hits: u8,
    #[serde(default)]
    golden_shield_hits: u8,
    /// This is one of the Dark Paradox versions: `Game::unstarted` puts
    /// exactly one of them in a game's pool.
    #[serde(default)]
    paradox: bool,
    #[serde(default)]
    immune_attacking: bool,
    #[serde(default)]
    all_types: bool,
    #[serde(default)]
    reborn_full: bool,
    #[serde(default)]
    makes_golden: bool,
    #[serde(default)]
    gift_unique: bool,
    #[serde(default)]
    end_of_turn_at_start: bool,
    countdown: Option<RawCountdown>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCountdown {
    turns: u8,
    /// "double_stats", "plain_copy" or "random_minion_of_type".
    effect: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawOnReborn {
    banshee: Option<[i32; 2]>,
    #[serde(default)]
    phantom: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAfterBuy {
    buff: [i32; 2],
    multiply: i32,
    golden_multiply: i32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAfterSummonGiveStats {
    times: u8,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAttackThreshold {
    total: u16,
    keyword: String,
}

/// Whether casting this spell at a random target, as Puzzle Box does, would
/// destroy or transform something: those are never offered
/// (docs/rules/heroes.md). Channel the Devourer sells a minion, Butchering
/// destroys one, Corrupted Cupcakes eats the Tavern's minions and Unmasked
/// Identity replaces the hero power.
fn destructive_at_random(c: &Card) -> bool {
    match c.spell_target {
        Some(SpellTarget::Devourer)
        | Some(SpellTarget::Butchering { .. })
        | Some(SpellTarget::Consume { .. })
        | Some(SpellTarget::DestroyGetRandom { .. }) => true,
        _ => matches!(c.spell_effect, Some(SpellEffect::DiscoverHeroPower)),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSpell {
    name: String,
    tier: u8,
    cost: u8,
    /// A tribe the spell is tied to (lobby_subsets in the export): it only
    /// exists in lobbies with that tribe.
    lobby: Option<String>,
    /// A spell that is never in the Tavern's spell pool (Pointy Arrow).
    #[serde(default)]
    token: bool,
    /// Bought with Health instead of Gold (Hasty Excavation).
    #[serde(default)]
    pay_health: bool,
    /// One of the five Bounties (they share one slot of the spell draw).
    #[serde(default)]
    bounty: bool,
    /// A Tavern spell (BATTLEGROUND_SPELL). Every pool spell is one; plain
    /// spells (Blood Gem, Gem Day, Triple Reward) say `tavern_spell = false`.
    #[serde(default = "default_true")]
    tavern_spell: bool,
    /// The Blood Gem itself.
    #[serde(default)]
    blood_gem: bool,
    /// A Spellcraft spell: it leaves the hand at the end of the turn.
    #[serde(default)]
    spellcraft: bool,
    /// A Darkmoon Prize's tier, 1 to 4 (the game data's
    /// `battlegroundsDarkmoonPrizeTurn`). A Prize is a plain spell that is
    /// never in the pool: only a Prize Discover offers it.
    prize: Option<u8>,
    on_discard: Option<String>,
    effect: Option<RawSpellEffect>,
    choose_one: Option<[RawSpellEffect; 2]>,
}

fn default_true() -> bool {
    true
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAfterPlayTribe {
    tribe: String,
    effects: Vec<RawEffect>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSpellEffect {
    target: Option<String>,
    /// "chosen", "random" or "highest_health", for a spell whose target
    /// consumes Tavern minions.
    consume: Option<String>,
    #[serde(default)]
    with_keywords: bool,
    /// The tribe the spell's target must have.
    tribe: Option<String>,
    buff: Option<[i32; 2]>,
    keyword: Option<String>,
    /// A random Bonus Keyword the target does not have yet (Vibrant Bubble).
    #[serde(default)]
    random_keyword: bool,
    times: Option<u8>,
    count: Option<u8>,
    #[serde(default)]
    golden_extra: bool,
    if_win_next_turn_buff: Option<[i32; 2]>,
    next_turn_board_buff: Option<[i32; 2]>,
    same_tribe_minion: Option<bool>,
    set_stats: Option<[i32; 2]>,
    make_golden_max_tier: Option<u8>,
    /// Get a plain copy of the target, a minion of this tier or below
    /// (Duplicating Lens).
    plain_copy: Option<u8>,
    gold: Option<u8>,
    gold_next_turn: Option<u8>,
    max_gold: Option<u8>,
    buff_deity: Option<[i32; 2]>,
    free_rolls: Option<u8>,
    overconfidence: Option<[u8; 2]>,
    random_tier_minion: Option<u8>,
    discover: Option<String>,
    boon_of_beetles: Option<u8>,
    start_of_combat: Option<String>,
    golden_touch: Option<bool>,
    /// "Give your minions..., repeat for your Dragons, repeat for your
    /// minions with Divine Shield."
    mighty_dragonbreath: Option<[i32; 2]>,
    /// "Give a minion stats, repeat for each different Bonus Keyword in your
    /// warband."
    #[serde(default)]
    per_bonus_keyword: bool,
    /// Get a random Murloc and a copy of it.
    #[serde(default)]
    cloning_conch: bool,
    /// Give minions in the Tavern stats for the rest of the game.
    tavern_buff_permanent: Option<[i32; 2]>,
    /// Misplaced Tea Set: one minion per type on the board.
    misplaced_tea_set: Option<[i32; 2]>,
    #[serde(default)]
    planar_telescope: bool,
    #[serde(default)]
    steal_random_tavern: bool,
    #[serde(default)]
    discover_locked_minion: bool,
    #[serde(default)]
    saloon_finest: bool,
    /// Refresh the Tavern with Battlecry minions at this price (Chillmere
    /// Mosaic).
    refresh_with_battlecries: Option<u8>,
    /// Get `count` copies of this generated card.
    get: Option<String>,
    /// The chosen minion: toggle Taunt with these stats.
    tricky_trousers: Option<[i32; 2]>,
    #[serde(default)]
    robust_evolution: bool,
    /// Eonar's Favor: Tavern minions of the chosen minion's type.
    tavern_buff_type: Option<[i32; 2]>,
    #[serde(default)]
    refresh_with_type: bool,
    #[serde(default)]
    devourer: bool,
    /// An unplayable Lockbox that opens after this many turns.
    lockbox: Option<u8>,
    /// Give the left-most minion on the board stats (Selfish Bounty).
    buff_leftmost: Option<[i32; 2]>,
    #[serde(default)]
    friendly_bounty: bool,
    #[serde(default)]
    menagerie_tableware: Option<[i32; 2]>,
    /// Add to the "after the Tavern is Refreshed" amount.
    refresh_buff: Option<[i32; 2]>,
    /// Get one of each of two named cards.
    get_pair: Option<[String; 2]>,
    /// Give the chosen friendly minion (of `tribe`) half the stats of the
    /// highest-Health Tavern minion.
    #[serde(default)]
    half_of_highest: bool,
    /// Buff by `buff`, plus this per minion of `tribe` played this turn.
    per_played_this_turn: Option<[i32; 2]>,
    /// Butchering: destroy the chosen friendly Undead, then this much Attack
    /// for every Undead this game.
    butchering: Option<i32>,
    /// A Blood Gem that also gives this keyword to a Quilboar (Jewelry Box's
    /// three special Gems).
    gem_keyword: Option<String>,
    /// Trigger the chosen friendly minion's Deathrattle outside combat
    /// (Floating Candle Set).
    #[serde(default)]
    trigger_deathrattle: bool,
    /// Tomb Turning: Discover an Undead that dies if played this turn.
    #[serde(default)]
    tomb_turning: bool,
    /// Set your Armor to this much (Armor Stash).
    set_armor: Option<u8>,
    /// Discover a new Hero Power (Unmasked Identity).
    #[serde(default)]
    discover_hero_power: bool,
    /// Heroes whose powers that Discover never offers, by hero card_id.
    #[serde(default)]
    exclude_heroes: Vec<String>,
    /// Destroy the chosen friendly minion of `tribe`, then get `count`
    /// random minions of it (Jailer Sticker).
    #[serde(default)]
    destroy_get_random: bool,
    /// Transform the chosen minion into one from a Tier higher (Token of the
    /// Old Gods).
    #[serde(default)]
    transform_higher: bool,
    /// Watch the chosen minion's stat gains for the Deity (Makeshift
    /// Master).
    #[serde(default)]
    watch_stats: bool,
    /// Gem Confiscation: play `confiscate` Blood Gems on the chosen minion
    /// and steal its neighbours' Blood Gem stats.
    confiscate: Option<u8>,
    /// Get this many Blood Gems (a Choose One option).
    get_gems: Option<u8>,
    /// Your Blood Gems give this much extra this game (a Choose One option).
    gem_bonus: Option<[i32; 2]>,
    /// Your Tavern spells give this much extra this game (Crystallization).
    spell_bonus: Option<[i32; 2]>,
    /// Play this many Blood Gems on every friendly minion (a Choose One
    /// option).
    gems_on_board: Option<u8>,
    /// Blood Gem Barrage: add to the per-Refresh Tavern gem amount.
    barrage: Option<[i32; 2]>,
    /// Cast a named spell this many times (a Choose One option).
    cast: Option<String>,
    /// Get a random minion of this tribe (a Choose One option).
    random_tribe_minion: Option<String>,
    /// Triple Reward: Discover a minion of the tier fixed when this copy was
    /// given.
    #[serde(default)]
    triple_reward: bool,
    // ---- the Darkmoon Prizes (block 5) ----
    /// Discover a Darkmoon Prize of this Prize tier (the Triple Prize,
    /// Tickatus Sticker).
    discover_prize: Option<u8>,
    /// Discover a Darkmoon Prize from each Prize tier below this one (Big
    /// Winner!).
    discover_prizes: Option<u8>,
    /// Get this many random Darkmoon Prizes, any tier (Hand of Fate).
    random_prize: Option<u8>,
    /// Fill the hand with copies of this card (B.A.N.A.N.A.S.).
    fill_hand: Option<String>,
    /// Fill the hand with random Tavern spells (Gorgeous Goblet).
    #[serde(default)]
    fill_hand_spells: bool,
    /// Gain 1 Gold, and get this card back at the end of every turn (The
    /// Unlimited Coin).
    #[serde(default)]
    return_each_turn: bool,
    /// Replace every card in the Tavern with one from a Tier higher
    /// (Evolving Tavern).
    #[serde(default)]
    evolving_tavern: bool,
    /// Steal every card in the Tavern, then Refresh it (Mindflayer Goggles).
    #[serde(default)]
    steal_all_tavern: bool,
    /// The Tavern offers one more minion, with these stats, for the rest of
    /// the game (New Recruit).
    extra_shop_minion: Option<[i32; 2]>,
    /// Free Refreshes now and at the start of every turn for the rest of the
    /// game, `[now, per_turn]` (Rocking and Rolling, Open Bar).
    free_rolls_per_turn: Option<[u8; 2]>,
    /// A random Tavern spell costing at least this much (Unfurled Codex).
    random_spell_at_least: Option<u8>,
    /// Tavern spells cost this much less this turn (Reserve Prices).
    spell_discount_turn: Option<u8>,
    /// Minions in the Tavern cost this much for the rest of the game
    /// (Friends and Family Discount).
    minion_price: Option<u8>,
    /// Your Battlecries trigger this many extra times this turn (Big Brann
    /// Play).
    battlecry_extra_turn: Option<u8>,
    /// A "then double it" buff: "attack" or "health" (I'm Still Just a Rat in
    /// a Cage, The Bouncer).
    then_double: Option<String>,
    /// Return the chosen minion to hand with these stats (Repeat Customer).
    return_to_hand: Option<[i32; 2]>,
    /// Make the chosen minion Golden and return it to hand (Raise the
    /// Stakes).
    #[serde(default)]
    make_golden_return: bool,
    /// Give the chosen minion "At the end of your turn, gain this" (Gruul
    /// Rules).
    grant_end_of_turn: Option<[i32; 2]>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEffect {
    buff: Option<[i32; 2]>,
    /// "chosen", "others" or "self", for a buff.
    target: Option<String>,
    /// "chosen" may pick a minion in the Tavern too: only the card's words
    /// narrow a target (docs/rules/design.md "Targets in the shop"), so a text that says
    /// "another Murloc" or "a Beast" (not "a friendly ...") reaches it.
    #[serde(default)]
    in_shop: bool,
    tribe: Option<String>,
    /// Added once more per different Bonus Keyword in the warband.
    per_keyword: Option<[i32; 2]>,
    #[serde(default)]
    random_keyword: bool,
    /// "chosen", "random" or "highest_health".
    consume: Option<String>,
    /// The consumer is a friendly minion the player picks (of `tribe`).
    #[serde(default)]
    consume_by_chosen: bool,
    #[serde(default)]
    with_keywords: bool,
    tavern_buff: Option<[i32; 2]>,
    #[serde(default)]
    this_turn: bool,
    damage_hero: Option<i32>,
    #[serde(default)]
    rewind: bool,
    #[serde(default)]
    magnetize_volumizer: bool,
    beetle_buff: Option<[i32; 2]>,
    gold_next_turn: Option<u8>,
    /// Raise the player's maximum Gold (Doubloon Grifter).
    max_gold: Option<u8>,
    random_volumizer: Option<bool>,
    spell_discount: Option<u8>,
    /// Raise the player's Tavern spell bonus this game.
    spell_bonus: Option<[i32; 2]>,
    buff_deity: Option<[i32; 2]>,
    per_spell: Option<[i32; 2]>,
    buff_self_and_deity: Option<[i32; 2]>,
    buff_deity_and_improve: Option<[i32; 2]>,
    buff_leftmost_per_discard: Option<[i32; 2]>,
    #[serde(default)]
    random_spell_up_to: bool,
    linked_spells: Option<u8>,
    linked_minions: Option<String>,
    discard_leftmost_spells: Option<u8>,
    summon_and_buff: Option<String>,
    summon_dead_aberrations: Option<u8>,
    #[serde(default)]
    damage_highest_health: bool,
    #[serde(default)]
    random_chromadrake: bool,
    /// Cast a named Tavern spell, `count` times.
    cast: Option<String>,
    #[serde(default)]
    trigger_rally: bool,
    buff_attacker: Option<[i32; 2]>,
    /// Buff random other friendly minions of `tribe`, with `keyword`.
    buff_random_other: Option<[i32; 2]>,
    keyword: Option<String>,
    get: Option<String>,
    count: Option<u8>,
    /// Become an exact copy of a minion the player picks (Faceless
    /// Manipulator).
    #[serde(default)]
    become_copy: bool,
    /// Get a plain copy of a random friendly minion (Reflective Pendant).
    #[serde(default)]
    plain_copy_random_friendly: bool,
    /// Get a copy of the last card discarded with double stats (Writhing
    /// Tentacles).
    #[serde(default)]
    copy_last_discarded: bool,
    /// Summon a card (token or pool minion) next to the minion.
    summon: Option<String>,
    /// Summon the highest-Attack minions of this tribe from hand, for this
    /// combat only; with `by_health`, the highest-Health ones (Bassgill).
    summon_from_hand: Option<String>,
    #[serde(default)]
    by_health: bool,
    #[serde(default)]
    shield: bool,
    #[serde(default)]
    golden_doubles_count: bool,
    /// Get a random pool minion of this tribe.
    random_minion: Option<String>,
    /// Name of the counter this buff scales by and then improves.
    improve_counter: Option<String>,
    /// Buff the minion that killed this one.
    buff_killer: Option<[i32; 2]>,
    gain_keyword: Option<String>,
    /// Get `count` random Tavern spells with this cost.
    random_spell_cost: Option<u8>,
    summon_random_deathrattle: Option<u8>,
    #[serde(default)]
    trigger_deathrattle: bool,
    /// Pick a Tavern card to replace with this card, for the left-most Beast
    /// to attack.
    replace_with: Option<String>,
    /// Refresh the Tavern with this card in it, for the left-most Beast to
    /// attack.
    refresh_with: Option<String>,
    extra_magnetization: Option<u8>,
    /// Give a random minion in hand stats (Winterfinner).
    buff_random_hand: Option<[i32; 2]>,
    /// Give the left-most minion in hand stats (Twilight Tidehunter).
    buff_leftmost_hand: Option<[i32; 2]>,
    /// Trigger a friendly minion's Battlecry, chosen by the player.
    #[serde(default)]
    trigger_battlecry: bool,
    /// If you control another minion of this tribe, Discover `count` of them.
    discover_tribe_if_another: Option<String>,
    /// Cast the Tavern spell this minion was taught.
    #[serde(default)]
    cast_taught: bool,
    /// Trigger the Battlecries of the adjacent minions, `count` times.
    trigger_adjacent_battlecries: Option<u8>,
    /// Add Fodders to the next `n` Refreshes.
    add_fodder_refreshes: Option<u8>,
    /// Add Fodders to the very next Refresh.
    add_fodder_to_next_refresh: Option<u8>,
    /// Your Demons each consume `n` random Tavern minions.
    demons_consume: Option<u8>,
    /// Raise this minion's `improvements` by `n`.
    improve_self: Option<u8>,
    /// Give the Tavern stats and one random keyword out of `keywords`.
    tavern_buff_random_keyword: Option<[i32; 2]>,
    keywords: Option<Vec<String>>,
    /// Discover `n` Demons up to the owner's tier, then damage the hero for
    /// their tiers.
    discover_demon_tier_damage: Option<u8>,
    /// Set a minion's (board or Tavern) stats (Tyrael).
    set_stats_any: Option<[i32; 2]>,
    /// Steal the `n` highest-Attack minions from the Tavern (Decoy Conjurer).
    steal_tavern: Option<u8>,
    /// Discover `n` Tavern spells up to the owner's tier (Rodeo Performer).
    discover_spell: Option<u8>,
    /// Rally: gain the target's Attack (Heroic Underdog).
    #[serde(default)]
    gain_target_attack: bool,
    /// Rally: remove these keywords from the target (Sin'dorei Straight
    /// Shot).
    strip_keywords: Option<Vec<String>>,
    /// Deathrattle: destroy the minion that killed this one (Leeroy the
    /// Reckless).
    #[serde(default)]
    destroy_killer: bool,
    /// Deathrattle: give `n` friendly Dragons Divine Shield (Nadina the
    /// Red).
    shield_dragons: Option<u8>,
    /// Get a random minion of exactly this tier (Highkeeper Ra).
    random_tier_minion: Option<u8>,
    /// Give a friendly minion of each type stats permanently (The Last One
    /// Standing).
    buff_each_type: Option<[i32; 2]>,
    /// Rally: damage the target and a neighbour (Obsidian Ravager).
    #[serde(default)]
    damage_target_and_neighbor: bool,
    /// Give this minion's stats to the left-most minion in hand (Futurefin).
    #[serde(default)]
    give_stats_leftmost_hand: bool,
    /// Get a Lockbox, or open the held one this many turns sooner.
    lockbox_sooner: Option<u8>,
    #[serde(default)]
    random_bounty: bool,
    /// Give all friendly minions the buff; golden minions keep it.
    keep_golden: Option<[i32; 2]>,
    /// Give adjacent minions this much Attack, repeated per Golden minion.
    adjacent_per_golden: Option<i32>,
    /// Added once more for each Gold spent this turn.
    per_gold_spent: Option<[i32; 2]>,
    /// Make friendly minions up to this tier Golden on the board.
    make_golden_max_tier: Option<u8>,
    /// Get random Golden minions of exactly this tier.
    golden_minion_tier: Option<u8>,
    /// Scale a buff by one plus the Golden minions played this game.
    #[serde(default)]
    per_golden_played: bool,
    /// Add to the "after the Tavern is Refreshed this game" amount.
    refresh_buff: Option<[i32; 2]>,
    free_refreshes: Option<u8>,
    /// Dark Gifts: one random Tavern spell of any cost.
    #[serde(default)]
    get_any_tavern_spell: bool,
    /// Dark Gifts: a random minion of the player's most common type.
    #[serde(default)]
    get_most_common_type_minion: bool,
    /// Multiply a buff by one plus the named counter, then raise it.
    #[serde(default)]
    improving_counter: Option<String>,
    /// Arm this minion to gain the stats of the next minion bought.
    #[serde(default)]
    arm_next_buy: bool,
    #[serde(default)]
    gain_highest_tavern_stats: bool,
    /// Multiply a buff by one plus this minion's `improvements`.
    #[serde(default)]
    per_improvement: bool,
    /// Your Undead have +buff this game (wherever they are).
    undead_bonus: Option<[i32; 2]>,
    /// The Attack `undead_bonus` gives when it runs outside combat.
    outside_attack: Option<i32>,
    /// Give random friendly minions of this tribe Reborn (`count` of them).
    give_reborn_random: Option<String>,
    /// Summon this card, which attacks right after the current attack.
    summon_attacker: Option<String>,
    #[serde(default)]
    get_plain_copy: bool,
    /// Get a random minion of this tribe that this Deathrattle can summon.
    get_random_held: Option<String>,
    #[serde(default)]
    summon_held: bool,
    #[serde(default)]
    destroy_left: bool,
    #[serde(default)]
    summon_salvaged: bool,
    /// Destroy a chosen friendly minion of this tribe, then Discover `count`
    /// of that tribe.
    destroy_chosen_discover: Option<String>,
    /// Destroy a chosen friendly minion of this tribe, then get `count`
    /// plain copies of it.
    #[serde(default)]
    destroy_chosen_get_copy: Option<String>,
    /// Give a different friendly minion of this tribe Reborn, destroy it, and
    /// gain the buff.
    give_reborn_destroy: Option<String>,
    /// Get this many Blood Gems.
    get_gems: Option<u8>,
    /// Get this many Blood Gems; the ones past a full hand are cast on the
    /// left-most minion.
    get_gems_overflow: Option<u8>,
    /// Play this many Blood Gems on this minion. `gem_permanent` keeps them
    /// after a fight (Vineweaver).
    gem_self: Option<u8>,
    #[serde(default)]
    gem_permanent: bool,
    /// Play this many Blood Gems on the other friendly minions.
    gem_others: Option<u8>,
    /// Play this many Blood Gems on the minion that just attacked.
    gem_attacker: Option<u8>,
    /// Your Blood Gems give this much extra this game.
    gem_bonus: Option<[i32; 2]>,
    /// Get this many random Choose One cards.
    random_choose_one: Option<u8>,
    /// Rally: summon a copy of this card with this minion's stats, which
    /// attacks the target first (Jailbird Juggernaut).
    summon_stat_copy: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
// The spell shape carries every spell-effect field, which makes the two
// variants very different in size; only a handful are ever built.
#[allow(clippy::large_enum_variant)]
enum RawChooseOption {
    Minion(RawMinionChooseOption),
    Spell(RawSpellEffect),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMinionChooseOption {
    buff: Option<[i32; 2]>,
    keyword: Option<String>,
    tribe: Option<String>,
    spell_buff: Option<[i32; 2]>,
    #[serde(default)]
    in_shop: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDeathrattle {
    summon: Option<String>,
    count: Option<u8>,
    #[serde(default)]
    golden_doubles_count: bool,
    beetle_buff: Option<[i32; 2]>,
    summon_by_attack: Option<String>,
    combat_beast_buff: Option<[i32; 2]>,
    random_volumizer: Option<bool>,
    get: Option<String>,
    effect: Option<RawEffect>,
    /// Give the Tavern stats for the rest of the game, improved by the dead
    /// minion's `improvements`.
    tavern_buff: Option<[i32; 2]>,
    per_improvement: Option<[i32; 2]>,
    /// Dark Gifts Golemancy: a token summoned with the dead minion's stats.
    summon_shaped: Option<String>,
    /// Dark Gifts Offensive Sacrifice, Defensive Sacrifice.
    #[serde(default)]
    give_own_attack: bool,
    #[serde(default)]
    give_own_health: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStartOfTurn {
    gold: u8,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEndOfTurn {
    buff_self: Option<[i32; 2]>,
    resourceful_robot: Option<bool>,
    buff_per_magnetization: Option<[i32; 2]>,
    improve_tier: Option<bool>,
    get: Option<String>,
    count: Option<u8>,
    copy_last_spell: Option<bool>,
    effect: Option<RawEffect>,
    #[serde(default)]
    trigger_own_battlecry: bool,
    #[serde(default)]
    magnetize_random_mech: bool,
    #[serde(default)]
    gift_buff_self: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStartOfCombat {
    combat_beast_buff: Option<[i32; 2]>,
    damage_all_others: Option<i32>,
    /// Any effect a fight can run.
    effect: Option<RawEffect>,
    #[serde(default)]
    attack_immediately: bool,
    hopebringer: Option<RawHopebringer>,
    #[serde(default)]
    flighty_scout: bool,
    #[serde(default)]
    gain_hand_attack: bool,
    #[serde(default)]
    gain_hand_stats: bool,
    /// Dark Gifts.
    #[serde(default)]
    double_health: bool,
    #[serde(default)]
    double_attack: bool,
    #[serde(default)]
    triple_stats: bool,
    #[serde(default)]
    gain_left_attack: bool,
    /// All-Seeing Elder: the left-most minion's Attack and the right-most
    /// one's Health.
    #[serde(default)]
    gain_ends: bool,
    #[serde(default)]
    gain_deity_stats: bool,
    #[serde(default)]
    trigger_own_deathrattles: bool,
    #[serde(default)]
    summon_own_copy: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawOnCastSpellOnTribe {
    tribe: String,
    buff: [i32; 2],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawWhileInHand {
    /// The tribe of the minion that has to be played.
    after_play: String,
    buff: [i32; 2],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawHopebringer {
    buff: [i32; 2],
    improve: [i32; 2],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAttackTrigger {
    /// Only attackers of this tribe.
    attacker_tribe: Option<String>,
    /// Only attackers with a Rally effect.
    #[serde(default)]
    attacker_rally: bool,
    /// Another minion: the minion with the trigger isn't counted.
    #[serde(default)]
    attacker_another: bool,
    effect: RawEffect,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAvenge {
    every: u8,
    effect: RawEffect,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMilestone {
    total: u16,
    effect: RawEffect,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawOnSummonBeast {
    buff: Option<[i32; 2]>,
    #[serde(default)]
    improve_permanently: bool,
    multiply_attack_in_combat: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawOnPlayMech {
    magnetize_satellite: String,
    stats: [i32; 2],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawOnCastSpellOnMech {
    magnetize_satellite: String,
    stats: [i32; 2],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawOnCastSpell {
    buff_self_and_deity: Option<[i32; 2]>,
    buff_all_and_deity: Option<[i32; 2]>,
    buff_divine_shield: Option<[i32; 2]>,
    buff_all: Option<[i32; 2]>,
    undead_bonus: Option<[i32; 2]>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBuff {
    buff: [i32; 2],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawActivate {
    cost: u8,
    effect: RawEffect,
    /// Ask the player to discard a card in hand first; with nothing
    /// discardable the Activate does nothing (Mangled Bandit).
    #[serde(default)]
    discard: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawOnEvent {
    /// "spells_cast", "gold_spent", "hero_damage", "minions_sold" or "cards_bought".
    event: String,
    every: u8,
    effect: RawEffect,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawOnSell {
    #[serde(default)]
    discover_tier7_if_golden: bool,
    get: Option<String>,
    count: Option<u8>,
    discover_by_tier: Option<bool>,
    gold_if_lost: Option<u8>,
    effect: Option<RawEffect>,
    /// Gain the stats of the sold minion when it is of this tribe (Arcane
    /// Behemoth); an empty string is any minion.
    gain_sold_stats: Option<String>,
}

pub(crate) fn tribe(s: &str) -> Result<Tribe, String> {
    match s {
        "beast" => Ok(Tribe::Beast),
        "mech" => Ok(Tribe::Mech),
        "dragon" => Ok(Tribe::Dragon),
        "murloc" => Ok(Tribe::Murloc),
        "demon" => Ok(Tribe::Demon),
        "pirate" => Ok(Tribe::Pirate),
        "elemental" => Ok(Tribe::Elemental),
        "undead" => Ok(Tribe::Undead),
        "quilboar" => Ok(Tribe::Quilboar),
        "aberration" => Ok(Tribe::Aberration),
        _ => Err(format!("unknown tribe {s:?}")),
    }
}

/// One entry of a card's tribe list: a tribe, "neutral" (none) or "all".
fn tribe_set(s: &str) -> Result<Tribes, String> {
    match s {
        "neutral" => Ok(Tribes::NONE),
        "all" => Ok(Tribes::ALL),
        _ => Ok(Tribes::of(tribe(s)?)),
    }
}

fn card_tribes(one: &Option<String>, many: &Option<Vec<String>>) -> Result<Tribes, String> {
    match (one, many) {
        (Some(t), None) => tribe_set(t),
        (None, Some(ts)) => {
            let mut set = Tribes::NONE;
            for t in ts {
                let one = tribe_set(t)?;
                set = Tribes(set.0 | one.0);
            }
            Ok(set)
        }
        (None, None) => Err("needs tribe or tribes".into()),
        (Some(_), Some(_)) => Err("has both tribe and tribes".into()),
    }
}

fn parse_event(s: &str) -> Result<EventKind, String> {
    match s {
        "spells_cast" => Ok(EventKind::SpellsCast),
        "all_spells_cast" => Ok(EventKind::AllSpellsCast),
        "gold_spent" => Ok(EventKind::GoldSpent),
        "hero_damage" => Ok(EventKind::HeroDamage),
        "minions_sold" => Ok(EventKind::MinionsSold),
        "cards_bought" => Ok(EventKind::CardsBought),
        "elementals_played" => Ok(EventKind::ElementalsPlayed),
        "discarded" => Ok(EventKind::Discarded),
        // "Whenever you get a Pirate" and the like: a card of the tribe
        // entering the hand.
        other => match other.strip_prefix("got_") {
            Some(t) => Ok(EventKind::CardGot(tribe(t)?)),
            None => Err(format!("unknown event {other:?}")),
        },
    }
}

/// One keyword's bit, or several written with commas ("Give a Dog a Bone":
/// `divine_shield,windfury`).
fn keyword_bit(s: &str) -> Result<u8, String> {
    let mut mask = 0u8;
    for part in s.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let bit = Keywords::ALL
            .iter()
            .find(|(_, n)| *n == part)
            .map(|(b, _)| *b)
            .ok_or_else(|| format!("unknown keyword {part:?}"))?;
        mask |= bit;
    }
    Ok(mask)
}

fn parse_discover(s: &str) -> Result<DiscoverKind, String> {
    match s {
        "tier_1_minion" => Ok(DiscoverKind::Tier1Minion),
        "tier_minion" => Ok(DiscoverKind::MyTierMinion),
        "tier_spell" => Ok(DiscoverKind::MyTierSpell),
        "deathrattle" => Ok(DiscoverKind::DeathrattleMinion),
        "battlecry" => Ok(DiscoverKind::BattlecryMinion),
        "tier_7_minion" => Ok(DiscoverKind::Tier7Minion),
        "choose_one_minion" => Ok(DiscoverKind::ChooseOneMinion),
        "tier_above_minion" => Ok(DiscoverKind::HigherTierMinion),
        "last_opponent_minion" => Ok(DiscoverKind::LastOpponentMinion),
        other => Err(format!("unknown discover {other:?}")),
    }
}

fn parse_spell_effect(
    e: &RawSpellEffect,
) -> Result<(Option<SpellTarget>, Option<SpellEffect>), String> {
    let kw = match &e.keyword {
        Some(k) => keyword_bit(k)?,
        None => 0,
    };
    let times = e.times.unwrap_or(1);
    if let Some([attack, health]) = e.buff_deity {
        return Ok((None, Some(SpellEffect::BuffDeity { attack, health })));
    }
    if let Some([attack, health]) = e.mighty_dragonbreath {
        return Ok((None, Some(SpellEffect::MightyDragonbreath { attack, health })));
    }
    match e.target.as_deref() {
        Some("chosen") => {
            if e.half_of_highest {
                let t = e.tribe.as_deref().ok_or("half_of_highest needs a tribe")?;
                return Ok((Some(SpellTarget::HalfOfHighestTavern { tribe: tribe(t)? }), None));
            }
            if let Some(attack) = e.butchering {
                return Ok((Some(SpellTarget::Butchering { attack }), None));
            }
            if let Some(gems) = e.confiscate {
                return Ok((Some(SpellTarget::Confiscation { gems }), None));
            }
            if let Some(k) = &e.gem_keyword {
                return Ok((Some(SpellTarget::GemKeyword { keyword: keyword_bit(k)? }), None));
            }
            if e.trigger_deathrattle {
                return Ok((Some(SpellTarget::TriggerDeathrattle), None));
            }
            if let Some(per) = e.per_played_this_turn {
                let [attack, health] = e.buff.ok_or("per_played_this_turn needs buff")?;
                let t = e.tribe.as_deref().ok_or("per_played_this_turn needs a tribe")?;
                return Ok((
                    Some(SpellTarget::BuffPerTribePlayed {
                        tribe: tribe(t)?,
                        attack,
                        health,
                        per: (per[0], per[1]),
                    }),
                    None,
                ));
            }
            if let Some(how) = &e.consume {
                let pick = match how.as_str() {
                    "chosen" => ConsumePick::Chosen,
                    "random" => ConsumePick::Random,
                    "highest_health" => ConsumePick::HighestHealth,
                    other => return Err(format!("unknown spell consume {other:?}")),
                };
                let tribe = e.tribe.as_deref().map(tribe).transpose()?;
                return Ok((
                    Some(SpellTarget::Consume {
                        tribe,
                        pick,
                        keywords: e.with_keywords,
                        count: e.count.unwrap_or(1),
                    }),
                    None,
                ));
            }
            if e.random_keyword && e.buff.is_none() {
                // "Give a Murloc a random Bonus Keyword" (Vibrant Bubble).
                return Ok((
                    Some(SpellTarget::Buff {
                        attack: 0,
                        health: 0,
                        keyword: 0,
                        times: 1,
                        if_win_next_turn_buff: None,
                        tribe: e.tribe.as_deref().map(tribe).transpose()?,
                        random_keyword: true,
                    }),
                    None,
                ));
            }
            // "Give a minion +2 Attack, then double its Attack" (I'm Still
            // Just a Rat in a Cage, The Bouncer): before the plain buff, which
            // would swallow its `buff`.
            if let Some(how) = &e.then_double {
                let stat = match how.as_str() {
                    "attack" => DoubleStat::Attack,
                    "health" => DoubleStat::Health,
                    other => return Err(format!("unknown then_double {other:?}")),
                };
                let [attack, health] = e.buff.unwrap_or([0, 0]);
                return Ok((
                    Some(SpellTarget::BuffThenDouble { attack, health, keyword: kw, stat }),
                    None,
                ));
            }
            if let Some([attack, health]) = e.buff {
                if e.per_bonus_keyword {
                    return Ok((Some(SpellTarget::BuffPerBonusKeyword { attack, health }), None));
                }
                let if_win = e.if_win_next_turn_buff.map(|[a, h]| (a, h));
                return Ok((
                    Some(SpellTarget::Buff {
                        attack,
                        health,
                        keyword: kw,
                        times,
                        if_win_next_turn_buff: if_win,
                        tribe: e.tribe.as_deref().map(tribe).transpose()?,
                        random_keyword: e.random_keyword,
                    }),
                    None,
                ));
            }
            if e.same_tribe_minion == Some(true) {
                return Ok((Some(SpellTarget::SameTribeMinion), None));
            }
            if let Some([attack, health]) = e.set_stats {
                return Ok((Some(SpellTarget::SetStats { attack, health }), None));
            }
            if let Some(max_tier) = e.make_golden_max_tier {
                return Ok((Some(SpellTarget::MakeGolden { max_tier }), None));
            }
            if let Some(max_tier) = e.plain_copy {
                return Ok((Some(SpellTarget::PlainCopy { max_tier }), None));
            }
            if let Some([attack, health]) = e.tricky_trousers {
                return Ok((Some(SpellTarget::TrickyTrousers { attack, health }), None));
            }
            if e.robust_evolution {
                return Ok((Some(SpellTarget::RobustEvolution), None));
            }
            if let Some([attack, health]) = e.tavern_buff_type {
                return Ok((Some(SpellTarget::TavernBuffType { attack, health }), None));
            }
            if e.refresh_with_type {
                return Ok((Some(SpellTarget::RefreshWithType), None));
            }
            if e.devourer {
                return Ok((Some(SpellTarget::Devourer), None));
            }
            if e.destroy_get_random {
                let t = e.tribe.as_deref().ok_or("destroy_get_random needs a tribe")?;
                return Ok((
                    Some(SpellTarget::DestroyGetRandom {
                        tribe: tribe(t)?,
                        count: e.count.unwrap_or(1),
                    }),
                    None,
                ));
            }
            if e.transform_higher {
                return Ok((Some(SpellTarget::TransformHigher), None));
            }
            if e.watch_stats {
                return Ok((Some(SpellTarget::WatchStats), None));
            }
            if let Some([attack, health]) = e.return_to_hand {
                return Ok((Some(SpellTarget::ReturnToHand { attack, health }), None));
            }
            if e.make_golden_return {
                return Ok((Some(SpellTarget::MakeGoldenReturn), None));
            }
            if let Some([attack, health]) = e.grant_end_of_turn {
                return Ok((Some(SpellTarget::GrantEndOfTurn { attack, health }), None));
            }
            Err("invalid chosen spell effect".into())
        }
        Some("chosen_tribe") => {
            let [attack, health] = e.buff.ok_or("chosen_tribe needs buff")?;
            Ok((Some(SpellTarget::BuffSameTribe { attack, health }), None))
        }
        Some("shop") => {
            let [attack, health] = e.buff.ok_or("shop target needs buff")?;
            Ok((None, Some(SpellEffect::BuffShop { attack, health })))
        }
        Some("board") => {
            let [attack, health] = e.buff.ok_or("board target needs buff")?;
            Ok((
                None,
                Some(SpellEffect::BuffBoard {
                    attack,
                    health,
                    times,
                    golden_extra: e.golden_extra,
                }),
            ))
        }
        Some("random_board") => {
            let [attack, health] = e.buff.ok_or("random_board needs buff")?;
            let count = e.count.ok_or("random_board needs count")?;
            Ok((None, Some(SpellEffect::BuffRandomBoard { count, attack, health })))
        }
        Some(other) => Err(format!("unknown spell target {other:?}")),
        None => {
            let eff = if let Some(g) = e.gold {
                SpellEffect::GainGold(g)
            } else if let Some(g) = e.gold_next_turn {
                SpellEffect::GoldNextTurn(g)
            } else if let Some(g) = e.max_gold {
                SpellEffect::MaxGold(g)
            } else if let Some(r) = e.free_rolls {
                SpellEffect::FreeRolls(r)
            } else if let Some([win, tie]) = e.overconfidence {
                SpellEffect::Overconfidence { win, tie }
            } else if let Some([attack, health]) = e.next_turn_board_buff {
                SpellEffect::NextTurnBoardBuff { attack, health, times }
            } else if let Some(t) = e.random_tier_minion {
                SpellEffect::RandomTierMinion(t)
            } else if let Some(d) = &e.discover {
                SpellEffect::Discover(parse_discover(d)?)
            } else if e.golden_touch == Some(true) {
                SpellEffect::GoldenTouch
            } else if e.cloning_conch {
                SpellEffect::CloningConch
            } else if let Some([attack, health]) = e.tavern_buff_permanent {
                SpellEffect::BuffTavernPermanent { attack, health }
            } else if let Some([attack, health]) = e.misplaced_tea_set {
                SpellEffect::MisplacedTeaSet { attack, health }
            } else if e.planar_telescope {
                SpellEffect::PlanarTelescope
            } else if e.steal_random_tavern {
                SpellEffect::StealRandomTavern
            } else if e.discover_locked_minion {
                SpellEffect::DiscoverLockedMinion
            } else if e.tomb_turning {
                SpellEffect::TombTurning
            } else if let Some(n) = e.set_armor {
                SpellEffect::SetArmor(n)
            } else if e.discover_hero_power {
                SpellEffect::DiscoverHeroPower
            } else if e.saloon_finest {
                SpellEffect::SaloonFinest
            } else if let Some(price) = e.refresh_with_battlecries {
                SpellEffect::RefreshWithBattlecries(price)
            } else if let Some([attack, health]) = e.refresh_buff {
                SpellEffect::RefreshBuff { attack, health }
            } else if let Some(n) = e.lockbox {
                SpellEffect::Lockbox(n)
            } else if let Some([attack, health]) = e.buff_leftmost {
                SpellEffect::BuffLeftmost { attack, health }
            } else if let Some([attack, health]) = e.menagerie_tableware {
                SpellEffect::MenagerieTableware { attack, health }
            } else if e.friendly_bounty {
                SpellEffect::FriendlyBounty
            } else if let Some(n) = e.get_gems {
                SpellEffect::GetGems(n)
            } else if let Some([attack, health]) = e.gem_bonus {
                SpellEffect::GemBonus { attack, health }
            } else if let Some([attack, health]) = e.spell_bonus {
                SpellEffect::SpellBonus { attack, health }
            } else if let Some(n) = e.gems_on_board {
                SpellEffect::GemsOnBoard(n)
            } else if let Some([attack, health]) = e.barrage {
                SpellEffect::Barrage { attack, health }
            } else if let Some(name) = &e.random_tribe_minion {
                SpellEffect::RandomTribeMinion { tribe: tribe(name)?, count: e.count.unwrap_or(1) }
            } else if let Some(tier) = e.discover_prize {
                SpellEffect::DiscoverPrize(tier)
            } else if let Some(tier) = e.discover_prizes {
                SpellEffect::DiscoverPrizesBelow(tier)
            } else if let Some(n) = e.random_prize {
                SpellEffect::RandomPrize(n)
            } else if e.fill_hand_spells {
                SpellEffect::FillHandSpells
            } else if e.evolving_tavern {
                SpellEffect::EvolvingTavern
            } else if e.steal_all_tavern {
                SpellEffect::StealAllTavern
            } else if let Some([attack, health]) = e.extra_shop_minion {
                SpellEffect::ExtraShopMinion { attack, health }
            } else if let Some([now, per_turn]) = e.free_rolls_per_turn {
                SpellEffect::FreeRollsPerTurn { now, per_turn }
            } else if let Some(n) = e.random_spell_at_least {
                SpellEffect::RandomSpellAtLeast(n)
            } else if let Some(n) = e.spell_discount_turn {
                SpellEffect::SpellDiscountThisTurn(n)
            } else if let Some(n) = e.minion_price {
                SpellEffect::MinionPrice(n)
            } else if let Some(n) = e.battlecry_extra_turn {
                SpellEffect::BattlecryExtraThisTurn(n)
            } else if e.triple_reward {
                SpellEffect::TripleReward
            } else if let Some(b) = e.boon_of_beetles {
                SpellEffect::BoonOfBeetles(b)
            } else if let Some(soc) = &e.start_of_combat {
                match soc.as_str() {
                    "double_leftmost_attack" => SpellEffect::BroodOfNozdormu,
                    "enemy_health_to_1" => SpellEffect::UpperHand,
                    "enemy_stats_to_leftmost" => SpellEffect::SharingIsCaring,
                    other => return Err(format!("unknown spell start_of_combat {other:?}")),
                }
            } else {
                return Err("invalid spell effect".into());
            };
            Ok((None, Some(eff)))
        }
    }
}

impl CardDb {
    pub fn from_toml(s: &str) -> Result<CardDb, String> {
        let raw: RawFile = toml::from_str(s).map_err(|e| format!("cards.toml: {e}"))?;
        let pool_cards = raw.minion.len();
        let all_minions: Vec<(RawCard, bool)> = raw
            .minion
            .into_iter()
            .map(|c| (c, false))
            .chain(raw.token.into_iter().map(|c| (c, true)))
            .collect();
        let total_cards = all_minions.len() + raw.spell.len();
        if total_cards > CardId::MAX as usize {
            return Err("cards.toml: too many cards".into());
        }
        if pool_cards > crate::game::MAX_POOL_CARDS {
            return Err(format!(
                "cards.toml: {} pool minions, the limit is {}",
                pool_cards,
                crate::game::MAX_POOL_CARDS
            ));
        }
        let mut names: Vec<String> = all_minions.iter().map(|(c, _)| c.name.clone()).collect();
        names.extend(raw.spell.iter().map(|sp| sp.name.clone()));
        for (i, n) in names.iter().enumerate() {
            if names[..i].contains(n) {
                return Err(format!("cards.toml: {n} appears twice"));
            }
        }
        let token_id = |name: &str| -> Result<CardId, String> {
            let id = names
                .iter()
                .position(|n| n == name)
                .ok_or_else(|| format!("unknown card {name:?}"))?;
            if id >= all_minions.len() || !all_minions[id].1 {
                return Err(format!("{name} is not a token"));
            }
            Ok(id as CardId)
        };
        let generated_id = |name: &str| -> Result<CardId, String> {
            let id = names
                .iter()
                .position(|n| n == name)
                .ok_or_else(|| format!("unknown card {name:?}"))?;
            if id < pool_cards {
                return Err(format!("{name} is a pool minion, not a token or spell"));
            }
            Ok(id as CardId)
        };

        // any minion, pool card or token, to summon
        let minion_id = |name: &str| -> Result<CardId, String> {
            let id = names
                .iter()
                .position(|n| n == name)
                .ok_or_else(|| format!("unknown card {name:?}"))?;
            if id >= all_minions.len() {
                return Err(format!("{name} is a spell, not a minion"));
            }
            Ok(id as CardId)
        };

        // a Tavern spell, for "cast <spell>"
        let spell_id = |name: &str| -> Result<CardId, String> {
            let id = names
                .iter()
                .position(|n| n == name)
                .ok_or_else(|| format!("unknown card {name:?}"))?;
            if id < all_minions.len() {
                return Err(format!("{name} is not a spell"));
            }
            Ok(id as CardId)
        };

        let mut cards = Vec::with_capacity(total_cards);
        let mut volumizers = Vec::new();
        let mut spells = Vec::with_capacity(raw.spell.len());
        let mut first_volumizer_slot: Option<CardId> = None;
        let mut beetle_token: Option<CardId> = None;
        let mut cobra_token: Option<CardId> = None;
        let mut chromadrakes = Vec::new();
        let mut taught_token: Option<CardId> = None;
        let mut fodder_token: Option<CardId> = None;

        let parse_effect = |b: &RawEffect| -> Result<Effect, String> {
            let t = b.tribe.as_deref().map(tribe).transpose()?;
            Ok(if let Some([attack, health]) = b.buff_deity {
                let per = b.per_spell.unwrap_or([0, 0]);
                Effect::BuffDeity { attack, health, per_spell: (per[0], per[1]) }
            } else if let Some([attack, health]) = b.buff_self_and_deity {
                Effect::BuffSelfAndDeity { attack, health }
            } else if let Some([attack, health]) = b.buff_deity_and_improve {
                Effect::BuffDeityAndImprove { attack, health }
            } else if let Some([attack, health]) = b.buff_leftmost_per_discard {
                Effect::BuffLeftmostPerDiscard { attack, health }
            } else if b.random_spell_up_to {
                Effect::GetRandomSpellUpTo { cost: b.random_spell_cost, count: b.count.unwrap_or(1) }
            } else if let Some(count) = b.linked_spells {
                Effect::GetLinkedSpells { count }
            } else if let Some(name) = &b.linked_minions {
                Effect::GetLinkedMinions { tribe: tribe(name)?, count: b.count.unwrap_or(2) }
            } else if let Some(count) = b.discard_leftmost_spells {
                let [attack, health] = b.buff.ok_or("discard_leftmost_spells needs buff")?;
                Effect::DiscardLeftmostSpells { count, attack, health }
            } else if let Some(name) = &b.summon_and_buff {
                let [attack, _] = b.buff.ok_or("summon_and_buff needs buff")?;
                Effect::SummonAndBuffAll {
                    card: minion_id(name)?,
                    count: b.count.unwrap_or(1),
                    attack,
                    golden_doubles_count: b.golden_doubles_count,
                }
            } else if let Some(count) = b.summon_dead_aberrations {
                Effect::SummonDeadAberrations { count }
            } else if b.damage_highest_health {
                Effect::DamageHighestHealth
            } else if let Some([attack, health]) = b.undead_bonus {
                Effect::UndeadBonus {
                    attack,
                    health,
                    outside_attack: b.outside_attack.unwrap_or(0),
                }
            } else if let Some(name) = &b.give_reborn_random {
                Effect::GiveRebornRandom { tribe: tribe(name)?, count: b.count.unwrap_or(1) }
            } else if let Some(name) = &b.summon_attacker {
                Effect::SummonAttacker { card: minion_id(name)? }
            } else if b.get_plain_copy {
                Effect::GetPlainCopy
            } else if let Some(name) = &b.get_random_held {
                Effect::GetRandomMinionHeld { tribe: tribe(name)? }
            } else if b.summon_held {
                Effect::SummonHeld
            } else if b.destroy_left {
                Effect::DestroyLeftNeighbor
            } else if b.summon_salvaged {
                Effect::SummonSalvaged
            } else if let Some(name) = &b.destroy_chosen_discover {
                Effect::DestroyChosenDiscover { tribe: tribe(name)?, count: b.count.unwrap_or(1) }
            } else if let Some(name) = &b.destroy_chosen_get_copy {
                Effect::DestroyChosenGetCopy { tribe: tribe(name)?, copies: b.count.unwrap_or(1) }
            } else if let Some(name) = &b.give_reborn_destroy {
                let [attack, health] = b.buff.ok_or("give_reborn_destroy needs buff")?;
                Effect::GiveRebornDestroy { tribe: tribe(name)?, attack, health }
            } else if let Some(n) = b.get_gems {
                Effect::GetGems(n)
            } else if let Some(n) = b.get_gems_overflow {
                Effect::GetGemsCastOverflow(n)
            } else if let Some(n) = b.gem_self {
                Effect::GemSelf { count: n, permanent: b.gem_permanent }
            } else if let Some(n) = b.gem_others {
                Effect::GemOthers(n)
            } else if let Some(n) = b.gem_attacker {
                Effect::GemAttacker(n)
            } else if let Some([attack, health]) = b.gem_bonus {
                Effect::GemBonus { attack, health }
            } else if let Some(n) = b.random_choose_one {
                Effect::GetRandomChooseOne(n)
            } else if let Some(name) = &b.summon_stat_copy {
                Effect::SummonStatCopyAttacker { card: minion_id(name)? }
            } else if b.get_any_tavern_spell {
                Effect::GetAnyTavernSpell
            } else if b.get_most_common_type_minion {
                Effect::GetMostCommonTypeMinion
            } else if let Some([attack, health]) = b.refresh_buff {
                Effect::RefreshBuff { attack, health }
            } else if let Some(n) = b.free_refreshes {
                Effect::FreeRefreshes(n)
            } else if let Some(name) = &b.improving_counter {
                let [attack, health] = b.buff.ok_or("improving_counter needs buff")?;
                Effect::BuffAllImproving { attack, health, counter: counter(name)? }
            } else if b.arm_next_buy {
                Effect::ArmNextBuy
            } else if b.gain_highest_tavern_stats {
                Effect::GainHighestTavernStats
            } else if b.per_improvement {
                let [attack, health] = b.buff.ok_or("per_improvement needs buff")?;
                Effect::BuffOthersPerImprovement { tribe: t, attack, health }
            } else if let Some(per_gold) = b.per_gold_spent {
                let [attack, health] = b.buff.ok_or("per_gold_spent needs buff")?;
                Effect::BuffChosenPerGold {
                    tribe: t,
                    attack,
                    health,
                    per_gold: (per_gold[0], per_gold[1]),
                    in_shop: b.in_shop,
                }
            } else if b.per_golden_played {
                let [attack, health] = b.buff.ok_or("per_golden_played needs buff")?;
                Effect::BuffOthersPerGolden { tribe: t, attack, health }
            } else if let Some([attack, health]) = b.keep_golden {
                Effect::BuffAllKeepGolden { attack, health }
            } else if let Some(attack) = b.adjacent_per_golden {
                Effect::BuffAdjacentPerGolden { attack }
            } else if let Some(max_tier) = b.make_golden_max_tier {
                Effect::MakeGoldenChosen { tribe: t, max_tier, count: b.count.unwrap_or(1) }
            } else if let Some(tier) = b.golden_minion_tier {
                Effect::GetGoldenMinion { tier, count: b.count.unwrap_or(1) }
            } else if let Some(n) = b.lockbox_sooner {
                Effect::GetLockbox { sooner: n }
            } else if b.random_bounty {
                Effect::GetRandomBounty
            } else if let Some([attack, health]) = b.buff {
                let per_keyword = b.per_keyword.map_or((0, 0), |[a, h]| (a, h));
                match b.target.as_deref() {
                    Some("chosen") => Effect::BuffChosen {
                        tribe: t,
                        attack,
                        health,
                        per_keyword,
                        random_keyword: b.random_keyword,
                        in_shop: b.in_shop,
                    },
                    Some("others") => Effect::BuffOthers { tribe: t, attack, health, per_keyword },
                    Some("self") => Effect::BuffSelf { attack, health },
                    Some("all") => Effect::BuffAll { tribe: t, attack, health },
                    Some("any") => Effect::BuffAnyMinion { attack, health },
                    Some("random") => {
                        let improve = b.improve_counter.as_deref().map(counter).transpose()?;
                        Effect::BuffRandomFriendly { tribe: t, attack, health, improve }
                    }
                    other => return Err(format!("battlecry target {other:?}")),
                }
            } else if let Some([attack, health]) = b.buff_killer {
                Effect::BuffKiller { attack, health }
            } else if let Some(name) = &b.summon {
                Effect::Summon {
                    card: minion_id(name)?,
                    count: b.count.unwrap_or(1),
                    golden_doubles_count: b.golden_doubles_count,
                }
            } else if let Some(name) = &b.summon_from_hand {
                Effect::SummonFromHand {
                    tribe: if name == "any" { None } else { Some(tribe(name)?) },
                    count: b.count.unwrap_or(1),
                    by_health: b.by_health,
                    shield: b.shield,
                }
            } else if b.become_copy {
                Effect::BecomeCopyChosen
            } else if b.plain_copy_random_friendly {
                Effect::PlainCopyRandomFriendly
            } else if b.copy_last_discarded {
                Effect::CopyLastDiscarded
            } else if let Some([attack, health]) = b.buff_random_hand {
                Effect::BuffRandomHand { attack, health }
            } else if let Some([attack, health]) = b.buff_leftmost_hand {
                Effect::BuffLeftmostHand { attack, health }
            } else if b.trigger_battlecry {
                Effect::TriggerBattlecry
            } else if let Some(name) = &b.discover_tribe_if_another {
                Effect::DiscoverTribeIfAnother {
                    tribe: tribe(name)?,
                    count: b.count.unwrap_or(1),
                }
            } else if b.cast_taught {
                Effect::CastTaughtSpell
            } else if let Some(n) = b.trigger_adjacent_battlecries {
                Effect::TriggerAdjacentBattlecries(n)
            } else if let Some(name) = &b.random_minion {
                Effect::GetRandomMinion { tribe: tribe(name)? }
            } else if let Some(k) = &b.gain_keyword {
                Effect::GainKeyword { keyword: keyword_bit(k)? }
            } else if let Some(cost) = b.random_spell_cost {
                Effect::GetRandomSpell { cost, count: b.count.unwrap_or(1) }
            } else if let Some(n) = b.summon_random_deathrattle {
                Effect::SummonRandomDeathrattle { count: n }
            } else if b.trigger_deathrattle {
                Effect::TriggerDeathrattle { times: b.count.unwrap_or(1) }
            } else if let Some(name) = &b.replace_with {
                Effect::ReplaceWithFishbait { card: token_id(name)? }
            } else if let Some(name) = &b.refresh_with {
                Effect::RefreshWithFishbait { card: token_id(name)? }
            } else if let Some(n) = b.extra_magnetization {
                Effect::ExtraMagnetization(n)
            } else if b.random_keyword {
                Effect::RandomKeyword
            } else if let Some(how) = &b.consume {
                let pick = match how.as_str() {
                    "chosen" => ConsumePick::Chosen,
                    "random" => ConsumePick::Random,
                    "highest_health" => ConsumePick::HighestHealth,
                    other => return Err(format!("unknown consume {other:?}")),
                };
                let by = if b.consume_by_chosen { ConsumeBy::Chosen(t) } else { ConsumeBy::This };
                Effect::Consume {
                    by,
                    pick,
                    keywords: b.with_keywords,
                    count: b.count.unwrap_or(1),
                }
            } else if let Some([attack, health]) = b.tavern_buff {
                Effect::TavernBuff { tribe: t, attack, health, this_turn: b.this_turn }
            } else if let Some([attack, health]) = b.tavern_buff_random_keyword {
                let mut mask = 0u8;
                for k in b.keywords.as_deref().unwrap_or(&[]) {
                    mask |= keyword_bit(k)?;
                }
                Effect::TavernBuffRandomKeyword { attack, health, mask }
            } else if let Some(n) = b.add_fodder_refreshes {
                Effect::AddFodderRefreshes(n)
            } else if let Some(n) = b.add_fodder_to_next_refresh {
                Effect::AddFodderToNextRefresh(n)
            } else if let Some(n) = b.demons_consume {
                Effect::DemonsConsume { count: n }
            } else if let Some(n) = b.improve_self {
                Effect::ImproveSelf(n)
            } else if let Some(n) = b.discover_demon_tier_damage {
                Effect::DiscoverDemonTierDamage(n)
            } else if let Some(n) = b.damage_hero {
                Effect::DamageHero(n)
            } else if b.rewind {
                Effect::RewindDamage
            } else if b.magnetize_volumizer {
                Effect::MagnetizeVolumizerAndCopy
            } else if let Some([attack, health]) = b.beetle_buff {
                Effect::BeetleBuff { attack, health }
            } else if let Some(g) = b.gold_next_turn {
                Effect::GoldNextTurn(g)
            } else if let Some(g) = b.max_gold {
                Effect::MaxGold(g)
            } else if b.random_volumizer == Some(true) {
                Effect::RandomVolumizer
            } else if let Some(d) = b.spell_discount {
                Effect::SpellDiscount(d)
            } else if let Some([attack, health]) = b.spell_bonus {
                Effect::SpellBonus { attack, health }
            } else if b.random_chromadrake {
                Effect::RandomChromadrake
            } else if let Some(name) = &b.cast {
                Effect::CastSpell { card: spell_id(name)?, times: b.count.unwrap_or(1) }
            } else if b.trigger_rally {
                Effect::TriggerRally
            } else if let Some([attack, health]) = b.buff_attacker {
                Effect::BuffAttacker { attack, health }
            } else if let Some([attack, health]) = b.buff_random_other {
                let keyword = b.keyword.as_deref().map(keyword_bit).transpose()?.unwrap_or(0);
                Effect::BuffRandomOther {
                    tribe: t,
                    attack,
                    health,
                    keyword,
                    count: b.count.unwrap_or(1),
                }
            } else if let Some([attack, health]) = b.set_stats_any {
                Effect::SetStatsAnyMinion { attack, health }
            } else if let Some(n) = b.steal_tavern {
                Effect::StealTavern { count: n }
            } else if let Some(n) = b.discover_spell {
                Effect::DiscoverSpell { count: n }
            } else if b.gain_target_attack {
                Effect::GainTargetAttack
            } else if let Some(keys) = &b.strip_keywords {
                let mut mask = 0u8;
                for k in keys {
                    mask |= keyword_bit(k)?;
                }
                Effect::StripTargetKeywords { mask }
            } else if b.destroy_killer {
                Effect::DestroyKiller
            } else if let Some(n) = b.shield_dragons {
                Effect::ShieldDragons { count: n }
            } else if let Some(tier) = b.random_tier_minion {
                Effect::GetRandomTierMinion { tier }
            } else if let Some([attack, health]) = b.buff_each_type {
                Effect::BuffEachType { attack, health }
            } else if b.damage_target_and_neighbor {
                Effect::DamageTargetAndNeighbor
            } else if b.give_stats_leftmost_hand {
                Effect::GiveOwnStatsToLeftmostHand
            } else if let Some(name) = &b.get {
                let card = generated_id(name)?;
                Effect::GetCard { card, count: b.count.unwrap_or(1) }
            } else {
                return Err("invalid battlecry".into());
            })
        };

        for (idx, (c, token)) in all_minions.iter().enumerate() {
            let id = idx as CardId;
            let err = |e: String| format!("cards.toml, {}: {e}", c.name);
            if !(1..=crate::CARD_MAX_TIER).contains(&c.tier) {
                return Err(err(format!("tier {} is not 1 to {}", c.tier, crate::CARD_MAX_TIER)));
            }
            let mut keywords = Keywords::default();
            for k in &c.keywords {
                keywords.set(keyword_bit(k).map_err(&err)?);
            }
            if c.beetle && *token {
                beetle_token = Some(id);
            }
            if c.secret_cobra && *token {
                if let Some(other) = cobra_token {
                    return Err(err(format!(
                        "secret_cobra is already {}",
                        all_minions[other as usize].0.name
                    )));
                }
                cobra_token = Some(id);
            }
            if c.chromadrake && *token {
                chromadrakes.push(id);
            }
            if c.taught_token && *token {
                taught_token = Some(id);
            }
            if c.fodder && *token {
                fodder_token = Some(id);
            }
            let is_volumizer = c.volumizer_buff.is_some();
            let pool_slot = if is_volumizer {
                volumizers.push(id);
                *first_volumizer_slot.get_or_insert(id)
            } else {
                id
            };
            let volumizer_buff = c.volumizer_buff.map(|[a, h]| (a, h));
            let spell_buff = c.spell_buff.map(|[a, h]| (a, h));

            let battlecry = c.battlecry.as_ref().map(&parse_effect).transpose().map_err(&err)?;

            let activate = match &c.activate {
                None => None,
                Some(a) => {
                    let effect = parse_effect(&a.effect).map_err(&err)?;
                    Some(Activate { cost: a.cost, effect, discard: a.discard })
                }
            };

            let on_event = match &c.on_event {
                None => None,
                Some(e) => {
                    let effect = parse_effect(&e.effect).map_err(&err)?;
                    if effect.needs_choice() {
                        return Err(err("an event trigger can't ask for a choice".into()));
                    }
                    if e.every == 0 {
                        return Err(err("on_event every must be at least 1".into()));
                    }
                    Some(EventTrigger {
                        kind: parse_event(&e.event).map_err(&err)?,
                        every: e.every,
                        effect,
                    })
                }
            };

            let after_battlecry = match &c.after_battlecry {
                None => None,
                Some(e) => {
                    let effect = parse_effect(e).map_err(&err)?;
                    if effect.needs_choice() {
                        return Err(err("an after_battlecry effect can't ask for a choice".into()));
                    }
                    Some(effect)
                }
            };

            let permanent_keep = match c.permanent_keep.as_deref() {
                None => None,
                Some("self") => Some(PermanentKeep::SelfKeep),
                Some("adjacent_dragons") => Some(PermanentKeep::AdjacentDragons),
                Some("all_dragons") => Some(PermanentKeep::AllDragons),
                Some(other) => return Err(err(format!("unknown permanent_keep {other:?}"))),
            };

            let attack_threshold = match &c.attack_threshold {
                None => None,
                Some(t) => Some(AttackThreshold {
                    total: t.total,
                    keyword: keyword_bit(&t.keyword).map_err(&err)?,
                }),
            };

            let mut after_hero_damage = Vec::new();
            for e in &c.after_hero_damage {
                let effect = parse_effect(e).map_err(&err)?;
                if effect.needs_choice() {
                    return Err(err("after_hero_damage can't ask for a choice".into()));
                }
                after_hero_damage.push(effect);
            }

            let after_play_tribe = match &c.after_play_tribe {
                None => None,
                Some(p) => {
                    let mut effects = Vec::new();
                    for e in &p.effects {
                        let effect = parse_effect(e).map_err(&err)?;
                        if effect.needs_choice() {
                            return Err(err("after_play_tribe can't ask for a choice".into()));
                        }
                        effects.push(effect);
                    }
                    Some(AfterPlayTribe {
                        tribe: tribe(&p.tribe).map_err(&err)?,
                        effects,
                    })
                }
            };

            let choose_one = match &c.choose_one {
                None => None,
                Some([o1, o2]) => {
                    let parse_opt = |o: &RawChooseOption| -> Result<ChooseOption, String> {
                        match o {
                            RawChooseOption::Minion(o) => {
                                if let Some([attack, health]) = o.buff {
                                    let kw =
                                        o.keyword.as_deref().ok_or("choose_one buff needs keyword")?;
                                    Ok(ChooseOption::MinionTarget {
                                        tribe: o.tribe.as_deref().map(tribe).transpose()?,
                                        attack,
                                        health,
                                        keyword: keyword_bit(kw)?,
                                        in_shop: o.in_shop,
                                    })
                                } else if let Some([attack, health]) = o.spell_buff {
                                    Ok(ChooseOption::SpellBonus { attack, health })
                                } else {
                                    Err("invalid minion choose_one".into())
                                }
                            }
                            RawChooseOption::Spell(o) => {
                                // "Get a Gem Day" as an option: the same
                                // generated-card form a spell's own effect
                                // uses, parsed here because it needs the
                                // card ids.
                                if let Some(name) = &o.get {
                                    let card = generated_id(name)?;
                                    return Ok(ChooseOption::SpellEffect(SpellEffect::GetCard {
                                        card,
                                        count: o.count.unwrap_or(1),
                                    }));
                                }
                                if let Some(name) = &o.cast {
                                    let card = spell_id(name)?;
                                    return Ok(ChooseOption::SpellEffect(SpellEffect::CastSpell {
                                        card,
                                        times: o.count.unwrap_or(1),
                                    }));
                                }
                                match parse_spell_effect(o)? {
                                    (Some(st), None) => Ok(ChooseOption::SpellTarget(st)),
                                    (None, Some(se)) => Ok(ChooseOption::SpellEffect(se)),
                                    _ => Err("invalid spell choose_one".into()),
                                }
                            }
                        }
                    };
                    Some([parse_opt(o1).map_err(&err)?, parse_opt(o2).map_err(&err)?])
                }
            };

            let deathrattle = match &c.deathrattle {
                None => None,
                Some(d) => Some(if let Some(name) = &d.summon {
                    let card = token_id(name).map_err(&err)?;
                    Deathrattle::Summon {
                        card,
                        count: d.count.unwrap_or(1),
                        golden_doubles_count: d.golden_doubles_count,
                        beetle_buff: d.beetle_buff.map(|[a, h]| (a, h)),
                    }
                } else if let Some(name) = &d.summon_by_attack {
                    let card = token_id(name).map_err(&err)?;
                    Deathrattle::SummonByAttack { card }
                } else if let Some([attack, health]) = d.combat_beast_buff {
                    Deathrattle::CombatBeastBuff { attack, health }
                } else if d.random_volumizer == Some(true) {
                    Deathrattle::RandomVolumizer
                } else if let Some(name) = &d.get {
                    let card = generated_id(name).map_err(&err)?;
                    Deathrattle::GetCard { card, count: d.count.unwrap_or(1) }
                } else if let Some(name) = &d.summon_shaped {
                    let card = token_id(name).map_err(&err)?;
                    Deathrattle::SummonShaped { card }
                } else if d.give_own_attack {
                    Deathrattle::GiveOwnAttack
                } else if d.give_own_health {
                    Deathrattle::GiveOwnHealth
                } else if let Some([attack, health]) = d.tavern_buff {
                    Deathrattle::TavernBuff {
                        attack,
                        health,
                        per_improvement: d.per_improvement.map_or((0, 0), |[a, h]| (a, h)),
                    }
                } else if let Some(e) = &d.effect {
                    let effect = parse_effect(e).map_err(&err)?;
                    if !effect.runs_in_fights() {
                        return Err(err("this deathrattle effect can't run in a fight".into()));
                    }
                    Deathrattle::Effect(effect)
                } else {
                    return Err(err("invalid deathrattle".into()));
                }),
            };

            let start_of_turn = c.start_of_turn.as_ref().map(|s| StartOfTurn::GainGold(s.gold));

            let end_of_turn = match &c.end_of_turn {
                None => None,
                Some(e) => Some(if let Some([attack, health]) = e.buff_self {
                    EndOfTurn::BuffSelf { attack, health }
                } else if e.resourceful_robot == Some(true) {
                    EndOfTurn::ResourcefulRobot
                } else if let Some([attack, health]) = e.buff_per_magnetization {
                    EndOfTurn::BuffPerMagnetization { attack, health }
                } else if e.improve_tier == Some(true) {
                    EndOfTurn::ImproveTier
                } else if let Some(name) = &e.get {
                    let card = generated_id(name).map_err(&err)?;
                    EndOfTurn::GetCard { card, count: e.count.unwrap_or(1) }
                } else if e.copy_last_spell == Some(true) {
                    EndOfTurn::CopyLastSpell
                } else if e.trigger_own_battlecry {
                    EndOfTurn::TriggerOwnBattlecry
                } else if e.magnetize_random_mech {
                    EndOfTurn::MagnetizeRandomMech
                } else if e.gift_buff_self {
                    EndOfTurn::GiftBuffSelf
                } else if let Some(eff) = &e.effect {
                    let effect = parse_effect(eff).map_err(&err)?;
                    if effect.needs_choice() {
                        return Err(err("an end of turn effect can't ask for a choice".into()));
                    }
                    EndOfTurn::Effect(effect)
                } else {
                    return Err(err("invalid end_of_turn".into()));
                }),
            };

            let start_of_combat = match &c.start_of_combat {
                None => None,
                Some(s) => Some(if let Some([attack, health]) = s.combat_beast_buff {
                    StartOfCombat::CombatBeastBuff { attack, health }
                } else if let Some(dmg) = s.damage_all_others {
                    StartOfCombat::DamageAllOthers(dmg)
                } else if s.attack_immediately {
                    StartOfCombat::AttackImmediately
                } else if let Some(h) = &s.hopebringer {
                    StartOfCombat::Hopebringer {
                        attack: h.buff[0],
                        health: h.buff[1],
                        improve: (h.improve[0], h.improve[1]),
                    }
                } else if s.flighty_scout {
                    StartOfCombat::FlightyScout
                } else if s.gain_hand_attack {
                    StartOfCombat::GainHighestHandAttack
                } else if s.gain_hand_stats {
                    StartOfCombat::GainHandStats
                } else if s.double_health {
                    StartOfCombat::DoubleHealth
                } else if s.double_attack {
                    StartOfCombat::DoubleAttack
                } else if s.triple_stats {
                    StartOfCombat::TripleStats
                } else if s.gain_ends {
                    StartOfCombat::GainEndsStats
                } else if s.gain_left_attack {
                    StartOfCombat::GainLeftAttack
                } else if s.gain_deity_stats {
                    StartOfCombat::GainDeityStats
                } else if s.trigger_own_deathrattles {
                    StartOfCombat::TriggerOwnDeathrattles
                } else if s.summon_own_copy {
                    StartOfCombat::SummonOwnCopy
                } else if let Some(eff) = &s.effect {
                    let effect = parse_effect(eff).map_err(&err)?;
                    if !effect.runs_in_fights() {
                        return Err(err("this start_of_combat effect can't run in a fight".into()));
                    }
                    StartOfCombat::Effect(effect)
                } else {
                    return Err(err("invalid start_of_combat".into()));
                }),
            };

            let fight_effect = |e: &RawEffect, what: &str| -> Result<Effect, String> {
                let effect = parse_effect(e).map_err(&err)?;
                if !effect.runs_in_fights() {
                    return Err(err(format!("this {what} effect can't run in a fight")));
                }
                Ok(effect)
            };

            let rally = c.rally.as_ref().map(|e| fight_effect(e, "rally")).transpose()?;

            let attack_trigger = |t: &RawAttackTrigger, what: &str| -> Result<AttackTrigger, String> {
                Ok(AttackTrigger {
                    filter: AttackFilter {
                        tribe: t.attacker_tribe.as_deref().map(tribe).transpose().map_err(&err)?,
                        rally: t.attacker_rally,
                        exclude_self: t.attacker_another,
                    },
                    effect: fight_effect(&t.effect, what)?,
                })
            };
            let whenever_friendly_attacks = c
                .whenever_friendly_attack
                .as_ref()
                .map(|t| attack_trigger(t, "attack trigger"))
                .transpose()?;
            let after_friendly_attacks = c
                .after_friendly_attack
                .as_ref()
                .map(|t| attack_trigger(t, "attack trigger"))
                .transpose()?;
            let avenge = match &c.avenge {
                None => None,
                Some(a) if a.every == 0 => return Err(err("avenge every must be at least 1".into())),
                Some(a) => Some(Avenge { every: a.every, effect: fight_effect(&a.effect, "avenge")? }),
            };
            let damage_milestone = match &c.damage_milestone {
                None => None,
                Some(m) => Some(DamageMilestone {
                    total: m.total,
                    effect: fight_effect(&m.effect, "damage milestone")?,
                }),
            };

            let on_summon_beast = match &c.on_summon_beast {
                None => None,
                Some(s) => Some(if let Some([attack, health]) = s.buff {
                    OnSummonBeast::Buff {
                        attack,
                        health,
                        improve_permanently: s.improve_permanently,
                    }
                } else if s.multiply_attack_in_combat == Some(true) {
                    OnSummonBeast::MultiplyAttackInCombat
                } else {
                    return Err(err("invalid on_summon_beast".into()));
                }),
            };

            let on_play_mech = match &c.on_play_mech {
                None => None,
                Some(p) => {
                    let card = token_id(&p.magnetize_satellite).map_err(&err)?;
                    Some(OnPlayMech::MagnetizeSatellite { card, attack: p.stats[0], health: p.stats[1] })
                }
            };

            let on_play_or_magnetize_mech = c
                .on_play_or_magnetize_mech
                .as_ref()
                .map(|b| OnPlayOrMagnetizeMech::Buff { attack: b.buff[0], health: b.buff[1] });

            let on_cast_spell_on_mech = match &c.on_cast_spell_on_mech {
                None => None,
                Some(g) => {
                    let card = token_id(&g.magnetize_satellite).map_err(&err)?;
                    Some(OnCastSpellOnMech::MagnetizeSatellite {
                        card,
                        attack: g.stats[0],
                        health: g.stats[1],
                    })
                }
            };

            let on_cast_spell_on_self = match &c.on_cast_spell_on_self {
                None => None,
                Some(e) => {
                    let effect = parse_effect(e).map_err(&err)?;
                    if effect.needs_choice() {
                        return Err(err("an on_cast_spell_on_self effect can't ask for a choice".into()));
                    }
                    Some(effect)
                }
            };

            let on_cast_spell_on_tribe = match &c.on_cast_spell_on_tribe {
                None => None,
                Some(t) => Some((
                    tribe(&t.tribe).map_err(&err)?,
                    Effect::BuffTribeBoardAndHand {
                        tribe: tribe(&t.tribe).map_err(&err)?,
                        attack: t.buff[0],
                        health: t.buff[1],
                    },
                )),
            };

            let when_damaged = match &c.when_damaged {
                None => None,
                Some(e) => {
                    let effect = parse_effect(e).map_err(&err)?;
                    if !effect.runs_in_fights() {
                        return Err(err("this when_damaged effect can't run in a fight".into()));
                    }
                    Some(effect)
                }
            };

            let while_in_hand = match &c.while_in_hand {
                None => None,
                Some(w) => Some(WhileInHand {
                    tribe: tribe(&w.after_play).map_err(&err)?,
                    attack: w.buff[0],
                    health: w.buff[1],
                }),
            };

            let on_cast_spell = match &c.on_cast_spell {
                None => None,
                Some(cs) => Some(if let Some([attack, health]) = cs.buff_self_and_deity {
                    OnCastSpell::BuffSelfAndDeity { attack, health }
                } else if let Some([attack, health]) = cs.buff_all_and_deity {
                    OnCastSpell::BuffAllAndDeity { attack, health }
                } else if let Some([attack, health]) = cs.buff_divine_shield {
                    OnCastSpell::BuffDivineShield { attack, health }
                } else if let Some([attack, health]) = cs.buff_all {
                    OnCastSpell::BuffAll { attack, health }
                } else if let Some([attack, health]) = cs.undead_bonus {
                    OnCastSpell::UndeadBonus { attack, health }
                } else {
                    return Err(err("invalid on_cast_spell".into()));
                }),
            };

            let on_sell = match &c.on_sell {
                None => None,
                Some(s) => Some(if s.discover_tier7_if_golden {
                    OnSell::DiscoverTier7IfGolden
                } else if let Some(name) = &s.get {
                    let card = generated_id(name).map_err(&err)?;
                    OnSell::GetCard { card, count: s.count.unwrap_or(1) }
                } else if s.discover_by_tier == Some(true) {
                    OnSell::DiscoverByTier
                } else if let Some(g) = s.gold_if_lost {
                    OnSell::GoldIfLost(g)
                } else if let Some(e) = &s.effect {
                    OnSell::Effect(parse_effect(e).map_err(&err)?)
                } else if let Some(t) = &s.gain_sold_stats {
                    let tribe = if t.is_empty() { None } else { Some(tribe(t)?) };
                    OnSell::GainSoldStats { tribe }
                } else {
                    return Err(err("invalid on_sell".into()));
                }),
            };

            let aura = match c.aura.as_deref() {
                None => None,
                Some("brann") => Some(Aura::Brann),
                Some("titus") => Some(Aura::Titus),
                Some("drakkari") => Some(Aura::Drakkari),
                Some("balinda") => Some(Aura::Balinda),
                Some("taunt_while_shielded") => Some(Aura::TauntWhileShielded),
                Some("holy_vanguard") => Some(Aura::HolyVanguard),
                Some("polarizing_beatboxer") => Some(Aura::PolarizingBeatboxer),
                Some("proud_privateer") => Some(Aura::ProudPrivateer),
                Some("hot_air_surveyor") => Some(Aura::HotAirSurveyor),
                Some(other) => return Err(err(format!("unknown aura {other:?}"))),
            };

            let after_discover = match &c.after_discover {
                None => None,
                Some(e) => {
                    let effect = parse_effect(e).map_err(&err)?;
                    if effect.needs_choice() {
                        return Err(err("an after_discover effect can't ask for a choice".into()));
                    }
                    Some(effect)
                }
            };

            let on_reborn = match &c.on_reborn {
                None => None,
                Some(r) => Some(if let Some([attack, health]) = r.banshee {
                    OnReborn::Banshee { attack, health }
                } else if r.phantom {
                    OnReborn::Phantom
                } else {
                    return Err(err("invalid on_reborn".into()));
                }),
            };

            let magnetize_tribes = if c.magnetize_to_tribes.is_empty() {
                let mut t = Tribes::NONE.with(Tribe::Mech);
                if c.magnetize_to_undead {
                    t = t.with(Tribe::Undead);
                }
                t
            } else {
                let mut t = Tribes::NONE;
                for n in &c.magnetize_to_tribes {
                    t = t.with(tribe(n).map_err(&err)?);
                }
                t
            };

            cards.push(Card {
                name: c.name.clone(),
                tier: c.tier,
                tribes: card_tribes(&c.tribe, &c.tribes).map_err(&err)?,
                lobby: c.lobby.as_deref().map(tribe).transpose().map_err(&err)?,
                attack: c.stats[0],
                health: c.stats[1],
                golden_stats: c.golden_stats.map(|[a, h]| (a, h)),
                keywords,
                prize: c.prize,
                amalgam: c.amalgam,
                amalgam_gain_copy: c.amalgam_gain_copy.unwrap_or(0),
                magnetize_tribes,
                mecha_demon: c.mecha_demon,
                buddy: c.buddy,
                token: *token,
                spell: false,
                pay_health: false,
                cost: 0,
                is_beetle: c.beetle,
                is_volumizer,
                pool_slot,
                volumizer_buff,
                spell_buff,
                battlecry,
                choose_one,
                deathrattle,
                start_of_turn,
                end_of_turn,
                start_of_combat,
                rally,
                on_summon_beast,
                on_play_mech,
                on_play_or_magnetize_mech,
                on_cast_spell_on_mech,
                on_cast_spell,
                on_cast_spell_on_self,
                on_cast_spell_on_tribe,
                when_damaged,
                while_in_hand,
                teaches_spell: c.teaches_spell,
                no_triple: c.no_triple,
                taught_token: c.taught_token,
                whenever_friendly_attacks,
                after_friendly_attacks,
                avenge,
                damage_milestone,
                buff_per_deathrattle: c.buff_per_deathrattle.map(|[a, h]| (a, h)),
                fixed_stats: c.fixed_stats,
                activate,
                on_event,
                after_battlecry,
                permanent_keep,
                attack_threshold,
                after_hero_damage,
                after_play_tribe,
                after_demon_damage: c.after_demon_damage.map(|[a, h]| (a, h)),
                is_fodder: c.fodder,
                health_refreshes: c.health_refreshes.unwrap_or(0),
                on_sell,
                aura,
                after_summon_give_stats: c
                    .after_summon_give_stats
                    .as_ref()
                    .map(|a| AfterSummonGiveStats { times: a.times }),
                spell_target: None,
                spell_effect: None,
                always_golden: c.always_golden,
                cleave: c.cleave,
                buff_per_golden_played: c.buff_per_golden_played.map(|[a, h]| (a, h)),
                buff_per_elemental_played: c.buff_per_elemental_played.map(|[a, h]| (a, h)),
                buff_per_magnetize: c.buff_per_magnetize.map(|[a, h]| (a, h)),
                on_kill: match c.on_kill.as_deref() {
                    None => None,
                    Some("max_stats_to_hand") => Some(KillReward::MaxStatsToHand),
                    Some(other) => return Err(err(format!("unknown on_kill {other:?}"))),
                },
                gains_friendly_deathrattles: c.gains_friendly_deathrattles,
                after_discover,
                is_bounty: false,
                overkill: c.overkill,
                triple_wildcard: c
                    .triple_wildcard
                    .as_deref()
                    .map(tribe)
                    .transpose()
                    .map_err(&err)?,
                play_shield: c.play_shield.as_deref().map(tribe).transpose().map_err(&err)?,
                after_buy: c.after_buy.as_ref().map(|a| AfterBuy {
                    attack: a.buff[0],
                    health: a.buff[1],
                    multiply: a.multiply,
                    golden_multiply: a.golden_multiply,
                }),
                buff_per_kind_death: c.buff_per_kind_death.map(|[a, h]| (a, h)),
                on_reborn,
                deity: c.deity.as_deref().map(|s| match s {
                    "cthun" => Ok(DeityKind::Cthun),
                    "yshaarj" => Ok(DeityKind::Yshaarj),
                    _ => Err(format!("unknown deity {s:?}")),
                }).transpose().map_err(&err)?,
                on_card_added: c.on_card_added.as_ref().map(|b| (b.buff[0], b.buff[1])),
                on_discard: None,
                tavern_spell: false,
                blood_gem: false,
                spellcraft: false,
                on_gem_played_on_self: c.on_gem_played_on_self,
                choose_one_cover: c.choose_one_cover.unwrap_or(0),
                after_choose_one_play: c.after_choose_one_play,
                exclude_heroes: Vec::new(),
                on_card_play: c.on_card_play.map(|[a, h]| (a, h)),
                buff_per_battlecry: c.buff_per_battlecry.map(|[a, h]| (a, h)),
                buff_per_tavern_spell: c.buff_per_tavern_spell.map(|[a, h]| (a, h)),
                buff_per_discard: c.buff_per_discard.map(|[a, h]| (a, h)),
                acquire_copies: c.acquire_copies,
                activate_twice: c.activate_twice,
                shield_hits: c.shield_hits,
                golden_shield_hits: c.golden_shield_hits,
                paradox: c.paradox,
                immune_attacking: c.immune_attacking,
                all_types: c.all_types,
                reborn_full: c.reborn_full,
                makes_golden: c.makes_golden,
                gift_unique: c.gift_unique,
                end_of_turn_at_start: c.end_of_turn_at_start,
                countdown: match &c.countdown {
                    None => None,
                    Some(cd) => Some(Countdown {
                        turns: cd.turns,
                        effect: match cd.effect.as_str() {
                            "double_stats" => CountdownEffect::DoubleStats,
                            "plain_copy" => CountdownEffect::PlainCopy,
                            "random_minion_of_type" => CountdownEffect::RandomMinionOfType,
                            other => return Err(err(format!("unknown countdown {other:?}"))),
                        },
                    }),
                },
            });
        }

        let mut bounties: Vec<CardId> = Vec::new();
        for sp in &raw.spell {
            let id = cards.len() as CardId;
            let err = |e: String| format!("cards.toml, {}: {e}", sp.name);
            // Tier 0 is for a generated spell (Lockbox); tier 7 spells are
            // never drawn (every draw stops at tier 6).
            if sp.tier > crate::CARD_MAX_TIER || (sp.tier == 0 && !sp.token) {
                return Err(err(format!("tier {} is not 1 to 7 (0 only for a token)", sp.tier)));
            }
            let (spell_target, spell_effect) = match &sp.effect {
                Some(e) if e.get.is_some() => {
                    let card = generated_id(e.get.as_deref().unwrap()).map_err(&err)?;
                    (None, Some(SpellEffect::GetCard { card, count: e.count.unwrap_or(1) }))
                }
                // Two cards that may be pool minions (generated copies of
                // them, with no pool copy): Temperature Shift.
                Some(e) if e.get_pair.is_some() => {
                    let [a, b] = e.get_pair.as_ref().unwrap();
                    let a = minion_id(a).map_err(&err)?;
                    let b = minion_id(b).map_err(&err)?;
                    (None, Some(SpellEffect::GetTwo { a, b }))
                }
                // Cast a named spell, `count` times (Veteran Brigand).
                Some(e) if e.cast.is_some() => {
                    let card = spell_id(e.cast.as_deref().unwrap()).map_err(&err)?;
                    (
                        None,
                        Some(SpellEffect::CastSpell { card, times: e.count.unwrap_or(1) }),
                    )
                }
                // Fill the hand with a card no pool holds (a Banana).
                Some(e) if e.fill_hand.is_some() => {
                    let card = generated_id(e.fill_hand.as_deref().unwrap()).map_err(&err)?;
                    (None, Some(SpellEffect::FillHand { card }))
                }
                // The spell comes back to hand at the end of every turn: it
                // names itself.
                Some(e) if e.return_each_turn => {
                    (None, Some(SpellEffect::ReturnEachTurn { card: id }))
                }
                Some(e) => parse_spell_effect(e).map_err(&err)?,
                None => (None, None),
            };
            let choose_one = match &sp.choose_one {
                None => None,
                Some([o1, o2]) => {
                    let to_opt = |o: &RawSpellEffect| -> Result<ChooseOption, String> {
                        match parse_spell_effect(o)? {
                            (Some(st), None) => Ok(ChooseOption::SpellTarget(st)),
                            (None, Some(se)) => Ok(ChooseOption::SpellEffect(se)),
                            _ => Err("invalid spell choose_one option".into()),
                        }
                    };
                    Some([to_opt(o1).map_err(&err)?, to_opt(o2).map_err(&err)?])
                }
            };
            if spell_target.is_none() && spell_effect.is_none() && choose_one.is_none() {
                return Err(err("spell has no effect or choose_one".into()));
            }
            if sp.bounty {
                // The five Bounties are one slot of the spell draw: only the
                // first stands in `spells`.
                if bounties.is_empty() && !sp.token {
                    spells.push(id);
                }
                bounties.push(id);
            } else if !sp.token {
                spells.push(id);
            }
            cards.push(Card {
                name: sp.name.clone(),
                tier: sp.tier,
                tribes: Tribes::NONE,
                lobby: sp.lobby.as_deref().map(tribe).transpose().map_err(&err)?,
                attack: 0,
                health: 0,
                golden_stats: None,
                keywords: Keywords::default(),
                prize: sp.prize,
                amalgam: false,
                amalgam_gain_copy: 0,
                magnetize_tribes: Tribes::NONE.with(Tribe::Mech),
                mecha_demon: false,
                buddy: false,
                token: sp.token,
                spell: true,
                pay_health: sp.pay_health,
                cost: sp.cost,
                is_beetle: false,
                is_volumizer: false,
                pool_slot: id,
                volumizer_buff: None,
                spell_buff: None,
                battlecry: None,
                choose_one,
                deathrattle: None,
                start_of_turn: None,
                end_of_turn: None,
                start_of_combat: None,
                rally: None,
                on_summon_beast: None,
                on_play_mech: None,
                on_play_or_magnetize_mech: None,
                on_cast_spell_on_mech: None,
                on_cast_spell: None,
                on_cast_spell_on_self: None,
                on_cast_spell_on_tribe: None,
                when_damaged: None,
                while_in_hand: None,
                teaches_spell: false,
                no_triple: false,
                taught_token: false,
                whenever_friendly_attacks: None,
                after_friendly_attacks: None,
                avenge: None,
                damage_milestone: None,
                buff_per_deathrattle: None,
                fixed_stats: false,
                activate: None,
                on_event: None,
                after_battlecry: None,
                permanent_keep: None,
                attack_threshold: None,
                after_hero_damage: Vec::new(),
                after_play_tribe: None,
                after_demon_damage: None,
                is_fodder: false,
                health_refreshes: 0,
                on_sell: None,
                aura: None,
                after_summon_give_stats: None,
                spell_target,
                spell_effect,
                always_golden: false,
                cleave: false,
                buff_per_golden_played: None,
                buff_per_elemental_played: None,
                buff_per_magnetize: None,
                on_kill: None,
                gains_friendly_deathrattles: false,
                after_discover: None,
                is_bounty: sp.bounty,
                overkill: false,
                triple_wildcard: None,
                play_shield: None,
                after_buy: None,
                buff_per_kind_death: None,
                on_reborn: None,
                deity: None,
                on_card_added: None,
                on_discard: sp.on_discard.as_deref().map(|s| match s {
                    "cast_twice" => Ok(OnDiscard::CastSpell(2)),
                    "max_gold_2" => Ok(OnDiscard::MaxGold(2)),
                    _ => Err(format!("unknown on_discard {s:?}")),
                }).transpose().map_err(&err)?,
                tavern_spell: sp.tavern_spell,
                blood_gem: sp.blood_gem,
                spellcraft: sp.spellcraft,
                on_gem_played_on_self: None,
                choose_one_cover: 0,
                after_choose_one_play: None,
                exclude_heroes: sp.effect.as_ref().map_or(Vec::new(), |e| e.exclude_heroes.clone()),
                on_card_play: None,
                buff_per_battlecry: None,
                buff_per_tavern_spell: None,
                buff_per_discard: None,
                acquire_copies: 0,
                activate_twice: false,
                shield_hits: 0,
                golden_shield_hits: 0,
                paradox: false,
                immune_attacking: false,
                all_types: false,
                reborn_full: false,
                makes_golden: false,
                gift_unique: false,
                end_of_turn_at_start: false,
                countdown: None,
            });
        }

        let mut hero_power_excludes: Vec<String> = Vec::new();
        for c in &cards {
            for id in &c.exclude_heroes {
                if !hero_power_excludes.contains(id) {
                    hero_power_excludes.push(id.clone());
                }
            }
        }
        let mecha_demons: Vec<CardId> = cards
            .iter()
            .enumerate()
            .filter(|(_, c)| c.mecha_demon)
            .map(|(i, _)| i as CardId)
            .collect();
        // The Darkmoon Prizes by tier, in card order so a seed replays.
        let mut prizes: [Vec<CardId>; 4] = Default::default();
        for (i, c) in cards.iter().enumerate() {
            if let Some(tier) = c.prize {
                if tier == 0 || tier > 4 {
                    return Err(format!("cards.toml {}: prize tier {tier}", c.name));
                }
                prizes[tier as usize - 1].push(i as CardId);
            }
        }
        let lockbox = cards
            .iter()
            .position(|c| matches!(c.spell_effect, Some(SpellEffect::Lockbox(_))))
            .map(|i| i as CardId);
        let blood_gem = cards.iter().position(|c| c.blood_gem).map(|i| i as CardId);
        let triple_reward = cards
            .iter()
            .position(|c| matches!(c.spell_effect, Some(SpellEffect::TripleReward)))
            .map(|i| i as CardId);
        let tiers: Vec<u8> = cards.iter().map(|c| c.tier).collect();
        let ids_up_to_tier = (0..=crate::CARD_MAX_TIER)
            .map(|hi| (0..pool_cards).filter(|&c| tiers[c] <= hi).map(|c| c as u8).collect())
            .collect();
        Ok(CardDb {
            cards,
            tiers,
            ids_up_to_tier,
            pool_cards,
            volumizers,
            spells,
            beetle_token,
            cobra_token,
            chromadrakes,
            taught_token,
            fodder_token,
            bounties,
            lockbox,
            blood_gem,
            triple_reward,
            hero_power_excludes,
            prizes,
            mecha_demons,
        })
    }

    /// One of the five Chromadrakes, uniformly; only in a lobby with Dragons
    /// (they are all Dragons). None otherwise.
    pub fn random_chromadrake(&self, lobby: Tribes, rng: &mut Rng) -> Option<CardId> {
        if self.chromadrakes.is_empty() || !lobby.has(Tribe::Dragon) {
            None
        } else {
            let k = rng.below(self.chromadrakes.len() as u32) as usize;
            Some(self.chromadrakes[k])
        }
    }

    pub fn get(&self, id: CardId) -> &Card {
        &self.cards[id as usize]
    }

    /// The types of a minion: its card's, or all ten when a Dark Gift gives
    /// it every type (Amalgamation). Every tribe question about a minion on a
    /// board, in a hand or in a fight goes through here.
    pub fn tribes_of(&self, m: &Minion) -> Tribes {
        if m.n_gifts != 0 && m.has_gift(self, |c| c.all_types) {
            return Tribes::ALL;
        }
        // A portrait can give a card more tribes (Bronzebeard Portrait:
        // your Branns are Murlocs and Dragons too), and that reaches every
        // tribe check through this one function.
        self.get(m.card).tribes.with_all(m.extra_tribes)
    }

    pub fn by_name(&self, name: &str) -> Option<CardId> {
        if let Some(i) = self.cards.iter().position(|c| c.name == name) {
            return Some(i as CardId);
        }
        let alt = match name {
            "Goldrinn the Great Wolf" | "Goldrinn" => "Goldrinn, the Great Wolf",
            "Beast" => "Foraging Bat",
            _ => return None,
        };
        self.cards.iter().position(|c| c.name == alt).map(|i| i as CardId)
    }

    /// A random Magnetic Mech from the pool (Dark Gifts Polarization). The
    /// minion that gets it is generated, so no pool copy moves.
    pub fn random_magnetic_mech(&self, lobby: Tribes, rng: &mut Rng) -> Option<CardId> {
        self.pick_lobby(
            |c| {
                self.cards[c].keywords.has(Keywords::MAGNETIC)
                    && self.cards[c].tribes.has(Tribe::Mech)
            },
            lobby,
            rng,
        )
    }

    pub fn random_volumizer(&self, lobby: Tribes, rng: &mut Rng) -> Option<CardId> {
        if self.volumizers.is_empty() || !lobby.has(Tribe::Mech) {
            None
        } else {
            let k = rng.below(self.volumizers.len() as u32) as usize;
            Some(self.volumizers[k])
        }
    }

    /// A random pool minion of a tribe, of Tier `max_tier` or lower (0 means
    /// any Tier). "Random minion" cards pick up to the player's current tier.
    /// The lobby is the filter: only minions with a lobby tribe (an all-type
    /// minion is every tribe) count.
    pub fn random_pool_minion(
        &self,
        tribe: Tribe,
        max_tier: u8,
        lobby: Tribes,
        rng: &mut Rng,
    ) -> Option<CardId> {
        let max_tier = if max_tier == 0 { crate::MAX_TIER } else { max_tier };
        let ok = |c: usize| {
            self.cards[c].tribes.has(tribe)
                && self.cards[c].tier <= max_tier
                && self.cards[c].lobby_ok(lobby)
        };
        let count = (0..self.pool_cards).filter(|&c| ok(c)).count() as u32;
        if count == 0 {
            return None;
        }
        let k = rng.below(count) as usize;
        (0..self.pool_cards).filter(|&c| ok(c)).nth(k).map(|c| c as CardId)
    }

    /// A random pool minion that has a Deathrattle, of Tier `max_tier` or
    /// lower (0 means any Tier). The lobby filters it.
    pub fn random_deathrattle_minion(
        &self,
        max_tier: u8,
        lobby: Tribes,
        rng: &mut Rng,
    ) -> Option<CardId> {
        let max_tier = if max_tier == 0 { crate::MAX_TIER } else { max_tier };
        self.pick_lobby(
            |c| self.cards[c].deathrattle.is_some() && self.cards[c].tier <= max_tier,
            lobby,
            rng,
        )
    }

    /// A random pool minion with a Battlecry, of Tier `max_tier` or lower (0
    /// means any Tier). The lobby filters it.
    pub fn random_battlecry_minion(
        &self,
        max_tier: u8,
        lobby: Tribes,
        rng: &mut Rng,
    ) -> Option<CardId> {
        let max_tier = if max_tier == 0 { crate::MAX_TIER } else { max_tier };
        self.pick_lobby(
            |c| self.cards[c].battlecry.is_some() && self.cards[c].tier <= max_tier,
            lobby,
            rng,
        )
    }

    /// A random pool minion that is Magnetic, of Tier `max_tier` or lower (0
    /// means any Tier). The lobby filters it.
    pub fn random_magnetic_minion(
        &self,
        max_tier: u8,
        lobby: Tribes,
        rng: &mut Rng,
    ) -> Option<CardId> {
        let max_tier = if max_tier == 0 { crate::MAX_TIER } else { max_tier };
        self.pick_lobby(
            |c| self.cards[c].keywords.has(Keywords::MAGNETIC) && self.cards[c].tier <= max_tier,
            lobby,
            rng,
        )
    }

    /// One pool card out of those `ok` accepts, uniformly, within `lobby`.
    fn pick_lobby(
        &self,
        ok: impl Fn(usize) -> bool,
        lobby: Tribes,
        rng: &mut Rng,
    ) -> Option<CardId> {
        let ok = |c: usize| ok(c) && self.cards[c].lobby_ok(lobby);
        let count = (0..self.pool_cards).filter(|&c| ok(c)).count() as u32;
        if count == 0 {
            return None;
        }
        let k = rng.below(count) as usize;
        (0..self.pool_cards).filter(|&c| ok(c)).nth(k).map(|c| c as CardId)
    }

    /// Whether a spell may be drawn when `exclude` is already taken. The
    /// shared Bounty slot is open while one of the five is not excluded.
    fn spell_open(&self, id: CardId, exclude: &[CardId]) -> bool {
        if self.cards[id as usize].is_bounty {
            self.bounties.iter().any(|b| !exclude.contains(b))
        } else {
            !exclude.contains(&id)
        }
    }

    /// A drawn Bounty slot becomes a random one of the five (not in
    /// `exclude`); any other spell is itself.
    fn resolve_bounty(&self, id: CardId, exclude: &[CardId], rng: &mut Rng) -> CardId {
        if !self.cards[id as usize].is_bounty {
            return id;
        }
        let n = self.bounties.iter().filter(|b| !exclude.contains(b)).count() as u32;
        if n == 0 {
            return id;
        }
        let k = rng.below(n) as usize;
        self.bounties.iter().copied().filter(|b| !exclude.contains(b)).nth(k).unwrap_or(id)
    }

    /// "A random Bounty": one of the five, uniformly. The Bounties are tied
    /// to Pirates, so only in a lobby with Pirates.
    pub fn random_bounty(&self, lobby: Tribes, rng: &mut Rng) -> Option<CardId> {
        if self.bounties.is_empty() || !self.cards[self.bounties[0] as usize].lobby_ok(lobby) {
            return None;
        }
        Some(self.bounties[rng.below(self.bounties.len() as u32) as usize])
    }

    /// A random spell that costs `cost`, of any Tavern Tier. The lobby
    /// filters every spell draw.
    pub fn random_spell_of_cost(&self, cost: u8, lobby: Tribes, rng: &mut Rng) -> Option<CardId> {
        self.random_spell_of_cost_up_to(cost, crate::MAX_TIER, lobby, rng)
    }

    pub fn random_spell_of_cost_up_to(
        &self,
        cost: u8,
        max_tier: u8,
        lobby: Tribes,
        rng: &mut Rng,
    ) -> Option<CardId> {
        let max_tier = max_tier.min(crate::MAX_TIER);
        let ok = |id: CardId| {
            self.cards[id as usize].cost == cost
                && self.cards[id as usize].tier <= max_tier
                && self.cards[id as usize].lobby_ok(lobby)
        };
        let count = self.spells.iter().filter(|&&id| ok(id)).count() as u32;
        if count == 0 {
            return None;
        }
        let k = rng.below(count) as usize;
        let id = self.spells.iter().copied().filter(|&id| ok(id)).nth(k)?;
        Some(self.resolve_bounty(id, &[], rng))
    }

    /// A random pool minion sharing a tribe with `tribes`, other than
    /// `exclude`, of Tier `max_tier` or lower. The lobby filters it.
    pub fn random_different_minion(
        &self,
        tribes: Tribes,
        exclude: CardId,
        max_tier: u8,
        lobby: Tribes,
        rng: &mut Rng,
    ) -> Option<CardId> {
        let max_tier = if max_tier == 0 { crate::MAX_TIER } else { max_tier };
        let ok = |c: usize| {
            self.cards[c].tribes.overlaps(tribes)
                && c as CardId != exclude
                && self.cards[c].tier <= max_tier
                && self.cards[c].lobby_ok(lobby)
        };
        let count = (0..self.pool_cards).filter(|&c| ok(c)).count() as u32;
        if count == 0 {
            return None;
        }
        let k = rng.below(count) as usize;
        (0..self.pool_cards).filter(|&c| ok(c)).nth(k).map(|c| c as CardId)
    }

    /// A random tier 7 minion, for the cards that say "a Tier 7 minion"
    /// (none in the lobby yet, Greedy Conniver is the first). Never
    /// reachable any other way: every other draw stops at the Tavern tiers.
    /// The lobby filters it.
    pub fn random_tier7_minion(&self, lobby: Tribes, rng: &mut Rng) -> Option<CardId> {
        self.random_tier_minion(crate::CARD_MAX_TIER, lobby, rng)
    }

    /// A random pool minion of exactly `tier`, within the lobby.
    pub fn random_tier_minion(&self, tier: u8, lobby: Tribes, rng: &mut Rng) -> Option<CardId> {
        self.pick_lobby(|c| self.cards[c].tier == tier, lobby, rng)
    }

    /// A random pool minion with a Choose One, any tribe, of Tier `max_tier`
    /// or lower, within the lobby (Bramble Tunneler, Fandral's Fortune).
    pub fn random_choose_one_minion(
        &self,
        max_tier: u8,
        lobby: Tribes,
        rng: &mut Rng,
    ) -> Option<CardId> {
        self.pick_lobby(
            |c| self.cards[c].choose_one.is_some() && self.cards[c].tier <= max_tier,
            lobby,
            rng,
        )
    }

    /// A random pool minion of any tribe, of Tier `max_tier` or lower, within
    /// the lobby (Dark Ritual's two random minions).
    pub fn random_any_minion(&self, max_tier: u8, lobby: Tribes, rng: &mut Rng) -> Option<CardId> {
        self.pick_lobby(|c| self.cards[c].tier <= max_tier, lobby, rng)
    }

    pub fn random_spell(&self, max_tier: u8, lobby: Tribes, rng: &mut Rng) -> Option<CardId> {
        let max_tier = max_tier.min(crate::MAX_TIER);
        let count = self
            .spells
            .iter()
            .filter(|&&id| {
                self.cards[id as usize].tier <= max_tier && self.cards[id as usize].lobby_ok(lobby)
            })
            .count() as u32;
        if count == 0 {
            return None;
        }
        let k = rng.below(count) as usize;
        let id = self
            .spells
            .iter()
            .copied()
            .filter(|&id| {
                self.cards[id as usize].tier <= max_tier && self.cards[id as usize].lobby_ok(lobby)
            })
            .nth(k)?;
        Some(self.resolve_bounty(id, &[], rng))
    }

    /// A random Tavern spell of Tier `max_tier` or lower for Puzzle Box to
    /// cast: never one of the destructive ones (docs/rules/heroes.md). Spells have
    /// no pool, so nothing is taken.
    pub fn random_castable_spell(&self, max_tier: u8, lobby: Tribes, rng: &mut Rng) -> Option<CardId> {
        let max_tier = max_tier.min(crate::MAX_TIER);
        let ok = |id: CardId| {
            let c = &self.cards[id as usize];
            c.tier <= max_tier
                && c.tavern_spell
                && c.lobby_ok(lobby)
                && !destructive_at_random(c)
        };
        let count = self.spells.iter().filter(|&&id| ok(id)).count() as u32;
        if count == 0 {
            return None;
        }
        let k = rng.below(count) as usize;
        let id = self.spells.iter().copied().filter(|&id| ok(id)).nth(k)?;
        Some(self.resolve_bounty(id, &[], rng))
    }

    /// A random Tavern spell of Tier `max_tier` or lower, other than the
    /// cards in `exclude`. The lobby filters every spell draw.
    pub fn random_spell_up_to(
        &self,
        max_tier: u8,
        exclude: &[CardId],
        lobby: Tribes,
        rng: &mut Rng,
    ) -> Option<CardId> {
        let max_tier = max_tier.min(crate::MAX_TIER);
        let ok = |id: CardId| {
            self.cards[id as usize].tier <= max_tier
                && self.cards[id as usize].lobby_ok(lobby)
                && self.spell_open(id, exclude)
        };
        let count = self.spells.iter().filter(|&&id| ok(id)).count() as u32;
        if count == 0 {
            return None;
        }
        let k = rng.below(count) as usize;
        let id = self.spells.iter().copied().filter(|&id| ok(id)).nth(k)?;
        Some(self.resolve_bounty(id, exclude, rng))
    }

    pub fn random_tier_spell(
        &self,
        tier: u8,
        exclude: &[CardId],
        lobby: Tribes,
        rng: &mut Rng,
    ) -> Option<CardId> {
        if tier > crate::MAX_TIER {
            return None;
        }
        let ok = |id: CardId| {
            self.cards[id as usize].tier == tier
                && self.cards[id as usize].lobby_ok(lobby)
                && self.spell_open(id, exclude)
        };
        let count = self.spells.iter().filter(|&&id| ok(id)).count() as u32;
        if count == 0 {
            return None;
        }
        let k = rng.below(count) as usize;
        let id = self.spells.iter().copied().filter(|&id| ok(id)).nth(k)?;
        Some(self.resolve_bounty(id, exclude, rng))
    }

    /// A random Tavern spell of `cost` Gold or more, up to `tier` (Unfurled
    /// Codex). The pool's own filter, so no tier 7 spell and no Bounty.
    pub fn random_spell_at_least(
        &self,
        cost: u8,
        tier: u8,
        lobby: Tribes,
        rng: &mut Rng,
    ) -> Option<CardId> {
        let ok = |id: CardId| {
            self.cards[id as usize].cost >= cost
                && self.cards[id as usize].tier <= tier
                && self.cards[id as usize].lobby_ok(lobby)
        };
        let count = self.spells.iter().filter(|&&id| ok(id)).count() as u32;
        if count == 0 {
            return None;
        }
        let k = rng.below(count) as usize;
        self.spells.iter().copied().filter(|&id| ok(id)).nth(k)
    }

    /// A Darkmoon Prize of one Prize tier that is not in `exclude`, picked
    /// evenly. Prizes are never in the pool, so nothing is taken from it.
    pub fn random_prize(&self, tier: u8, exclude: &[CardId], rng: &mut Rng) -> Option<CardId> {
        let list = self.prizes.get(tier.checked_sub(1)? as usize)?;
        let open = list.iter().filter(|id| !exclude.contains(id)).count() as u32;
        if open == 0 {
            return None;
        }
        let k = rng.below(open) as usize;
        list.iter().copied().filter(|id| !exclude.contains(id)).nth(k)
    }

    /// A random one of the four Mecha-Demons, which the Mecha-Jaraxxus
    /// Sticker hands out as generated copies ("random" is one of the four,
    /// equally likely: docs/rules/trinkets.md).
    pub fn random_mecha_demon(&self, rng: &mut Rng) -> Option<CardId> {
        if self.mecha_demons.is_empty() {
            return None;
        }
        let k = rng.below(self.mecha_demons.len() as u32) as usize;
        Some(self.mecha_demons[k])
    }

    /// Every Darkmoon Prize of any tier, uniformly: what "a random Darkmoon
    /// Prize" means when no tier is named (Hand of Fate).
    pub fn random_prize_any(&self, rng: &mut Rng) -> Option<CardId> {
        let total: usize = self.prizes.iter().map(|l| l.len()).sum();
        if total == 0 {
            return None;
        }
        let mut k = rng.below(total as u32) as usize;
        for list in &self.prizes {
            if k < list.len() {
                return Some(list[k]);
            }
            k -= list.len();
        }
        None
    }

    /// A fresh generated copy of a card (`pool_copies = 0`).
    pub fn make(&self, id: CardId, golden: bool) -> Minion {
        let c = self.get(id);
        // Aureate Laureate is always Golden, and gives no Triple Reward.
        let golden = golden || c.always_golden;
        let m = if golden { 2 } else { 1 };
        let (attack, health) = if golden {
            c.golden_stats.unwrap_or((c.attack * 2, c.health * 2))
        } else {
            (c.attack * m, c.health * m)
        };
        Minion {
            card: id,
            attack,
            health,
            keywords: c.keywords,
            golden,
            no_reward: c.always_golden,
            temp_shield: false,
            pool_copies: 0,
            winners_bread: 0,
            magnetizations: 0,
            volumizers: u16::from(c.is_volumizer),
            // A Lockbox's `improvements` is the turns it has left.
            improvements: match c.spell_effect {
                Some(SpellEffect::Lockbox(n)) => n as u16,
                _ => 0,
            },
            extra_magnetize: 0,
            taught_spell: None,
            locked_turns: 0,
            locked_tier: 0,
            locked_gold: 0,
            doomed: false,
            activated: false,
            uses_left: 0,
            gem_attack: 0,
            gem_health: 0,
            both_effects: false,
            linked: 0,
            attached: [Attached::default(); MAX_ATTACHED],
            n_attached: 0,
            gifts: [GiftOn::default(); crate::MAX_GIFTS],
            n_gifts: 0,
            // A card whose Divine Shield takes several hits (Dark Paradox's
            // Tier 5 version, and its golden with one hit more).
            shield_hits: if c.shield_hits > 1 {
                if golden {
                    match c.golden_shield_hits {
                        0 => c.shield_hits * 2,
                        n => n,
                    }
                } else {
                    c.shield_hits
                }
            } else {
                0
            },
            hats: 0,
            tickets: 0,
            watched: false,
            temp_reborn: false,
            temp_attack: 0,
            frozen: false,
            reanimate: false,
            end_turn_gain: (0, 0),
            extra_tribes: crate::cards::Tribes::NONE,
        }
    }
}
