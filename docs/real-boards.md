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

**Games on era-laptop.** The laptop's client writes its own `Power.log`
(`log.config` copied there on 2026-10-25). Its timer `bg-logs.timer` runs
`~/.local/bin/bg-logs-sync` every 15 minutes: it copies the log folders to
the laptop's `~/bg-logs/` and, when era-arch answers over ssh, rsyncs them
to `~/bg-logs-laptop/` here. The collector reads both archives; laptop games
are named `boards/real/laptop-<log folder>-g<N>.json`. Stop it on the
laptop with `systemctl --user disable --now bg-logs.timer`.

## The file

```json
{"source": "Hearthstone_2026_10_24_15_25_26", "game": 2, "place": 3,
 "boards": [
  {"who": "me", "turn": 5, "result": "win", "hero": "...", "hero_card": "...",
   "hero_base": "BG34_HERO_001", "tier": 3, "health": 27, "armor": 9,
   "minions": [{"card_id": "BG25_022", "name": "Scarlet Skull", "attack": 2,
                "health": 1, "golden": false, "keywords": ["REBORN"],
                "dark_gifts": [], "numbers": [6, 0, 0, 0]}],
   "counters": {"gem_attack": 3, "gem_health": 2, "beetle_attack": 4,
                "beetle_health": 2, "undead_attack": 6, "spell_attack": 1,
                "spells_cast": 4, "knight_deaths": 2, "baller": 1,
                "tasty_lobster": 6},
   "deity": {"card_id": "BGFYM_000", "name": "C'Thun", "attack": 4, "health": 6},
   "hand": [{"card_id": "BG36_202", "name": "Tasty Lobster", "attack": 6,
             "health": 3, "golden": false, "keywords": ["DEATHRATTLE"],
             "dark_gifts": [], "numbers": [14, 7, 0, 0]}],
   "hand_hidden": 0},
  {"who": "opp", "turn": 5, "result": "lose", "...": "..."}]}
```

- `who` is `me` (my board, every turn of my game) or `opp` (the opponent I
  fought that turn: one random player per turn, good for volume).
- The boards are as they went into the fight, before Start of Combat.
- `tier`, `health` and `armor` are the player's at the first attack.
- `hero_card` can be a skin; `hero_base` is the hero's own card id.
- `hero_power`, `trinkets` and `rewards` (quest rewards) are the side's at
  the first attack, each with its card id, name and current numbers (the
  {0} {1} {2} {3} of its text); a trinket also says whether it sits in the
  greater slot. Not used by training yet.
- `counters` is what the side's player had at the first attack: the numbers
  a fight reads off the player. A key is written only when the log shows the
  number; a missing one means the base value (0 spells cast, no spell
  bonus, Blood Gems +1/+1), never "unknown". Where each one is in the log:

  | key | log |
  |---|---|
  | `gem_attack`, `gem_health` | `BACON_BLOODGEMBUFFATKVALUE` / `_HEALTHVALUE` on the player (the `BG26_159pe` enchantment carries the same pair) |
  | `beetle_attack`, `beetle_health` | `BG31_808pe` (Beetle Army player enchant) numbers 1 and 2 |
  | `undead_attack`, `undead_health` | `BG25_011pe` (Undead Bonus Attack player enchant) number 1 |
  | `spell_attack`, `spell_health` | `TAVERN_SPELL_ATTACK_INCREASE` / `TAVERN_SPELL_HEALTH_INCREASE` on the player |
  | `spells_cast` | `NUM_SPELLS_PLAYED_THIS_GAME` on the player (mine only: the stand-in player carries none for the opponent) |
  | `knight_deaths` | `BG25_008pe` (Eternal Knight player enchant) number 1 |
  | `tasty_lobster`, `baller` | the `Counter` counts: a Tasty Lobster's numbers on the board or in hand, and `BG31_816pe` (Baller player enchant) number 1 |

- `deity` is the side's Deity at the first attack (`BG_OldGod` in SECRET:
  the minion it awakens, its stats) or `null`. Its Aberration deaths still
  needed are in the log too, but our `Deity` has no field for them.
- `hand` is the minions in the side's hand at the first attack, the same
  shape as `minions` (the opponent's hand is in the log too); `hand_hidden`
  counts the cards the log shows without a card id, which are left out.
- `numbers` on a minion are its current `TAG_SCRIPT_DATA_NUM_1..4`, the
  {0} {1} {2} {3} of its text (Lurking Leviathan's growth, Spark Snapper's
  Satellite, a Tasty Lobster's buff). Written only for cards that have any.
- The raw logs stay in `~/bg-logs/`, so anything else the tracker can read
  can be added later: delete `boards/real/.state.json` and run the script
  again with `--no-archive` to rewrite every file (the flag skips copying
  the client's logs into `~/bg-logs`, which is outside this project).
- `place` is my final place (0 while the game is still going).
- No player names are stored.

On 2026-10-26 it held 914 boards in 39 files, from 11 games of mine and the
laptop's, all from 2026-10-24 to 2026-10-29 (older logs were already deleted
by the game): 566 boards carry counters, 379 a hand, 486 a Deity.

## How the boards are used

A training game can seat a **real seat** that plays these boards instead of
shopping (docs/ENGINE.md "Real-board seats"): at the start of each of its
turns it takes a fresh random board of that turn, keeps the file's attack,
health, golden flag and keywords (a Dark Gift's stats are already inside
them; its effect is not built, so it is lost), takes that player's hand and
side-wide numbers, and its turn is over. Nothing accumulates: the next
turn's board replaces the board, the hand and every number outright. The
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
minions, buddies, and a few that only ever sat in a hand): they are dropped,
the board around them stays, and the loader prints them by name. Whatever is
in the folder when a run starts is what it loads; new games add boards every
15 minutes.

A minion's `numbers` become the engine state they stand for, for the cards
whose scaling it models as `Minion.improvements` (`real_boards.rs`): Lurking
Leviathan (growth / 3, / 6 golden), Spark Snapper (Satellite growth / 2, / 4
golden), Ultraviolet Ascendant (the third number is its count) and Patient
Scout (its tier, minus 1). A Tasty Lobster's numbers are the player's
`tasty_lobster` count instead (`attack / 2 - 1`, `/ 4 - 1` golden): every
Lobster shares one, and the board keeps the best of its board and hand. Other
cards keep 0 whatever numbers they carry, since their numbers are something
else (a Scarlet Survivor's is its Attack threshold, a Forest Rover's are its
own stats, a Glambot's are a fixed Satellite).

Training: `ppo.py --lineup 4,2,0,0,0,2 --real-boards boards/real` seats two
real seats (the lineup is learner,snapshot,standard,rush,random,real), and
every eval then also plays one learner against 7 real seats
(`place_vs_real`). Without a `boards/` folder, the small fixture folder
`crates/bgcore/tests/data/real-boards/` plays the same role for tests,
`bgbench fuzz|games N --real-boards <folder>` and smoke runs.
