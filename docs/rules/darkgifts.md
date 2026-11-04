# How Dark Gifts work

Era's rules and sources for Dark Gifts. Same weight as `docs/rules/design.md` Part 2.
Built 2026-10-31 (the table is `darkgifts.toml`, the code is
`crates/bgcore/src/darkgifts.rs` and `crates/bgcore/src/game/darkgifts.rs`,
see `docs/engine.md` "Dark Gifts"; Dark Paradox stays out). The evidence
from the game data and Era's logs is in a private notes file.

## What a gift is

A gift sits on its minion like an attached card. A triple's golden keeps
the gifts of its copies, a Magnetized Mech keeps them, and a sold minion
takes them with it. A gift works the same on a golden minion (it doesn't
double). The 47 gift cards and their current numbers are in
`ref/bg-meta.json` (`dark_gifts`), read from the game files.

## Dark Discovery: which minions it offers

Every player has the Dark Discovery button (3 Gold, 3 uses, unlocks on
turn 3). It offers three minions, each with a gift. Era's source (an
infographic from hs-manacost.ru, "minion tiers with Dark Gifts") gives the
tiers of the three offers by turn. "Yours" means a minion of your type
(your most common minion type).

| Turn | Offer 1 | Offer 2 | Offer 3 |
|---|---|---|---|
| 3 | Tier 2 | Tier 2 | Tier 2 |
| 4 | Tier 2 | Tier 2 | Tier 3 |
| 5 | Tier 3 | Tier 3 | Tier 3 |
| 6 | Tier 3, yours | Tier 4 | Tier 4 |
| 7 | Tier 4, yours | Tier 4 | Tier 4 |
| 8 | Tier 4, yours | Tier 5 | Tier 5 |
| 9 | Tier 4 or 5, yours | Tier 5 | Tier 6 |
| 10+ | Tier 5, yours | Tier 5 or 6, yours | Tier 5 or 6 |

Era's logs confirm this table on all 127 presses:
on turn 9 the first slot was Tier 4 four times and Tier 5 three times, and
on turn 10 the second and third were 5 or 6. The first slot is the
player's most common type on the board (41 of 43 presses from turn 6; the
hand doesn't count).

**The button (logs, 43 games):** costs 3 Gold, never changes, never
discounted. Once per turn. 3 uses per game. Locked until turn 3.

**The gifts on the offers (logs, 381 offers):** one gift per offered
minion, fixed before the pick and shown on the offer. The three gifts of
a press are always different, and so are the three minions. A gift is
offered only within its turn window (game data tags 4853 and 4854, first
and last turn; one exception in 369 offers). Gilding's offer is the golden
copy of the minion. Host conditions confirmed in the logs: Echoing Voice
(Battlecry), Jaws of Death (Deathrattle), Consanguinity (Quilboar),
Polarization (Mech), Tarecgosa's Blessing (Dragon), Demonology (Demon),
Time Turning (end-of-turn effect), Amalgamation (no type). Mystic Essence,
Fresh Perspective, Golemancy and the Sacrifices go on any minion.

**Numbers fixed when the gift lands:** Steady Growth's gain depends on the
turn it was offered: +2/+2 on turn 3, +2/+3 on turn 4, +3/+4 on turn 5.
Incubation, Replication and Affinity count down 2 turns. They are end of
turn effects (Era, 2026-11-02; the card text says "At the end of every 2
turns", and Era's log has Replication's copy in hand at the end of the
turn): the count goes down and the gift fires at the end of your turn.
Incubation is the exception in the logs (corrected 2026-11-04): its count
reaches 0 at the end of your turn, but the doubling lands as your next
turn starts, after the fight (`Hearthstone_2026_10_28_19_47_59-g1` end of
turn 7: Incubation's counter goes to 0 at 19:59:50 and the Hot-Air Surveyor
stays 12/16 through the fight, then doubles to 24/32 at 19:59:53 on the next
turn). Replication and Affinity pay at the end of the turn as written.
On the turn it fires, the payout runs as many times as the player's end of
turn effects do: Drakkari Enchanter sets that to 2 (3 golden; two Drakkari
still make 2), and Ghastly Sticker adds one on top (Drakkari plus Sticker:
3). Nothing doubles; see docs/rules/design.md "'Twice' versus 'an extra time'". It
never speeds up the count, which stays every 2 turns (Era, 2026-11-02).
After paying out, **Replication and Affinity stay on the minion and start
their 2-turn count again; Incubation does not restart** (it fires once and
is gone) (Era, 2026-11-03). Replication's copy is a plain copy (the base card), and if it is your
third copy the three merge at the start of your next turn, not at the end of
this one (docs/rules/design.md "Nothing triples during combat").

The pool the offers come from:
- No Battlecry or Choose One minions before turn 5.
- No Magnetic minions.
- No minions with "When you sell this".
- No minions whose effect works while they sit in hand.
- Some minions are left out for balance, for example Leeroy the Reckless
  and Deadly Spore.

## Era's rules (2026-10-31)

- The offered minions come from the shared pool, like any Discover: each
  takes a pool copy, and the two not picked go back.
- How a gift is picked for an offered minion: at random among the gifts
  whose turn window holds this turn and whose condition the minion meets,
  never a gift already on another offer of the same press. The four gifts
  with weight 50 in the game data (Persisting Horror, Invulnerability,
  Titanic Strength, Worship) are half as likely as the rest. The other
  weights (200) are not used.
- Conditions of the gifts that never showed up in Era's logs: Toxicity on
  Murlocs that aren't Venomous; Efficiency on Activate minions; Worship
  only when you have a Deity; Sunken Persistence on Spellcraft minions;
  Titanic Strength never on Dragons; Persisting Horror never on minions
  with no type; Invulnerability never on Taunt minions or minions with no
  type; Desecration on any minion. Admiration is probably never on
  Dragons (Era isn't sure).
- The other sources (Sinister Invitation, Ominous Stone, Wax Lance,
  Maldraxxus Dagger, Tras'tath, Xavius) use the same gift rules, with the
  minion rules from their own text. Tras'tath's Discover of a Tier 5
  minion picks its gifts as if it were turn 7.
- **Sinister Invitation never offers Gilding** (Era, 2026-11-03): its
  minion is already golden, so Gilding is not in its gift pool.
- **Gifts stack.** A triple's golden keeps every copy's gifts, duplicates
  included: three copies with Toughened Shield, Fresh Perspective and
  Toughened Shield give a golden with two Toughened Shields and one Fresh
  Perspective, and both Toughened Shields fire. Assume every gift stacks
  (two Jaws of Death trigger the Deathrattles twice) unless stacking means
  nothing: keyword gifts (Harpy's Talons, Furtiveness, Toxicity,
  Amalgamation) and gifts that set a state (Toreth's Blessing's three
  hits, Gilding) don't add up.
  Two more things show in the logs, and the engine follows them
  (2026-11-04): **countdown** gifts stack like the rest, and both counters
  pay (`Hearthstone_2026_10_26_00_48_15-g2` turn 7: the golden Cadaver
  Caretaker 4024 carries two Replication enchantments, 4025 and 4026, both
  counting 2, and its end of turn hands over two plain Cadaver Caretakers,
  5585 and 5587); and a keyword gift stays on a golden as its own
  enchantment beside the keywords
  (`Hearthstone_2026_10_24_15_25_26-g4` turn 10: the golden Eredar Escapist
  12267 carries the Harpy's Talons enchantment 12269, `EDR_100t13e`, with
  `IS_NIGHTMARE_BONUS` and Divine Shield and Windfury on it, the copy it
  came from carrying 12266 the same way). The log keeps **two** of a
  keyword gift on a golden as well (`Hearthstone_2026_10_28_02_28_32`: the
  golden `BG34_140_G`, entity 11176, with two Harpy's Talons enchantments,
  11177 and 11179, from two different gift cards); the engine keeps one,
  since two Harpy's Talons do the same thing ("don't add up" above), and
  that count is the one thing here Era has not called. Corrected from the
  raw `Power.log` on 2026-11-04: this paragraph used to say that a golden
  keeps one of two countdown gifts and no keyword gift at all, which was
  what the collector wrote rather than what the log shows.
- **Magnetic minions and gifts.** A Magnetic minion is never offered with a
  gift. The only way one gets a gift is Maldraxxus Dagger (a copy of a
  friendly minion). When a gifted Magnetic minion is Magnetized onto a
  Mech, its gift is lost (Era thinks so). The Mech it lands on keeps its
  own gifts.
- **Dark Paradox** (`BG36_360`, "This has a different Dark Gift, stats,
  and Tier each game!") is never offered by Dark Discovery. Built
  2026-10-31 as five pool cards, "Dark Paradox t3/t4/t5/t6/t9" in
  cards.toml (the game data's own card-id suffixes; `Game::paradox` is the
  version this game's pool holds, and `crates/bgcore/tests/darkgifts.rs`
  covers it). The game data has five versions, each a minion of every type
  with a built-in gift: Tier 2 2/6 (+3 Attack per card played), Tier 3 3/3
  (+3/+3 per Battlecry triggered), Tier 4 2/6 (Rally: a minion of your most
  common type), Tier 5 8/4 (Divine Shield that takes 3 hits), Tier 6 10/2
  (Deathrattle: a Golem with its stats). Era's logs show all five.
  Era (2026-10-31): one version per game, the same for every player; a
  normal Tavern minion of its version's tier with the usual pool copies;
  in every lobby. The golden versions are in the game data: Tier 5 golden's
  Divine Shield takes 4 hits (not double 3), and Tier 6 golden summons one
  Golem with double its stats (not two Golems).
