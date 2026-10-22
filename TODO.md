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

## Now: a real five-tribe lobby

Beast, Mech, Dragon, Murloc and Demon, exactly as in the current patch
(DESIGN.md Part 3), one tribe at a time:

1. Beasts and Mechs
2. Dragons
3. Murlocs
4. Demons
5. Neutral minions and spells
6. Observation, bindings and bots for the full lobby, then a long training
   run

## Hidden numbers (checked 2026-10-23)

The amounts these cards' text hides, as I know them from the game:

| Card | Rule |
|---|---|
| Lurking Leviathan | +3 Attack per summoned Beast, improving by +3 each summon (golden +6). The count survives tripling: the golden one reapplies it at +6. Summons in the shop and in combat both count. |
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
