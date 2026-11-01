//! The observation: what one player can see, written as flat f32 arrays.
//! Values are raw (gold 7 is 7.0); scaling belongs to the training side.
//! Card ids are written as id + 1 (0 for none); the model embeds them.
//!
//! Layout, in order:
//! - SCALARS own values (239):
//!   0  gold
//!   1  tier
//!   2  upgrade_cost
//!   3  health
//!   4  turn
//!   5  frozen
//!   6  actions_left (remaining actions before the turn cap)
//!   7  alive (number of living players in the lobby)
//!   8  fighting_ghost (1 if paired against the ghost this turn)
//!   9  won_last_combat
//!   10 lost_last_combat
//!   11 gold_next_turn (extra gold queued for next turn)
//!   12 extra_max_gold (permanent increase to the turn gold from Strike Oil)
//!   13 free_rolls (free Refreshes left)
//!   14 next_roll_health (1 if the next Refresh is paid with Health, Malchezaar)
//!   15 health_refreshes_left (Health Refreshes left this turn)
//!   16 can_rewind (1 if a minion on the board gives hero damage back)
//!   17 spell_discount (cost reduction on the next Tavern spell bought)
//!   18 shop_spell_cost (cost of the spell slot after the discount, 0 if empty)
//!   19 shop_spell_pay_health (1 if the spell slot is bought with Health)
//!   20 spell_attack (total Tavern spell Attack bonus, permanent + board auras)
//!   21 spell_health (total Tavern spell Health bonus, permanent + board auras)
//!   22 volumizer_attack (permanent Volumizer Attack bonus)
//!   23 volumizer_health
//!   24 beetle_attack (permanent Beetle Attack bonus)
//!   25 beetle_health
//!   26 boon_of_beetles (Boon of Beetles charges)
//!   27 brood_of_nozdormu (pending Brood of Nozdormu Start of Combat triggers)
//!   28 upper_hand (pending Upper Hand Start of Combat triggers)
//!   29 patient_scout_tier (highest Discover tier of a Patient Scout on board, 0 if none)
//!   30 tasty_lobster (Lobster Deathrattles this game, the "improve" count)
//!   31 beast_bonus_attack (shop Goldrinn bonus on Beasts until next turn)
//!   32 beast_bonus_health
//!   33 deathrattles_triggered (this game, Falling Sky Golem)
//!   34 fodder_next (Fodders the next explicit Refresh brings)
//!   35 fodder_queued (Fodders queued over all coming Refreshes)
//!   36 fodder_refreshes (coming Refreshes in the queue, with or without Fodder)
//!   37 magicfin_left (Magicfin Mycologist teachings left this turn)
//!   38 overconfidence_win (gold next turn if the coming fight is won)
//!   39 overconfidence_tie (gold next turn if it is a tie)
//!   40 winners_bread_attack (what each Winner's Bread stack gives on a win)
//!   41 winners_bread_health
//!   42 next_turn_buff_attack (board buff queued for the start of next turn, summed)
//!   43 next_turn_buff_health
//!   44..87 Tavern buffs: 11 filters (any, then the ten tribes in `Tribe::EVERY`
//!   order) x 4 values (this game attack, this game health, this turn attack,
//!   this turn health)
//!   88 last_spell (card id + 1 of the last Tavern spell cast, 0 if none)
//!   89..98 lobby, multi-hot over `Tribe::EVERY`: the five tribes this game's
//!   lobby holds
//!   99 gem_attack (what one of this player's Blood Gems gives)
//!   100 gem_health
//!   101 barrage_attack (Blood Gem Barrage: Gems every Refresh gives)
//!   102 barrage_health
//!   103 refresh_attack (Waveling / En-Djinn / Easterly Winds: what the next
//!   Refresh gives one Tavern minion, summed)
//!   104 refresh_health
//!   105 baller (Ballers sold this game)
//!   106 undead_attack (the Undead bonus, this game wherever they are)
//!   107 undead_health
//!   108 knight_deaths (Eternal Knights that died this game)
//!   109 elementals_played (Elementals played from hand this turn)
//!   110 golden_played (Golden minions played from hand this game)
//!   111 gold_spent (gold spent this turn)
//!   112 discarded (cards discarded this game)
//!   113 spells_cast (Tavern spells cast this game)
//!   114 trailblazer_covers (Choose One cards left this turn that get both
//!   effects: the combines still on the board's Trailblazers)
//!   115 sharing_is_caring (pending Sharing is Caring Start of Combat triggers)
//!   116 last_damage (damage the hero took last, what a rewind gives back)
//!   117..122 own Deity, DEITY_FEATURES values: present, C'Thun, Y'Shaarj
//!   (one-hot), attack, health, golden
//!   Pending choice (while one is open, only actions that answer it are legal):
//!   123 pending_target (a minion target on the board only)
//!   124 pending_target_shop (a minion target on the board or in the Tavern)
//!   125 pending_shop_slot (a Tavern slot target: a chosen consume or Fishbait)
//!   126 pending_choose_one
//!   127 pending_discover
//!   128 pending_discard (a "discard a card" pick, Mangled Bandit)
//!   129 pending_source (board slot + 1 of the minion whose effect it is, 0 for a spell)
//!   130 pending_card (card id + 1 of that minion, or of the last spell cast
//!   for a spell target or Choose One spell; 0 for a Discover)
//!   131 pending_runs (times the target effect runs: Brann, Balinda)
//!   132 pending_attack (Attack the target effect gives or sets, 0 if none)
//!   133 pending_health
//!   134..140 target kind, one-hot: buff, consume, spell, trigger_rally,
//!   trigger_battlecry, any_buff, any_set_stats
//!   141 shop_consume_left (Tavern minions the chosen consume still takes)
//!   142 shop_fishbait (1 if the Tavern slot target puts a Fishbait there)
//!   143 discover_locked (1 if the Discovered minion is locked in hand a turn)
//!   144 discover_damage (1 if the hero takes damage equal to the chosen Tier)
//!   145 discover_doomed (1 if the Discovered minion dies when played this
//!   turn, Tomb Turning)
//!   146 discover_both (1 if the Discovered Choose One minion gets both
//!   effects, Fandral's Fortune)
//!   147 discover_queue (more Discovers waiting after this one)
//!   148..159 Choose One options, 2 x 6: kind one-hot (minion target, spell
//!   bonus, spell target, spell effect), attack, health
//!   Heroes:
//!   160 armor
//!   161 hero (index into the hero table + 1, 0 with none)
//!   162 power hero (whose power it is, index + 1; Unmasked Identity can
//!   change it away from the hero)
//!   163 power cost
//!   164 power uses left this turn
//!   165 power used this game
//!   166 power unlocked
//!   167 power usable (a button, unlocked, uses left, the Gold for it, and a
//!   target for target powers)
//!   168 power progress: how far it has counted toward its next trigger (the
//!   level reached for the growing ones, the Gold spent for Choose Your
//!   Champion, the lock left for Lucky Roll)
//!   169 power left: how many more it needs (or has left, counting down)
//!   170..177 the SECOND power the same way (whose power it is, cost, uses
//!   left, used this game, unlocked, usable, progress, left): Genn's and the
//!   Reinvigorating Light trinket's
//!   178 pending hero pick (the offer before turn 1 is open)
//!   179 pending power pick (a Hero Power Discover or a chooser is open)
//!   180..183 the powers it offers, one per Choose slot (CHOICES = 4, power
//!   index + 1, 0 for an empty slot): a Hero Power Discover offers three,
//!   Bru'kan's Elements four, so the bot picks a power and not blind
//!   184..186 hero offer (DISCOVER slots, hero index + 1, 0 for empty)
//!   Trinkets (DESIGN.md "Trinkets and Dark Gifts"):
//!   187..189 the lesser slot, greater slot and hero power trinket (index
//!   into the trinket table + 1, 0 for empty)
//!   190..192 their "every N" counters
//!   193..195 the minion type each slot locked in at the pick (index + 1, 0
//!   for none: the Colorful and Gold-plated Compasses)
//!   196 a trinket offer is open, 197 which slot it fills (1 lesser,
//!   2 greater, 3 the power's)
//!   198..205 the four options: trinket index + 1, then the price
//!   206..210 Dark Gifts and Dark Paradox
//!   211 free_minion_left (Spectral Sight: free Tavern minions left this turn)
//!   212..214 Conviction: minion count, Attack buff, Health buff
//!   215..217 Lock and Load: stored minion card id + 1, Attack, Health
//!   218..221 Friendly Wager: offer open, 2 offered opponents, picked opponent
//!   222..226 Detective for Hire: offer open, 2 offered card ids + 1, 2 empty flags
//!   227 Ice Block held (Safety Patch: the save is still there)
//!   228 Immune this turn
//!   229 the Prize tier the open Discover offers (0 if it is not a Prize one)
//!   230 Triple Prizes replace Triple Rewards (Corrupted Tome)
//!   231..234 the Secret zone: Venomstrike Trap, Autodefense Matrix, Pack
//!   Tactics, Redemption (1 when held; the Ice Block is the flag above)
//!   235..238 the Secrets an open Prestidigitation offers, one per Choose
//!   slot (kind + 1 in the order above, 0 for an empty slot)
//!   (Its legality is in the action mask, so it isn't repeated here.)
//! - Own card slots (OWN_SLOTS = 28, each SLOT_FEATURES values):
//!   shop minions (SHOP = 7), shop spell (SHOP_SPELL = 1), hand (HAND = 10),
//!   board (BOARD = 7), Discover options (DISCOVER = 3). A Tavern spell that
//!   sits in a minion slot (Saloon's Finest) is written there as a spell.
//! - 7 opponents, in seat order starting after me (each OPP_BLOCK values):
//!   OPP_SCALARS (30): alive, health, armor, tier, hero (index + 1, 0 with
//!   none), seen (1 if I ever fought them),
//!   turns_since (since that fight), next (my opponent this turn, or the ghost
//!   of this player), last (my opponent last turn), placement (0 while
//!   alive), the tribe counts of their last seen board (ten values,
//!   `Tribe::EVERY` order), their Deity at the first attack of that fight,
//!   after Start of Combat (DEITY_FEATURES values, laid out like mine), and
//!   their Blood Gem value and Tavern spell bonus at that moment (attack,
//!   health each); then that board (BOARD = 7 card slots).
//!
//! A card slot is SLOT_FEATURES (90) values:
//!   0  present (1 if occupied)
//!   1  card id + 1 (one id space covering minions, tokens and spells)
//!   2  is_spell
//!   3  attack (0 for spells)
//!   4  health (0 for spells)
//!   5  fight_attack: the Attack it would fight with on this player's board
//!      now: plus Holy Vanguard's +30 (+60 golden) when that hero has 15
//!      Health or less, and for a Beast not on the board yet the shop
//!      Goldrinn bonus it gets when played. Equal to attack otherwise.
//!   6  fight_health
//!   7  golden
//!   8  tier (1 to 7)
//!   9..18 tribes, multi-hot over `Tribe::EVERY` (an all-type minion sets all
//!          ten, a neutral none)
//!   19..25 keywords: taunt, divine_shield, windfury, venomous, reborn,
//!          magnetic, stealth
//!   26 magnetizations (total Magnetizations on this minion)
//!   27 volumizers (Volumizers this minion is or carries)
//!   28 improvements (the minion's own count: Leviathan, Spark Snapper, Patient
//!      Scout, Hopebringer, Sacrificial Wrathguard, event counters)
//!   29 events_left (the "(N left!)" of its event trigger, 0 if it has none)
//!   30 from_pool (1 if backed by pool copies, 0 if generated, token or spell)
//!   31 winners_bread (pending Winner's Bread stacks)
//!   32 taught_spell (card id + 1 of the spell a Magicfin token casts, 0 if none)
//!   33 locked (turns it stays locked in hand for, Search Through Time; 1
//!      while a Tavern tier or a Gold count holds it, DESIGN.md "Cards
//!      locked in hand")
//!   34 activate_cost (Gold of its Activate, 0 if it has none or it was
//!      already used this turn: once per turn per minion)
//!   35 extra_magnetize (extra times the next Magnetization to it happens, Drone Duplicator)
//!   36 cost (a spell's printed cost; the buy cost for a minion)
//!   37 gem_attack (Attack this minion has gained from Blood Gems)
//!   38 gem_health
//!   39 temp_shield (its Divine Shield is Ichoron's, off at the start of the
//!      next turn)
//!   40 doomed (dies right after it is played this turn, Tomb Turning)
//!   41 both_effects (a Choose One card with both effects combined, Fandral's
//!      Fortune's pick or a Trailblazer cover)
//!   42 linked (one of a group of cards that came together; playing one
//!      discards the others)
//!   43 link_group (the group id it shares with them)
//!   44 tavern_spell (a spell that is a Tavern spell, not a plain one: Blood
//!      Gem, Triple Reward)
//!   45 no_reward (a Golden play of it gives no Triple Reward, Aureate
//!      Laureate and Silent Deliverer's minions)
//!   46..61 attached: card id + 1 of each effect-carrying card Magnetized
//!          into it, in the order they merged (0 for an empty slot)
//!   62..77 attached_golden: whether that attachment is golden
//!   78..85 the Dark Gifts on the minion, as their effect card id + 1
//!   86 hats (Hat Trick: each is +1/+1, already in its stats)
//!   87 ticket (1 when the Tavern card carries a Darkmoon Ticket, Come One,
//!      Come All!)
//!   88 frozen (frozen on its own in the Tavern, Twice as Nice)
//!   89 reanimate (1 when marked by Rapid Reanimation for Start of Combat)
//! An empty slot is all zeros.

use crate::cards::{Aura, CardDb, ChooseOption, Counter, DeityKind, SpellEffect, SpellTarget, Tribe};
use crate::fight::Deity;
use crate::game::{Game, Opponent, Player, PowerState, ShopTargetKind, TargetKind};
use crate::heroes::PowerEffect;
use crate::minion::{Keywords, MAX_ATTACHED, Minion};
use crate::{BOARD, DISCOVER, HAND, PLAYERS, SHOP};

pub const SHOP_SPELL: usize = 1;
pub const SLOT_FEATURES: usize = 90;
pub const MINION_FEATURES: usize = SLOT_FEATURES;
pub const OPP_SCALARS: usize = 33;

pub const OWN_SLOTS: usize = SHOP + SHOP_SPELL + HAND + BOARD + DISCOVER;
pub const OPP_SLOTS: usize = (PLAYERS - 1) * BOARD;
pub const TOTAL_SLOTS: usize = OWN_SLOTS + OPP_SLOTS;
pub const OWN_END: usize = SCALARS + OWN_SLOTS * SLOT_FEATURES;
pub const OPP_BLOCK: usize = OPP_SCALARS + BOARD * SLOT_FEATURES;
pub const OBS_SIZE: usize = OWN_END + (PLAYERS - 1) * OPP_BLOCK;

pub const SHOP_START: usize = SCALARS;
pub const SHOP_SPELL_START: usize = SHOP_START + SHOP * SLOT_FEATURES;
pub const HAND_START: usize = SHOP_SPELL_START + SHOP_SPELL * SLOT_FEATURES;
pub const BOARD_START: usize = HAND_START + HAND * SLOT_FEATURES;
pub const DISCOVER_START: usize = BOARD_START + BOARD * SLOT_FEATURES;

pub const SCALAR_GOLD: usize = 0;
pub const SCALAR_TIER: usize = 1;
pub const SCALAR_UPGRADE_COST: usize = 2;
pub const SCALAR_HEALTH: usize = 3;
pub const SCALAR_TURN: usize = 4;
pub const SCALAR_FROZEN: usize = 5;
pub const SCALAR_ACTIONS_LEFT: usize = 6;
pub const SCALAR_ALIVE: usize = 7;
pub const SCALAR_GHOST: usize = 8;
pub const SCALAR_WON_LAST_COMBAT: usize = 9;
pub const SCALAR_LOST_LAST_COMBAT: usize = 10;
pub const SCALAR_GOLD_NEXT_TURN: usize = 11;
pub const SCALAR_EXTRA_MAX_GOLD: usize = 12;
pub const SCALAR_FREE_ROLLS: usize = 13;
pub const SCALAR_NEXT_ROLL_HEALTH: usize = 14;
pub const SCALAR_HEALTH_REFRESHES_LEFT: usize = 15;
pub const SCALAR_CAN_REWIND: usize = 16;
pub const SCALAR_SPELL_DISCOUNT: usize = 17;
pub const SCALAR_SHOP_SPELL_COST: usize = 18;
pub const SCALAR_SHOP_SPELL_PAY_HEALTH: usize = 19;
pub const SCALAR_SPELL_ATTACK: usize = 20;
pub const SCALAR_SPELL_HEALTH: usize = 21;
pub const SCALAR_VOLUMIZER_ATTACK: usize = 22;
pub const SCALAR_VOLUMIZER_HEALTH: usize = 23;
pub const SCALAR_BEETLE_ATTACK: usize = 24;
pub const SCALAR_BEETLE_HEALTH: usize = 25;
pub const SCALAR_BOON_OF_BEETLES: usize = 26;
pub const SCALAR_BROOD_OF_NOZDORMU: usize = 27;
pub const SCALAR_UPPER_HAND: usize = 28;
pub const SCALAR_PATIENT_SCOUT_TIER: usize = 29;
pub const SCALAR_TASTY_LOBSTER: usize = 30;
pub const SCALAR_BEAST_BONUS_ATTACK: usize = 31;
pub const SCALAR_BEAST_BONUS_HEALTH: usize = 32;
pub const SCALAR_DEATHRATTLES: usize = 33;
pub const SCALAR_FODDER_NEXT: usize = 34;
pub const SCALAR_FODDER_QUEUED: usize = 35;
pub const SCALAR_FODDER_REFRESHES: usize = 36;
pub const SCALAR_MAGICFIN_LEFT: usize = 37;
pub const SCALAR_OVERCONFIDENCE_WIN: usize = 38;
pub const SCALAR_OVERCONFIDENCE_TIE: usize = 39;
pub const SCALAR_WINNERS_BREAD_ATTACK: usize = 40;
pub const SCALAR_WINNERS_BREAD_HEALTH: usize = 41;
pub const SCALAR_NEXT_TURN_BUFF_ATTACK: usize = 42;
pub const SCALAR_NEXT_TURN_BUFF_HEALTH: usize = 43;
/// 11 filters (any, then the ten tribes in `Tribe::EVERY` order) x 4 values.
pub const SCALAR_TAVERN_BUFFS: usize = 44;
pub const TAVERN_BUFF_FILTERS: usize = 11;
pub const SCALAR_LAST_SPELL: usize = 88;
/// LOBBY_TRIBES values, `Tribe::EVERY` order: the tribes of this game's lobby.
pub const SCALAR_LOBBY: usize = 89;
pub const LOBBY_TRIBES: usize = Tribe::EVERY.len();
/// What one of this player's Blood Gems gives.
pub const SCALAR_GEM_ATTACK: usize = 99;
pub const SCALAR_GEM_HEALTH: usize = 100;
/// Blood Gem Barrage: Gems every Refresh gives the Tavern (the turn-start
/// roll included when the Tavern changed).
pub const SCALAR_BARRAGE_ATTACK: usize = 101;
pub const SCALAR_BARRAGE_HEALTH: usize = 102;
/// What a Refresh gives one Tavern minion (Waveling, En-Djinn
/// Blazer, Easterly Winds), summed.
pub const SCALAR_REFRESH_ATTACK: usize = 103;
pub const SCALAR_REFRESH_HEALTH: usize = 104;
/// Ballers sold this game.
pub const SCALAR_BALLER: usize = 105;
/// The Undead bonus, this game wherever they are.
pub const SCALAR_UNDEAD_ATTACK: usize = 106;
pub const SCALAR_UNDEAD_HEALTH: usize = 107;
/// Eternal Knights that died this game.
pub const SCALAR_KNIGHT_DEATHS: usize = 108;
/// Elementals played from hand this turn.
pub const SCALAR_ELEMENTALS_PLAYED: usize = 109;
/// Golden minions played from hand this game.
pub const SCALAR_GOLDEN_PLAYED: usize = 110;
/// Gold spent this turn.
pub const SCALAR_GOLD_SPENT: usize = 111;
/// Cards discarded this game.
pub const SCALAR_DISCARDED: usize = 112;
/// Tavern spells cast this game.
pub const SCALAR_SPELLS_CAST: usize = 113;
/// Choose One cards left this turn that get both effects (Thorned
/// Trailblazer).
pub const SCALAR_TRAILBLAZER_COVERS: usize = 114;
/// Pending Sharing is Caring Start of Combat triggers.
pub const SCALAR_SHARING_IS_CARING: usize = 115;
/// Damage the hero took last, what a rewind gives back.
pub const SCALAR_LAST_DAMAGE: usize = 116;
/// Own Deity, DEITY_FEATURES values: present, C'Thun, Y'Shaarj (one-hot),
/// attack, health, golden.
pub const SCALAR_DEITY: usize = 117;
pub const DEITY_FEATURES: usize = 6;
pub const SCALAR_DEITY_CTHUN: usize = SCALAR_DEITY + 1;
pub const SCALAR_DEITY_YSHAARJ: usize = SCALAR_DEITY + 2;
pub const SCALAR_DEITY_ATTACK: usize = SCALAR_DEITY + 3;
pub const SCALAR_DEITY_HEALTH: usize = SCALAR_DEITY + 4;
pub const SCALAR_DEITY_GOLDEN: usize = SCALAR_DEITY + 5;
pub const SCALAR_PENDING_TARGET: usize = 123;
pub const SCALAR_PENDING_TARGET_SHOP: usize = 124;
pub const SCALAR_PENDING_SHOP_SLOT: usize = 125;
pub const SCALAR_PENDING_CHOOSE_ONE: usize = 126;
pub const SCALAR_PENDING_DISCOVER: usize = 127;
pub const SCALAR_PENDING_DISCARD: usize = 128;
pub const SCALAR_PENDING_SOURCE: usize = 129;
pub const SCALAR_PENDING_CARD: usize = 130;
pub const SCALAR_PENDING_RUNS: usize = 131;
pub const SCALAR_PENDING_ATTACK: usize = 132;
pub const SCALAR_PENDING_HEALTH: usize = 133;
/// One-hot over buff, consume, spell, trigger_rally, trigger_battlecry,
/// any_buff, any_set_stats.
pub const SCALAR_TARGET_KIND: usize = 134;
pub const TARGET_KINDS: usize = 7;
pub const SCALAR_SHOP_CONSUME_LEFT: usize = 141;
pub const SCALAR_SHOP_FISHBAIT: usize = 142;
pub const SCALAR_DISCOVER_LOCKED: usize = 143;
pub const SCALAR_DISCOVER_DAMAGE: usize = 144;
pub const SCALAR_DISCOVER_DOOMED: usize = 145;
pub const SCALAR_DISCOVER_BOTH: usize = 146;
pub const SCALAR_DISCOVER_QUEUE: usize = 147;
/// 2 options x CHOOSE_FEATURES: kind one-hot (minion target, spell bonus,
/// spell target, spell effect), attack, health.
pub const SCALAR_CHOOSE_ONE: usize = 148;
pub const CHOOSE_FEATURES: usize = 6;
pub const SCALAR_ARMOR: usize = 160;
/// Own hero, index into the hero table + 1 (0 with none).
pub const SCALAR_HERO: usize = 161;
/// Whose power the player has (Unmasked Identity changes it), index + 1.
pub const SCALAR_POWER_HERO: usize = 162;
pub const SCALAR_POWER_COST: usize = 163;
pub const SCALAR_POWER_USES_LEFT: usize = 164;
pub const SCALAR_POWER_USED_GAME: usize = 165;
pub const SCALAR_POWER_UNLOCKED: usize = 166;
pub const SCALAR_POWER_USABLE: usize = 167;
/// How far the power has counted toward its next trigger (the level it has
/// grown to for the growing ones, the Gold spent for Choose Your Champion),
/// and how many more it needs (or has left, counting down).
pub const SCALAR_POWER_PROGRESS: usize = 168;
pub const SCALAR_POWER_LEFT: usize = 169;
/// The second power, laid out like the main one: whose power it is (index
/// into the power table + 1), cost, uses left, used this game, unlocked,
/// usable, its own count and what is left of it. Every power fills these two
/// from its own `PowerState`, so a new power needs no new scalar.
pub const SCALAR_POWER2: usize = SCALAR_POWER_HERO + POWER_FEATURES;
pub const POWER_FEATURES: usize = 8;
pub const SCALAR_POWER2_HERO: usize = SCALAR_POWER2;
pub const SCALAR_POWER2_COST: usize = SCALAR_POWER2 + 1;
pub const SCALAR_POWER2_USES_LEFT: usize = SCALAR_POWER2 + 2;
pub const SCALAR_POWER2_USED_GAME: usize = SCALAR_POWER2 + 3;
pub const SCALAR_POWER2_UNLOCKED: usize = SCALAR_POWER2 + 4;
pub const SCALAR_POWER2_USABLE: usize = SCALAR_POWER2 + 5;
pub const SCALAR_POWER2_PROGRESS: usize = SCALAR_POWER2 + 6;
pub const SCALAR_POWER2_LEFT: usize = SCALAR_POWER2 + 7;
pub const SCALAR_PENDING_HERO: usize = SCALAR_POWER2 + POWER_FEATURES;
pub const SCALAR_PENDING_POWER: usize = SCALAR_PENDING_HERO + 1;
/// The powers an open Hero Power Discover or chooser offers, one per Choose
/// slot: `CHOICES` values, power index (into `HeroDb::powers`) + 1, 0 for an
/// empty slot. The model embeds them with the same table as the power ids.
pub const SCALAR_POWER_OFFER: usize = SCALAR_PENDING_POWER + 1;
/// DISCOVER slots of the hero offer before turn 1, hero index + 1, 0 empty.
pub const SCALAR_HERO_OFFER: usize = SCALAR_POWER_OFFER + crate::CHOICES;
/// Trinkets: the lesser slot, the greater slot and the one a hero power
/// became (Marin the Manager, Buttons), each as an index into the trinket
/// table plus one (0 for empty), then the "every N" counter of each.
pub const SCALAR_TRINKET: usize = SCALAR_HERO_OFFER + DISCOVER;
pub const SCALAR_TRINKET_COUNT: usize = SCALAR_TRINKET + crate::TRINKET_SLOTS;
/// The minion type each trinket slot locked in at the pick (`Tribe` as an
/// index plus one, 0 for none): the Colorful Compasses hand out minions of it
/// and the Gold-plated Compass makes the next one bought Golden, so the bot
/// sees which type is worth buying (docs/trinkets.md, "Block 6 rules").
pub const SCALAR_TRINKET_TRIBE: usize = SCALAR_TRINKET_COUNT + crate::TRINKET_SLOTS;
/// A trinket offer is open, and which slot it is for (1 lesser, 2 greater,
/// 3 the hero power's).
pub const SCALAR_PENDING_TRINKET: usize = SCALAR_TRINKET_TRIBE + crate::TRINKET_SLOTS;
pub const SCALAR_TRINKET_OFFER_SLOT: usize = SCALAR_PENDING_TRINKET + 1;
/// The four options: trinket index + 1 and the price it is sold at.
pub const SCALAR_TRINKET_OPTION: usize = SCALAR_TRINKET_OFFER_SLOT + 1;
pub const TRINKET_OPTION_FEATURES: usize = 2;
pub const SCALAR_TRINKETS_END: usize =
    SCALAR_TRINKET_OPTION + crate::TRINKET_CHOICES * TRINKET_OPTION_FEATURES;
/// The Dark Discovery button: the uses it has left this game, whether it was
/// pressed this turn, and whether it can be pressed now (unlocked, a use
/// left, not pressed, and the Gold for it).
pub const SCALAR_DARK_GIFT_USES: usize = SCALAR_TRINKETS_END;
pub const SCALAR_DARK_GIFT_PRESSED: usize = SCALAR_DARK_GIFT_USES + 1;
pub const SCALAR_DARK_GIFT_READY: usize = SCALAR_DARK_GIFT_USES + 2;
/// Whether the open Discover is a Dark Gift offer (its gifts are in the
/// Discover slots' gift fields, so the bot sees them before it picks).
pub const SCALAR_DARK_OFFER: usize = SCALAR_DARK_GIFT_USES + 3;
/// The Dark Paradox version in this game's pool, card id + 1 (0 when the
/// card set has none). The five versions are one card each and only one is
/// in a game's pool, so the id tells the bot which Tier, stats and gift the
/// card has even when no copy is in sight.
pub const SCALAR_PARADOX: usize = SCALAR_DARK_OFFER + 1;
/// Gallywix's Trade Prince: 1 when the next minion you buy this turn costs 0.
pub const SCALAR_FREE_MINION_LEFT: usize = SCALAR_PARADOX + 1;
/// Cariel Roame's Conviction: count, Attack, and Health of the next cast.
pub const SCALAR_CONVICTION_COUNT: usize = SCALAR_FREE_MINION_LEFT + 1;
pub const SCALAR_CONVICTION_ATTACK: usize = SCALAR_CONVICTION_COUNT + 1;
pub const SCALAR_CONVICTION_HEALTH: usize = SCALAR_CONVICTION_ATTACK + 1;
/// Tavish's Lock and Load: card id + 1, Attack, and Health of the minion stored
/// in the hero power (0 when empty).
pub const SCALAR_LOCKED_CARD: usize = SCALAR_CONVICTION_HEALTH + 1;
pub const SCALAR_LOCKED_ATTACK: usize = SCALAR_LOCKED_CARD + 1;
pub const SCALAR_LOCKED_HEALTH: usize = SCALAR_LOCKED_ATTACK + 1;
/// Lord Barov's Friendly Wager: whether the 2-way wager choice is open, the
/// two seats (seat + 1, 0 when closed), and the chosen seat + 1 (0 if none).
pub const SCALAR_PENDING_WAGER: usize = SCALAR_LOCKED_HEALTH + 1;
pub const SCALAR_WAGER_OFFER: usize = SCALAR_PENDING_WAGER + 1;
pub const SCALAR_WAGER_PICK: usize = SCALAR_WAGER_OFFER + 2;
/// Murloc Holmes's Detective for Hire: whether the 2-way guess is open, the
/// two offered card ids + 1 (0 when none or empty-board option), and whether
/// each option represents an empty board.
pub const SCALAR_PENDING_HOLMES: usize = SCALAR_WAGER_PICK + 1;
pub const SCALAR_HOLMES_OFFER: usize = SCALAR_PENDING_HOLMES + 1;
pub const SCALAR_HOLMES_EMPTY: usize = SCALAR_HOLMES_OFFER + 2;
/// Block 5: a Safety Patch's Ice Block held and unspent (the trinket slot
/// says the trinket is held; this says the save is still there), whether the
/// hero is Immune this turn, the Prize tier a Prize Discover is offering (0
/// when the open Discover is not a Prize Discover), and whether Triple
/// Rewards have become Triple Prizes (Corrupted Tome).
pub const SCALAR_ICE_BLOCK: usize = SCALAR_HOLMES_EMPTY + 2;
pub const SCALAR_IMMUNE: usize = SCALAR_ICE_BLOCK + 1;
pub const SCALAR_PRIZE_OFFER_TIER: usize = SCALAR_IMMUNE + 1;
pub const SCALAR_TRIPLE_PRIZE: usize = SCALAR_PRIZE_OFFER_TIER + 1;
/// The Secret zone (docs/heroes.md "Secrets"): one flag per combat Secret,
/// in `secrets` module order (Venomstrike Trap, Autodefense Matrix, Pack
/// Tactics, Redemption), then the Secrets an open Prestidigitation offers
/// (kind + 1, 0 for an empty slot). Only the player's OWN zone is ever
/// written: an opponent's Secrets stay hidden, as the client hides them.
pub const SCALAR_SECRET: usize = SCALAR_TRIPLE_PRIZE + 1;
pub const SCALAR_SECRET_OFFER: usize = SCALAR_SECRET + crate::secrets::COMBAT_SECRETS as usize;
pub const SCALARS: usize = SCALAR_SECRET_OFFER + crate::CHOICES;

pub const SLOT_PRESENT: usize = 0;
pub const SLOT_CARD_ID: usize = 1;
pub const SLOT_IS_SPELL: usize = 2;
pub const SLOT_ATTACK: usize = 3;
pub const SLOT_HEALTH: usize = 4;
pub const SLOT_FIGHT_ATTACK: usize = 5;
pub const SLOT_FIGHT_HEALTH: usize = 6;
pub const SLOT_GOLDEN: usize = 7;
pub const SLOT_TIER: usize = 8;
/// Ten values, `Tribe::EVERY` order: Beast, Mech, Dragon, Murloc, Demon,
/// Pirate, Elemental, Undead, Quilboar, Aberration.
pub const SLOT_TRIBES: usize = 9;
pub const SLOT_BEAST: usize = SLOT_TRIBES;
pub const SLOT_MECH: usize = SLOT_TRIBES + 1;
pub const SLOT_DRAGON: usize = SLOT_TRIBES + 2;
pub const SLOT_MURLOC: usize = SLOT_TRIBES + 3;
pub const SLOT_DEMON: usize = SLOT_TRIBES + 4;
pub const SLOT_PIRATE: usize = SLOT_TRIBES + 5;
pub const SLOT_ELEMENTAL: usize = SLOT_TRIBES + 6;
pub const SLOT_UNDEAD: usize = SLOT_TRIBES + 7;
pub const SLOT_QUILBOAR: usize = SLOT_TRIBES + 8;
pub const SLOT_ABERRATION: usize = SLOT_TRIBES + 9;
/// Seven values, `Keywords::ALL` order.
pub const SLOT_KEYWORDS: usize = 19;
pub const SLOT_STEALTH: usize = SLOT_KEYWORDS + 6;
pub const SLOT_MAGNETIZATIONS: usize = 26;
pub const SLOT_VOLUMIZERS: usize = 27;
pub const SLOT_IMPROVEMENTS: usize = 28;
pub const SLOT_EVENTS_LEFT: usize = 29;
pub const SLOT_FROM_POOL: usize = 30;
pub const SLOT_WINNERS_BREAD: usize = 31;
pub const SLOT_TAUGHT_SPELL: usize = 32;
pub const SLOT_LOCKED: usize = 33;
pub const SLOT_ACTIVATE_COST: usize = 34;
pub const SLOT_EXTRA_MAGNETIZE: usize = 35;
pub const SLOT_COST: usize = 36;
/// Attack and Health this minion has gained from Blood Gems.
pub const SLOT_GEM_ATTACK: usize = 37;
pub const SLOT_GEM_HEALTH: usize = 38;
/// Its Divine Shield lasts only until the next turn (Ichoron the Protector).
pub const SLOT_TEMP_SHIELD: usize = 39;
/// It dies right after it is played this turn (Tomb Turning).
pub const SLOT_DOOMED: usize = 40;
/// A Choose One card with both effects combined (Fandral's Fortune pick).
pub const SLOT_BOTH_EFFECTS: usize = 41;
/// One of a group of cards that came linked; playing one discards the rest.
pub const SLOT_LINKED: usize = 42;
/// The id its group shares.
pub const SLOT_LINK_GROUP: usize = 43;
/// A Tavern spell (not a plain spell like a Blood Gem or Triple Reward).
pub const SLOT_TAVERN_SPELL: usize = 44;
/// A Golden play of it gives no Triple Reward (Aureate Laureate, Silent
/// Deliverer's minions).
pub const SLOT_NO_REWARD: usize = 45;
/// MAX_ATTACHED values: card id + 1 of each effect-carrying card Magnetized
/// into it, in merge order (0 for an empty slot).
pub const SLOT_ATTACHED: usize = 46;
/// MAX_ATTACHED values: whether each attachment is golden.
pub const SLOT_ATTACHED_GOLDEN: usize = SLOT_ATTACHED + MAX_ATTACHED;
/// MAX_GIFTS values: the Dark Gifts on the minion, as the gift's effect card
/// id + 1 (the model embeds them like card ids), 0 for an empty slot.
pub const SLOT_GIFTS: usize = SLOT_ATTACHED_GOLDEN + MAX_ATTACHED;
/// Hat Trick: the hats this minion wears (each is +1/+1, already in its
/// stats), 0 for none.
pub const SLOT_HATS: usize = SLOT_GIFTS + crate::MAX_GIFTS;
/// Come One, Come All!: 1 when the Tavern card carries a Darkmoon Ticket.
pub const SLOT_TICKETS: usize = SLOT_HATS + 1;
/// Frozen on its own in the Tavern (Twice as Nice froze the two cards it
/// copied), a card that stays one more turn.
pub const SLOT_FROZEN: usize = SLOT_TICKETS + 1;
/// Marked by Teron Gorefiend's Rapid Reanimation to be killed at Start of
/// Combat and resummoned as an exact copy when space allows.
pub const SLOT_REANIMATE: usize = SLOT_FROZEN + 1;

pub const OPP_ALIVE: usize = 0;
pub const OPP_HEALTH: usize = 1;
pub const OPP_TIER: usize = 2;
pub const OPP_SEEN: usize = 3;
pub const OPP_TURNS_SINCE: usize = 4;
pub const OPP_NEXT: usize = 5;
pub const OPP_LAST: usize = 6;
pub const OPP_PLACEMENT: usize = 7;
/// Ten values, `Tribe::EVERY` order.
pub const OPP_TRIBES: usize = 8;
/// Their Deity as it went into the last fight, DEITY_FEATURES values laid out
/// like `SCALAR_DEITY`.
pub const OPP_DEITY: usize = 18;
pub const OPP_DEITY_CTHUN: usize = OPP_DEITY + 1;
pub const OPP_DEITY_YSHAARJ: usize = OPP_DEITY + 2;
pub const OPP_DEITY_ATTACK: usize = OPP_DEITY + 3;
pub const OPP_DEITY_HEALTH: usize = OPP_DEITY + 4;
pub const OPP_DEITY_GOLDEN: usize = OPP_DEITY + 5;
/// Their Blood Gem value as it went into that fight.
pub const OPP_GEM_ATTACK: usize = 24;
pub const OPP_GEM_HEALTH: usize = 25;
/// Their Tavern spell bonus as it went into that fight.
pub const OPP_SPELL_ATTACK: usize = 26;
pub const OPP_SPELL_HEALTH: usize = 27;
/// Their hero, index into the hero table + 1 (0 with none): heroes are
/// always visible.
pub const OPP_HERO: usize = 28;
/// Their armor, live.
pub const OPP_ARMOR: usize = 29;
/// Their trinkets (lesser, greater, the hero power's), trinket index + 1.
pub const OPP_TRINKET: usize = 30;

const _: () = assert!(SCALAR_TAVERN_BUFFS + TAVERN_BUFF_FILTERS * 4 == SCALAR_LAST_SPELL);
const _: () = assert!(SCALAR_LOBBY + LOBBY_TRIBES == SCALAR_GEM_ATTACK);
const _: () = assert!(SCALAR_DEITY + DEITY_FEATURES == SCALAR_PENDING_TARGET);
const _: () = assert!(SCALAR_TARGET_KIND + TARGET_KINDS == SCALAR_SHOP_CONSUME_LEFT);
const _: () = assert!(SCALAR_CHOOSE_ONE + 2 * CHOOSE_FEATURES == SCALAR_ARMOR);
const _: () = assert!(SCALAR_POWER_OFFER + crate::CHOICES == SCALAR_HERO_OFFER);
const _: () = assert!(SCALAR_HERO_OFFER + DISCOVER == SCALAR_TRINKET);
const _: () = assert!(SCALAR_TRINKET_TRIBE + crate::TRINKET_SLOTS == SCALAR_PENDING_TRINKET);
const _: () = assert!(SCALAR_TRIPLE_PRIZE + 1 == SCALAR_SECRET);
const _: () = assert!(SCALAR_SECRET_OFFER + crate::CHOICES == SCALARS);
const _: () = assert!(SLOT_TRIBES + Tribe::EVERY.len() == SLOT_KEYWORDS);
const _: () = assert!(SLOT_KEYWORDS + Keywords::ALL.len() == SLOT_MAGNETIZATIONS);
const _: () = assert!(SLOT_ATTACHED + MAX_ATTACHED == SLOT_ATTACHED_GOLDEN);
const _: () = assert!(SLOT_ATTACHED_GOLDEN + MAX_ATTACHED == SLOT_GIFTS);
const _: () = assert!(SLOT_REANIMATE + 1 == SLOT_FEATURES);
const _: () = assert!(OPP_TRIBES + Tribe::EVERY.len() == OPP_DEITY);
const _: () = assert!(OPP_DEITY + DEITY_FEATURES == OPP_GEM_ATTACK);
const _: () = assert!(OPP_GEM_ATTACK + 4 == OPP_HERO);
const _: () = assert!(OPP_HERO + 2 == OPP_TRINKET);
const _: () = assert!(OPP_TRINKET + crate::TRINKET_SLOTS == OPP_SCALARS);

/// Holy Vanguard's bonus on this minion if its hero is at `health`: +30/+30
/// (+60 golden) at 15 Health or less, added at the start of a fight.
pub fn vanguard_bonus(db: &CardDb, m: &Minion, health: i32) -> i32 {
    if health > 15 {
        return 0;
    }
    m.parts()
        .filter(|&(c, _)| db.get(c).aura == Some(Aura::HolyVanguard))
        .map(|(_, golden)| if golden { 60 } else { 30 })
        .max()
        .unwrap_or(0)
}

/// How a slot's minion relates to the player whose view this is, for its
/// fight stats.
#[derive(Clone, Copy)]
struct Owner {
    /// Health of the hero whose board it is (or would be on).
    health: i32,
    /// Bonus a Beast gets when played (0 for a minion already on a board).
    beast_bonus: (i32, i32),
    /// The owner's Tavern tier and the Gold it has spent this game, which a
    /// card locked in hand reads (a locked card never leaves the hand, so an
    /// opponent's slots always read unlocked).
    tier: u8,
    gold_spent: u32,
}

/// A Deity block, `DEITY_FEATURES` values starting at `base`: present, which
/// god (one-hot), attack, health, golden. All zeros when there is none.
fn write_deity(o: &mut [f32], db: &CardDb, d: Option<Deity>) {
    let Some(d) = d else { return };
    o[0] = 1.0;
    match db.get(d.card).deity {
        Some(DeityKind::Cthun) => o[1] = 1.0,
        Some(DeityKind::Yshaarj) => o[2] = 1.0,
        None => {}
    }
    o[3] = d.attack as f32;
    o[4] = d.health as f32;
    o[5] = d.golden as u8 as f32;
}

fn write_slot(game: &Game, o: &mut [f32], m: &Minion, owner: Owner) {
    let db = &game.data.cards;
    let c = db.get(m.card);
    o[SLOT_PRESENT] = 1.0;
    o[SLOT_CARD_ID] = m.card as f32 + 1.0;
    o[SLOT_IS_SPELL] = c.is_spell() as u8 as f32;
    o[SLOT_TIER] = c.tier as f32;
    o[SLOT_TAVERN_SPELL] = c.tavern_spell as u8 as f32;
    o[SLOT_LINKED] = (m.linked != 0) as u8 as f32;
    o[SLOT_LINK_GROUP] = (m.linked & 0x3fff) as f32;
    o[SLOT_BOTH_EFFECTS] = m.both_effects as u8 as f32;
    o[SLOT_NO_REWARD] = m.no_reward as u8 as f32;
    if c.is_spell() {
        o[SLOT_COST] = c.cost as f32;
        return;
    }
    o[SLOT_COST] = game.data.rules.buy_cost as f32;
    o[SLOT_ATTACK] = m.attack as f32;
    o[SLOT_HEALTH] = m.health as f32;
    let v = vanguard_bonus(db, m, owner.health);
    let (ba, bh) = if c.tribes.has(Tribe::Beast) { owner.beast_bonus } else { (0, 0) };
    o[SLOT_FIGHT_ATTACK] = (m.attack + v + ba) as f32;
    o[SLOT_FIGHT_HEALTH] = (m.health + v + bh) as f32;
    o[SLOT_GOLDEN] = m.golden as u8 as f32;
    for (j, t) in Tribe::EVERY.into_iter().enumerate() {
        o[SLOT_TRIBES + j] = c.tribes.has(t) as u8 as f32;
    }
    for (j, (bit, _)) in Keywords::ALL.iter().enumerate() {
        o[SLOT_KEYWORDS + j] = m.keywords.has(*bit) as u8 as f32;
    }
    o[SLOT_MAGNETIZATIONS] = m.magnetizations as f32;
    o[SLOT_VOLUMIZERS] = m.volumizers as f32;
    o[SLOT_IMPROVEMENTS] = m.improvements as f32;
    o[SLOT_EVENTS_LEFT] = m.events_left(db).map_or(0.0, |n| n as f32);
    o[SLOT_FROM_POOL] = (m.pool_copies > 0) as u8 as f32;
    o[SLOT_WINNERS_BREAD] = m.winners_bread as f32;
    o[SLOT_TAUGHT_SPELL] = m.taught_spell.map_or(0.0, |s| s as f32 + 1.0);
    // Locked in hand (DESIGN.md "Cards locked in hand"): the turns it stays
    // locked, or 1 while a condition holds it (a Tavern tier, Gold spent).
    o[SLOT_LOCKED] = if m.locked_turns > 0 {
        m.locked_turns as f32
    } else {
        m.locked_in_hand(owner.tier, owner.gold_spent) as u8 as f32
    };
    // zero once used this turn, so the slot reads as "nothing to activate"
    o[SLOT_ACTIVATE_COST] = if m.activated {
        0.0
    } else {
        m.parts().find_map(|(card, _)| db.get(card).activate).map_or(0.0, |a| a.cost as f32)
    };
    o[SLOT_EXTRA_MAGNETIZE] = m.extra_magnetize as f32;
    o[SLOT_GEM_ATTACK] = m.gem_attack as f32;
    o[SLOT_GEM_HEALTH] = m.gem_health as f32;
    o[SLOT_TEMP_SHIELD] = m.temp_shield as u8 as f32;
    o[SLOT_DOOMED] = m.doomed as u8 as f32;
    for (j, a) in m.attached_slice().iter().enumerate() {
        o[SLOT_ATTACHED + j] = a.card as f32 + 1.0;
        o[SLOT_ATTACHED_GOLDEN + j] = a.golden as u8 as f32;
    }
    for (j, g) in m.gift_slice().iter().enumerate() {
        o[SLOT_GIFTS + j] = g.card() as f32 + 1.0;
    }
    o[SLOT_HATS] = m.hats as f32;
    o[SLOT_TICKETS] = (m.tickets == 1) as u8 as f32;
    o[SLOT_FROZEN] = m.frozen as u8 as f32;
    o[SLOT_REANIMATE] = m.reanimate as u8 as f32;
}

fn write_slots<'a>(
    game: &Game,
    out: &mut [f32],
    slots: usize,
    ms: impl Iterator<Item = &'a Minion>,
    owner: Owner,
) -> usize {
    let n = slots * SLOT_FEATURES;
    out[..n].fill(0.0);
    for (k, m) in ms.take(slots).enumerate() {
        let o = &mut out[k * SLOT_FEATURES..(k + 1) * SLOT_FEATURES];
        write_slot(game, o, m, owner);
    }
    n
}

/// The Attack and Health a Tavern spell target gives or sets, if it has
/// numbers.
fn spell_target_stats(st: SpellTarget) -> (i32, i32) {
    match st {
        SpellTarget::Buff { attack, health, .. }
        | SpellTarget::BuffSameTribe { attack, health }
        | SpellTarget::SetStats { attack, health }
        | SpellTarget::BuffPerBonusKeyword { attack, health }
        | SpellTarget::TrickyTrousers { attack, health }
        | SpellTarget::TavernBuffType { attack, health } => (attack, health),
        _ => (0, 0),
    }
}

fn spell_effect_stats(se: SpellEffect) -> (i32, i32) {
    match se {
        SpellEffect::BuffShop { attack, health }
        | SpellEffect::BuffBoard { attack, health, .. }
        | SpellEffect::BuffRandomBoard { attack, health, .. }
        | SpellEffect::NextTurnBoardBuff { attack, health, .. }
        | SpellEffect::MightyDragonbreath { attack, health }
        | SpellEffect::BuffTavernPermanent { attack, health }
        | SpellEffect::MisplacedTeaSet { attack, health } => (attack, health),
        _ => (0, 0),
    }
}

/// Index of a target kind in the one-hot, and the stats it gives or sets.
fn target_kind(k: TargetKind) -> (usize, (i32, i32)) {
    match k {
        TargetKind::MinionBuff { attack, health, .. } => (0, (attack, health)),
        TargetKind::Consume { .. } => (1, (0, 0)),
        TargetKind::Spell(st) => (2, spell_target_stats(st)),
        TargetKind::TriggerRally => (3, (0, 0)),
        TargetKind::TriggerBattlecry => (4, (0, 0)),
        TargetKind::AnyBuff { attack, health } => (5, (attack, health)),
        TargetKind::AnySetStats { attack, health } => (6, (attack, health)),
        // Same slot as the Make Golden spell.
        TargetKind::MakeGolden { .. } => (2, (0, 0)),
        // Maw Caster's destroy shares the Consume slot, Dead Bellringer's the
        // buff slot.
        TargetKind::DestroyDiscover { .. } | TargetKind::DestroyCopy { .. } => (1, (0, 0)),
        TargetKind::RebornDestroy { attack, health, .. } => (0, (attack, health)),
        // A Tavern card steal shares the any-buff slot (a shop pick), and so
        // does becoming a copy of one.
        TargetKind::StealShopCard { .. } | TargetKind::BecomeCopy => (5, (0, 0)),
        // Stir the Pot's throw is a pick on the board or the Tavern, like a
        // Tavern steal; the swap is a board pick of its own.
        TargetKind::ThrowInPot { .. } | TargetKind::SwapWithShop => (5, (0, 0)),
        TargetKind::RemoveTierBelow => (1, (0, 0)),
        // Hero batch 2: a keyword buff shares the buff slot, a replacement or
        // a stat steal the any-buff one, and the multicard picks the buff one
        // (a board pick, as Devour's is).
        TargetKind::TempReborn | TargetKind::DestroyGetTribe { .. } => (0, (0, 0)),
        TargetKind::StealShopAsStats { attack, health } => (6, (attack, health)),
        TargetKind::ReplaceHigher
        | TargetKind::ReplaceSameTier
        | TargetKind::ImprisonShop { .. }
        | TargetKind::SpiritSwapFirst
        | TargetKind::SpiritSwapSecond
        | TargetKind::DevourFirst
        | TargetKind::DevourSecond
        | TargetKind::SummonExactCopy
        | TargetKind::LockAndLoad
        | TargetKind::RapidReanimation => (5, (0, 0)),
    }
}

fn choose_option(opt: ChooseOption) -> (usize, (i32, i32)) {
    match opt {
        ChooseOption::MinionTarget { attack, health, .. } => (0, (attack, health)),
        ChooseOption::SpellBonus { attack, health } => (1, (attack, health)),
        ChooseOption::SpellTarget(st) => (2, spell_target_stats(st)),
        ChooseOption::SpellEffect(se) => (3, spell_effect_stats(se)),
    }
}

/// Magicfin Mycologist teachings left this turn: the uses on the
/// Mycologists on the board.
pub fn magicfin_left(pl: &Player, db: &CardDb) -> u8 {
    pl.board
        .iter()
        .filter(|m| m.parts().any(|(c, _)| db.get(c).teaches_spell))
        .map(|m| m.uses_left)
        .fold(0u8, u8::saturating_add)
}

/// How far a power has counted toward its next trigger, and how many more it
/// needs: `(progress, left)`. One pair for every power, so a power that counts
/// something needs no scalar of its own:
/// - the counting ones report the count and what is left of it (Natural
///   Balance's Tiers, Glaive Ricochet, Verdant Spheres, Gone Fishing, Heroic
///   Inspiration, For the Horde!, Buried Treasure's digs, Stir the Pot,
///   Upbeat Harmony, BUY, INSECT!, Reliquary Research's every third spell);
/// - the growing ones report the level reached and the cards left to the next
///   step (Sharpen Blades), or the Gold they pay now (Piggy Bank);
/// - Choose Your Champion reports the Gold spent and the Gold still to go;
/// - a chosen power counting down to its bonus reports the count and what is
///   left (Galewing's flightpaths), and Lucky Roll's lock is in both.
///
/// A power with no count reports (0, 0).
fn power_progress(game: &Game, p: usize, st: &PowerState) -> (f32, f32) {
    let power = game.data.heroes.power(st.of);
    let counts = game.players[p].counts;
    let pair = |done: u32, every: u32| (done as f32, every.saturating_sub(done) as f32);
    // A chosen power counting down to its bonus (a flightpath's "In 2 turns").
    if power.parent.is_some() && power.complete_turns > 0 {
        return pair(st.progress, power.complete_turns as u32);
    }
    match power.effect {
        // The level the buff has grown to, and the cards to the next one.
        PowerEffect::BuffPerBuys { every, .. } => {
            let every = every.max(1) as u32;
            let level = counts.cards_bought / every;
            let left = every - counts.cards_bought % every;
            (level as f32, left as f32)
        }
        PowerEffect::BuyMilestone { count, .. }
        | PowerEffect::AttackMilestone { count, .. } => pair(st.progress, count as u32),
        PowerEffect::DamageMilestone { damage, .. } => pair(st.progress, damage),
        PowerEffect::RandomTribeAfterSells { every, .. }
        | PowerEffect::CardAfterBuys { every, .. }
        | PowerEffect::UpgradeDiscountAfterElementals { every, .. }
        | PowerEffect::TavernBuffAfterBuys { every, .. }
        | PowerEffect::DigForGolden { digs: every }
        | PowerEffect::StirThePot { count: every }
        | PowerEffect::HandCopyEvery { turns: every }
        | PowerEffect::CopyBuyAfterMinions { every }
        | PowerEffect::MechGyver { every } => pair(st.progress, every as u32),
        PowerEffect::TripleRewardAfterTiers { every }
        | PowerEffect::TripleRewardAfterAttacks { every }
        | PowerEffect::Champion { gold: every } => {
            // Choose Your Champion counts the Gold spent, not its own count.
            let done = if matches!(power.effect, PowerEffect::Champion { .. }) {
                counts.gold_spent
            } else {
                st.progress
            };
            pair(done, every)
        }
        // The Gold it would pay now, and no count of its own.
        PowerEffect::GrowingGold { base } => {
            (base.saturating_add(st.progress.min(u8::MAX as u32) as u8) as f32, 0.0)
        }
        // The lock the die left, in both.
        PowerEffect::LuckyRoll => (st.progress as f32, st.progress as f32),
        // Every third Tavern spell bought is free.
        PowerEffect::FreeThirdSpell => {
            pair(counts.tavern_spells_bought % 3, 3)
        }
        // Tavern Lighting: the extra +1/+1 each Tavern spell has now, and the
        // turns to the next one.
        PowerEffect::TavernLighting { .. } => {
            let turns = st.progress.max(1);
            ((1 + turns / 3) as f32, (3 - turns % 3) as f32)
        }
        // Saturday C'Thuns!: the repeats the end of this turn gives.
        PowerEffect::SaturdayCthun { .. } => (st.progress as f32, 0.0),
        // Come One, Come All!: the Tickets taken, and how many are still to
        // come.
        PowerEffect::DarkmoonTickets { count, .. } => {
            let goal = count.max(1);
            pair(game.players[p].tickets as u32, goal as u32)
        }
        // Broodmother: the permanent +1/+1 bonus the Whelps have grown by.
        PowerEffect::Broodmother { .. } => (st.progress as f32, 0.0),
        // Tentacular: the Tentacle's current size (2/2 + 1/1 per minion sold).
        PowerEffect::Tentacular { .. } => ((2 + counts.minions_sold) as f32, 0.0),
        // Wax Warband: the current +1/+1 size and the Gold left to the next step.
        PowerEffect::WaxWarband => (
            (1 + counts.gold_spent / 10) as f32,
            (10 - counts.gold_spent % 10) as f32,
        ),
        PowerEffect::FreeFirstMinion => (game.players[p].free_minion_left as f32, 0.0),
        // Prestidigitation: the combat Secrets held, and how many are still
        // missing (the options its offer has, 0 when the button is dead).
        PowerEffect::DiscoverSecret => {
            let held = game.players[p].secrets.combat_len() as f32;
            (held, crate::secrets::COMBAT_SECRETS as f32 - held)
        }
        // Prize Wall: the Prize tier the next Prize turn offers, and the turns
        // to it (0 on a Prize turn, when the Discover is already open).
        PowerEffect::PrizeWall { every } => {
            let every = every.max(1) as u32;
            let turn = game.turn;
            let to = st.progress;
            let tier = if to == 0 {
                (turn / every).clamp(1, 4)
            } else {
                ((turn + to) / every).clamp(1, 4)
            };
            (tier as f32, to as f32)
        }
        PowerEffect::TakeFirstKilled
        | PowerEffect::EmbraceYourRage
        | PowerEffect::RapidReanimation => (st.armed as u8 as f32, 0.0),
        PowerEffect::Conviction => (
            game.players[p].conviction.0 as f32,
            game.players[p].conviction.1 as f32,
        ),
        PowerEffect::LockAndLoad => (
            game.players[p].locked_and_loaded.as_ref().map_or(0.0, |m| m.card as f32 + 1.0),
            0.0,
        ),
        _ => (0.0, 0.0),
    }
}

fn write_scalars(game: &Game, p: usize, s: &mut [f32]) {
    let pl = &game.players[p];
    let db = &game.data.cards;
    let r = &game.data.rules;
    let (spell_atk, spell_hp) = game.total_spell_bonus(p);
    s.fill(0.0);
    s[SCALAR_GOLD] = pl.gold as f32;
    s[SCALAR_TIER] = pl.tier as f32;
    s[SCALAR_UPGRADE_COST] = game.upgrade_price(p) as f32;
    s[SCALAR_HEALTH] = pl.health as f32;
    s[SCALAR_TURN] = game.turn as f32;
    s[SCALAR_FROZEN] = pl.frozen as u8 as f32;
    s[SCALAR_ACTIONS_LEFT] = r.max_actions_per_turn.saturating_sub(pl.actions) as f32;
    s[SCALAR_ALIVE] = game.alive() as f32;
    s[SCALAR_GHOST] = matches!(pl.opponent, Some(Opponent::Ghost(_))) as u8 as f32;
    s[SCALAR_WON_LAST_COMBAT] = pl.won_last_combat as u8 as f32;
    s[SCALAR_LOST_LAST_COMBAT] = pl.lost_last_combat as u8 as f32;
    s[SCALAR_GOLD_NEXT_TURN] = pl.gold_next_turn as f32;
    s[SCALAR_EXTRA_MAX_GOLD] = pl.extra_max_gold as f32;
    s[SCALAR_FREE_ROLLS] = pl.free_rolls as f32;
    s[SCALAR_NEXT_ROLL_HEALTH] =
        (pl.free_rolls == 0 && pl.can_refresh_with_health(db, r)) as u8 as f32;
    s[SCALAR_HEALTH_REFRESHES_LEFT] = pl.health_refreshes_left(db) as f32;
    s[SCALAR_CAN_REWIND] = pl.can_rewind(db) as u8 as f32;
    s[SCALAR_SPELL_DISCOUNT] = pl.spell_discount as f32;
    if let Some(sp) = pl.shop_spell {
        let c = db.get(sp);
        s[SCALAR_SHOP_SPELL_PAY_HEALTH] = c.pay_health as u8 as f32;
        s[SCALAR_SHOP_SPELL_COST] =
            if c.pay_health { c.cost } else { c.cost.saturating_sub(pl.spell_discount) } as f32;
    }
    s[SCALAR_SPELL_ATTACK] = spell_atk as f32;
    s[SCALAR_SPELL_HEALTH] = spell_hp as f32;
    s[SCALAR_VOLUMIZER_ATTACK] = pl.volumizer_bonus.0 as f32;
    s[SCALAR_VOLUMIZER_HEALTH] = pl.volumizer_bonus.1 as f32;
    s[SCALAR_BEETLE_ATTACK] = pl.beetle_bonus.0 as f32;
    s[SCALAR_BEETLE_HEALTH] = pl.beetle_bonus.1 as f32;
    s[SCALAR_BOON_OF_BEETLES] = pl.boon_of_beetles as f32;
    s[SCALAR_BROOD_OF_NOZDORMU] = pl.brood_of_nozdormu as f32;
    s[SCALAR_UPPER_HAND] = pl.upper_hand as f32;
    s[SCALAR_PATIENT_SCOUT_TIER] = pl.patient_scout_tier(db) as f32;
    s[SCALAR_TASTY_LOBSTER] = pl.improve[Counter::TastyLobster.index()] as f32;
    s[SCALAR_BALLER] = pl.improve[Counter::Baller.index()] as f32;
    s[SCALAR_BEAST_BONUS_ATTACK] = pl.beast_bonus.0 as f32;
    s[SCALAR_BEAST_BONUS_HEALTH] = pl.beast_bonus.1 as f32;
    s[SCALAR_DEATHRATTLES] = pl.deathrattles_triggered as f32;
    s[SCALAR_FODDER_NEXT] = pl.fodder_refreshes.first().copied().unwrap_or(0) as f32;
    s[SCALAR_FODDER_QUEUED] = pl.fodder_refreshes.iter().map(|&n| n as u32).sum::<u32>() as f32;
    s[SCALAR_FODDER_REFRESHES] = pl.fodder_refreshes.len() as f32;
    s[SCALAR_MAGICFIN_LEFT] = magicfin_left(pl, db) as f32;
    s[SCALAR_OVERCONFIDENCE_WIN] = pl.overconfidence_win as f32;
    s[SCALAR_OVERCONFIDENCE_TIE] = pl.overconfidence_tie as f32;
    s[SCALAR_WINNERS_BREAD_ATTACK] = pl.winners_bread_buff.0 as f32;
    s[SCALAR_WINNERS_BREAD_HEALTH] = pl.winners_bread_buff.1 as f32;
    let (mut na, mut nh) = (0, 0);
    for &(a, h, times) in &pl.next_turn_board_buffs {
        na += a * times as i32;
        nh += h * times as i32;
    }
    s[SCALAR_NEXT_TURN_BUFF_ATTACK] = na as f32;
    s[SCALAR_NEXT_TURN_BUFF_HEALTH] = nh as f32;
    for b in &pl.tavern_buffs {
        let filter = b.tribe.map_or(0, |t| t as usize + 1);
        let k = SCALAR_TAVERN_BUFFS + filter * 4 + if b.this_turn { 2 } else { 0 };
        s[k] += b.attack as f32;
        s[k + 1] += b.health as f32;
    }
    s[SCALAR_LAST_SPELL] = pl.last_spell.map_or(0.0, |c| c as f32 + 1.0);
    for (j, t) in Tribe::EVERY.into_iter().enumerate() {
        s[SCALAR_LOBBY + j] = game.lobby.has(t) as u8 as f32;
    }
    s[SCALAR_GEM_ATTACK] = pl.gem_attack as f32;
    s[SCALAR_GEM_HEALTH] = pl.gem_health as f32;
    s[SCALAR_BARRAGE_ATTACK] = pl.barrage.0 as f32;
    s[SCALAR_BARRAGE_HEALTH] = pl.barrage.1 as f32;
    s[SCALAR_REFRESH_ATTACK] = pl.refresh_buff.0 as f32;
    s[SCALAR_REFRESH_HEALTH] = pl.refresh_buff.1 as f32;
    s[SCALAR_UNDEAD_ATTACK] = pl.undead_bonus.0 as f32;
    s[SCALAR_UNDEAD_HEALTH] = pl.undead_bonus.1 as f32;
    s[SCALAR_KNIGHT_DEATHS] = pl.knight_deaths as f32;
    s[SCALAR_ELEMENTALS_PLAYED] = pl.elementals_played_turn as f32;
    s[SCALAR_GOLDEN_PLAYED] = pl.golden_played as f32;
    s[SCALAR_GOLD_SPENT] = pl.gold_spent_turn as f32;
    s[SCALAR_DISCARDED] = pl.discarded as f32;
    s[SCALAR_SPELLS_CAST] = pl.spells_cast as f32;
    s[SCALAR_TRAILBLAZER_COVERS] = pl
        .board
        .iter()
        .filter(|m| db.get(m.card).choose_one_cover > 0)
        .map(|m| m.uses_left)
        .fold(0u8, u8::saturating_add) as f32;
    s[SCALAR_SHARING_IS_CARING] = pl.sharing_is_caring as f32;
    s[SCALAR_LAST_DAMAGE] = pl.last_damage as f32;
    write_deity(&mut s[SCALAR_DEITY..], db, pl.deity);

    // The card a pending choice comes from: the source minion, or for a
    // spell (no source) the spell just cast.
    let card_at = |src: Option<u8>| match src {
        Some(s) => pl.board.get(s as usize).map_or(0.0, |m| m.card as f32 + 1.0),
        None => pl.last_spell.map_or(0.0, |c| c as f32 + 1.0),
    };
    if let Some(pt) = pl.pending_target {
        let k = if pt.kind.takes_shop() { SCALAR_PENDING_TARGET_SHOP } else { SCALAR_PENDING_TARGET };
        s[k] = 1.0;
        s[SCALAR_PENDING_SOURCE] = pt.source.map_or(0.0, |x| x as f32 + 1.0);
        s[SCALAR_PENDING_CARD] = card_at(pt.source);
        s[SCALAR_PENDING_RUNS] = pt.runs as f32;
        let (kind, (a, h)) = target_kind(pt.kind);
        s[SCALAR_TARGET_KIND + kind] = 1.0;
        s[SCALAR_PENDING_ATTACK] = a as f32;
        s[SCALAR_PENDING_HEALTH] = h as f32;
    } else if let Some(pst) = pl.pending_shop_target {
        s[SCALAR_PENDING_SHOP_SLOT] = 1.0;
        s[SCALAR_PENDING_SOURCE] = pst.source as f32 + 1.0;
        s[SCALAR_PENDING_CARD] = card_at(Some(pst.source));
        match pst.kind {
            ShopTargetKind::Consume { remaining, .. } => {
                s[SCALAR_SHOP_CONSUME_LEFT] = remaining as f32;
            }
            ShopTargetKind::Fishbait { .. } => s[SCALAR_SHOP_FISHBAIT] = 1.0,
        }
    } else if let Some(pco) = pl.pending_choose_one {
        s[SCALAR_PENDING_CHOOSE_ONE] = 1.0;
        s[SCALAR_PENDING_SOURCE] = pco.source.map_or(0.0, |x| x as f32 + 1.0);
        s[SCALAR_PENDING_CARD] = card_at(pco.source);
        for (j, &opt) in pco.options.iter().enumerate() {
            let (kind, (a, h)) = choose_option(opt);
            let base = SCALAR_CHOOSE_ONE + j * CHOOSE_FEATURES;
            s[base + kind] = 1.0;
            s[base + 4] = a as f32;
            s[base + 5] = h as f32;
        }
    } else if pl.pending_discard.is_some() {
        s[SCALAR_PENDING_DISCARD] = 1.0;
    } else if !pl.discover.is_empty() {
        s[SCALAR_PENDING_DISCOVER] = 1.0;
        s[SCALAR_DISCOVER_LOCKED] = pl.discover_locked as u8 as f32;
        s[SCALAR_DISCOVER_DAMAGE] = pl.discover_damage_from.is_some() as u8 as f32;
        s[SCALAR_DISCOVER_DOOMED] = pl.discover_doomed as u8 as f32;
        s[SCALAR_DISCOVER_BOTH] = pl.discover_both as u8 as f32;
    }
    s[SCALAR_DISCOVER_QUEUE] = pl.discover_queue.len() as f32;

    s[SCALAR_ARMOR] = pl.armor as f32;
    s[SCALAR_HERO] = pl.hero.map_or(0.0, |h| h as f32 + 1.0);
    for (second, base) in [(false, SCALAR_POWER_HERO), (true, SCALAR_POWER2_HERO)] {
        let Some(st) = pl.power_at(second) else { continue };
        s[base] = st.of as f32 + 1.0;
        // What the press costs right now, its running changes included.
        s[base + 1] = game.power_cost(p, second) as f32;
        s[base + 2] = st.uses_left as f32;
        s[base + 3] = st.used_game as f32;
        s[base + 4] = st.unlocked as u8 as f32;
        s[base + 5] = game.power_usable_at(p, second) as u8 as f32;
        let (progress, left) = power_progress(game, p, &st);
        s[base + 6] = progress;
        s[base + 7] = left;
    }
    s[SCALAR_PENDING_HERO] =
        (game.turn == 0 && pl.hero_offer.iter().any(|&h| h != u8::MAX)) as u8 as f32;
    s[SCALAR_PENDING_POWER] = (!pl.power_offer.is_empty()) as u8 as f32;
    for (k, &of) in pl.power_offer.iter().enumerate() {
        s[SCALAR_POWER_OFFER + k] = of as f32 + 1.0;
    }
    for (k, &h) in pl.hero_offer.iter().enumerate() {
        s[SCALAR_HERO_OFFER + k] = if h == u8::MAX { 0.0 } else { h as f32 + 1.0 };
    }
    for slot in 0..crate::TRINKET_SLOTS {
        let st = pl.trinkets[slot];
        s[SCALAR_TRINKET + slot] = st.of as f32;
        s[SCALAR_TRINKET_COUNT + slot] = st.count as f32;
        s[SCALAR_TRINKET_TRIBE + slot] = st.tribe.map_or(0, |t| t as usize + 1) as f32;
    }
    s[SCALAR_PENDING_TRINKET] = (!pl.trinket_offer.is_empty()) as u8 as f32;
    s[SCALAR_TRINKET_OFFER_SLOT] = pl.trinket_offer_slot as f32;
    for (k, &(of, price)) in pl.trinket_offer.iter().enumerate() {
        s[SCALAR_TRINKET_OPTION + k * TRINKET_OPTION_FEATURES] = of as f32 + 1.0;
        s[SCALAR_TRINKET_OPTION + k * TRINKET_OPTION_FEATURES + 1] = price as f32;
    }
    // The Dark Discovery button: uses left, pressed this turn, and whether it
    // can be pressed now (its legality is `is_legal`'s, asked the same way the
    // action mask asks it).
    s[SCALAR_DARK_GIFT_USES] = pl.dark_gift_uses as f32;
    s[SCALAR_DARK_GIFT_PRESSED] = pl.dark_gift_pressed as u8 as f32;
    s[SCALAR_DARK_GIFT_READY] = (game.turn >= r.dark_gift_turn as u32
        && pl.dark_gift_uses > 0
        && !pl.dark_gift_pressed
        && pl.gold >= r.dark_gift_cost
        && !game.data.gifts.is_empty()) as u8 as f32;
    s[SCALAR_DARK_OFFER] = pl.dark_offer as u8 as f32;
    s[SCALAR_PARADOX] = game.paradox.map_or(0.0, |c| c as f32 + 1.0);
    s[SCALAR_FREE_MINION_LEFT] = pl.free_minion_left as f32;
    s[SCALAR_CONVICTION_COUNT] = pl.conviction.0 as f32;
    s[SCALAR_CONVICTION_ATTACK] = pl.conviction.1 as f32;
    s[SCALAR_CONVICTION_HEALTH] = pl.conviction.2 as f32;
    if let Some(m) = &pl.locked_and_loaded {
        s[SCALAR_LOCKED_CARD] = m.card as f32 + 1.0;
        s[SCALAR_LOCKED_ATTACK] = m.attack as f32;
        s[SCALAR_LOCKED_HEALTH] = m.health as f32;
    }
    if let Some((a, b)) = pl.wager_offer {
        s[SCALAR_PENDING_WAGER] = 1.0;
        s[SCALAR_WAGER_OFFER] = a as f32 + 1.0;
        s[SCALAR_WAGER_OFFER + 1] = b as f32 + 1.0;
    }
    s[SCALAR_WAGER_PICK] = pl.wagers.first().map_or(0.0, |&(q, _)| q as f32 + 1.0);
    if let Some((opts, _)) = pl.holmes_offer {
        s[SCALAR_PENDING_HOLMES] = 1.0;
        for (k, c) in opts.into_iter().enumerate() {
            if c == u16::MAX {
                s[SCALAR_HOLMES_EMPTY + k] = 1.0;
            } else {
                s[SCALAR_HOLMES_OFFER + k] = c as f32 + 1.0;
            }
        }
    }
    // Block 5: a held Ice Block and this turn's Immunity, the Prize tier the
    // open Discover is offering (the prizes themselves are in the Discover's
    // own card ids, like every other option) and Corrupted Tome's Triple
    // Prize tier.
    s[SCALAR_ICE_BLOCK] = pl.secrets.ice_block() as u8 as f32;
    s[SCALAR_IMMUNE] = pl.immune as u8 as f32;
    // The Secret zone: which of the four combat Secrets are held, and the
    // Secrets an open Prestidigitation offers (kind + 1, 0 for an empty slot).
    for k in 0..crate::secrets::COMBAT_SECRETS as usize {
        s[SCALAR_SECRET + k] = pl.secrets.has(k as u8) as u8 as f32;
    }
    for (k, &kind) in pl.secret_offer.iter().enumerate() {
        s[SCALAR_SECRET_OFFER + k] = kind as f32 + 1.0;
    }
    s[SCALAR_PRIZE_OFFER_TIER] = pl
        .discover
        .iter()
        .find_map(|&c| db.get(c).prize)
        .unwrap_or(0) as f32;
    s[SCALAR_TRIPLE_PRIZE] = pl.tp.triple_prize.map_or(0.0, |_| 1.0);
}

/// Write player `p`'s view into `out`, which must hold OBS_SIZE values.
pub fn write(game: &Game, p: usize, out: &mut [f32]) {
    assert_eq!(out.len(), OBS_SIZE);
    let pl = &game.players[p];
    let db = &game.data.cards;
    write_scalars(game, p, &mut out[..SCALARS]);
    let on_board = Owner {
        health: pl.health,
        beast_bonus: (0, 0),
        tier: pl.tier,
        gold_spent: pl.counts.gold_spent,
    };
    let off_board = Owner {
        health: pl.health,
        beast_bonus: pl.beast_bonus,
        tier: pl.tier,
        gold_spent: pl.counts.gold_spent,
    };
    let mut i = SCALARS;
    i += write_slots(game, &mut out[i..], SHOP, pl.shop.iter(), off_board);
    out[i..i + SHOP_SPELL * SLOT_FEATURES].fill(0.0);
    if let Some(sp) = pl.shop_spell {
        let m = pl.make_card(db, sp, false, 0);
        write_slot(game, &mut out[i..i + SLOT_FEATURES], &m, off_board);
    }
    i += SHOP_SPELL * SLOT_FEATURES;
    i += write_slots(game, &mut out[i..], HAND, pl.hand.iter(), off_board);
    i += write_slots(game, &mut out[i..], BOARD, pl.board.iter(), on_board);
    out[i..i + DISCOVER * SLOT_FEATURES].fill(0.0);
    for (k, &c) in pl.discover.iter().take(DISCOVER).enumerate() {
        let pool_copies = u8::from(!db.get(c).is_spell());
        let mut m = pl.make_card(db, c, false, pool_copies);
        // A Dark Gift offer: the gift it comes with, so the pick is made from
        // what the offer shows (the gift lands when the minion is taken).
        if pl.dark_offer && let Some(g) = pl.discover_gifts.get(k) {
            m.push_gift(*g);
        }
        let o = &mut out[i + k * SLOT_FEATURES..i + (k + 1) * SLOT_FEATURES];
        write_slot(game, o, &m, off_board);
    }
    i += DISCOVER * SLOT_FEATURES;
    for k in 1..PLAYERS {
        let q = (p + k) % PLAYERS;
        let them = &game.players[q];
        let seen = &pl.seen[q];
        let is = |o: Option<Opponent>| match o {
            Some(Opponent::Player(x)) | Some(Opponent::Ghost(x)) => x as usize == q,
            None => false,
        };
        let o = &mut out[i..i + OPP_SCALARS];
        o.fill(0.0);
        o[OPP_ALIVE] = them.alive as u8 as f32;
        o[OPP_HEALTH] = them.health as f32;
        o[OPP_TIER] = them.tier as f32;
        o[OPP_SEEN] = (seen.turn > 0) as u8 as f32;
        o[OPP_TURNS_SINCE] = if seen.turn > 0 { (game.turn - seen.turn) as f32 } else { 0.0 };
        o[OPP_NEXT] = is(pl.opponent) as u8 as f32;
        o[OPP_LAST] = is(pl.last_opponent) as u8 as f32;
        o[OPP_PLACEMENT] = them.placement as f32;
        for m in &seen.board {
            let c = db.get(m.card);
            for (j, t) in Tribe::EVERY.into_iter().enumerate() {
                o[OPP_TRIBES + j] += c.tribes.has(t) as u8 as f32;
            }
        }
        write_deity(&mut o[OPP_DEITY..], db, seen.deity);
        o[OPP_GEM_ATTACK] = seen.gem.0 as f32;
        o[OPP_GEM_HEALTH] = seen.gem.1 as f32;
        o[OPP_SPELL_ATTACK] = seen.spell.0 as f32;
        o[OPP_SPELL_HEALTH] = seen.spell.1 as f32;
        o[OPP_HERO] = them.hero.map_or(0.0, |h| h as f32 + 1.0);
        o[OPP_ARMOR] = them.armor as f32;
        // Their trinkets are visible (DESIGN.md: every player sees theirs).
        for slot in 0..crate::TRINKET_SLOTS {
            o[OPP_TRINKET + slot] = them.trinkets[slot].of as f32;
        }
        i += OPP_SCALARS;
        // A locked card stays in its owner's hand, so their board slots are
        // never locked; their tier is the one we last saw.
        let owner = Owner { health: them.health, beast_bonus: (0, 0), tier: seen.tier, gold_spent: 0 };
        i += write_slots(game, &mut out[i..], BOARD, seen.board.iter(), owner);
    }
    debug_assert_eq!(i, OBS_SIZE);
}
