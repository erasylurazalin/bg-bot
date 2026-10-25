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
