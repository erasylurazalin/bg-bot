# How trinkets work

Era's rules for trinkets: the offer and the trinkets whose card text
leaves room. They carry the same weight as `DESIGN.md` Part 2. Rules we
are still guessing at are in `docs/UNSURE.md`. The log evidence behind the
offer and discount rules is in `docs/briefs/trinket-facts.md`.

## The offer

A lesser trinket is offered at the start of turn 6 and a greater one at
the start of turn 9 (Marin and Buttons get theirs earlier, from their
powers). 4 options, pick 1, pay its cost. You can't decline, and you can
always afford one. A trinket with no cost in the game data is free.

- Tribe trinkets are only offered when their tribe is in the lobby.
- **The guarantee:** with enough minions of one tribe, the offer is
  guaranteed to hold that tribe's trinket: 2 or more for a lesser offer, 3
  or more for a greater one. When two tribes qualify, one of them (at
  random) is guaranteed.
- **What counts:** Murlocs count on the board and in hand together (a
  Winterfinner on the board plus a Flighty Scout in hand is 2). Every other
  tribe counts the board only: players keep a second minion of a tribe in
  hand on purpose to avoid its trinkets.
- **The other options** are mostly neutral or of your tribes, rarely of a
  lobby tribe you don't play (Era's logs: about half neutral, 42% your
  tribes, 7% others). They lean toward trinkets that match your board's
  mechanics (Battlecries, Deathrattles, spells cast).
- **The discount is not random.** A tribe trinket whose tribe doesn't
  reach the threshold costs 2 Gold less (never below 0). A trinket of a
  qualifying tribe, and a neutral trinket, keep their price. In short: the
  trinket the game thinks you want is full price, the half-related ones are
  cheap (fits 77 of 82 offers in Era's logs).
- Two players can be offered the same trinket. Every player sees their own
  trinkets and their opponents'.
- Lesser and greater versions of one trinket are separate cards.

## Gold, Health and the Tavern

- "Whenever you spend Gold" fires once per spending, whatever the amount:
  a 1 Gold Refresh and a 3 Gold buy each fire it once.
- Paying with Health is not spending Gold. It is self-damage: it counts for
  the Demons that care about self-damage, and Soul Rewinder or Ashen
  Corruptor can rewind it.
- A free buy (Warcry Totem) costs 0: no Gold spent, so "spend Gold"
  triggers don't fire. It is still a buy.
- Deaths in the shop count for death counters (Butchering, Mutanus...).
- The Tavern never has more than 7 slots, one of them the spell (see
  `docs/heroes.md`, Ysera, for the exceptions). **Glowing Gauntlet** makes
  it 6 minions and 1 spell, and refills at once: a bought or consumed
  minion is replaced by a minion, a bought spell by another spell.

## Trinkets that replace trinkets

- **Orb of the Unknown:** its random trinket replaces your trinket of that
  slot.
- **Trip Vouchers** (bought on turn 6): a greater offer on turn 8, and the
  normal greater offer on turn 9 still comes.
- **Mystery Cube:** when you get it, choose 1 of 2 random lesser trinkets
  (free) to replace it. The cube's effect stays with the slot: at the start
  of every turn for the rest of the game, you choose 1 of 2 new lesser
  trinkets to replace the one you hold. You can't keep the current one.
  The held trinket's own start-of-turn effect fires first (the Pendant, for
  example), then the swap. Some trinkets are never offered by the cube:
  `cube = false` in `trinkets.toml` (one-time effects, Orb of the Unknown
  and the like). Era may add to that list.

## Copies and cards given

- A copy a trinket gives is an exact copy, buffs included (Transcribing
  Typewriter). Players buff a Tavern minion first, then buy it. A copy of a
  golden minion is golden and gives its own Triple Reward when played (with
  Golden Touch, two goldens give two Triple Rewards).
- **Spellcraft:** a Spellcraft spell (from some trinkets and a Dark Gift)
  is temporary. It leaves your hand at the end of the turn if you haven't
  cast it.
- **Minions only trinkets make:** Tide Oracle Morgl, Arcane Behemoth, Dr.
  Boom's Monster, Bassgill, Faceless Manipulator and Doubloon Grifter.
  They are generated cards with no pool copies, like Beetles.
- Numbers a trinket sets while playing (Shark Cannon, Bewitched Ribbon,
  Steady Growth) come from Era's logs.

## Portraits that rewrite a card

Czarina, Mindslasher, Flaming, Azerite and Bronzebeard Portraits, Rewinder,
Eternal, Trailblazer, Souvenir Stand: the change applies to every copy of
that card you have or will have, including the ones already on the board
and in hand. Not built yet.

## Dark Gifts

Not built yet. What we know: a gift sits on its minion like an attached
card. A triple's golden keeps the gifts of its copies, a Magnetized Mech
keeps them, and a sold minion takes them with it. A gift works the same on
a golden minion (it doesn't double). Era has a source that explains the
rest.
