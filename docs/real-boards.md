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

## The action file

`tools/collect_actions.py` reads the same logs for the other half of a game:
what I did in every shop phase, as named steps. It writes
`boards/actions/<log folder>-g<N>.json`, numbered exactly like `boards/real/`
(47 games, 548 turns). It keeps its own `.state.json` and skips a log whose
size has not changed; `--rewrite` reads them all again. A run is about a
minute. No player names are stored.

```json
{"source": "Hearthstone_2026_10_24_04_08_00", "game": 1,
 "hero": {"card_id": "TB_BaconShop_HERO_57", "name": "Nozdormu"},
 "unread": {},
 "turns": [
  {"turn": 1,
   "start": {"gold": 3, "gold_max": 3, "tier": 1, "upgrade_cost": 5,
             "health": 30, "armor": 13, "free_refreshes": 0, "frozen": false,
             "hero_powers": [{"card_id": "TB_BaconShop_HP_063",
                              "name": "Clairvoyance", "numbers": [0, 0, 0, 0],
                              "cost": 0, "used_this_turn": 0}],
             "trinkets": [],
             "tavern": [{"id": 333, "slot": 1, "card_id": "BG26_135",
                         "name": "Southsea Busker", "attack": 3, "health": 1,
                         "golden": false, "keywords": [], "kind": "minion",
                         "frozen": false}],
             "hand": [], "board": [], "hand_hidden": 0},
   "actions": [
     {"step": "buy", "slot": 1, "option": 4, "paid": {"gold": 3, "health": 0},
      "target": {"id": 333, "card_id": "BG26_135", "name": "Southsea Busker",
                 "kind": "slot", "slot": 1},
      "card": {"id": 333, "card_id": "BG26_135", "name": "Southsea Busker",
               "attack": 3, "health": 1, "golden": false},
      "moved": {"id": 333, "from": {"where": "tavern", "slot": 1},
                "to": {"where": "hand", "index": 1}}},
     {"step": "end_turn", "paid": {"gold": 0, "health": 0}}],
   "end": {"gold": 0, "health": 30, "armor": 13, "hand_hidden": 0, "board": [],
           "hand": [{"id": 333, "index": 1, "card_id": "BG26_135",
                     "name": "Southsea Busker", "attack": 3, "health": 1,
                     "golden": false, "keywords": [], "kind": "minion"}],
           "tavern": [{"...": "..."}]}}]}
```

- A turn starts at the first decision the game offers me in it, not at the
  TURN tag's first change. That decision can be a choice rather than my
  options: 86 turns open with a trinket offer and 27 with some other Discover.
- `start` and `end` hold my hand, my board and Bob's Tavern the way
  `boards/real/` writes a minion (card id, name, attack, health, golden,
  keywords, dark gifts, script `numbers`) plus where each card sits: `slot` in
  the Tavern, `index` in my hand, `position` on my board. A hand card the log
  does not name is kept with a null card id and counted in `hand_hidden`; none
  of the 47 games has one, so that count is 0 everywhere.
- `gold` is `RESOURCES + TEMP_RESOURCES - RESOURCES_USED` on my player entity,
  the number the client shows; `gold_max` is `RESOURCES`. `health` and `armor`
  are my hero's, apart. `upgrade_cost` is the COST of the tier up button (null
  at the top tier), `hero_powers` are the hero power entities in play with
  their current numbers, cost and uses this turn, `trinkets` are my trinkets.
- A Tavern spell carries its Gold price as `cost`; a minion has none, because
  the log does not print one (a minion costs 3 Gold by the game's rule).
- `end` is the state when the turn ended, after the end of turn effects and
  before the fight (null when the log ends inside the turn).

Every step says what it was and, where there is one, on which card or slot:

| `step` | fields |
|---|---|
| `buy` | `slot`, `card`, `spell` for a spell, `target` (the Tavern slot) |
| `sell` | `position`, `card`, `target` |
| `play` | `index`, `card`, `target` (a Battlecry's or spell's target), `sub_option` |
| `activate` | `position`, `card`, `target`: a minion's own Activate (it costs Gold) |
| `move` | `position`, `to`, `card`: a minion dragged on my board |
| `roll` | `cost`, `free` (free Refreshes left when the button showed a price), `free_from` (the cards that filled the pool), `tavern` after |
| `freeze` | `on` |
| `level` | `cost`, `tier_after` |
| `hero_power` | `which` (main or second), `cost`, `target` |
| `dark_discovery` | `cost` (the Dark Gift button's price) |
| `discover` | `kind`, `source`, `options`, `picked`: a Discover, a Choose One, a Dark Discovery's three minions with their gifts, a triple's reward |
| `trinket_pick` | `slot_kind` (lesser or greater), `source`, `options`, `picked` |
| `end_turn` | nothing: where my End Turn click lands, and where the game's end of turn effects go when they belong to my last step |

- Every step also carries `option` (which of the client's own options I picked,
  for reading the log), `paid` (the Gold and the Health it actually took, from
  the client's own numbers) and `cost` when the client puts a price on the card
  or button (`buy` of a spell, `roll`, `level`, `hero_power`,
  `dark_discovery`). A card priced in Health is paid out of Armor first, so
  `paid.health` counts Health and Armor together.
- A step is measured against the state the step before it left, so nothing the
  game does between two of my decisions is lost. Gold the game takes with no
  step of mine (an end of turn effect, a trinket) is on the step as
  `late_gold` and counted in `unread`.
- What a step changed is written as `moved` (where the card it acted on went,
  `{from: {where, slot|index|position}, to: ...}` or `to: null` when it left),
  `entered` and `left` (cards that came into my hand or board, or left them,
  with `from`, the log's CREATOR, the card that made them), `changed` (cards
  whose numbers, keywords or gifts moved: a random buff's target, and the card
  id when three copies turned one golden), `tavern` (the Tavern's offers after
  a step that changed them) and `hand_hidden`.
- A card that comes into my hand with no CREATOR is a card stolen out of the
  Tavern (Enchanted Lasso, Decoy Conjurer's Activate): it keeps its entity id,
  so the `entered` entry says `stolen: {where: tavern, slot: N}`, the slot it
  came from.
- A step that took Health and got it back says `health_cost` (what it took,
  the deepest dip) and `rewound: {health, by}` with the card that gave it back.
  A Soul Rewinder or an Ashen Corruptor on my board undoes the damage a Health
  price does, so `paid.health` is the net and the rewind is apart. The Health
  price itself is paid out of Armor first.
- A step that gained Gold says `gold_gain: {gold, by}` with the card that gave
  it (a Tavern spell, Stuffed Coin Purse reaching Tier 6, a trinket), so
  `paid.gold` stays the net.
- A step that used or was given free Refreshes says `free_refreshes: {before,
  after}`, and a reroll the pool paid for says `free_from`, the cards that
  filled it (Leaf Through the Pages, Wolf Pup, Warband Whistle and the like).
  The count is `BACON_FREE_REFRESH_COUNT` on `Bacon_Free_Refresh_Player_Ench`;
  the turn's start shows it as `free_refreshes`.
- `entered`/`left` follow the game's own entity ids: a card keeps one as it goes
  from the Tavern to the hand to the board, so a replay can follow it.
- A click that took and changed nothing (a minion picked up and put back, one
  inspected, a Tavern card clicked and left there) is not a step: it is counted
  in the turn as `canceled`.
- `unread` counts what the log could not tell, per game. It is empty in every
  game but two: one turn I took no step in, and one change the game made after
  my last step.

`tools/check_actions.py` reads the files back with nothing from our engine and
adds every turn up: the Gold the steps took against the Gold the turn started
and ended with (and Health the same way), and my hand and board at the end
against the start plus everything the steps moved. On the 47 games it finds
546 of 548 turns balancing (99.6%); the two that do not are turns the log ends
inside, before the fight.

Every price the client printed that the steps did not pay as printed is named
by the file, and nothing is left unnamed:

| what happened | how many | by |
|---|---|---|
| a free reroll from free Refreshes | 541 | Leaf Through the Pages 91, Refreshing Anomaly 77+36 goldens, Sly Infiltrator 51, Underrot Spawn 27, Clairvoyance 21, Decoy Conjurer 20, Eternal Knight 17 and 49 more kinds |
| a Gold gain inside a step | 262 | Wand of Divination 75, Tavern Coin 51, Wealthy Bounty 34, Snare Trapper 23, Corrupted Coin 22, Hasty Excavation 22, Stuffed Coin Purse 1 and 10 more |
| a price paid in Health and given back | 60 | Soul Rewinder 51, Ashen Corruptor 9 |
| a card stolen out of the Tavern | 24 | Decoy Conjurer 17 (+1 golden), Enchanted Lasso 6 |
| a price paid in Health | 13 | Hasty Excavation |

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

## The fight check

`bgbench fightcheck` plays Era's real fights through the engine many times and
says which real results the engine finds unlikely, and which cards those
fights share. It is a bug finder: it fixes nothing (docs/briefs/fight-check.md).

```
./target/release/bgbench fightcheck [--boards DIR] [--runs N] [--out DIR] [--flag PCT]
```

Defaults: `boards/real`, 1000 runs, `boards/checks` (git-ignored, under
`boards/`), a 2% flag cut. Needs `ref/bg-cards.json` for the card id map, as
`--real-boards` does. Every core is used: 558 fights x 1000 runs takes about
0.2 s on 32 cores.

- **The fights:** a board is paired with the other side's board of the same
  turn of the same game (`pair_fights`). The result is the one the game
  recorded on Era's board (`win`, `lose` or `tie`). The damage a fight did is
  the drop in Health plus armor from this turn's board to the next turn's
  board of the same game.
- **Setting it up:** `bgcore::fight_check::prepare` seats both sides exactly
  as they went in, through the same code a real seat uses
  (`RealBoard::build_one`, `Player::from_real_board`), then adds the hero
  power and the trinkets, which a real seat still ignores:
  - the minions with the file's attack, health, golden flag, keywords, Dark
    Gifts (their stats already in attack and health) and the scaling numbers
    the loader maps, the hand, the counters, the Deity, the Tavern tier,
    Health and armor;
  - the hero power (`HeroDb::power_by_card_id`) with the one number a fight
    reads off its text (Onyxia's Broodmother bonus). The three powers whose
    fight effect only runs when the power was pressed in the same turn (I'll
    Take That!, Embrace Your Rage, Rapid Reanimation) are played as pressed:
    whoever holds one and can pay for it presses it before a fight;
  - the trinkets (`TrinketDb::by_card_id`) in the slot the file marks, with
    their "improve this" number from the printed stats where a fight effect
    grows them (Fang Anklet, All-Purpose Kibble) and the small standing rules
    the fight reads (a power's fight effect running twice, the Hammer of
    Twilight aura on minions the fight summons).
- **Incomplete:** a fight is incomplete when the engine does not build
  something it carried, and its missing pieces are listed: a minion (it
  keeps its stats and keywords as a Stand-in, Era's rule for the loader), a
  hero power, a trinket, a Dark Gift, a quest reward (no quest is modelled),
  a hand card the log does not name. Two gaps that come from the file itself
  count too: a power whose fight effect reads a number the file does not
  carry (Ozumat's Tentacles grow with minions sold, Tavish's Lock and Load
  needs the minion it locked), and Prestidigitation, because a Secret zone is
  not in the file. An incomplete fight is never flagged.
- **Flags and calibration:** a complete fight is **flagged** when the engine
  gives the recorded result less than `--flag` percent. Complete fights are
  also grouped by the engine's predicted win % in ten buckets with the real
  win rate of each, so a systematic skew shows up as a bucket whose real rate
  does not follow the engine's.
- **Output** (`boards/checks/`):
  - `fights.tsv`: one row per fight: the file, game and turn, the real
    result, our win, tie and loss %, the chance we give the real result,
    complete or not and why, our damage when Era loses (mean, p10, p50, p90),
    the damage the file shows Era took, and the share of runs that hit the
    attack cap;
  - `cards.tsv`: per minion, hero power, trinket, Dark Gift and quest reward:
    the fights it was in, how many of those were complete, how many were
    flagged, and the ratio, most suspicious first;
  - `report.md`: the summary, why fights were incomplete, the calibration
    table, the damage comparison, and the 30 most surprising complete fights
    with both boards written out.

What the check cannot see, because the file does not carry it, and so cannot
blame on the engine: the game's lobby (a card that draws a random card inside
the fight draws from every tribe), the shop-phase numbers a fight reads
(Goldrinn's Beast bonus, Boon of Beetles, Gold spent, minions sold), a trinket
aura's per-card steps (Hammer of Twilight's discards), and the Ice Block a
Safety Patch gives (it moves Health between turns, not a fight's result).
