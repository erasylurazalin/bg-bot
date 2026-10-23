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

## Hidden numbers (checked 2026-10-23)

The amounts these cards' text hides, as I know them from the game:

| Card | Rule |
|---|---|
| Lurking Leviathan | +3 Attack per summoned Beast, improving by +3 each summon (golden +6). A triple keeps the highest count of the three; the golden one applies it at +6. Summons in the shop and in combat both count. |
| Spark Snapper | Satellites start 2/3 and grow +2/+3 per Mech played: 2/3, 4/6, 6/9... |
| Hopebringer | each lost Divine Shield adds +4/+3 to its buff (golden +8/+6) |
| Hackerfin | +3/+2 more per different Bonus Keyword |
| Sacrificial Wrathguard | each Activate adds +2/+2 to its Tavern buff |

## Next

- Check our fights against `~/projects/bg`, the tracker, which simulates
  real boards from my logs. With real cards on both sides this finally
  makes sense.
- Peak boards taken from bots that force each build, to check balance
  between the builds.
- Better scripted bots as yardsticks.

## Later

- Hero powers and hero choice.
- Trinkets, quests and whatever the season adds.
- All ten tribes with five banned per lobby, like the real game.
- The suggestion window, fed by the tracker's log reader.
