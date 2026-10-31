# How Dark Gifts work

Era's rules and sources for Dark Gifts. Same weight as `DESIGN.md` Part 2.
Not built yet. The evidence from the game data and Era's logs is in
`docs/briefs/darkgift-facts.md`.

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

The star counts in the picture are small; the table above is Claude's
reading of it, to be checked against the logs (the button's own tier
tags).

The pool the offers come from:
- No Battlecry or Choose One minions before turn 5.
- No Magnetic minions.
- No minions with "When you sell this".
- No minions whose effect works while they sit in hand.
- Some minions are left out for balance, for example Leeroy the Reckless
  and Deadly Spore.
