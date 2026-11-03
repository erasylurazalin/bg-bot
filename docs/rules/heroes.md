# How the non-obvious heroes work

Era's rules for heroes whose card text leaves room. They carry the same
weight as `docs/rules/design.md` Part 2. The general hero frame (the pick,
health and armor, what a power costs) is in `docs/rules/design.md` "Heroes".
The rules still being guessed at are in a private notes file
(`docs/notes/UNSURE.md`).

**Built 2026-10-31 (hero frame 2):** everything on this page except the
guessing powers (Barov, Holmes, I Spy): the two powers (Genn, the
Reinvigorating Light trinket), the powers that change (Ragnaros, Bru'kan,
Galewing, the Rat King), the skipped turns (A. F. Kay, Faelin) and Mukla's
Bananas.

**Built 2026-11-01 (hero batch 1):** 28 heroes and the rules under "Hero
batch rules (Era, 2026-11-01)" that are theirs; the rest of that section
waits for later batches.

**Built 2026-11-01 (hero batch 2):** 22 heroes and their rules under "Hero
batch rules (Era, 2026-11-01)" ("Stats and targets", "The Tavern",
"Guessing" for I Spy).

**Built 2026-11-01 (hero batch 3):** 24 heroes and their rules under "Hero
batch rules (Era, 2026-11-01)" ("Start of Combat and fights", "Kills and
counters" and the Power of the Storm line under "Other"), with the guessing
powers (Lord Barov, Murloc Holmes) from "Guessing".

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
`docs/rules/design.md` "Tavern spells versus other spells".

## Hero batch rules (Era, 2026-11-01)

Era's answers for the plain heroes. Where the rule says "plain" or
"exact", it is the card's own word: a plain copy has printed stats and no
buffs; an exact copy keeps everything (buffs, keywords, golden, Dark
Gifts) and takes nothing from the pool.

**Gold and prices**
- Pirate Parrrrty!: each Pirate bought takes 1 off the next power press;
  the discounts stack until you press, never below 0.
- Lead Explorer: +1 cost per use for the whole game.
- Piggy Bank: once per game; starts at 2 Gold and grows by 1 each turn you
  wait.
- Lucky Roll: a fair die; the power is locked for as many turns as rolled.
- Reliquary Research: every third Tavern spell bought is free, then the
  count starts over.
- Manastorm: minions and Refreshes cost 2; Tavern spells keep their own
  cost; upgrading costs 1 more at every tier.
- Swap, Lock & Shop It: your minion goes into the Tavern (buyable), the
  Tavern minion takes its place on your board; both keep their buffs; not
  a buy, not a play.
- The Perfect Crime: steals the Tavern's cards left to right (at most 7,
  the spell included). When the hand is full, the rest are not added: they
  disappear. Steals, so no buy triggers.
- The Galaxy's Lens, The Perfect Crime: the next press costs 1 less at the
  end of each turn, stacking down to 0. The Lens's copy is made fresh.

**Counters** (all passives repeat; none is once per game unless it says
so)
- Sharpen Blades: +1/+1 more every 4 cards bought, all game.
- For the Horde!: the Tavern buff grows +1/+1 every 3 minions bought;
  spells don't count.
- Glaive Ricochet: a plain copy of a random one of the three.
- Natural Balance: counts the tiers of cards bought (a spell by its tier);
  each 20 gives a Triple Reward (your tier + 1), and it starts over.
- Battle Brand: Battlecry minions bought; once per game, as written.
- Gone Fishing: any sale counts, by effects too; repeats every 5.
- Verdant Spheres: minions bought across turns, repeats every 3.
- Avalanche: the discounts stack on the current upgrade price, down to 0.
- Choose Your Champion: the Tier 7 minion is Discovered at the start of
  the game and sits in hand locked (`docs/rules/design.md` "Cards locked in hand")
  until you have spent 60 Gold; only Gold counts, not Health.
- Upbeat Harmony: with an empty hand nothing happens.
- Heroic Inspiration: every 15 friendly attacks, a Triple Reward (your
  tier + 1), repeating.

**Discover and generate**
- Reclaimed Souls: any minion that died last combat, on either side,
  tokens included (a Beetle can be found).
- Tinker: Magnetic Mechs up to your tier, from the pool.
- Trash for Treasure: removing the minion is not a sale (no Gold, no sell
  triggers) and its pool copy goes back. The Discover is one tier lower,
  from the pool; on a Tier 1 minion it Discovers another Tier 1.
- Three Wishes: 3 wishes per game. Two copies of one plain card (hand or
  board) make a pair, and the third comes from the pool; goldens don't
  count. With two pairs, one is picked at random.
- Puzzle Box: the spell picks a random target and counts as a Tavern
  spell cast. It never casts destructive spells (Channel the Devourer,
  Butchering, Unmasked Identity and the like).
- Stir the Pot: throw a minion from your board or the Tavern (not hand);
  it is gone for good and doesn't go back to the pool. A neutral minion (no
  type) can't be thrown. At 3, Discover with
  one option per thrown minion's type: a Demon, a Beast and a Mech give one
  of each; three Mechs give three Mechs.
- Buried Treasure: digs 1 to 3 give nothing; the 4th gives a golden minion
  up to your tier.
- Kel'Thuzad's Kitty: Discovers from the dead player's board as it was at
  its last moment (not as you saw it), with all buffs and Dark Gifts;
  goldens can be offered. Heroes die at the end of a fight, so
  when several die in one round, you get one Discover per dead hero, back
  to back, at the start of the next turn.

**Stats and targets**
- See the Light: the minion goes to your hand as a 2/2, as a steal.
- Major and Minor Hymn: the stat switches each turn (Attack, then Health);
  the bonus is your tier at the press.
- Reborn Rites, Spirit Swap: any minion, the Tavern's included, until next
  turn. Spirit Swap: choose one minion, then a different one; they swap
  Attack at once.
- Rune of Damnation: a real death (Deathrattles fire), any friendly
  Undead.
- Hat Trick: a minion can wear several hats; each stays +1/+1. A sold
  minion's hats move to a random friendly minion, so hats are only lost
  when the whole board is sold or a hatted minion is destroyed (Butchering).
  Hats survive a triple.
- Cloning Gallery: an exact copy (golden, gifts, everything), summoned
  (summon triggers fire), no pool copy taken. Not on a full board. The copy
  counts toward a triple.
- Double Time: a golden from 2 copies keeps both copies' buffs, gives a
  Tavern Coin instead of a Triple Reward, and works on pairs already held.
- Saturday C'Thuns!: one more repeat each turn, all game, each repeat on a
  random friendly minion; Drakkari doubles the whole power.
- Tavern Lighting: +1/+1 more every 3 turns, on top of spell power.
- Devour: works like Channel the Devourer: sells the minion (Gold and sell
  triggers) and gives its stats, buffs included, to another; only stats,
  not effects.
- Arcane Alteration: your board and the Tavern, spells included. A Tavern
  card's buffs carry to the new card.

**The Tavern**
- Temporal Tavern: the two higher-tier minions take two normal slots; at
  most Tier 6.
- Twice as Nice: after a Refresh (the start-of-turn roll included), it
  freezes the Tavern's highest-tier minion and adds an exact copy of it to
  the Tavern, frozen too, outside the normal slots (5 minions and a spell
  become 6 and a spell). Works with Golden Touch: gild the top minion,
  freeze it, and next turn the copy is golden too.
- Bob's Burgles: plain copies of the opponent you last fought, made fresh.
- Enhancification: a random minion each time (both can land on the same
  one); skips a minion with every keyword.
- Imprison: the card goes to your hand without a buy, locked 2 turns, and
  leaves the pool; like any locked card it can be buffed in hand.
- Galakrond's Greed: Discover from exactly one tier higher; the old card
  goes back to the pool and its own buffs go with it. The pick lands in the
  Tavern, so it gets the Tavern's standing "this game" buffs like any card
  in the Tavern (Era, 2026-11-02; the log shows a 6/8 offer landing 46/48).
- Come One, Come All!: a Darkmoon Ticket is a mark on a Tavern minion or
  spell (assume a 33% chance per card). Buying the card takes the Ticket
  and adds 1; at 3 you get a spell like a Triple Reward that Discovers a
  minion of your own tier (a Triple Reward is tier + 1).

**Start of Combat and fights**
- Wingmen: both minions attack, left then right, before every other Start
  of Combat; a Start of Combat minion they kill never fires.
- ALL Will Burn!: every minion on both sides at Start of Combat gets +2
  Attack and keeps it, the opponent's included; later summons don't get it.
- Sprout It Out!: a passive, every fight: every minion you summon in
  combat, Reborn copies and tokens included.
- Embrace Your Rage: the summon fights that fight only; the card goes to
  hand even if the board was full; it counts as a summon.
- Fragrant Phylactery: ties at random; the stats are taken when the
  minion dies. Best on a minion that grows during the fight (Heroic
  Underdog). If Start of Combat effects trigger twice (Valdrakken Wind
  Chimes and the like), the minion gets the Deathrattle twice.
- Broodmother: the Whelp attacks a random enemy; +1/+1 for the game.
- Tentacular: once per fight; the growth lasts the game.
- Rapid Reanimation: the destroyed minion's Deathrattle fires first; the
  exact copy waits for space.
- Wax Warband: like Misplaced Tea Set (one minion per type); +1/+1 more per
  10 Gold spent.
- Frostwolf Fervor, Stormpike Strength: once per fight, an exact copy; ties
  at random.
- Lock and Load: a 0-cost power, once per turn: pick a Tavern minion; next
  combat it is summoned as soon as you have space and attacks at once,
  then acts like any summoned minion.
- Pilot the Shredder: you start with Sneed's New Shredder, a 2/1 with
  "Deathrattle: Summon the highest-Health minion from your hand and give it
  Divine Shield for this combat" (golden: the 2 highest, 4/2).

**Kills and counters**
- I'll Take That!: any kill by your side counts, Deathrattle and Rally
  damage included; a plain copy.
- MechGyver: shop deaths and tokens count; repeats every 9.
- Glory of Combat: only the minion that attacked and killed gets +1.
- Demon Hunter Training: once it triggers it stays: the first minion each
  turn is free. A Windfury minion's second attack counts.
- Bloodfury: the portal gives you 2 random Demons at the start of each
  turn.
- Conviction: after each fight choose one of Defensive Stance (+1 Health),
  Offensive Stance (+1 Attack) or United Stance (one more minion), for the
  game. United goes up to 7 minions; after that only the other two are
  offered.

**Other**
- Power of the Storm: two new powers at the start of every turn, before
  Gold, each fresh; the same exclusions as Unmasked Identity and Genn.
- Menagerist, Avatar of N'Zoth: the Amalgam and the Fish are normal
  minions you start with and can sell.
- Friendly Wager (Lord Barov): a tied fight gives 1 Gold.

## Later, or never

- Later: Akazamzarak's Secrets (they are in the game), Denathrius' quests,
  Putricide's Build-An-Undead, E.T.C.'s buddies.
- Wanted, not built yet: Tickatus' Darkmoon Prizes (a trinket gives them
  too).
- Probably never: the StarCraft heroes (Raynor, Artanis, Kerrigan).

## Secrets (Era, 2026-11-01)

The Great Akazamzarak's Prestidigitation costs 0 and can be used once each turn from turn 1. It offers every combat Secret you don't already have, and you pick one. With all four up, the power can't be used.

- **The pool is 4 Secrets:** Venomstrike Trap, Autodefense Matrix, Redemption, Pack Tactics. The other Secrets in the data (the Better versions, Snake Trap, Splitting Image, Effigy, Avenge, Hand of Salvation, Competitive Spirit, Reckoning) aren't in the game. Eleventh Hour belongs to an Anomaly.
- A Secret stays, turn after turn, until it fires. Then it's gone.
- Several Secrets can fire on the same event. They fire in the order you got them.
- A Secret that summons something waits until there's room: on a full board it doesn't fire and stays for later.
- **Venomstrike Trap:** when one of your minions is attacked, summon a 2/3 Venomous Cobra (a Beast).
- **Autodefense Matrix:** when one of your minions is attacked, give it Divine Shield before the hit, so the hit is blocked.
- **Pack Tactics:** when one of your minions is attacked, summon an exact copy of it to its right (golden, Deathrattles, keywords, Dark Gifts, everything) whose stats are set to 3/3. "Set to 3/3" sets the base stats and drops ordinary buffs, but bonuses that apply wherever the card is go on top: an Eternal Knight copy is 7/5 if one Knight has died, Undead get their game-long Attack, and Goldrinn's "your Beasts have +7/+7 until next turn" reaches the copy (and the Cobra) too.
- **Redemption:** when a friendly minion dies, return a fresh copy of it (base card, buffs gone) with 1 Health on the same spot. If its Deathrattle fills the board, it waits for room.
- Opponents don't see your Secrets, and neither does the bot (only its own).
- **Ice Block** (Safety Patch) is labelled a Secret but has nothing to do with these four. In the engine it lives in the same Secret zone, and it doesn't count toward Akazamzarak's offer.

## Start of Combat powers that trigger twice (Era, 2026-11-01)

Valdrakken Wind Chimes and Ancient Wishbone make every Start of Combat hero power trigger an extra time, the older ones (Al'Akir, the Invocations) included, not just batch 3's.

## Exact copies in a fight (Era, 2026-11-01)

Frostwolf Fervor, Stormpike Strength and Rapid Reanimation copy the minion's state right now: damage included (a 10/10 that took 6 damage gives a 10/4 copy), and a Divine Shield that already popped stays popped on the copy.

