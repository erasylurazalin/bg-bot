# bg-bot roadmap

A bot that plays Hearthstone Battlegrounds well, trained by self-play in my
own simulator. It never touches the real game client. The plan is a
suggestion window: I play, it suggests, I decide.

## Done

- v0.1: a mini Battlegrounds in Rust, fast enough for training (tens of
  thousands of games a second), with a fuzzer that checks the rules after
  every action.
- A training loop: Python bindings, PPO self-play against older copies of
  itself and scripted bots, evals with curve and tribe reports.
- A first overnight run on v0.1, which beat the scripted bot easily.
- Stage 2 engine: Magnetic with nesting, Tavern spells, Start of Combat,
  Rally, End of Turn, game-long counters, Activate, Bonus Keywords and
  Stealth, consuming Tavern minions, Tavern buffs, hero health as a
  resource.

## Done: a real five-tribe lobby

Beast, Mech, Dragon, Murloc and Demon exactly as in the current patch,
plus the neutral minions and spells, the observation and bots for them, a
sandbox to play shop phases, and three network options (flat, pooled,
attention). An overnight comparison of the three found them close per
game; none learned to level properly against the scripted bots.

## Now: every tribe

Pirate, Elemental, Undead, Quilboar and Aberration (Naga left the game),
one at a time, after the tier 7 minions:

1. Tier 7 for the five tribes and neutrals
2. Pirates
3. Elementals
4. Undead
5. Quilboar
6. Aberration
7. A lobby of five random tribes per game, like the real one, with the
   observation and bots for all of them

Then my real boards as the "good player" in each lobby, so staying low
stops paying off.

## After the tribes: teaching it strategies

Self-play alone never found a plan that takes five turns to pay off (the
night runs stayed low with mixed boards). So I show it the comps instead
of waiting for it to stumble on them:

1. **Scripted comp bots**, one per known build (Snazzy Undead, Blood Gem
   Quilboar...; `ref/hsreplay-comps-2026-10-22.md` is a start). They play
   toward their comp on purpose, as opponents and as a source of
   demonstration games.
2. **Learning from demonstrations** (imitation learning / behaviour
   cloning): copy the comp bots' and my own games first, then let PPO
   improve past them. My games come from the tracker's log reader.
3. **A strategy input** (goal-conditioned policy): tell the network which
   comp it's going for, so it keeps every comp instead of collapsing to
   one, and the suggestion window can say "going Snazzy, buy this".
4. **Starting mid-plan** (reverse curriculum): begin training games from
   half-built comps, then move the start earlier until it builds them
   from turn 1.

The catch: copying comps teaches it to force them even when the lobby
doesn't give the pieces. Self-play afterwards is what teaches it when to
commit and when to pivot. Later, a short search at decision time (try a
few shop options, simulate the fights, score them with the value network)
is the other big upgrade, TD-Gammon style.

## Hidden numbers (checked 2026-10-23)

The amounts these cards' text hides, as I know them from the game:

| Card | Rule |
|---|---|
| Lurking Leviathan | +3 Attack per summoned Beast, improving by +3 each summon (golden +6). A triple keeps the highest count of the three; the golden one applies it at +6. Summons in the shop and in combat both count. |
| Spark Snapper | Satellites start 2/3 and grow +2/+3 per Mech played: 2/3, 4/6, 6/9... |
| Hopebringer | each lost Divine Shield adds +4/+3 to its buff (golden +8/+6) |
| Hackerfin | +3/+2 more per different Bonus Keyword |
| Sacrificial Wrathguard | each Activate adds +2/+2 to its Tavern buff |

## Next: finish the tribes, then a clean slate

1. ~~Remove v0.1 and the made-up cards.~~ Done 2026-10-24: every old rule
   test now runs on real cards (`tests/core_rules.rs`).
2. **A lobby of five random tribes per game**, like the real one (Deity
   only with Aberrations, dual-tribe minions in if either tribe is).
3. **A new observation.** The layout has been frozen since the five-tribe
   lobby, so the bot can't see most of what came after: Blood Gem value,
   the Deity, the Undead bonus, the Refresh buff, pending discards, Triple
   Rewards, the lobby's tribes. Then bots for all ten tribes and a smoke
   run.
4. **Extend the card export** to heroes, hero powers, trinkets and Dark
   Gifts, so every later run starts from the game's own data.

## Then: the rest of the game

1. **Heroes, the frame first:** hero choice, starting health and armor,
   damage hitting armor first, Armor Stash, hero powers as a system
   (cost, once per turn, passives, start of turn and Start of Combat
   timing).
2. **Hero powers in batches**, grouped by mechanic like the tribes,
   starting with the heroes in my own games (the tracker's logs show
   which).
3. **Trinkets:** the lesser and greater choices at their set turns.
4. **Dark Gifts**, and Dark Paradox with them.

Each season changes some of this; the export and the audit script make
catching up after a patch cheap.

## Also on the list

- Check our fights against `~/projects/bg`, the tracker, which simulates
  real boards from my logs.
- Peak boards taken from bots that force each build, to check balance
  between the builds.
- Better scripted bots as yardsticks.
- Split `game.rs` (about 5,000 lines) by system, so agents read less.
- The suggestion window, fed by the tracker's log reader.
