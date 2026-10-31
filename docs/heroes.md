# How the non-obvious heroes work

Era's rules for heroes whose card text leaves room. They carry the same
weight as `DESIGN.md` Part 2. The general hero frame (the pick, health and
armor, what a power costs) is in `DESIGN.md` "Heroes". Rules we are still
guessing at are in `docs/UNSURE.md`.

**Built 2026-10-31 (hero frame 2):** everything on this page except the
guessing powers (Barov, Holmes, I Spy): the two powers (Genn, the
Reinvigorating Light trinket), the powers that change (Ragnaros, Bru'kan,
Galewing, the Rat King), the skipped turns (A. F. Kay, Faelin) and Mukla's
Bananas.

## Powers in general

- A button power works once per turn unless its text says otherwise. Uses
  vary from power to power: some are once per game, some twice per turn.
- **Two hero powers.** A player can end up with two buttons: a MAIN power
  and a SECOND power. Unmasked Identity changes only the main power.
  Nothing changes or disables the second one. A trinket that gives a hero
  power adds it as a second power.

## Two powers: Genn Greymane

King of Duality: on turn 4, Discover two hero powers. Genn ends with both,
two buttons. The order matters: the first power picked becomes the SECOND
power, and the second one picked becomes the MAIN power (the one Unmasked
Identity would change).

**Which powers a Discover of hero powers offers** (Genn, Reinvigorating
Light, Unmasked Identity, Sir Finley): random starting powers of the heroes
we've built, never your own, minus the ones that make no sense as a found
power. The list is Unmasked Identity's `exclude_heroes` in `cards.toml`
(Sir Finley, A. F. Kay, Faelin, Patchwerk), plus Kerrigan once StarCraft
heroes exist. Era will add to it.

## Skipped turns

**A. F. Kay** does nothing on turns 1 and 2. The Gold of those turns is
wasted. Freezing the Tavern is the only action besides ending the turn (no
selling, no moving). A. F. Kay still fights and takes damage. Turn 3 starts with two cards in hand: "Discover a Tier 3
minion" and "Discover a Tier 4 minion". On turn 3 the player has 5 Gold
and always levels to Tier 2 (cost 3), so players freeze a 2-Gold spell on
turns 1 and 2 to spend the 2 Gold left over.

**Faelin (Expedition Plans)** skips turn 1 the same way and usually loses
that fight. Its three Discovers, one each from Tiers 2, 4 and 6, happen at the start
of the game (the start of turn 1). Each minion
is locked in hand until the Tavern reaches its tier. A locked minion can
be buffed in hand (Winterfinner, for example) but can't be played or
summoned. The usual line: level to Tier 2 on turn 2, play the Tier 2
minion and win that fight (everyone else has Tier 1 minions), then rush
Tier 4 to unlock the next one.

## Powers that turn into other powers

**Ragnaros (BUY, INSECT!):** after buying 12 cards, the power becomes
Sulfuras ("At the end of your turn, give your left and right-most minions
+8/+8"). Every card bought counts toward the 12, spells and free buys included;
a card taken without buying ("steal") doesn't. Sulfuras already fires at
the end of the turn it arrives. With
one minion on the board, that minion gets both buffs: +16/+16.

**Bru'kan (Embrace the Elements):** choose one of four Elements (Earth,
Fire, Water, Lightning). It fires at Start of Combat of the next fight,
and then the button comes back: choose again, a different Element if you
like, for the next fight. The first choice can be made on turn 1. Earth
Invocation picks 4 different random friendly minions (one Deathrattle
each). Lightning Invocation hits 5 different random enemy minions, never
the same one twice, and a minion with 1 Health dies at once (like any
Start of Combat damage). Fire Invocation doubles the left-most minion's Attack at Start of Combat, buffs
included.

**Galewing (Dungar's Gryphon):** choose one of three flightpaths and
complete it for a bonus. After it completes, choose again, but not the one
just completed: the next choice is between the other two.

**King of Beasts, Rat King:** the tribe swaps every turn to a random
different one, only among the tribes in the lobby. It never stays the same
two turns in a row.

## The Tavern's shape

**Ysera (Dream Portal):** the extra Dragon at each Refresh takes a minion
slot. The spell slot is always there. The only exceptions known are the
Whistle Band trinket and a Tavern full of 7 Fodders.

**Stay Frosty** is still in the game.

## Guessing

**Lord Barov:** press the button, pick one of the fights about to happen
between two other heroes, and guess the winner. Never your own fight,
unless only two players are left.

**Murloc Holmes:** guess which minion was in a warband at the start of the
turn. If that board was empty, one of the options is "empty".

**I Spy:** Discover a plain copy of a minion from your next opponent's
warband as it was at the start of the turn. The button can't be pressed
if that board was empty.

## Bananas: Mukla

Bananarama gives Bananas to everyone, every turn: 2 to Mukla, 1 to every
other player. Each Banana is a card in hand, played on a minion: +1/+1, or
a Big Banana (+2/+2) with a 33% chance, the same chance for every player.
They are not the Tavern dish Banana, and they are spells but not Tavern
spells: "after you cast a spell" fires for them (Devilish Distractor),
"after you cast a Tavern spell" doesn't (Forsaken Weaver). See
`DESIGN.md` "Tavern spells versus other spells".

## Later, or never

- Later: Akazamzarak's Secrets (they are in the game), Denathrius' quests,
  Putricide's Build-An-Undead, E.T.C.'s buddies.
- Wanted, not built yet: Tickatus' Darkmoon Prizes (a trinket gives them
  too).
- Probably never: the StarCraft heroes (Raynor, Artanis, Kerrigan).
