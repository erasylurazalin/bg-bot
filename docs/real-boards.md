# Real boards from my games

`tools/collect_boards.py` keeps the boards from every Battlegrounds game I
play. The game writes each game to `Logs/Hearthstone_<date>/Power.log`
whether or not the tracker runs, and deletes old log folders after a few
sessions, so the script first copies them to `~/bg-logs/`. Then it reads
each new or grown log with the tracker's parser (`~/projects/bg`) and writes
one file per game to `boards/real/<log folder>-g<N>.json` (git-ignored).

A systemd user timer runs it every 15 minutes:

```
systemctl --user list-timers bg-boards.timer    # when it runs next
systemctl --user start bg-boards.service        # run it now
journalctl --user -u bg-boards.service          # what it printed
systemctl --user disable --now bg-boards.timer  # stop it
```

The unit files are `~/.config/systemd/user/bg-boards.{service,timer}`.

## The file

```json
{"source": "Hearthstone_2026_10_24_15_25_26", "game": 2, "place": 3,
 "boards": [
  {"who": "me", "turn": 5, "result": "win", "hero": "...", "hero_card": "...",
   "hero_base": "BG34_HERO_001", "tier": 3, "health": 27, "armor": 9,
   "minions": [{"card_id": "BG25_022", "name": "Scarlet Skull", "attack": 2,
                "health": 1, "golden": false, "keywords": ["REBORN"],
                "dark_gifts": []}]},
  {"who": "opp", "turn": 5, "result": "lose", "...": "..."}]}
```

- `who` is `me` (my board, every turn of my game) or `opp` (the opponent I
  fought that turn: one random player per turn, good for volume).
- The boards are as they went into the fight, before Start of Combat.
- `tier`, `health` and `armor` are the player's at the first attack.
- `hero_card` can be a skin; `hero_base` is the hero's own card id.
- `place` is my final place (0 while the game is still going).
- No player names are stored.

On 2026-10-25 it held 11 games, 131 boards of mine and 131 opponent boards,
all from 2026-10-24 (older logs were already deleted by the game).

## How the boards are used

A training game can seat a **real seat** that plays these boards instead of
shopping (docs/ENGINE.md "Real-board seats"): at the start of each of its
turns it takes a fresh random board of that turn, keeps the file's attack,
health, golden flag and keywords (a Dark Gift's stats are already inside
them; its effect is not built, so it is lost), and its turn is over. The
boards are not pool cards: they hold no pool copies and nothing goes back to
the pool when they die or the seat is knocked out. A real seat gets no hero
(no power, no hero armor, and it uses up no hero from the deck) and plays
with a flat 30 Health and 10 armor whatever its boards; it takes damage like
anyone and is knocked out at 0 Health.

Era's rules for the draw: a fresh random board each turn from all boards of
that turn, mine and opponents' alike, equally likely; turn 13 and later draw
from every board of turn 13 or later; a turn before 13 with no boards of its
own uses the nearest earlier turn that has some.

Loading (train/real_boards.py, also `bgbench --real-boards DIR`): every
minion's card id is mapped to a card name through `ref/bg-cards.json` (a
golden id, the game's `_G` and `TB_BaconUps_*` forms, maps to the base card
and sets golden), then to the card in `cards.toml` by name. About 1% of
minions match nothing we have (Dark Paradox, Fish of N'Zoth, timewarped
minions, buddies): they are dropped, the board around them stays, and the
loader prints them by name. Whatever is in the folder when a run starts is
what it loads; new games add boards every 15 minutes.

Training: `ppo.py --lineup 4,2,0,0,0,2 --real-boards boards/real` seats two
real seats (the lineup is learner,snapshot,standard,rush,random,real), and
every eval then also plays one learner against 7 real seats
(`place_vs_real`). Without a `boards/` folder, the small fixture folder
`crates/bgcore/tests/data/real-boards/` plays the same role for tests,
`bgbench fuzz|games N --real-boards <folder>` and smoke runs.
