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

## Rules for single trinkets (Era, 2026-10-31)

- **Dragonwing Glider:** any card played from hand counts, Blood Gems and
  Bananas included.
- **War Drum:** the first Battlecry you trigger each turn triggers two
  extra times.
- **Dragon Skull:** fires once per Battlecry trigger, so a Battlecry
  doubled by Brann fires it twice.
- **Bluegill Flippers:** two minions: the left-most in hand and the
  left-most on the board.
- **Lovely Locket:** after you cast a spell on a friendly minion, it casts
  the same spell on another friendly minion, picked at random. The copy is
  a real cast: it counts for cast triggers, and Hot Air Surveyor and
  Balinda work with it. The random pick ignores the spell's own target
  rules, which makes it wild: Eyes of the Earth Mother cast on a Tier 4
  minion can land on a Tier 5 to 7 one and make it golden; Butchering or
  Channel the Devourer can eat your board; a spell that fills the Tavern
  with the target's tribe, cast on a Beast, can land on a neutral minion
  and fill the Tavern with neutrals (a normal cast can't target a neutral
  there, the Locket can).
- **Young Murk-Eye Sticker, Hackerfin Portrait** (Battlecries at end of
  turn): a Battlecry that needs a target picks one at random, and that can
  be a Tavern minion. Brann and Drakkari both add triggers, so they can
  fire many times.
- **Insurrectionist's Blade** (all friendly Rallies at end of turn): a
  Rally that needs the minion it attacks does nothing in the shop. Drakkari
  adds a trigger.
- **Trickster's Sleeve:** keeps alternating for the rest of the game: after
  a minion buy, then after a Tavern spell buy, then a minion again.
- **Inductive Gyroblade:** the Satellite grows +4/+4 per Tavern spell cast
  this turn (Greater: +12/+12).
- **Dalaran Cheese Wheel, Nether Pendant:** each improvement adds +1/+1
  (Greater: +2/+2).
- **Hammer of Twilight:** +1 Attack per card discarded (Greater: +2/+1),
  counting discards from before you got it.
- **Weighted Gauntlet:** doubles with no cap (+1, +2, +4...).
- **Honeycomb Ring:** +1/+1 more this turn per spell cast on a minion.
  **Multilayered Shield:** +1/+2 per different minion type on your board.
- **Snarky Portrait:** improves by +4/+4 per Fishbait death.
- **Soul Fermenter:** the 3 destroyed minions are real deaths: Deathrattles,
  Reborn and every other death trigger fire. After your last minion dies
  they come back exactly as they were at Start of Combat.
- **Jewelry Box:** a random one of the three special Blood Gems (Taunt,
  Divine Shield, Reborn) each turn. On a minion that isn't a Quilboar it's
  a plain Blood Gem.
- **Dragonhead Staff, Ruby Tusk:** a Windfury minion's two attacks count
  2. The trinket they become is a random Greater trinket of that tribe,
  free.
- **Reinvigorating Light:** Discover from 3 random starting powers of the
  heroes we've built, never your own. Powers that make no sense as a second
  power are excluded (A. F. Kay's, Faelin's, Kerrigan's and the like); Era
  will add to the list.

**Rules for the last trinket batch (Era, 2026-11-01):**
- **Eternal Portrait (Greater):** "+4/+2 this game" reaches every Eternal
  Knight you have or will have, in hand too.
- **Bronzebeard Portrait:** your Branns count as Murlocs and Dragons
  everywhere (board, hand, "your most common type", tribe synergies, a
  golden Brann), except Dark Discovery's "your type" slot.
- **Curator Sticker:** Golden Mishmash ("Whenever your Amalgam gains
  stats, this gains them twice") and a 10/10 Venomous Amalgam. Mishmash is
  a trinket-only token tagged as a buddy, for when buddies come.
- **Safety Patch:** Ice Block, "When your hero takes fatal damage, prevent
  it and become Immune this turn", once. Tagged as a Secret for
  Akazamzarak later. Not hidden: everyone sees everyone's trinkets live.
- **Darkmoon Prizes:** 32 cards in the game data, 4 tiers
  (`battlegroundsDarkmoonPrizeTurn` in `ref/hsjson-cards.json`). A Triple
  Prize (Corrupted Tome) is "Discover a Tier 3 Darkmoon Prize", nothing
  more. Tickatus Sticker: a Tier 3 Prize, every 3 turns. Tickatus' hero
  power Prize Wall: Tier 1 on turn 4, Tier 2 on turn 8, Tier 3 on turn 12,
  Tier 4 on turn 16 and every 4 turns after (a guess past 16, games rarely
  get there).
- **Surprise Portrait** ("More appear when you least expect them..."):
  nobody knows the real odds. Model: every buy, sell and Refresh has a 5%
  chance of a surprise, never twice in a row. On a buy or a sell, you get
  an Elemental of Surprise. On a Refresh, either one Elemental of Surprise
  is added to the Tavern, or all 7 slots are filled with them.
- **Yogg Wheel:** its results are cards in the data (`TB_BaconShop_HERO_35_Buddy_t*`).
  Mysterybox there is "a random Tavern minion turns golden" (the other
  Mysterybox, "your Hero Power triggers an extra time", isn't on the
  Wheel). Yogg-Tastic Pastry is built anyway: the Wheel's six results
  (Mysterybox, Hand of Fate, Curse of Flesh, Devouring Hunger, Rod of
  Roasting, Mindflayer Goggles), each equally likely (a guess).
- **Mecha-Jaraxxus Sticker:** the four Magnetic Mecha-Demons are tokens in
  the data (`BG25_807t` to `t4`): Rusted Reggie 5/5 Windfury, Magtheridon
  Prime 1/10 Taunt, Baltharak 10/1 Reborn, Shield of the Legion 7/3 Divine
  Shield. They can Magnetize to Mechs and to Demons. "Random" is one of the
  four, equally likely.

**Not in the game, or later:** Shrine of Evolution (Era thinks it's gone,
though the data still lists it), Kiri's Double Eclipse (never seen in
Era's games or pro play, though the data lists it in the pool: never
build it), Wisdomball Supply, Safety Patch (Secrets), Yogg-Tastic
Putricide Sticker, Tickatus Sticker and Corrupted Tome (they need the
Darkmoon Prizes exported first), and the Dark Gift trinkets (built since).

## Portraits that rewrite a card

Czarina, Mindslasher, Flaming, Azerite and Bronzebeard Portraits, Rewinder,
Eternal, Trailblazer, Souvenir Stand: the change applies to every copy of
that card you have or will have, including the ones already on the board
and in hand.

Built 2026-10-31 (block 4): every portrait but Bronzebeard and Eternal (the
Eternal Portrait's Taunt and Reborn are a Start of Combat effect, built with
the fight block). The change is data on the player, keyed by card
(`docs/ENGINE.md` "The portraits that rewrite a card"). Bronzebeard Portrait
is not built at all and is not in `trinkets.toml`, so it is never offered:
its "also Murlocs and Dragons" needs a per-minion tribe override, which
reaches every tribe check in the engine, and half a portrait would mislead
the bots.

## Dark Gifts

See `docs/darkgifts.md`.
