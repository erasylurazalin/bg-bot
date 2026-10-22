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

## Numbers I don't know yet

These cards improve by an amount the card text hides. The engine uses a
guess for each until I check the real numbers.

| Card | What's hidden | Guess in the engine |
|---|---|---|
| Lurking Leviathan | how much each summon improves it | +3 Attack more per summon (+3, +6, +9...) |
| Spark Snapper | how much its Satellites improve | no improvement yet, Satellites stay 2/3 |
| Hopebringer | how much each lost Divine Shield adds | +1/+1 per shield, golden the same |
| Hackerfin | how much each different Bonus Keyword adds | +1/+1 per keyword (run 3) |
| Sacrificial Wrathguard | how much Activate improves it | +1/+1 more on its Tavern buff per Activate (run 4) |

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
