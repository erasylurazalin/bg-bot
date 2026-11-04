# How trinkets work

Era's rules for trinkets: the offer and the trinkets whose card text
leaves room. They carry the same weight as `docs/rules/design.md` Part 2.
The rules still being guessed at are in a private notes file
(`docs/notes/UNSURE.md`), and so is the log evidence behind the offer and
discount rules.

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
  `docs/rules/heroes.md`, Ysera, for the exceptions). **Glowing Gauntlet** makes
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
  Balinda work with it. Corrected from logs (2026-11-04): the minion the
  copy lands on does NOT fire its own "whenever you cast a spell on this /
  on a Mech" (`Hearthstone_2026_11_02_00_10_22-g2` turn 12: Gem copies on a
  Glambot give +1/+1 and no Satellite, its Magnetize count never moves).
  Checked in the raw log the same day: nothing answers the copy at all. No
  trigger block of any kind follows any of the game's 140 copies (no
  Glambot, no Felboar "after you cast 3 spells": 7 Felboar triggers for the
  turn's 7 real casts, none for its 6 copies), and no cast counter moves.
  The Locket itself fires once per cast, so each extra cast a Hot-Air
  Surveyor or Balinda adds brings its own copy. The random pick ignores the spell's own target
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
- **Chillmere Mosaic (Era, 2026-11-03):** its Spellcraft Refresh deals
  only Battlecry minions, and nothing can step into that Refresh: no Fodder
  refill, no arrivals, no other card joining it. The Battlecry minions it
  deals cost (1); the Tavern spell keeps its normal price.
- **Snarky Portrait:** whenever a Fishbait dies, every friendly Beast gets
  +4/+4, and the amount improves by +4/+4 per Fishbait death. On top, the
  Fishbait's own Deathrattle gives its killer (the left-most Beast) +5/+5,
  or +10/+10 golden (Era, 2026-11-03).
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
build it), Wisdomball Supply and Putricide Sticker.

Built in block 5 (2026-11-01): the Darkmoon Prizes (31 Prize spells and the
Prize minion All-Seeing Elder, `CardDb.prizes` and a `prize` tier on each
card), Corrupted Tome, Tickatus Sticker, Tickatus and his Prize Wall, the
greater Eternal Portrait, the Curator Sticker, Safety Patch (tagged as a
Secret), the Bronzebeard Portrait with its tribe change, the Surprise
Portrait's hidden part, Yogg-Tastic Pastry and the Mecha-Jaraxxus Sticker.

## Portraits that rewrite a card

Czarina, Mindslasher, Flaming, Azerite and Bronzebeard Portraits, Rewinder,
Eternal, Trailblazer, Souvenir Stand: the change applies to every copy of
that card you have or will have, including the ones already on the board
and in hand.

Built 2026-10-31 (block 4): every portrait but Bronzebeard and Eternal (the
Eternal Portrait's Taunt and Reborn are a Start of Combat effect, built with
the fight block). The change is data on the player, keyed by card
(`docs/engine.md` "The portraits that rewrite a card").

Built 2026-11-01 (block 5): the Bronzebeard Portrait's "also Murlocs and
Dragons" is a card modifier like the others, `also_tribes`, and it is read by
`CardDb::tribes_of` through a `Minion.extra_tribes` field: every tribe check
in the engine sees it (board, hand, the most common type, tribe synergies, a
golden Brann, a fight), with the one exception Era named, the Dark
Discovery's "your type" slot. The Eternal Portrait (Greater) is the greater
card (BG36_MagicItem_216), with a Golden Eternal Knight and +4/+2 for every
friendly Undead death.

## Dark Gifts

See `docs/rules/darkgifts.md`.

## Block 5 corrections (Era, 2026-11-01)

- **Rat in a Cage and The Bouncer** are spells that target a friendly minion, so Balinda Stonehearth casts them twice: the doubling happens twice, which quadruples the stat.
- **New Recruit**'s +2/+2 is the same buff Waveling and Easterly Winds give (after each Refresh, a random Tavern minion gets it). They share one buff, so they stack. The extra Tavern slot stays for good.
- **Evolving Tavern** replaces every card in the Tavern, the spell included, with one a Tier higher, capped at Tier 6.
- **Golden Mishmash** also copies what the Amalgam gains in combat. Like any combat gain, it's gone by the next shop phase, unless Mishmash is next to a Persistent Poet: Mishmash is every type, so it counts as a Dragon and keeps them.

## Block 6 rules (Era, 2026-11-01)

- **Colorful Compass:** the Lesser one gives 1 random minion of its type on pick and at the start of each turn, and the Greater one gives 2.
- **The type is fixed at the pick.** It's your most common minion type at the moment you pick the trinket, and it never changes after. Take the compass on a board of Mechs, rebuild into Demons, and you still get Mechs. Gold-plated Compass works the same way.
- **Gold-plated Compass:** the type in "the next {type} you buy" is your most common minion type.
- **Guiding Candle** ("your first two Refreshes each turn contain only Tier 6 cards"): the Refresh still has its Tavern spell, and the spell is Tier 6 too. A Fodder that feeds later in the turn refills with Tier 6 while the candle has Refreshes left (Era, 2026-11-04).
- **Gold-plated Compass** gives a Triple Reward when its golden is played (THE GOLDEN RULE; Era confirmed 2026-11-04).
- **Minion Bait:** the {type} would be your most common type too, but the trinket never appears in Era's 47 logged games, so it's left out.
- **Poet Portrait:** gives a Timewarped Poet, a trinket-only token. It works like Persistent Poet, but for all your Dragons.
- **Wisdomball Supply and Putricide Sticker:** out for now.
- The game data has `92` (the Naga race id) in these cards' text slot. The live game rewrites it while the trinket is offered: logs show 17 (Mech), 43 (Quilboar), 126 and 0.

Built in block 6 (2026-11-01): the Poet Portrait with its Timewarped Poet
token (`permanent_keep = "all_dragons"`), the lesser and greater Colorful
Compass and the Gold-plated Compass. The type they read at the pick is
`TrinketState.tribe` (`lock_type` in trinkets.toml, docs/engine.md "The
compasses and the Poet Portrait (block 6)"). Minion Bait, Wisdomball Supply,
Putricide Sticker, Kiri, Shrine and the Naga and Timewarp trinkets are not
built.

