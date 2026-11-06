# bg-bot design

Two parts. The first describes Hearthstone Battlegrounds as completely as I
can, as the reference for everything the engine will grow into. The second
is the game we build: the real tribes, cards and rules of the current patch,
with every rule I decided.

`docs/engine.md` maps the code, `docs/adding-cards.md` is the working
reference for adding cards.

Facts marked **(check)** are from memory and may be stale or wrong. Settle
them from Power.log (my tracker reads it) or by playing, before the engine
depends on them. Fight rules marked as checked come from a set of notes
where they were compared against real logs.

# Part 1: Battlegrounds

## The lobby

- 8 players, solo. Each picks a hero from a small offer (2 free, 4 with the
  Tavern Pass). The hero sets starting health, armor and a hero power.
- Health is 30 for most heroes, plus armor that differs by hero (stronger
  heroes get less) **(check the current values)**.
- Each lobby uses 5 of the 10 minion types: Beast, Demon, Dragon, Elemental,
  Mech, Murloc, Naga, Pirate, Quilboar, Undead. Neutral minions are always
  in. A minion with two types is in if either type is.
- **How to tell the lobby's 5 types from a log (Era, 2026-11-06):** the log
  never names them. A type is in the lobby when a minion of only that type
  shows up in the Tavern (a Demon-only minion in the shop means Demons are
  in). Minions with two types, "all types" minions and cards from outside
  the Tavern (Discovers, hero powers, generated cards) don't count. Checked
  on 2026-11-06 over boards/actions: exactly 5 types in 77 of 78 games; the
  78th is a 2-turn log fragment.
- Players are knocked out when their health reaches 0. Placement is the
  order of knockouts: the last player standing is 1st. Rating (MMR) goes up
  for good placements and down for bad ones, top 4 roughly counts as a win.

## A turn

Every turn has a shop phase (all 8 players at once, on a timer) and then a
fight phase.

### Gold

- Turn 1 gives 3 gold, then 1 more each turn, up to 10. Unspent gold is lost
  at the end of the turn.
- Buying a minion costs 3. Selling gives 1. Refreshing the shop (a roll)
  costs 1. Freezing is free.
- Some cards and hero powers change these (cheaper rolls, extra gold, spells
  with their own costs).

### Tavern tiers

- Tiers 1 to 6. Upgrading costs gold, and the cost goes down by 1 every turn
  you don't upgrade, to a minimum of 0 **(check)**. Base costs as I remember
  them: tier 2 costs 5, tier 3 costs 7, tier 4 costs 8, tier 5 costs 11, tier
  6 costs 11 (Era's logs, 2026-11-02: 44 of 44 level-ups to tier 5 show 11
  next; these change between patches). The countdown for the
  next tier starts after you upgrade.
- A higher tier shows more cards and lets the shop offer minions up to your
  tier. Shop size by tier: 3, 4, 4, 5, 5, 6 minions **(check)**, plus one
  spell slot in the current game.
- Tier 7 exists only through special effects (anomalies, some cards) and
  isn't in the normal game.

### The shop and the pool

- All players draw from one shared pool. Each minion has a fixed number of
  copies by tier, roughly 16, 15, 13, 11, 9, 7 for tiers 1 to 6.
- When the shop refreshes, unbought minions go back to the pool and new ones
  are drawn at random from what is left, among minions of your tier or lower.
  Fewer copies left means lower odds of seeing that minion.
  Every copy is equally likely, so a tier's odds are its share of the copies
  left: at Tavern tier 6 a slot is about 12% tier 1, 16% tier 2, 21% tier
  3, 22% tier 4, 20% tier 5 and 8% tier 6. Checked against Era's logs
  (2026-11-04: 851 Tavern minions at the start of tier 6 turns gave exactly
  those shares), and the engine's model predicts the same.
- The shop refreshes at the start of each turn and on every roll. A frozen
  shop keeps its minions for the next turn, and its empty slots fill up.
  Moving a minion on your board doesn't touch the freeze: the Tavern stays
  frozen (Era, 2026-11-04; the logs that show it unfrozen after a move are
  a log quirk).
- Selling a minion returns it to the pool. Tokens (summoned minions that
  aren't in the pool) just disappear.

### Hand and board

- Hand holds 10 cards, the board 7 minions. A bought minion goes to hand.
  Playing it puts it on the board at a position you choose and triggers its
  Battlecry.
- **A full hand (Era, 2026-11-02).** A card you get with 10 cards in hand,
  during your shop phase or in combat, is lost (Sky Admiral Rogers' Bounty;
  Blood Gems from a Roadboar that attacks 15 times). A card that arrives at
  the start of your turn (a compass trinket's tribe card and the like) is
  queued instead and joins your hand as soon as there is space.
  A triple is one action that takes all three copies, so a full hand still
  takes a board copy when the merge itself frees the slot the golden needs
  (log `10_24_04_08_00-g1` turn 10 step 25; 2026-11-04).
  With a copy in hand the merge always frees the slot it needs, so a hand
  is never too full for it. Three copies all on the board (a fight or a
  summon can leave them there) need a free hand slot for the golden: with a
  full hand they stay where they are and merge as soon as a hand slot opens
  (Era, 2026-11-04). The same goes for a triple held from a fight.
- The board order matters in the fight, and you can rearrange it freely
  during the shop phase.

### Triples

- Getting a third copy of the same minion (in hand or on board) merges the
  three into a golden version in your hand.
- A golden minion has double the printed stats plus the bonus stats of all
  three copies that made it. A damaged copy's damage doesn't carry: the game sums
  the copies and heals the golden (log `10_24_00_56_06-g1`, golden
  Plaguerunner 25/4 from a damaged copy; 2026-11-04). The triple check runs
  before a card the buy hands over (Transcribing Typewriter's copy joins
  after, and is checked again; log `10_26_00_48_15-g1` turn 6). Volumizers
  of different colours don't triple together (log `10_29_22_20_04-g3` turn
  10). A copy's own aura isn't one of its bonus stats: Holy Vanguard's
  "+30/+30 if you have 15 or less Health" stays out of the sum, and the
  golden gets its own once, at its doubled +60/+60 (log `10_28_22_11_50-g5`
  turn 10: golden Holy Vanguard 208/116, not 238/146; 2026-11-04). Holy Vanguard's
  +30/+30 is part of its stats wherever it is (board, hand, Tavern) while
  you have 15 or less Health: a Demon that eats a Tavern Holy Vanguard gains
  the +30/+30 too (Era, 2026-11-04; the logs show it in the card's stats in
  all three places). Armor doesn't count, only Health, and Health never
  comes back in Battlegrounds, so once you are at 15 or less it stays on
  (Era, 2026-11-04). Its effect is doubled (twice the numbers, or the
  effect triggers twice).
- Playing a golden minion gives a Triple Reward: a 0-cost plain spell (not
  a Tavern spell) that goes to your hand. Casting it is casting a spell:
  every "cast a spell" counter and trigger sees it (a Felboar counts it), a
  "Tavern spell" one doesn't (a Timecap'n Hooktail stays silent), and the
  whole cast waits for the Discover it opens, so those effects land at the
  pick (logs `Hearthstone_2026_11_02_00_10_22` game 4 turn 8,
  `11_03_15_22_36` game 3 turn 8; 2026-11-04). Its tier is fixed when you get
  it: one above your tier then (tier 6 at most). Played on tier 4, it
  Discovers tier 5 even if you level before casting it. You can also keep
  it and never cast it. It comes before anything the golden minion's
  Battlecry adds to your hand; with a full hand it's lost.
- **THE GOLDEN RULE (Era, 2026-11-03; asked too many times, do not re-ask):
  every golden minion gives a Triple Reward when played from hand. The only
  exception is a golden whose card or effect EXPLICITLY says it gives none.**
  The ones we know:
  - Aureate Laureate,
  - Silent Deliverer's goldens,
  - Sinister Invitation's golden,
  - the Dark Gift that makes a minion golden without a Triple Reward
    (Gilding).

  There may be other ways to get a golden without a reward that Era doesn't
  remember; each one must say so in its own text. Everything else gives one:
  a golden that carries any other Dark Gift, a golden from three copies where
  one had a gift, a Lockbox's golden, a Tavern minion Golden Touch made golden
  and then bought, and summoned tokens (three Skeletons from Cadaver Caretaker
  make a golden Skeleton in hand, with its Triple Reward when played). Code,
  collectors and replays default to "gives a reward" and need a named
  exception to say otherwise. (A minion that turns golden on the board, like
  Elite Navigator or Captain Sanders, was never played golden, so it gives
  nothing: see "Triple rewards" below.)

### Spells

The shop offers Tavern spells (one slot, its own costs). Many minions react
to spells cast ("Spellcraft" gives a one-turn spell when played, some
minions grow per spell). Spell counts can last the whole game: the Dark Gift
Spell Siphon counts spells cast through the game, not the fight.

### Hero powers

Each hero has one. Some are active (pay gold, click), some passive, some
fire at Start of Combat or during the fight. They range from small stat
bonuses to rule-changing ones. The older simulator's notes have the ones
that act in a fight.

### Season mechanics (as my logs show them on 2026-10-18)

- **Trinkets:** each player picks a Lesser trinket and later a Greater one
  from offers, on turns 6 and 9. Passive effects, many in the fight.
- **Quests and rewards:** a quest counts events through the game and gives a
  reward when done. Sire Denathrius's hero power is built around them.
- **Dark Gifts:** extra effects attached to minions.
- **The Deity:** a counter that awakens an Old God minion (C'Thun or
  Y'Shaarj) in the fight.
- Anomalies (one game-wide rule change per lobby) are not in the game right
  now. Buddies and Duos are out of scope.

## The leaderboard (Era, 2026-11-06)

- Every player sees, for each player on the leaderboard:
  - **Tribe:** the top tribe and its count, e.g. "Aberrations 5", from that
    player's board at the start of the last fight (after their shop turn,
    before combat), so it changes once per round, not with the live board.
    A neutral or empty board shows nothing. Two or more tribes with the same
    top count show as mixed minions. An all-types minion (Amalgam) counts
    for every tribe: 3 Beasts and 2 Amalgams show "Beasts 5" (Era). A
    two-type minion counts for both its types (inferred from that, not
    confirmed).
  - **Last three fights:** the outcome of each and the damage dealt or taken.
- Era judges opponents from this, the turn, their tier and their trinkets,
  not from remembering old boards (docs/notes/era-strategy.md).

## The fight

Each player is paired with another. With an odd number alive, one player
fights a ghost: the board of a player who was knocked out **(check how the
ghost is picked)**. Pairing avoids repeats: you don't face the same opponent
again for a few rounds when there's a choice **(check the exact rule)**.

Both boards are copied into the fight, so nothing in it changes your real
board, except effects that say "permanently".

### Order of events

1. **Start of Combat.** Illidan's Wingmen go first of all. Then hero powers,
   the Lesser trinket, the Greater trinket, then minions left to right. Between
   the two players it is interleaved: a coin flip picks who starts each step,
   then they take turns. Medium confidence.
2. **Attacks.** The side with more minions attacks first, a coin flip if
   equal. The sides then alternate. Each side attacks with its minions left
   to right and wraps around to the left after the last one. A minion with 0
   Attack never attacks.
3. **Targets.** The defender is picked at random among the enemies. Taunt
   minions must be picked first, and Stealth minions can't be picked. Stealth
   is lost when the minion attacks. Checked.
4. **Damage.** Both minions deal their Attack to each other at once.
   Checked.
5. **Deaths.** Once the attack is done, every minion at 0 Health or less
   dies. They are handled in board order, the attacking side first. Then
   Deathrattles run, then Reborn, and the whole step repeats until nothing
   more dies. Deathrattles before Reborn is confirmed. The order within a batch
   is an approximation.
6. **The end.** The fight ends when one side has no minions (or both). A side
   with minions left wins. Both empty, or neither side able to attack, is a
   tie.

### Keywords

- **Taunt:** must be attacked first.
- **Divine Shield:** blocks the next damage of any size, then is gone.
  Checked.
- **Windfury:** attacks twice per turn. Mega-Windfury attacks four times.
- **Venomous:** the next time it damages a minion, that minion dies. Then
  Venomous is gone. Checked. **Poisonous** is the same but permanent.
  They are two separate keywords, and a minion can have both (Era,
  2026-11-04; log `11_02_00_10_22-g7` turn 14: Tide Oracle Morgl, Poisonous
  by its text, gains Venomous from Sewer Escapee). Venomous is a Bonus
  Keyword and counts for Hackerfin; Poisonous is not a Bonus Keyword and
  doesn't count (logs, 2026-11-04: `11_02_00_10_22-g7` turn 9 step 31 and
  turn 10 step 17, Hackerfin's own number is +6/+4 and +9/+6 with Morgl's
  Poisonous on the board, which counts only Venomous, then Venomous and
  Taunt; Era thought Poisonous counted).
- **Reborn:** when it dies, it comes back once with its printed Attack and 1
  Health, without Reborn. Checked.
- **Cleave:** the attack also hits the target's neighbours, and they don't hit
  back.
- **Stealth:** can't be attacked until it attacks.
- **Deathrattle:** an effect when the minion dies. Summons appear where the
  dead minion stood.
- **Avenge (N):** fires after N friendly minions die.
- **Rally:** fires when the minion attacks.
- **Magnetic:** a Mech that can be played onto another Mech to merge into it.
- **Start of Combat, End of Turn, Battlecry, Spellcraft:** timing keywords.

### Limits

- A board holds 7 minions. Summons past that are lost.
- "Whenever" and "after" triggers fire in board order **(check per card)**.

### Damage to the hero

The loser takes damage equal to the winner's tavern tier plus the tiers of
the winner's surviving minions. Every minion has a tier, tokens too (usually
tier 1). The damage is capped at 5 on turns 1 to 4, 10 on turns 5 to 7, and
15 from turn 8 on. Once 4 or fewer players are left there is no cap at all. A
tie deals no damage.

## What a player can see

- Their own hand, board, shop, gold and tier.
- For every opponent: hero, health, armor, tavern tier, and the board from the
  last time they fought each other. The real game shows a few more hints (the
  opponent's main type, triple count, last fights result) **(check which)**.
- Nobody sees other players' shops, hands or gold.

The bot must only get what a player sees, or it learns to cheat and the
advisor won't work.

# Part 2: the game we build

The game the bot learns now. Everything in Part 1 applies, with the
choices below fixed. Where Part 1 says **(check)**, the engine uses the
value in `rules.toml`.

## Settled: read before asking Era

These come up again and again. Each line is settled; the full rule is in the
section named. Grep this file, `docs/rules/heroes.md`, `docs/rules/trinkets.md`
and `docs/rules/darkgifts.md`, plus the private notes on unsure rules, before
asking Era any rule question.

- **Goldens and Triple Rewards:** every golden gives one unless its own text
  explicitly says not (Laureate, Silent Deliverer, Sinister Invitation,
  Gilding). See THE GOLDEN RULE under "Triples".
- **Nothing triples during combat:** an end-of-turn or in-fight third copy
  merges at the start of the next turn ("Nothing triples during combat").
- **Spell versus Tavern spell:** "cast a spell" counts every spell (Blood
  Gems, Bananas, Gem Day, Spellcraft); "cast a Tavern spell" only
  BATTLEGROUND_SPELL ("Tavern spells versus other spells", "Read the card's
  exact words") A Triple Reward is a plain spell: it
  counts for "cast a spell", never for "Tavern spell" (logs, 2026-11-04).
- **Targets:** a spell, Battlecry or Activate may pick your board AND the
  Tavern unless the text limits it ("Targets in the shop", "Who a card can
  reach").
- **How many times:** Drakkari and Balinda SET a count (2, golden 3); Hot-Air
  Surveyor and Ghastly Sticker ADD one. Nothing doubles ("Set versus add",
  "'Twice' versus 'an extra time'").
- **Battlecries:** Brann counts twice; a Choose One is not a Battlecry
  ("Counting Battlecries").
- **A minion's cast isn't yours** ("A spell a minion casts for you").
- **Start versus end of turn:** read the card's text. If it says "at the end
  of" (your turn, every 2 turns), it is an end-of-turn effect and Drakkari
  Enchanter repeats it. Anything else ("next turn", "in 2 turns", "each
  turn", "at the start of") happens at the start of the turn and Drakkari
  doesn't repeat it. This covers every card, not only Dark Gifts: counters,
  discounts, countdowns (Era, 2026-11-03 and 2026-11-04; Incubation's "in 2
  turns" pays at the start of the turn and doubles once, Replication's "at
  the end of every 2 turns" pays at the end and Drakkari repeats it).
- **"A minion of each type" (Tea Set):** each minion gets at most one buff,
  and as many minions as possible get one; random only between equal choices
  like two Demons ("Types").
- **Gold:** any spend action of any amount counts; Health payments are
  self-damage, not Gold; a free buy spends nothing ("Spending gold",
  docs/rules/trinkets.md).
- **Copies:** an exact copy copies the state right now (damaged, a popped
  shield stays popped); a plain copy is the base card, not golden, no buffs
  ("Exact copies", Lichling Hoarder under "Undead", Replication in
  docs/rules/darkgifts.md).
- **Full hand:** 10 cards. A card you get in the shop or a fight with a full
  hand is lost; one you get at the start of your turn waits in a queue ("A full
  hand", Part 1 "Hand and board").
- **Cards locked in hand and the Lockbox** ("Cards locked in hand",
  "Lockbox").
- **Limited uses run out per minion; a golden from a triple starts fresh**
  ("Uses run out per minion").
- **Refreshes:** the button versus "Refresh" effects; Malchezaar's bug is
  copied ("Two kinds of Refresh", "Malchezaar and free Refreshes").
- **Demon Fodder:** stays in the Tavern while you have no Demon; feeds itself
  as soon as you have one ("Demon Fodder").
- **Generated versus pool cards** ("Generated versus pool").
- **Eating a minion with a spell:** its "whenever you play a card" gift
  procs first ("Eating a minion with a spell").
- **The order inside one cast:** the target's "whenever you cast a spell on
  this" fires before the spell's own effect; a Tavern buff the spell's own
  effect gives lands after that cast's "after you cast" triggers ("The order
  inside one cast", from logs).
- **Butchering:** all Tavern spell power, Health included, "wherever they
  are" ("Butchering and spell power").
- **Halves round up** ("Halves round up").
- **Dark Gifts:** they stack (duplicates too), travel with the card, work the
  same on a golden; countdowns fire at the end of your turn (Incubation's
  payout lands as the next turn starts, from the logs), Replication and
  Affinity restart, Incubation doesn't (docs/rules/darkgifts.md).
- **Trinkets:** the Compass locks its type at the pick; Mystery Cube fires
  the old trinket's start of turn first, then switches; offers on turns 6
  and 9 (docs/rules/trinkets.md).
- **Heroes:** Genn's first pick is the second power; A. F. Kay only freezes
  turns 1 and 2; the rest in docs/rules/heroes.md.
- **Shop replay answers from 2026-11-03** (Lionfish, Kelp Keeper, Decoy
  Conjurer, Fishbait, Scarab): "Shop replay answers".

## The pool

- All ten tribes of the current patch: Beast, Mech, Dragon, Murloc, Demon,
  Pirate, Elemental, Undead, Quilboar, Aberration (Naga left the game).
  Every real solo-pool minion of those tribes, plus neutrals, tiers 1 to 7.
  A card with two tribes is in if either is. Each game draws 5 of the
  ten (see "The lobby's tribes").
- Spells: every solo-pool Tavern spell.
- Out because they need systems this game doesn't have: Dark Paradox
  (Dark Gifts). Armor Stash and Unmasked Identity come with the heroes
  run. No Duos-only cards.
- The game's own card data is exported by `tools/export_cards.py`, see
  `docs/card-data.md`.
- Generated cards that belong to these tribes are in too, though never in
  the pool: Chromadrakes, Demon Fodder, Fishbait, Satellites, Beetles and
  so on.
- Spells have no limited pool. (Assumption; I haven't checked it.)
- **Patch 37.0.2 (2026-11-03), Era 2026-11-04:** cards that leave the game
  (Expert Aviator; trinkets Spell-powered Wrench, Quilligraphy Set, Fridge
  Magnet, Writhing Tentacles) are marked out of the pool, never deleted:
  old logs and real boards still hold them. Stalwart Kodo's "a different
  minion" means not another Stalwart Kodo. Last One Standing's "improve
  this" adds +10/+10 each Rally (+10/+10, then +20/+20, +30/+30, ...).
  The full change list is `docs/notes/patch-37.0.2.md`.
  In the engine: `out_of_pool = true` in `cards.toml` or `trinkets.toml`
  (no Tavern, Discover, random draw, Dark Gift pool or trinket offer hands
  it out); `banned_tribes` on a trinket (Wand of Divination in a Quilboar
  lobby); `exclude` on a Dark Gift (Furtiveness never on Relentless
  Deflector).
- Some groups share one pool slot: the three Volumizers are one slot and a
  drawn one gets a random color. The five Bounty spells are one slot the
  same way, and a drawn Bounty is a random one of the five.

**Growing to every tribe (2026-10-23).** The current patch has ten tribes:
Beast, Mech, Dragon, Murloc, Demon, Pirate, Elemental, Undead, Quilboar,
Aberration. Naga left the game; Aberration replaced them. They come in one
run at a time: tier 7, Pirates, Elementals, Undead, Quilboar, Aberration,
then a lobby of five random tribes like the real game.

**Tier 7.** You can't level into tier 7. A triple at tier 6 Discovers a
tier 6 minion. A tier 7 minion only comes from a card that explicitly
says tier 7 (Greedy Conniver). Tier 7 minions are in the pool with the
same copy count as tier 6 (a guess; Blizzard doesn't show it).

**Rally timing.** Every Rally triggers when its minion attacks, before the
attack lands (Obsidian Ravager's damage, Heroic Underdog's gain, The Last
One Standing's buff all happen first). Deathstrider is the exception: it
says "after a friendly Rally minion attacks", so it fires after.

**A Rally that kills the target.** If the Rally kills the target, the
attack doesn't land: the attacker takes no damage back, and "after a
friendly minion attacks" still fires. That's what makes Obsidian Ravager
strong. If the target survives the Rally, the normal hit and its damage
back happen.

**Reborn and maximum stats.** A Reborn minion counts as damaged down to 1
Health, not as a minion whose maximum is 1. A Reborn Stalwart Kodo has a
maximum of 32 Health and 1 left; with Goldrinn's +7 Health it has 8 left
and a maximum of 39. Anything that reads maximum Health sees the full
value. (Comes back for the Undead tribe.)

**Polarizing Beatboxer.** It sees every Magnetization to a different
minion, wherever it comes from: a Magnetic from hand, Spark Snapper's and
Glambot's Satellites, Drone Duplicator's extras, and Magnetizations to a
minion in the Tavern (Glambot plus a spell on a Tavern Mech). The copy is
of the Magnetic as it attached, buffs included: a Lullabot that Mechagnome
Interpreter made 5/3 gives Beatboxer a 5/3 copy. Interpreter sees the copy
as a Magnetization but doesn't buff it again (no 8/4). Spark Snapper only
triggers on a Mech played from hand, never on a copy or a Satellite. A
copied Volumizer counts as Magnetized for the first time (in
docs/notes/UNSURE.md).
Each Beatboxer copy of one step is minted afresh, so a copied Volumizer is
bigger by every "your Volumizers have +A/+H" rise the copies before it in the
same step paid (`Hearthstone_2026_10_29_22_20_04-g3` turn 14 step 20: two
Beatboxers taking three copies each of one golden Green Volumizer end
+4140 and +4146, not three even +1374s). Drone Duplicator's own repeats are
not fresh copies and merge the card as it attached. In the log each round's
copies are minted before the host merge; the engine merges the host first,
so it lands one rise per round short (UNSURE). (Logs, 2026-11-04.)

**A Magnetic played from hand is a minion of its own until it merges.** The
raw log's PLAY block puts it in play, runs the "whenever you play a Mech"
triggers against it there and only then merges it into its host, so Spark
Snapper's Satellite joins the played minion and every copy of that minion
carries the Satellite. `Hearthstone_2026_10_29_22_20_04-g3` turn 13 step
10: the 2/4 Annoy-o-Module takes the 84/126 Satellite, merges into a Red
Volumizer (which gains +86/+130) and leaves a Polarizing Beatboxer
+170/+256, the Satellite and the module. The host takes one Magnetization,
not two.

**A Magnetization onto a Beatboxer sets no other Beatboxer off.** It is
also what stops two Beatboxers from looping.
`Hearthstone_2026_10_29_22_20_04-g3` turn 14 step 16: a Beatboxer played
from hand takes Spark Snapper's Satellite (5/10 to 105/160) and the
Beatboxer already on the board stays where it is. That is narrower than
"it sees every Magnetization" above and comes from one log.

**Volumizers inside a host (Era, 2026-11-03).** A Volumizer's "your
Volumizers have +N/+M this game (wherever they are)" reaches the Volumizers
in the Tavern, in your hand and on your board, but never the ones already
Magnetized into a host: a magnetized Volumizer brings the stats it has at
that moment and stops being one of "your Volumizers". Log
`Hearthstone_2026_10_29_22_20_04-g3` turn 6: Resourceful Robot magnetizes a
3/1 Red Volumizer and gains exactly +3/+1, while the Red's +3 Attack reaches
the copy in hand (6/1) and the Green in the Tavern (3/3 to 6/3).

**Tribes of a card versus tribes of the lobby.** A minion with two tribes
is in the pool if either tribe is in the lobby. A card always keeps all
its tribes, whatever the lobby has: with no Beasts in the lobby, Treasure
Parrot is still a Beast, so "a friendly minion of each type" can buff it
as the Beast and a Pirate next to it as the Pirate.

## Rules beyond Part 1

**Shop.** One spell slot on top of the minion slots, at every tier. It
rolls and freezes with the minions. Spells go to hand and are cast from
there; a spell with no legal target can't be cast.

**Magnetic.** A Magnetic Mech played directly to the left of a Mech on the
board merges into it: the host gets its stats, keywords and effects. That
counts as playing a Mech. Effects that attach straight onto a Mech
(Glambot's Satellite, Resourceful Robot) are a Magnetization but not a
play. Nesting works all the way down: a Magnetic carrying Magnetics brings
all of them, every effect keeps working, and the Magnetization count adds
up for Utility Drone. A merged pool copy is consumed and doesn't return to
the pool.

**Activate.** A minion with `Activate (N)` can be used once per turn for N
gold, unless its text says otherwise. A golden minion does its effect
stronger or twice (per its golden text), not more uses.

**Consuming.** A minion that consumes a Tavern minion gains its stats
(some also its Bonus Keywords). The consumed copy goes back to the pool.

**Bonus Keywords** are Taunt, Divine Shield, Windfury, Venomous, Reborn
and Stealth. "Gain a random Bonus Keyword" picks one the minion doesn't
have yet.

**Stealth** is never ignored. A Stealth minion can't be attacked until it
attacks. A side facing only Stealth minions loses its turn and the other
side attacks; if neither can attack, the fight is a tie. Only side damage
(Cleave, Wildfire Elemental) reaches Stealth.

**Hero health as a resource.**
- Health costs (Malchezaar's Refreshes, spells bought with Health) can't
  kill: they're only payable if the hero stays above 0, unless a rewind
  (Soul Rewinder, Ashen Corruptor) is on the board.
- Damage from effects (Wrath Weaver, Imposing Percussionist) can kill.
  A hero at 0 or below after the damage reactions dies on the spot, in
  the middle of its turn, and isn't paired for that turn's fights.
- Paying Health is the hero taking damage: every "after your hero takes
  damage" card reacts to it.
- Rewinds and every other "after your hero takes damage" minion only work
  while that minion is on your board, which means during your turn. The
  damage from losing a fight lands after the fight, when your board isn't
  in play, so it isn't rewound and doesn't trigger them.
- Damage counts before a rewind gives it back, so "after your hero takes
  damage" effects and counters like Eredar Escapist still fire, and
  Devout Hellcaller still grows from a Demon's damage that gets rewound.

**Who a card can reach.** The wording decides: "friendly" means your
board, "in the Tavern" means the Tavern, and no qualifier ("another
minion", "a minion") means your board or the Tavern. Never the hand:
cards in hand can't be targeted, sold or touched unless the text says
"in your hand". A minion's life is buy, play, sell; you can't sell from
hand.

**Types.** "Type" means tribe. Neutral minions have no type and can't be
the chosen minion of a "minion of its type" card (Eonar's Favor, Lost
Staff of Hamuul, Natural Blessing...). A dual-tribe or all-type minion
picked for such a card uses one of its types at random. "A friendly
minion of each type" (Misplaced Tea Set) gives each minion at most one
buff, even a dual or all-type one, and can pick the card itself (The
Last One Standing). "Each type" means the types on your board.
The buffs are shared out so that as many minions as possible get one:
a type is never wasted while another minion could take it (Era,
2026-11-03). With a Demon/Dragon and a Dragon on board, a Tea Set always
buffs the Demon/Dragon as the Demon and the Dragon as the Dragon. It is
random only where the choice doesn't change how many get a buff, as with
two Demons. Backed by `Hearthstone_2026_10_24_15_25_26-g1` turns 10 and
11: all 49 Tea Sets buffed every typed minion once.

**Discard** doesn't exist in this lobby (it's Aberration-specific).
"If you discard this" text does nothing.

**Robust Evolution** transforms into a random minion exactly one tier
higher (a tier 6 into another tier 6), whatever your Tavern tier. Only
Attack and Health carry over, even when lower than the new card's base
stats; keywords, buffs and effects don't. The old copy goes back to the
pool and the new one comes from it.

**Patient Scout** starts at Tier 1, improves by 1 at the end of each of
your turns while on your board (Drakkari doubles it), up to 6, and its
sale Discovers a minion of exactly that tier.

**Triples keep the highest count.** When three copies become a golden
minion, any count a copy kept (Lurking Leviathan's summons, Hopebringer's
lost shields, Patient Scout's tier...) is the highest of the three, never
the sum. Counts that belong to the player, not a minion, aren't touched:
Tasty Lobster's count of every Lobster Deathrattle this game (shop or
fight, golden or not) is the player's, so every Lobster, already on the
board or made later, reads the same number. A base Lobster gives +2/+1
times (count + 1), golden double.

**Nothing triples during combat (Era, 2026-11-03).** A third copy can only
merge into a golden during your shop phase. A copy that arrives at the end of
your turn (Replication's plain copy) or during a fight waits: the three merge
at the start of your next turn, and the golden lands in your hand then, with
its Triple Reward as usual (THE GOLDEN RULE).

**Balinda** doubles every spell cast on a friendly minion (not spells
with no target, nor spells cast on a Tavern minion), including
Robust Evolution (the second cast evolves the result again) and Lost
Staff of Hamuul ("cast on this" triggers fire twice). Channel the
Devourer's second cast does nothing: the minion is already sold. Each of
Balinda's casts is a cast of its own, a Choose One spell's targeted option
too: "after you cast a spell" counters see each, and a Felboar eats between
them (log `Hearthstone_2026_11_02_00_10_22-g4` turn 14 step 24, Alliance
Flag with a golden Balinda: three casts, both Felboars counting one per
cast; 2026-11-04). Hot-Air Surveyor adds no cast to a Gem a minion plays
(Turbo Hogrider, Roogug): only Gems played from your hand (log
`10_28_02_28_32-g4` turn 13 step 27; 2026-11-04).

**Two kinds of Refresh.** An explicit Refresh is the button (paid with
Gold, Health or free) or an effect that says "Refresh" (Snarky Shark).
Fodders queued for "your next Refreshes" only go into explicit ones. The
roll at the start of a turn also counts as a Refresh for "after the
Tavern is Refreshed" buffs (Waveling, En-Djinn Blazer, Easterly Winds,
Blood Gem Barrage), but only if the Tavern changed: an unfrozen Tavern
always changes; a frozen one counts only if it had to fill an empty slot.
A fully frozen, full Tavern isn't refreshed. So buying one card and
freezing makes the next turn's roll fill that slot, and the buffs land
again on every Tavern minion, the frozen ones included (Barrage) or on one
random one (Waveling's amount).

**Demon Fodder** feeds itself to a random friendly Demon on the Refresh it
appears in, and its slot refills with a minion (the spell slot too). With
no Demon on board then, it waits in the Tavern as a plain 2/2 Demon you
can buy or consume, and the moment any Demon lands on your board (after
the action that put it there), every waiting Fodder feeds.

**Golden cards** follow their real golden card text.

**Triple rewards.** Any golden minion played from hand to the board gives
a triple reward, unless its text explicitly says otherwise (THE GOLDEN RULE
under "Triples": Aureate Laureate, Silent Deliverer, Sinister Invitation,
Gilding). That includes a Lockbox's minion and a Tavern minion that
Golden Touch made golden, then bought and played. A minion that becomes
golden on the board (Elite Navigator, Captain Sanders) gives nothing, and
it doesn't count as a golden minion "played" (Maritime Extortionist,
Hooktusk). Golden-making effects (Eyes of the Earth Mother up to Tier 4,
Captain Sanders up to Tier 6, never Tier 7) work on any friendly minion
within their limit, tokens included: a Sneed's Shredder or a Beetle can be
made golden this way, even though a token never triples.

**Exact copies (Era, 2026-11-01).** "An exact copy" copies the minion's
state right now, everything included: buffs, keywords, golden, Dark Gifts,
damage taken (a 10/10 that took 6 damage gives a 10/4 copy), and keywords it
has lost (a popped Divine Shield stays popped). This holds for every card
that says it: Cloning Gallery, Rapid Reanimation, Frostwolf Fervor,
Stormpike Strength, Stitched Salvager, Archlich Kel'Thuzad and the rest.

**Targets in the shop (Era, 2026-11-02).** A spell, Battlecry or Activate
that targets "a minion" can target a minion on your board or in the Tavern.
Only the card's own words narrow it: "a friendly minion" means your board
(and hand where it says so), "a minion in the Tavern" means the Tavern, "a
Beast" means a Beast in either. A Choose One with a target asks for the
option first, then the target (Sprightly Scarab, Alliance Flag, Forest's
Bounty). For a Choose One spell, "after you cast a spell" fires only after
both: cast, pick the option, pick the target, then the cast triggers (Era,
2026-11-02).

**Counting Battlecries (Era, 2026-11-02).** "For each Battlecry you've
triggered" (Dark Paradox t6, Murky Sticker, Battle Scars) counts Battlecry
triggers: a Battlecry doubled by Brann counts twice, and a minion without a
Battlecry counts nothing when played. A Choose One minion is not a
Battlecry. "Wherever this is" includes Bob's Tavern: a copy there grows too.

**Eating a minion with a spell (Era, 2026-11-02).** When Channel the
Devourer or Butchering takes one of your minions, everything that fires on
the cast fires first: "whenever you play a card" Dark Gifts (Toughened
Shield, Sharpened Sword), "whenever you cast a spell" minions (Eventide
Brute, The Glad-iator, Wizened Captain, Vicious Mindslasher), "whenever you
cast a spell on this" (Fleeing Fugitive; Gatekeeper Amalgam casts its Tea
Set), and only then is the minion eaten or destroyed, so the eater gains the
buffed stats. "After you cast a spell" triggers fire after the spell has
resolved.

**The order inside one cast (logs, 2026-11-04).** A spell cast on one of
your minions runs the target's "whenever you cast a spell on this" triggers
first and its own effect after them: Methodical Madness on a Devilish
Distractor eats Tavern minions carrying the Distractor's fresh +1/+2
(`Hearthstone_2026_10_24_15_25_26-g4` turn 11 step 23: the Distractor's
TRIGGER block and its Tavern stat pushes come before the spell's POWER block,
whose consumed stats include them). A Tavern buff that the spell's own effect
gives ("give minions in the Tavern +2/+2 this game") reaches the Tavern's
minions only after that cast's "after you cast" triggers have run: Staff of
Enrichment's +2/+2 lands after the Felboar has eaten the Tichondrius at its
pre-spell stats (`Hearthstone_2026_10_28_22_11_50-g4` turn 11 step 41). A stat
buff a spell puts straight on a minion is not held back: a Blood Gem's stats
reach the Felboar eating for the same cast (`2026_11_02_00_10_22-g4` turn 9
step 39). Not Era's ruling; read from the raw logs (Era's RQ1 is half
answered by them, see docs/notes/UNSURE.md).

**Butchering and spell power (Era, 2026-11-02, 2026-11-03).** Butchering's
"+8 Attack this game" is a Tavern spell number, so ALL Tavern spell power adds
to it, Health included: with +3/+3 spell power it gives your Undead +11/+3
"wherever they are" (this game, every Undead, on the board, in hand, or
summoned later). A 1/1 Skeleton with Reborn then comes back as a 12/4.

**Shop replay answers (Era, 2026-11-03).**
- Lurking Lionfish: "choose a card in the Tavern" may pick a Tavern spell too.
- Kelp Keeper: its Activate on a minion whose Battlecry Discovers gives a
  random card from that Discover's pool instead of opening the Discover.
- **Read from the logs, Era agreed (2026-11-04).**
  - A Refresh's Fodder queue puts at most seven Fodders into one Refresh.
  - A Health price is paid before the bought spell joins the hand; a Gold
    price after it (so a card the damage triggers comes first, and a
    Gold-spent card comes after).
- **Shop replay answers (Era, 2026-11-04).**
  - A Battlecry triggered at the end of the turn (Young Murk-Eye, Drakkari)
    on a Discover Battlecry gives a random card, the same as Kelp Keeper:
    no Discover opens It doesn't count for "after you Discover". This
    includes a Magicfin token's taught spell run at the end of the turn:
    the raw log makes its three options and takes one in the same instant,
    with no choice offered (`10_28_02_28_32`, 02:41:12, Young Murk-Eye on a
    Magicfin Apprentice; Rodeo Performer through Echoing Voice and Drakkari
    in `10_24_17_59_26` at 18:12:53 the same way).
  - Mind Muck: you choose the Demon; the Tavern minion it eats is random.
  - A minion's own Choose One or Battlecry can't target the minion itself
    (Sprightly Scarab played as your only Beast buffs nothing; log
    `10_28_02_28_32-g4` turn 10).
  - Ominous Seer (Tier 1 Demon and Naga, "the next Tavern spell you buy
    costs (1) less") is what makes Tavern spells cost 0 or 1 in the logs.
  - Victorious Geomant's Activate costs 2; the one log step that showed 1
    was a misread step, not a discount.
  - A Tavern spell dealt into a minion slot (Mana Per Minute, Saloon's
    Finest: "Refresh the Tavern with Tavern spells") costs its normal spell
    price, the same as in the spell slot.
  - Two casts on one minion versus one each on two (Misplaced Tea Set) are
    both fine random outcomes; only the replay's comparison tells them
    apart.
- Decoy Conjurer: when several Tavern minions tie for the highest Attack, the
  stolen one is random among them.
- Fishbait: the minion that kills it (your left-most Beast, which attacks it)
  gets +5/+5, +10/+10 for a golden Fishbait. Snarky Portrait's +4/+4 to all
  your Beasts is separate and comes on top (docs/rules/trinkets.md).
- Sprightly Scarab: either option may target a Beast on your board or in the
  Tavern (`in_shop = true` means "the Tavern too").
- A pick from a Triple Reward is a Discover: "after you Discover a card"
  fires on it (Hooktusk, Master Marauder). Era's logs show it twice.

**Malchezaar and free Refreshes (Era, 2026-11-02; a bug in the real game
that we copy).** Malchezaar, Prince of Dance makes two Refreshes each turn
cost Health. Free Refreshes are used first, but the last free Refresh also
uses up one of Malchezaar's two, so with any free Refresh that turn only
one Health Refresh is left. Era's logs show it on every such turn (seven
turns in three games): the count goes 2 to 1 on the last free Refresh, then
to 0 on the one Health Refresh. With no free Refresh, both Health Refreshes
happen as printed.
Pack Tactics is an exact copy whose stats are then set to 3/3.

**"Twice" versus "an extra time".** "Your X trigger twice" or "cast twice"
(Brann, Drakkari, Balinda, Proud Privateer) sets a fixed count: two copies
don't stack, and a golden copy sets its own count from its golden text.
"An extra time" (Titus) adds on top, so several stack.

**Set versus add, as one mechanic.** Every "how many times" count works
the same way: the count is the highest value any setter gives (1 if none),
plus what each adder adds (one, or its golden number). Setters: Brann, Drakkari, Balinda, Proud
Privateer ("twice", golden "three times"). Adders: Titus, Hot-Air
Surveyor ("an extra time"). Future cards slot into one side or the other.
A repeated Battlecry or spell that picks a target uses the same target
every time: Brann doubling a Magicfin token's taught Tricky Trousers casts
it twice on the one minion picked (Era, 2026-11-04; log
`Hearthstone_2026_10_28_02_28_32-g1` turn 10 step 2). A Magicfin token's
cast is its Battlecry, so Brann doubles it.

Making a minion golden is the exception: a repeat still counts as a
trigger or a cast, but the golden happens once. Brann plus Elite Navigator
makes one Pirate golden, while the Battlecry still counts twice for
Kalecgos and every other "Battlecry triggered" card. A golden Elite
Navigator picks two Pirates. Eyes of the Earth Mother with Balinda is the
same: the spell casts twice (both casts count), the golden happens once
(Era, 2026-11-04).

A repeated Choose One works normally: it remembers your choice, so every
repeat (Brann, Balinda) uses the same option on the same target.

A spell a Magicfin token casts gets no Choose One choice (Era,
2026-11-04): if the taught spell is a Choose One (Boundless Potential), its
option is random. A Discover inside it is a normal Discover that you pick
(log `Hearthstone_2026_10_28_02_28_32` 02:41: the token's Boundless
Potential offers three Tavern spells and Era picks one), and it counts for
"after you Discover" cards. With Brann the token's Battlecry triggers
twice: two casts, each with its own random option. A target the spell
needs is still yours to pick (Tricky Trousers above).

Kelp Keeper activating a minion whose Battlecry Discovers (a Rodeo
Performer's Discover a spell) gives a random card from that Discover's pool
with no Discover offered, and that random card is not a Discover for
"after you Discover" cards (Hooktusk) (Era, 2026-11-04).

**Spending gold.** Anything that actually uses gold counts: buying,
upgrading, rolling, Activate costs. Health paid instead of gold and free
Refreshes don't. "After you spend N Gold" counters carry over turns
(Gunpowder Courier is every 5 Gold, as its text says, checked in a real
game 2026-10-24; its "(6 Gold left!)" display is wrong);
"gold spent this turn" resets each turn.

**Lockbox.** A Lockbox sits in hand, can't be played and takes a hand
slot. It counts down at the end of your turn (Drakkari doubles it). At 0
it opens into a random golden minion with a type, up to your tier, which
gives a triple reward as usual. Getting another Lockbox while you hold one
makes it open sooner instead (1 turn, or 2 if the card says so).
Timing: a Lockbox got on turn 1 shows 5 turns, shows 4 after the end of
turn 1, and opens at the end of turn 5, before that turn's fight. Its
minion can't be played that turn, but it is in hand for the fight, so
anything that summons from hand can summon it.

**Cards locked in hand (Era, 2026-11-01).** Several cards put a card in
your hand locked: it takes a hand slot, can't be played, sold or
discarded, but it is a real card in hand: effects that buff cards in hand
(Winterfinner and the like) reach it, and a locked minion counts for
anything that looks at your hand. It unlocks on a condition, then plays
normally. Every one of these uses the same lock:

| Card | Locked until |
|---|---|
| Faelin, Expedition Plans | the Tavern reaches the minion's tier (2, 4, 6) |
| Tras'tath, Void Power | turn 7 |
| Thorim, Choose Your Champion | you have spent 60 Gold (the Tier 7 is Discovered at the start of the game and sits in hand locked) |
| Maiev, Imprison | 2 turns |
| Search Through Time (Tier 2 spell, 2 Gold) | 1 turn |
| Lockbox | its countdown (below; it opens into a golden minion) |

Kaleidoscope (2 turns) and Double Stitch Needle (1 turn) lock the same
way, but the game data has them out of the current trinket pool.
Timewarped Beanstalk locks too, but Timewarp is out.

**A spell cast twice counts twice everywhere.** Proud Privateer's second
Bounty cast is a second spell for every trigger and every "spells cast"
counter.

**Halves round up.** Any "half" in card text rounds up (half of 5 is 3).

**Elementals (2026-10-23).**
- **Ballers** share one player count across Fire, Snow and Air Baller:
  every Baller sold improves every future Baller's sell effect by one
  step. A step is +1 Attack for Fire, +1 Health for Snow, +2/+2 for Air;
  a golden Baller's step is double (+2, +2, +4/+4).
- **"After the Tavern is Refreshed this game"** (Waveling, En-Djinn Blazer,
  Easterly Winds) fires on every Refresh, the turn-start roll included when
  the Tavern changed (see "Two kinds of Refresh"). All
  of them add into one player-level amount, and each Refresh gives that
  whole amount to one random Tavern minion: two dead Wavelings give one
  minion +8/+8, never +4/+4 to two minions.
- **Ichoron** shields Elementals played from hand only. The shield goes at
  the start of the next shop phase, used or not (golden: it stays).
- **Arcane Absorption** can only be cast on a friendly Elemental, so it's
  unplayable with none on the board. With an empty Tavern it can still be
  cast, gives nothing, and still counts as a spell cast.
- **Free Refreshes** are a player count that adds up and carries over
  turns.
- **Every Tavern spell counts** for "whenever you cast a Tavern spell"
  (Living Azerite) and spell counters: generated ones, spells cast by an
  effect (Air Revenant's Easterly Winds), Bounties, and each extra cast
  (Proud Privateer, Balinda).
- **Elemental of Surprise** stands in for any Elemental in a triple: two
  copies of X plus a Surprise, or two Surprises plus X, make a golden X.
  The Surprise's Divine Shield and buffs carry into the golden X. A golden
  Surprise can still triple with two copies of X into a golden X, the only
  card that triples while already golden.
  X can be anything that is an Elemental, by any route: a printed
  Elemental, an all-type card (Gatekeeper Amalgam, Dark Paradox) or a
  minion the Amalgamation Dark Gift made every type. The golden result
  always gets Divine Shield (Era, 2026-11-03).
- **"After you play an X"** never counts the card being played, for every
  card (Ultraviolet Ascendant, Unbound Tempest, Wrath Weaver, Insatiable
  Ur'zul, the Pirate and Demon cards). It fires once the played minion's
  Battlecry, and whatever it asks (a Choose One, a target, a Discover), has
  resolved; a Brann repeat resolves all its runs first (logs, 2026-11-04:
  Wrath Weavers gain at an Imposing Percussionist's pick, not at the play).
- **Stone Age Slab before Living Prison:** on the same buy, Slabs buff the
  minion first, then an armed Prison gains its stats (buffs included).
- **Arcane Absorption** takes a random minion on a Health tie. Spell power
  (the Tavern spell bonus) adds on top, so +1/+1 gives +1/+1 more.
- **Ultraviolet Ascendant** improves only while it's on your board.
- **Unbound Tempest** fires every 4 Elementals played; the count carries
  over turns.
- **Stone Age Slab:** +20/+20, then double (golden: triple), once per turn
  per Slab. Several Slabs all fire on the same buy, left to right.

**Generated versus pool.** A card that gets or summons a card ("get a
random Undead", tokens, copies) makes it from nothing: no pool copy, and
selling it later adds nothing to the pool. A Discover takes its pick from
the pool, up to your tier. Lockbox and Silent Deliverer take pool copies
(their own rules above). Blizzard doesn't say, so these are guesses.

**Undead (2026-10-24).** Rot Hide Gnoll is not in the current pool, though
the JSON flags it.
- **"Your Undead have +N this game (wherever they are)"** (Nerubian
  Deathswarmer, Plaguerunner, Butchering, Forsaken Weaver) is a player
  bonus every Undead you own reads: board, hand, Tavern, summoned in combat
  (Skeletons too), and Undead you get later. A sold Undead loses it. Spell
  power adds to Butchering's: with a +1 Health Tavern spell bonus,
  Butchering also gives your Undead +1 Health wherever they are.
- **Butchering** gives +8 Attack; with a Health spell bonus its text shows
  +8/+N and it gives the Health too.
  The bonus lands before the destroy, so the Reborn copy comes back with
  it and Snazzy Phantom sees the higher Attack.
  Balinda does nothing for Butchering: like Channel the Devourer, the
  second cast does nothing because its target is gone, and it doesn't count
  as a spell cast (same for Channel the Devourer).
- **Avenge** counts only deaths in a fight, never shop deaths.
- **Forsaken Weaver** also reacts to spells cast during a fight.
- **Plaguerunner** gives +4 when it dies outside combat, +2 in a fight.
- **Reborn, in a fight or in the shop:** the minion fully dies (its
  Deathrattles fire, death triggers count), then comes back in the same
  slot as its plain card with 1 Health and no Reborn. Its own buffs are
  gone, but player bonuses (Undead Attack and Health wherever they are)
  apply, so it can come back with 50 Attack and 2 Health. This is the
  core of the Snazzy Phantom build. A minion Reborn in the shop keeps its
  1 Health (plus bonuses) for later turns and fights; it never heals back.
- **"A different friendly Undead"** (Dead Bellringer, Mummifier) excludes
  itself and any other copy of the same card.
- **Dead Bellringer:** you choose the target. It gets Reborn, then is
  destroyed (fully dies, comes back through Reborn), and Bellringer gains
  +4/+4.
- **Mummifier:** a random Undead without Reborn; none left, wasted.
- **Maw Caster, Butchering:** you choose the Undead to destroy; with none,
  nothing happens (Butchering can't be cast).
- **Eredar Escapist's countdown keeps the overshoot** (logs, 2026-11-04): it
  counts one per point of hero damage and fires at 0; after a batch past the
  countdown it shows the overshoot, not what was left. On "(4 left!)", 5
  damage fires it once and leaves "(1 left!)", so the next point fires it
  again (`10_27_04_30_04-g3` turn 10 step 25: two Cupcakes, from a 5 and a
  1). Every counter card works this way (Era, 2026-11-04): Enterprising
  Escapee, Sky Admiral Rogers, Gunpowder Courier and the rest keep the
  overshoot too. Counter cards count only on the board, never in hand
  (Era, 2026-11-04).
- **Tomb Turning:** the Discovered Undead dies right after it's played
  that turn, after its Battlecry (in docs/notes/UNSURE.md). Kept in hand to a
  later turn, it's a normal minion.
- **Lichling Hoarder:** a plain copy (no golden, no buffs) of a random
  minion that was on your board at the end of the shop phase, itself and
  dead ones included.
- **Deathly Striker:** a random Undead up to your tier. Its Deathrattle
  summons the Undead it got this fight; after the fight it's gone.
- **Stitched Salvager:** the minion to its left fully dies at Start of
  Combat (Deathrattles, death triggers, its own Reborn). The Deathrattle's
  exact copy keeps everything.
- **Barrier Banshee, Snazzy Phantom** trigger on other minions' Reborn,
  never their own. Phantom gives the right-most Undead +X/+X, X being the
  Reborn minion's Attack as it comes back (player bonuses included).

**Tier 7 spells** (Sacred Gift, Sharing is Caring, Hallowed Ritual,
Menagerie Tableware) are real cards but never come up in normal play: the
Tavern and every "up to your tier" draw stop at tier 6, so only an effect
that names tier 7 could give one.

**Disguised Graverobber** is in now that Undead are. Dark Paradox stays
out until Dark Gifts exist.

**Tavern spells versus other spells.** A Tavern spell is a
BATTLEGROUND_SPELL in the game data: the spells the Tavern offers, and
some generated ones (Pointy Arrow, Arcane Absorption, Conflagration).
Blood Gems, Gem Day and the Lockbox are plain spells, not Tavern spells.
Only Tavern spells count where a card says "Tavern spell" (Living Azerite,
Forsaken Weaver, spell counters like Felboar's), and only they get spell
power (the Tavern spell bonus).

**Read the card's exact words, every time.** "After you cast a spell"
fires for EVERY spell: Tavern spells, Blood Gems, Gem Day, Mukla's
Bananas, Spellcraft spells. "After you cast a Tavern spell" fires only for
Tavern spells. Devilish Distractor (Tier 5 Demon) works with a Blood Gem
or a Mukla Banana; Forsaken Weaver (Tier 6 Undead) doesn't. The engine
already tells the two apart (built with Blood Gems); keep it that way for
every new card and every new spell.

**A Choose One minion's play is not a spell cast (logs, 2026-11-04).** No
"whenever you cast a spell" card sees the option a Choose One minion picks
(`Hearthstone_2026_11_03_00_29_06-g1` turn 8: Vicious Mindslasher counts
three for a Veteran Brigand's three Blood Gem Barrages, not four). The
Tavern spells its option casts are casts.

**A spell a minion casts for you is not your cast (Era, 2026-11-02,
2026-11-03).** When a minion's effect casts a spell (Gatekeeper Amalgam's
Misplaced Tea Set, Air Revenant's Easterly Winds, a Magicfin token's taught
spell), the spell happens and does what it says, but it doesn't count as a
spell you cast: no "after you cast a spell" or "whenever you cast a Tavern
spell" trigger and no cast counter sees it. Your own casts do count,
including Balinda's and Hot-Air Surveyor's extra casts. The evidence:
- Era's log `Hearthstone_2026_11_03_15_22_36-g3` turn 10: ten Blood Gems on
  a Gatekeeper Amalgam, each firing its Tea Set (a Tavern spell). Both
  Felboars' counters drop by exactly one per gem, and a golden Timecap'n
  Hooktail gives nothing for the Tea Sets, while it fires on Era's own Tavern
  spells the same turn.
- Era knows for sure: a Magicfin token's spells don't count for Living
  Azerite or Forsaken Weaver.
- No count of yours adds to a minion's cast either: Balinda doesn't cast it
  twice (log `Hearthstone_2026_11_03_00_29_06-g1` turn 11 step 17, a
  Magicfin Apprentice's Fortify gives +0/+3 with Balinda out; logs,
  2026-11-04).
- Victorious Geomant's leftover Gems are the exception the other way: they
  are cast by you (the target's "whenever you cast a spell on this" fires,
  the "after you cast" counters see them), but they are not played from
  the hand (no Hot-Air Surveyor extra cast, no Surveyor Portrait bonus)
  (logs `10_24_17_59_26-g1` turn 12 step 40, `10_28_22_11_50-g3` turn 10;
  2026-11-04). Each leftover Gem is cast once, and Balinda casts it again
  (it targets a friendly minion): `11_03_15_22_36-g4` turn 12 step 8, one
  Gem entity cast twice on the left-most minion with Balinda out.

The open part is a spell a minion casts **on** another minion and that
minion's own "whenever you cast a spell on this" (Gatekeeper Amalgam,
Glambot's "on a Mech"). The rotated-out Fauna Whisperer (Naga, "at the end
of your turn, cast Natural Blessing on adjacent minions") did fire an
adjacent Amalgam's Tea Set: a confirmed interaction. Whether today's minion
casts do the same (a Magicfin token's Banana cast on an Amalgam) is
untested; see docs/notes/UNSURE.md. The engine today hides minion casts from
every cast trigger, "cast on this" included.

**Quilboar (2026-10-24).**
- **Blood Gem:** a 0-cost plain spell, "Give a minion +1/+1", generated,
  takes a hand slot. What one Gem gives is a player value starting at
  +1/+1; "your Blood Gems give an extra +X/+Y this game" adds to it (a
  Sanguine Refiner Rally in a fight still raises it for good). Gems played
  in a fight last that fight only; Razorfen Vineweaver's "permanent" ones
  carry out. No spell power on Gems.
- **Casting Gems more than once** follows "Set versus add" above: Balinda
  sets a Gem cast on your minion to 2 casts (golden 3), each Hot-Air
  Surveyor adds one. Balinda plus one Surveyor: 3 casts. Each cast is a
  Gem played.
- **Geomagus Roogug** reacts to every Gem played on it (from hand, extra
  casts, and Gems a minion or spell plays: Turbo Hogrider, Gem
  Confiscation, Blood Gem Barrage) except a Gem another Roogug played, and
  plays its Gem on a random different friendly minion. So two Roogugs never
  loop (Era, 2026-11-04; Hogrider confirmed in
  `Hearthstone_2026_10_28_02_28_32`: each Hogrider Gem on a Roogug opens a
  Roogug trigger. No log shows one Roogug's Gem landing on the other, so
  the Roogug exception is Era's rule, not log-backed).
- **Stats from Blood Gems** are tracked per minion (stats, not a gem
  count, so bigger Gems count fully), from every source, Barrage included.
- **Gem Confiscation:** plays 3 Gems on the target, then takes all the
  Blood Gem stats from its neighbours: they lose them, the target gains
  them.
- **Blood Gem Barrage:** on every Refresh (see "Two kinds of Refresh"),
  every Tavern minion
  gets +2/+2 worth of Blood Gems, plus the Gem bonus once (with Gems at
  +2/+2, that's +3/+3), plus spell power (it's a Tavern spell). Each cast
  adds on top (Veteran Brigand's three casts too).
- **Turbo Hogrider** waits until the Choose One card is fully resolved, so
  a Quilboar you just played (Sly Infiltrator) gets the Gems too.
- **Balinda on Gem Confiscation:** the second cast plays 3 more Gems on the
  target and steals nothing (the neighbours are already empty).
- **Gem Day** costs 0, only comes from Gem Rat, is not a Tavern spell, and
  is a Choose One card (Turbo Hogrider, Thorned Trailblazer).
- **Discard** (Mangled Bandit now, Aberration later): you choose a card in
  hand; any playable card, a Blood Gem included, never an unplayable one
  like the Lockbox. A discarded pool minion's copy goes back to the pool;
  a generated one adds nothing.
- **Thorned Trailblazer:** the first Choose One card you play each turn
  gets both effects, automatically. Several Trailblazers each cover one
  more card, left to right, so none is wasted.
- **Maximum Gold** (Snare Trapper, like Strike Oil) adds on top of the
  turn's gold and can go past 10.
- **Jailbird Juggernaut:** the Golem is summoned to its right, attacks the
  target first, and stays as a normal tier 1 minion with whatever stats
  it has left. If it kills the target, the Juggernaut's attack fizzles
  like Obsidian Ravager's. With a full board (7) no Golem is summoned,
  like any summon, and the Juggernaut just attacks normally.
- **Fandral's Fortune:** Discovers a Choose One card from any tribe, up to
  your tier, from the pool; it has both effects combined.

**Aberration and the Deity (2026-10-24).**
- **The Deity** exists only in lobbies with Aberrations. The whole lobby
  gets the same god, chosen at random: C'Thun or Y'Shaarj. Every player's
  Deity starts as a 1/1; "give your Deity +X/+Y" grows it for the rest of
  the game, also from a fight (Drifting Sacrifice). In each fight, after 4
  friendly Aberrations die (tokens and Reborn copies count), it awakens at
  the right end with its current stats as a normal summon; with no room it
  waits for space. At most one awakening per fight; the count starts over
  each fight. C'Thun splits its stats among your other minions (equal
  shares, the remainder +1 to random ones; golden double). Y'Shaarj's
  Deathrattle brings back your first 2 Aberrations that died that fight
  with their maximum stats (golden 4). A golden Deity only comes from a
  trinket, which isn't built yet.
- **"If you discard this, cast it twice"** (Sludge Corrosion, Energizing
  Chamber): those are two real Tavern spell casts (Vicious Mindslasher,
  Sha of Fear, Faceless Converter, Living Azerite, Forsaken Weaver, spell
  counters, spell power). Corrupted Coin discarded isn't cast at all: it
  just raises your maximum Gold by 2 (played normally, it's a Tavern spell
  that gives 2 Gold).
- **Linked cards** (Wandering Willbreaker, Faceless Operative): playing one
  discards the other, a real discard with every discard trigger. The link
  lasts across turns. Golden: four cards, playing two discards the rest.
- **Random Tavern spells** (Willbreaker, Abyssal Envoy, Unwilling Slacker)
  come from the Tavern spells up to your tier, generated.
- **Mysterious K'Thir** discards your 3 left-most Tavern spells in hand at
  the end of turn (skipping other cards), as normal discards.
- **The Shadow of Doubt** reacts to every card added to your hand: bought
  minions, Blood Gems, Triple Rewards, Discover picks, everything.
- **Hidden numbers:** Mindbending Recruiter's Activate costs 0. Harbinger
  Aph'lass gives +2/+1 and improves by +2/+1 per discard. Parasitic
  Fleshling improves by +2/+2 per card discarded this game. Faceless
  Converter improves by +2/+1 per Tavern spell cast this game.
- **De-volition-ist:** a Health tie picks at random, and it fires even if
  it died attacking (like Blade Collector or Wildfire Elemental).
- Corrupted Coin and Energizing Chamber come in with this tribe.

**The lobby's tribes (2026-10-24).** Each game picks exactly 5 of the 10
tribes, uniformly at random. In the pool: minions with at least one lobby
tribe (a dual-tribe minion is in if either tribe is), plus neutrals and
spells, except neutrals and spells the game data ties to a tribe outside
the lobby (`lobby_subsets` in the export: Nadina the Red needs Dragon,
Disguised Graverobber needs Undead, and 26 Tavern spells are tied the same
way). The Deity only exists with Aberrations in. Nothing from outside the
lobby ever shows up: every Discover, "get a random X" and random draw uses
the lobby. Planar Telescope and Friendly Bounty count only lobby tribes
for "your most common type" (a Felboar without Quilboar in the lobby is
just a Demon). (Rare trinket, hero and timewarp effects break this in the real
game; none exist here yet, and each can widen its own draw when it comes.)

**What the bot sees (the observation, 2026-10-25).** What a player sees
in the real client, nothing more. Own side: everything the interface or
the cards show (gold, tier, gem value, spell power, the Deity's stats,
"N left" counters, improved numbers). Opponents: health, tier, placement,
the lobby, and from your last fight against them their board, Deity stats,
spell power and gem value (shown during combat only, so they are "as last
seen"). My tracker reads exactly this from a real game (its `Fight`
record), and the suggestion window will build the observation from it.

**Uses run out per minion, and a golden from a triple starts fresh.**
Effects with limited uses (Activate once per turn, Stone Age Slab once per
turn, Thorned Trailblazer's combines each turn) belong to the minion. A
triple's golden comes with its own full uses, and the copies take their
leftovers with them: an unused Trailblazer tripled gives a golden with
exactly 2 combines that turn, not 3; a used-up one tripled gives a fresh 2.
(The strategy is to use them up before tripling.) Progress counters are
different: they keep the highest count (see "Triples keep the highest
count").

**Pairing:** at the start of each turn, shuffle the living players and
pair them, avoiding the previous opponent if possible (up to 16 shuffles).
Pairs are known during the shop phase. With an odd number, one plays the
ghost of the most recently knocked out player, and damage to the ghost is
ignored.

**Damage:** the winner's tier plus the tiers of its surviving minions
(tokens count as tier 1), capped by turn as `rules.toml` says (5 on turns 1
to 4, 10 on turns 5 to 7, 15 after that, none once 4 or fewer players are
left).

**Knockouts and the end:** players knocked out in the same turn are ranked
by health left, ties at random; the best ranked of them becomes the ghost.
A knocked-out player's cards go back to the pool. The game ends when one
player is left, or at turn 40 (the rest ranked by health).

**Fight details** where Part 1 leaves room (they match the older private
simulator's engine, so the two can be compared): dead minions are handled
one at a time in board order, each one's Deathrattle and then its Reborn
before the next; the next attacker is tracked with a pointer that shifts
when minions left of it die or appear; Windfury swings twice in a row.
Safety cap: 500 attacks, then a tie.

## Heroes (2026-10-25)

**Which heroes.** The data is `ref/bg-meta.json`. Each hero plays with its
STARTING power, `heroes[].hero_power_dbf_id`. The `hero_powers` list also
holds powers a hero only gets later (Aranna's Spectral Sight is unlocked,
Bru'kan's Water Invocation is chosen): those are never a starting power.
Naga are gone, so Naga-tied heroes are out. A hero tied to tribes is
offered only if at least one of its tribes is in the lobby. Only the heroes
built so far are offered; the rest come in batches. Buddies are out, except
through E.T.C.'s power (a late batch).

**The pick.** Before turn 1 each player is offered `hero_choices` heroes
(rules.toml, 2 by default, a free account; the Battle Pass gives 4 and a
reroll, which we skip) and picks one. No hero is offered twice in a lobby.
The pick happens before turn 1 starts, so start-of-turn powers work on
turn 1. Training and `bgbench` games skip the pick: every player gets a
random hero, no two the same (Era, 2026-10-25). The pick stays for the
sandbox. Unmasked Identity never offers Patchwerk's power; Finley can
find it, and then starts with its 30 extra Health.

**Health and armor.** A hero has 30 Health plus its armor from the data
(Patchwerk: 60 Health from his power, no armor). Every point of damage to
the hero, from fights and from the shop (Health costs, "take damage"),
takes armor first. Armor never comes back unless a card says so. Self
damage (Health costs, "take damage") counts the same whether armor or
Health took it, like every other self damage, and Soul Rewinder and Ashen
Corruptor rewind all of it: the armor lost comes back as armor.
Armor Stash ("Set your Armor to 5") sets, it doesn't add.

**The power.** A button power costs its Gold and works once per turn
unless its text says otherwise ("Twice per turn", "Once per game", "Unlocks
at Tier N", "Unlocks on Turn N"). A power with no cost in the data costs 0
or is passive.

**Heroes with rules beyond their text** (two powers, skipped turns,
powers that change, guessing, Bananas): `docs/rules/heroes.md`. It carries the
same weight as this file.

**Getting a card from the Tavern without buying it** ("steal"): the card
goes to hand for free, leaves the pool like a bought copy, and is not a
buy, so "after you buy" triggers don't fire.

## Trinkets and Dark Gifts (2026-10-30)

The offer (turns, guarantee, discount), Gold and Tavern rules, Mystery
Cube, copies, Spellcraft, trinket-only minions, rewrite portraits and what
we know of Dark Gifts: `docs/rules/trinkets.md`. It carries the same weight as
this file.

## Actions

Buy a minion or the spell, sell, play (minion to a position, or cast a
spell), pick a target on the board, pick a Tavern minion, choose an option
(Discover, Choose One), move, Activate, roll, freeze, level, end turn. One
flat list with a legality mask from the engine; `crates/bgcore/src/action.rs`
has the layout. A turn is capped at 60 actions.

## Not in this game yet

Most hero powers (they come in batches), about 40 trinkets, quests, Dark Gifts,
Duos. Naga are gone from the real game.
