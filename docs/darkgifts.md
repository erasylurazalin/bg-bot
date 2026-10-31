# How Dark Gifts work

Era's rules and sources for Dark Gifts. Same weight as `DESIGN.md` Part 2.
Built 2026-10-31 (the table is `darkgifts.toml`, the code is
`crates/bgcore/src/darkgifts.rs` and `crates/bgcore/src/game/darkgifts.rs`,
see `docs/ENGINE.md` "Dark Gifts"; Dark Paradox stays out). The evidence from
the game data and Era's logs is in `docs/briefs/darkgift-facts.md`.

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

Era's logs confirm this table on all 127 presses (`docs/briefs/darkgift-facts.md`):
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
Incubation, Replication and Affinity count down 2 turns, ticking at the
start of your turn.

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
- **Gifts stack.** A triple's golden keeps every copy's gifts, duplicates
  included: three copies with Toughened Shield, Fresh Perspective and
  Toughened Shield give a golden with two Toughened Shields and one Fresh
  Perspective, and both Toughened Shields fire. Assume every gift stacks
  (two Jaws of Death trigger the Deathrattles twice) unless stacking means
  nothing: keyword gifts (Harpy's Talons, Furtiveness, Toxicity,
  Amalgamation) and gifts that set a state (Toreth's Blessing's three
  hits, Gilding) don't add up.
- **Magnetic minions and gifts.** A Magnetic minion is never offered with a
  gift. The only way one gets a gift is Maldraxxus Dagger (a copy of a
  friendly minion). When a gifted Magnetic minion is Magnetized onto a
  Mech, its gift is lost (Era thinks so). The Mech it lands on keeps its
  own gifts.
