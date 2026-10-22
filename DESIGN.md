# bg-bot design

Three parts. The first describes Hearthstone Battlegrounds as completely as I
can, as the reference for everything the engine will grow into. The second
defines v0.1, the mini game the first bot learned on; it's history now, kept
because the v0.1 rule tests still run on it. The third is the game we build
now: a lobby of five real tribes, exactly as in the current patch.

`docs/ENGINE.md` maps the code, `docs/CONCEPT.md` explains the card pool
choices.

Facts marked **(check)** are from memory and may be stale or wrong. Settle
them from Power.log (`~/projects/bg` reads it) or from playing before the
engine depends on them. Fight rules marked as checked come from
`~/projects/bg/docs/scripted.md`, section 1, where they were compared against
real logs.

# Part 1: Battlegrounds

## The lobby

- 8 players, solo. Each picks a hero from a small offer (2 free, 4 with the
  Tavern Pass). The hero sets starting health, armor and a hero power.
- Health is 30 for most heroes, plus armor that differs by hero (stronger
  heroes get less) **(check the current values)**.
- Each lobby uses 5 of the 10 minion types: Beast, Demon, Dragon, Elemental,
  Mech, Murloc, Naga, Pirate, Quilboar, Undead. Neutral minions are always
  in. A minion with two types is in if either type is.
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
  6 costs 10 **(check, these change between patches)**. The countdown for the
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
- The shop refreshes at the start of each turn and on every roll. A frozen
  shop keeps its minions for the next turn, and its empty slots fill up.
- Selling a minion returns it to the pool. Tokens (summoned minions that
  aren't in the pool) just disappear.

### Hand and board

- Hand holds 10 cards, the board 7 minions. A bought minion goes to hand.
  Playing it puts it on the board at a position you choose and triggers its
  Battlecry.
- The board order matters in the fight, and you can rearrange it freely
  during the shop phase.

### Triples

- Getting a third copy of the same minion (in hand or on board) merges the
  three into a golden version in your hand.
- A golden minion has double the printed stats plus the bonus stats of all
  three copies that made it. Its effect is doubled (twice the numbers, or the
  effect triggers twice).
- Playing a golden minion gives a Triple Reward: a spell that discovers a
  minion from one tier above your current tier (tier 6 at most).

### Spells

The shop offers Tavern spells (one slot, its own costs). Many minions react
to spells cast ("Spellcraft" gives a one-turn spell when played, some
minions grow per spell). Spell counts can last the whole game: the Dark Gift
Spell Siphon counts spells cast through the game, not the fight.

### Hero powers

Each hero has one. Some are active (pay gold, click), some passive, some
fire at Start of Combat or during the fight. They range from small stat
bonuses to rule-changing ones. `~/projects/bg/docs/scripted.md` section 7 has
the ones that act in a fight.

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

# Part 2: v0.1, the mini game (history)

Superseded by Part 3. The engine still carries these rules in
`crates/bgcore/tests/data/v01-*.toml` for the old tests.


Goal: the smallest game that still has the real decisions (economy, leveling,
rolling, triples, board order) and plays fast. Everything left out is listed
at the end.

## Rules

Same as Part 1, with these choices fixed.

- **Players:** 8. Health 30, no armor, no hero powers, no hero choice.
- **Gold:** turn `t` gives `min(t + 2, 10)`. Buy 3, sell 1, roll 1, freeze
  free.
- **Tiers:** 1 to 6, base upgrade costs 5, 7, 8, 11, 10, minus 1 per turn
  without upgrading, minimum 0. These are config values, not constants in the
  code.
- **Shop:** 3, 4, 4, 5, 5, 6 minions by tier. No spell slot.
- **Pool:** copies by tier 16, 15, 13, 11, 9, 7, only minions from the v0.1
  card list. One lobby always uses both types.
- **Refresh:** start of each turn and on each roll. Unbought minions go back
  to the pool. A frozen shop keeps its minions and fills empty slots at the
  start of the turn. Freezing lasts one refresh.
- **Selling:** returns the minion to the pool, a golden one returns 3 copies.
  Tokens just vanish.
- **Triples:** as in Part 1. The golden minion gets double printed stats plus
  the bonus stats of all three copies. Playing it adds a Discover choice from
  tier `min(tier + 1, 6)`, offered as 3 different minions taken from the
  pool. The two not picked go back.
- **Golden effects:** buff numbers doubled. Summons make the same number of
  tokens, but golden ones (double stats).
- **Pairing:** at the start of each turn, shuffle the living players and
  pair them, avoiding the previous opponent if possible (up to 16 shuffles).
  Pairs are known during the shop phase, as in the real game. With an odd
  number, one plays the ghost of the most recently knocked out player, and
  damage to the ghost is ignored.
- **Damage:** winner's tier plus the tiers of its surviving minions (tokens
  count as tier 1). Capped as in Part 1: 5 on turns 1 to 4, 10 on turns 5
  to 7, 15 after that, none once 4 or fewer players are left. The turns and
  caps are config values.
- **Knockouts:** players knocked out in the same turn are ranked by health
  left, ties at random. The best ranked of them becomes the ghost. A
  knocked-out player's cards go back to the pool.
- **End:** when one player is left, or at turn 40 (rank the rest by health).
- **Fight:** exactly the rules in Part 1, minus Start of Combat (no v0.1 card
  has it). Safety cap: 500 attacks, then a tie. Where Part 1 leaves room,
  the engine does what `~/projects/bg`'s engine does, so the two can be
  compared: dead minions are handled one at a time in board order, each
  one's Deathrattle and then its Reborn before the next; the next attacker
  is tracked with a pointer that shifts when minions left of it die or
  appear; Windfury swings twice in a row.

## Cards

Two types, Beast and Mech, plus neutrals. 24 minions, 4 per tier, all made up
for v0.1 and loosely modelled on real ones. Tokens are not in the pool.

| Tier | Name | Type | Stats | Text |
|---|---|---|---|---|
| 1 | Wolf Pup | Beast | 2/2 | |
| 1 | Rust Bot | Mech | 1/2 | Divine Shield |
| 1 | Den Mother | Beast | 1/1 | Deathrattle: summon a 1/1 Pup token (Beast) |
| 1 | Scrap Hound | Neutral | 2/1 | Battlecry: give a friendly minion +1/+1 |
| 2 | Ram | Beast | 2/4 | Taunt |
| 2 | Shield Drone | Mech | 2/3 | Divine Shield |
| 2 | Viper | Beast | 2/1 | Venomous |
| 2 | Tinkerer | Neutral | 3/3 | Battlecry: give a friendly Mech +2/+2 |
| 3 | Stag | Beast | 4/4 | Battlecry: give your other Beasts +1/+1 |
| 3 | Gear Golem | Mech | 3/5 | Taunt. Deathrattle: summon a 2/2 Gear token (Mech) |
| 3 | Twinblade | Neutral | 3/3 | Windfury |
| 3 | Ember Spirit | Neutral | 3/2 | Reborn |
| 4 | Alpha | Beast | 5/5 | Deathrattle: summon two 2/2 Wolf tokens (Beast) |
| 4 | Shredder | Mech | 4/6 | Divine Shield |
| 4 | Warden | Neutral | 3/7 | Taunt |
| 4 | Stalker | Beast | 5/3 | Venomous |
| 5 | Great Bear | Beast | 7/7 | Taunt. Battlecry: give your other Beasts +2/+2 |
| 5 | War Engine | Mech | 6/6 | Divine Shield, Windfury |
| 5 | Undying Knight | Neutral | 6/4 | Taunt, Reborn |
| 5 | Brute | Neutral | 8/8 | |
| 6 | Ancient Beast | Beast | 9/9 | Deathrattle: summon three 3/3 Beast tokens |
| 6 | Titan Mech | Mech | 8/8 | Divine Shield, Taunt. Deathrattle: summon a 4/4 Mech token |
| 6 | Champion | Neutral | 10/10 | Windfury |
| 6 | Plaguebringer | Neutral | 6/6 | Venomous, Reborn |

Numbers will need balancing once bots play it. A card that wins every game
is a balance bug, not a finding. Cards live in a data file (`cards.toml`),
and the rule numbers in `rules.toml`, so changing a number doesn't touch the
logic.

Targeted Battlecries (Scrap Hound, Tinkerer) let the player pick another
friendly minion. With one valid target it is picked automatically, with none
the minion is played and the Battlecry does nothing. Battlecries that buff
"your other Beasts" (Stag, Great Bear) reach the board only, not the hand.

## Actions

One player's shop phase is a sequence of actions:

| Action | Arguments | Legal when |
|---|---|---|
| Buy | shop slot 0..5 | slot filled, gold ≥ 3, hand not full |
| Sell | board slot 0..6 | slot filled |
| Play | hand slot 0..9, position 0..7 | board not full |
| Target | board slot 0..6 | a Battlecry is waiting for a target |
| Choose | option 0..2 | a Discover is waiting |
| Move | board slot to board slot | both in range |
| Roll | | gold ≥ 1 |
| Freeze | | always (toggles) |
| Level | | tier < 6, gold ≥ current cost |
| End turn | | always |

While a Target or Discover is waiting, only Target or Choose is legal, End
turn included.

The action space is a flat list of all of these (149 entries) with a
legality mask from the engine. A turn is capped at 60 actions, after which
the engine ends it, and a waiting Target or Discover takes its first
option.

**Order between players.** The real shop phase is simultaneous, and the only
thing players share is the pool. v0.1 runs players one action at a time in a
random order that changes every turn. That's close to "whoever clicks first
gets the minion" without needing real time.

## Observation

Per player, as fixed-size arrays (no strings):

- Own: gold, tier, upgrade cost, health, turn, frozen flag, the shop (6
  slots), hand (10), board (7), pending Target or Discover.
- Each minion slot: card index, attack, health, golden, keyword flags, empty
  flag.
- Each opponent (7 slots, in a fixed order relative to me): alive, health,
  tier, turns since I last fought them, and that board (7 minion slots).
- Who I fight next, once pairings are known (the real game shows it).

The exact tensor layout belongs to the training side. The engine exposes
the state as plain structs and a function that writes the arrays.

## Engine structure (Rust)

```
bg-bot/
  cards.toml          card data for the current version
  rules.toml          rule numbers: gold, costs, pool, damage caps, limits
  crates/
    bgcore/           rules only: state, shop, fight, bots, no I/O, no Python
    bgpy/             PyO3 bindings: a vectorised env for training (Stage 1)
    bgbench/          benchmarks, fuzzer, bot matches
  train/              Python: PyTorch, PPO, evaluation (Stage 1)
```

- **State:** plain structs with fixed capacity (`arrayvec` or arrays plus a
  length). A minion is a card index, attack, health, a keyword bit set and a
  golden flag. No heap allocation inside a fight or a shop action.
- **Effects as data:** card text is an enum of effects (`Summon { card, n }`,
  `Buff { who, attack, health }`, ...) attached to triggers (`Battlecry`,
  `Deathrattle`, later `StartOfCombat`, `Avenge(n)`, ...). New mechanics add
  variants, not one-off code per card, and the cards file picks from them.
- **Randomness:** one seeded RNG per game, xoshiro256++ written in bgcore
  (not a crate, so crate updates can't change replays). A seed plus the list of actions replays a game exactly,
  which is how bugs get reported and tests get written.
- **Fight as a pure function:** `fight(&Board, &Board, &mut Rng) ->
  Outcome { winner, damage }`. It can be benchmarked, tested and compared
  with bg's engine by itself.
- **Vectorised env:** bgpy holds thousands of games, steps them all with one
  batch of actions from Python and returns one batch of observations and
  masks. That keeps the GPU fed and the Python overhead per step small.
  Games run in parallel on all cores with `rayon`.

## Testing

- **Unit tests per rule:** Taunt, Divine Shield, Reborn, Venomous, Windfury,
  Deathrattle position, the 7-minion limit, triples, pool counts, upgrade
  discounts.
- **Invariants checked in debug builds:** pool copies plus copies held by
  players always equal the starting count; gold never negative; boards never
  over 7, hands never over 10.
- **Cross-check with bg:** v0.1 cards are made up, but the keyword rules are
  the same. Build the same keyword-only boards in both engines, run 10,000
  fights each, and the win, tie and loss rates should agree within noise.
- **Random-play fuzzing:** millions of random legal actions, no panics, no
  broken invariants.

## Speed targets

Measured with random legal actions on 32 threads, release build:

- a fight: a few microseconds
- a full 8-player game: well under a millisecond on one core
- in total: at least 10,000 games per second

If v0.1 can't do that, fix the engine before training. Later content will
only make it slower.

## Training (outline, details later)

- PPO with action masks. One network shared by all seats, so one game gives 8
  players' worth of experience.
- Reward: placement only, mapped to roughly +1 for 1st down to -1 for 8th,
  given at knockout or game end.
- Opponents: the current network, plus older copies, so it doesn't learn
  moves that only beat itself.
- Baselines to beat: random legal moves, and a rule bot (buy the
  highest-tier minion, level on a fixed curve, triple when possible, never
  roll below 3 gold). Progress is measured as average placement against
  them.

## Not in v0.1

Hero choice and hero powers, armor, spells, Magnetic, Avenge, Rally, Start of
Combat, End of Turn effects, Cleave, Stealth, Poisonous, Mega-Windfury,
trinkets, quests, Dark Gifts, the Deity, the other 8 types, type bans, the
real card pool, the turn timer. They come back in later
versions, roughly in the order in `TODO.md`.

# Part 3: the five-tribe lobby

The game the bot learns now. Everything in Part 1 applies, with the
choices below fixed. Where Part 1 says **(check)**, the engine uses the
value in `rules.toml`.

## The pool

- Five tribes: Beast, Mech, Dragon, Murloc, Demon. Every real solo-pool
  minion of those tribes, plus neutrals, tiers 1 to 6. A card with two
  tribes is in if either is.
- Spells: every solo-pool Tavern spell, minus the ones the card data ties
  to a tribe outside the five (`battlegroundsAssociatedRaces`).
- Out because they need systems this game doesn't have: Dark Paradox
  (Dark Gifts), Armor Stash (armor), Energizing Chamber (the Deity),
  Unmasked Identity (hero powers). No Duos-only cards.
- That is 130 minions and 50 spells, about a real lobby's density. The
  list is built from Blizzard's card data, see `docs/ENGINE.md`.
- Generated cards that belong to these tribes are in too, though never in
  the pool: Chromadrakes, Demon Fodder, Fishbait, Satellites, Beetles and
  so on.
- Spells have no limited pool. (Assumption; I haven't checked it.)
- Some groups share one pool slot: the three Volumizers are one slot and a
  drawn one gets a random color. (The Bounty spells do the same in the
  real game; they're Pirate cards, not in this lobby.)

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

**Activate.** A minion with `Activate (N)` can be used any number of times
a turn for N gold each.

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
buff, even a dual or all-type one.

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

**Balinda** doubles every spell cast on a friendly minion (not spells
with no target, nor spells cast on a Tavern minion), including
Robust Evolution (the second cast evolves the result again) and Lost
Staff of Hamuul ("cast on this" triggers fire twice). Channel the
Devourer's second cast does nothing: the minion is already sold.

**Two kinds of Refresh.** The Tavern roll at the start of a turn is not a
Refresh for card purposes. Only an explicit Refresh is (the button, paid
with Gold, Health or free, and effects that say "Refresh" like Snarky
Shark's). Fodders queued for "your next Refreshes" only go into explicit
ones.

**Demon Fodder** feeds itself to a random friendly Demon on the Refresh it
appears in, and its slot refills with a minion (the spell slot too). With
no Demon on board then, it never feeds: it's a plain 2/2 Demon you can
buy or consume.

**Golden cards** follow their real golden card text.

**Pairing, damage, ending** as in Part 2: shuffle and avoid the last
opponent, ghost for an odd count, damage is tier plus surviving minions'
tiers with the cap from `rules.toml`, game ends with one player left or at
turn 40.

## Actions

Buy a minion or the spell, sell, play (minion to a position, or cast a
spell), pick a target on the board, pick a Tavern minion, choose an option
(Discover, Choose One), move, Activate, roll, freeze, level, end turn. One
flat list with a legality mask from the engine; `crates/bgcore/src/action.rs`
has the layout. A turn is capped at 60 actions.

## Not in this game yet

Hero choice and hero powers, armor, trinkets, quests, Dark Gifts, the
Deity, the other five tribes, tribe bans per lobby, Duos.
