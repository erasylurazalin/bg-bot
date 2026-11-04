# Real boards from my games

`bgbench collect-boards` (`crates/bgbench/src/collect_boards.rs`, on the log
reader in `crates/bglog`) keeps the boards from every Battlegrounds game I
play. The game writes each game to `Logs/Hearthstone_<date>/Power.log`
whether or not a log tracker runs, and deletes old log folders after a few
sessions, so the command first copies them to a log archive outside the
repo. Then it reads each new or grown log and writes one file per game to
`boards/real/<log folder>-g<N>.json` (git-ignored). `boards/real/.state.json`
maps each log folder to the size of the `Power.log` last read from it, so a
rerun only reads logs that are new or grown; delete it to rewrite every file.
`--out DIR` is where the files and the state go (`boards/real` by default),
`--archive DIR` is where the logs are copied, and `--no-archive` skips that
copy.

A minion's `dark_gifts` lists a gift once per copy: a golden that kept the
same gift from two copies carries it twice, and the "+0/+0" buff a gift leaves
is not a gift (`gifts_by_minion` in `crates/bglog/src/views.rs`). The Python
collector this replaced (until 2026-11-03) kept one id per gift; every file
was rewritten with the Rust command that day (the old files are backed up
outside the repo).

A user timer runs it every 15 minutes, from a copy of the release binary
outside `target/`, so a rebuild never breaks a run. After changing the
collector, rebuild with `cargo build --release -p bgbench` and copy the new
binary to wherever the timer runs it from.

**A second machine.** Its client writes its own `Power.log`, and a timer
there syncs the log folders into a second archive that the collector reads
too; those games are named `boards/real/laptop-<log folder>-g<N>.json`.

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
                "upper_hand": 1},
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
  | `gem_attack`, `gem_health` | `BACON_BLOODGEMBUFFATKVALUE` / `_HEALTHVALUE` on the player (the `BG26_159pe` enchantment carries the same pair). The tags hold the player's own Gem value only: a trinket's "Blood Gems played from your hand give an extra +6/+6" (Surveyor Portrait) is written on the Gem cards (their third and fourth numbers) instead, so the counter leaves it out and the replay's loader adds it back |
  | `beetle_attack`, `beetle_health` | `BG31_808pe` (Beetle Army player enchant) numbers 1 and 2 |
  | `undead_attack` | `BG25_011pe` (Undead Bonus Attack player enchant) number 1 |
  | `undead_health` | the `Undead Army` enchant (`BG25_011e2`) on any Undead of mine, number 2: the game has no player-level enchant for the Health half, and every Undead on my board carries the whole pair (number 1 is the Attack). A board with no Undead of mine shows no pair, and the Attack then comes from the player enchant above. The shop replay's collector writes it; the board files do not yet |
  | `spell_attack`, `spell_health` | `TAVERN_SPELL_ATTACK_INCREASE` / `TAVERN_SPELL_HEALTH_INCREASE` on the player. This is the total the game gives every Tavern spell, so it already holds a minion's aura (Humon'gozz's +1/+2: the tag drops by exactly that pair when the Humon'gozz is sold, `Hearthstone_2026_10_27_17_42_08-g1` turn 9) and a trinket's standing rule (Honeycomb Ring's +1/+1 on the pick). It also holds what this turn's spells on a minion improved, which the start of a turn drops: `Hearthstone_2026_11_03_15_22_36-g4` turn 10 starts at 2/3 and its last step is 38/39. The replay's loader takes the engine's own part back out, and seats the part the turn added as the bonus that is this turn's only (`Player.tp.spell_bonus_turn`) |
  | `spells_cast` | `NUM_SPELLS_PLAYED_THIS_GAME` on the player (mine only: the stand-in player carries none for the opponent) |
  | `knight_deaths` | `BG25_008pe` (Eternal Knight player enchant) number 1 |
  | `baller` | `BG31_816pe` (Baller player enchant) number 1 |
  | `upper_hand` | not a number in the log: the Upper Hands the side cast this turn (the spell's card goes back to SETASIDE), counted from the `PLAY` blocks of `BG28_573` |

  A Tasty Lobster's count is not in `counters`: the loader reads it off the
  card's own numbers, on the board or in the hand (see the paragraph about
  `numbers` below).

- `deity` is the side's Deity at the first attack (`BG_OldGod` in SECRET:
  the minion it awakens, its stats) or `null`. Its Aberration deaths still
  needed are in the log too, but our `Deity` has no field for them.
- `hand` is the minions in the side's hand at the first attack, the same
  shape as `minions` (the opponent's hand is in the log too); `hand_hidden`
  counts the cards the log shows without a card id, which are left out.
- `reanimate` is `true` on the minion Rapid Reanimation (Teron Gorefiend)
  chose: the log attaches the power's mark, the enchantment
  `BG25_HERO_103pe`, to that minion while both sit in PLAY, and the file
  writes the mark at the moment the fight is staged, which is the board it
  holds. At the first attack the chosen minion is already gone and the mark
  sits on the copy the power summoned, so reading it there would name the
  wrong minion. The key is written only when the mark is attached; a fight
  with the power and no mark means the player did not use it that turn.
- `numbers` on a minion are its current `TAG_SCRIPT_DATA_NUM_1..4`, the
  {0} {1} {2} {3} of its text (Lurking Leviathan's growth, Spark Snapper's
  Satellite, a Tasty Lobster's buff). Written only for cards that have any.
- `secrets` is the side's Secret zone at the first attack: the card ids of the
  Secrets in it, in the order they were got, which is the order they fire in.
  The five are `TB_Bacon_Secrets_01` (Venomstrike Trap), `_07` (Autodefense
  Matrix), `_15` (Pack Tactics), `_10` (Redemption) and `_12` (the Ice Block a
  Safety Patch gives). The zone also holds the Deity and quests, which are not
  Secrets and are not written here. The list is always written, empty when
  there is nothing, so a missing key means a file from before it was
  collected. The opponent's Secrets are in the log too and are written: the
  bot never sees them, but the fight check needs what was up.
- `hero_power` numbers are the power's `TAG_SCRIPT_DATA_NUM_1..4` at the first
  attack. For Ozumat's Tentacular the first one is the Attack the Tentacle has
  now (2 summons a 2/2 Tentacle, 3 a 3/3), which is how the check gives the
  fight the sales the Tentacle grows by. `hero_power.waiting` is the minion
  Lock and Load still owes the board, in the same shape as a board minion, or
  `null` when it owes none (every other power has `null` too): the power keeps
  it in `TAG_SCRIPT_DATA_ENT_1`, and the file reads its stats off the copy the
  power summons, because the held entity itself is not created in the log when
  the opponent stole it. The key is always written with the power, so a
  missing one means a file from before it was collected.
- The raw logs stay in the archive, so anything else my tracker can read
  can be added later: delete `boards/real/.state.json` and run the script
  again with `--no-archive` to rewrite every file (the flag skips copying
  the client's logs into the archive, which is outside this project).
- `place` is my final place (0 while the game is still going).
- No player names are stored.

On 2026-10-26 it held 914 boards in 39 files, from 11 games of mine and the
laptop's, all from 2026-10-24 to 2026-10-29 (older logs were already deleted
by the game): 566 boards carry counters, 379 a hand, 486 a Deity.

## The action file

`bgbench collect-actions` reads the same logs for the other half of a game:
what I did in every shop phase, as named steps. It writes
`boards/actions/<log folder>-g<N>.json`, numbered exactly like `boards/real/`
(62 games, 717 turns, 15,674 steps in `boards/actions` on 2026-11-03; the
board folder grows every 15 minutes). It keeps its own `.state.json` and
skips a log whose size has not changed; `--rewrite` reads them all again. A
full run is about 30 seconds. No player names are stored. The same timer runs
it right after `collect-boards`.

The command is `crates/bgbench/src/collect_actions.rs`, ported from the
Python `tools/collect_actions.py` (removed 2026-11-03), which wrote the same
files byte for byte but for one thing: a minion that carries the same Dark
Gift twice (two gift cards gave it) lists the gift twice, where the Python
listed one id per gift (`gifts_by_minion`). On 2026-11-03 every file was
rewritten with it: 62 games, 717 turns, 15,674 steps.

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
           "tavern": [{"...": "..."}],
           "counters": {"gold_spent": 7}, "secrets": []}}]}
```

- A turn starts at the first decision the game offers me in it, not at the
  TURN tag's first change. That decision can be a choice rather than my
  options: 86 turns open with a trinket offer and 27 with some other Discover.
- `start` and `end` hold my hand, my board and Bob's Tavern the way
  `boards/real/` writes a minion (card id, name, attack, health, golden,
  keywords, dark gifts, script `numbers`, and its own Blood Gem stats as
  `gem_attack`/`gem_health`) plus where each card sits: `slot` in
  the Tavern, `index` in my hand, `position` on my board. A hand card the log
  does not name is kept with a null card id and counted in `hand_hidden`; none
  of the 47 games has one, so that count is 0 everywhere. A card a source
  handed out carries `from`, the log's CREATOR (the card that made it), which
  is what the replay reads a linked pair off (below).
- `gold` is `RESOURCES + TEMP_RESOURCES - RESOURCES_USED` on my player entity,
  the number the client shows; `gold_max` is `RESOURCES`. `health` and `armor`
  are my hero's, apart. `upgrade_cost` is the COST of the tier up button (null
  at the top tier), `hero_powers` are the hero power entities in play with
  their current numbers, cost and uses this turn, `trinkets` are my trinkets.
- A Tavern spell carries its Gold price as `cost`; a minion has none, because
  the log does not print one (a minion costs 3 Gold by the game's rule). A
  card the game is charging something else for says so on itself:
  `cost_override` is the whole new price (`BACON_OVERRIDE_BG_COST`, Chillmere
  Mosaic's Spellcraft deals Battlecry minions that cost (1)), `cost_reduce` is
  what comes off the rule's price (`BACON_REDUCE_BUY_COST`, a free buy takes
  the whole 3 off every Tavern minion until it is used: Aranna's Spectral
  Sight), and `costs_health` says the card is paid in Health
  (`BACON_COSTS_HEALTH_TO_BUY`: Bazaar Sticker's one spell a turn, Hasty
  Excavation, Demonic Tapestry). The shop replay seats a card's price from
  those marks, the Tavern spell's included: a minion's is the Tavern's price
  or a free buy, and a spell's is the player's next-spell discount. A Tavern
  spell the log bought for less than the card itself says (the step's own
  `paid.gold`) is seated the same way, because the file cannot say what made
  it cheaper.
- `fodder_refreshes` is the Demon Fodder queue: one number per coming Refresh,
  how many Fodders that Refresh gets, read off the game's own "Fodder Player
  Enchant" (`BG35_150pe`, one number per Refresh in its four script numbers).
  A Trapped Clapper's Deathrattle gives one to each of the next three Refreshes
  (`[1, 1, 1]`), the Refresh that follows takes the first number and leaves the
  rest, and a Laboratory Assistant run twice by Brann gives two to each of the
  same three (`[2, 2, 2]`). One number never passes seven: a Refresh deals a
  full Tavern at most, so six queued and three more read as 7, not 9
  (`11_01_17_59_13-g1` turn 9 step 16). A writer that does not read the enchant
  leaves the key out, and then the replay's step is judged without the queue.
- `start`, the `after` of every step and `end` hold the same things, so the
  replay (below) loads any of them the same way. Beyond the cards and the
  numbers above they carry the marks and the player-level numbers a step reads:
  - `counters`, in the board files' keys (`docs/real-boards.md` "The file":
    Blood Gem size, Beetle and Undead bonuses, the Tavern spell bonus, spells
    cast, Eternal Knight deaths, Tasty Lobster, Baller; `gem_attack`/`gem_health`
    are the whole pair one Blood Gem gives, the printed +1/+1 plus the bonus the
    `BACON_BLOODGEMBUFF*` tags hold, so a game with Gem Day has 1/2 Gems) plus
    the ones only a shop step reads: `gold_spent` (`NUM_RESOURCES_SPENT_THIS_GAME`); the Tavern's
    "this game" buffs, `tavern_attack`/`tavern_health` (`BG_ShopBuff`, Staff of
    Enrichment and its kin) and `tavern_elemental_attack`/
    `tavern_elemental_health` (`BG_ShopBuff_Elemental`, Obsidian Ravager);
    `refresh_attack`/`refresh_health` (`BG34_854pe`, "after the Tavern is
    Refreshed this game": Waveling, En-Djinn Blazer, Easterly Winds);
    `barrage_attack`/`barrage_health` (`BG34_689e2`, Blood Gem Barrage casts);
    `volumizer_attack`/`volumizer_health`, "your Volumizers have +A/+H this
    game (wherever they are)", which the game keeps on each Volumizer of mine
    as its own `Volumized` enchantment (`BG34_170e`, numbers 1 and 2) and not
    on the player: every Volumizer of mine holds the same pair, so any one of
    them answers for it, and one Magnetized into a host stops getting it and
    keeps the pair it had, which is why the collector takes the largest pair
    among the enchantments still in play; and
    `golden_played`, the Golden minions I played from hand this game, which
    no tag holds: the collector counts my `play` steps whose card is Golden and
    still on my board after it (a Magnetic minion that merged into a host is a
    Magnetization, not a play). A key is written only when the game has a
    number for it, and one Golden minion played is a number. A file written
    before the key existed has the count nowhere but on a card that improves
    with it: the loader reads Hooktusk, Master Marauder's own numbers (the pair
    it gives now is the printed +1/+1 times one more than the count).
  - A minion's own Blood Gem stats (`Minion::gem_attack`/`gem_health`) are
    carried as `gem_attack`/`gem_health` on the card, in the board, the hand
    and the Tavern: the pair of tags the game keeps on the minion itself,
    which the client prints as the numbers `4270` and `4271` (it has no name
    for them; a player's Gem *size* is the older named pair
    `BACON_BLOODGEMBUFF*VALUE`, and that one is in `counters`). The tags move
    with the minion, so one state says what it holds: log
    `2026_10_26_00_48_15-g2` turn 7 step 24 has the Sly Infiltrator at
    `4270`/`4271` 4/4 before the Gem Confiscation and 0/0 after it, and
    `2026_10_29_22_20_04-g1` turn 11 step 23 the Gatekeeper Amalgam at
    804/469 and then 0/0, the pair that step takes from its neighbours. The
    keys are written only when the minion has some, and a file written before
    they existed has them nowhere: the loader then seats 0, as it always did,
    and Gem Confiscation steals nothing.
  - `magnetizations` on a minion: how many Magnetizations it carries, which
    Utility Drone's "at the end of your turn, give your minions +4/+5 for each
    Magnetization they have" reads. The game keeps that count on the minion
    itself, in a tag with no name in the log (`3670`): one per Magnetic that
    merged into it, and one for the minion itself when the minion is Magnetic
    (a Blue Volumizer alone in hand shows 1, one with a Satellite merged into
    it 2). `Minion.magnetizations` counts only what merged in, so the
    collector takes a Magnetic minion's own one off. It is *not* the number of
    `Magnetic` enchantments attached (`BG34_170te` for a Red Volumizer,
    `BG31_171te` for a Satellite): the game piles a Satellite's stats into one
    such enchantment, so a Glambot with 94 Magnetizations shows a single one,
    and that count gave Utility Drone nothing (`Hearthstone_2026_11_02_00_10
    _22-g2` turn 10: the Drone's +376/+470 on the Glambot against the replay's
    +4/+5). A Magnetization that happens "an extra time" (Drone Duplicator's
    Activate) is one tag step, so the log shows the count without that repeat;
    the replay's own state has the repeat for the step that made it only. The
    key is written only when the count is not 0, and a file written before it
    existed has the count nowhere: the loader then seats 0, as it always did,
    and Utility Drone gives nothing. A log with no `3670` tag at all falls
    back to counting the enchantments, one per merge.
  - `extra_magnetize` on a minion: the extra times the next Magnetization
    onto it happens, which is a Drone Duplicator's "Activate (1): the next
    Magnetization to this minion this turn happens an extra time"
    (`Minion.extra_magnetize`). The game keeps the held one in a tag with no
    name in the log (`4945`), 1 for both the plain and the golden card, and
    the golden one applies it twice, so the file carries the number of extra
    times: the tag, times two for a golden. The Activate sets it and the
    Magnetization it waits for clears it again
    (`Hearthstone_2026_10_29_22_20_04-g3` turn 14: step 19 sets it on the
    golden Drone Duplicator, step 20's Magnetization spends it and the merge
    happens three times, +4128/+5793 on a host at 74/100). The key is written
    only while one is held, and a file written before it existed holds none:
    the loader then seats 0, as it always did.
  - `secrets`, the Secret zone by card id in the order the Secrets were got, as
    the board files write it (always written, empty when there is nothing).
  - `waiting` on a hero power: the minion Lock and Load holds, as a board
    minion, or null (the board files' key).
  - `reanimate` on a board minion: the Rapid Reanimation mark, as the board
    files write it.
  - `no_reward` on a Golden minion: it gives no Triple Reward when played,
    written for the sources whose own card says so (docs/rules/design.md, THE
    GOLDEN RULE: Silent Deliverer's Battlecry goldens, Sinister Invitation's,
    the Gilding Dark Gift, Aureate Laureate). The replay adds the mark itself
    for an `always_golden` card, so a Laureate needs no key in the file.
  - `last_opponent_board`, the warband of the opponent I last fought, as the
    log last showed it in combat: the board that went into the fight, before
    start-of-combat effects, in the shape `board` has (a Dark Gift is its card
    id and name, a copy made by Rapid Reanimation keeps its `reanimate` mark).
    It is taken from the last fight the log finished, never from a fight it has
    already staged for the round to come, and it is null before the first
    fight. The powers that copy a warband read it: Bob's Burgles and Time
    Thief read the last opponent's seen board (`Player.seen`) and I Spy reads
    the next opponent's (`Player.next_board`), which the log writes only after
    that opponent's own shop phase, so this board is the closest a state has to
    it. A file written before the key existed has the warband nowhere, and then
    those powers have nothing to copy.
- `end` is the state when the turn ended, after the end of turn effects and
  before the fight (null when the log ends inside the turn). It is the same
  object as the last step's `after`.
- The game's `tribes` are the tribes the log showed me: every tribe of every
  minion that sat in my Tavern, my hand or my board. The lobby itself is not in
  the log, so this is a lower bound on it (a game often has a tribe the Tavern
  never offered), which a replayed pool can only draw fewer cards from.

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

- Every step also carries `after`, the whole state the step left (the same
  shape as `start`), which is what the shop replay loads before the next step
  and compares this one with. And `option` (which of the client's own options I picked,
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

`bgbench check-actions` reads the files back with nothing from our engine and
adds every turn up: the Gold the steps took against the Gold the turn started
and ended with (and Health the same way), and my hand and board at the end
against the start plus everything the steps moved. On the logs read for this
round (2026-11-03, 62 games) it finds 715 of 717 turns balancing (99.7%); the
two that do not are turns the log ends inside, before the fight. Usage:
`bgbench check-actions [--verbose] [PATTERN] [--actions DIR]`.

Every price the client printed that the steps did not pay as printed is named
by the file but one: a free Refresh in one of the new games, which the log
gives no card for.

| what happened | how many | by |
|---|---|---|
| a free reroll from free Refreshes | 551 | Leaf Through the Pages 92, Refreshing Anomaly 77+36 goldens, Sly Infiltrator 51, Underrot Spawn 27, Clairvoyance 21, Decoy Conjurer 20, Amber Guardian 18 and 50 more kinds |
| a Gold gain inside a step | 267 | Wand of Divination 75, Tavern Coin 52, Wealthy Bounty 34, Hasty Excavation 25, Snare Trapper 23, Corrupted Coin 22 and 16 more |
| a price paid in Health and given back | 104 | Soul Rewinder 62, Ashen Corruptor 42 |
| a card stolen out of the Tavern | 25 | Decoy Conjurer 18 (+1 golden), Enchanted Lasso 6 |
| a price paid in Health | 15 | Hasty Excavation 13, 2 with no card |

## How the boards are used

A training game can seat a **real seat** that plays these boards instead of
shopping (docs/engine.md "Real-board seats"): at the start of each of its
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
golden), Ultraviolet Ascendant (the third number is its count), Patient
Scout (its tier, minus 1) and Sacrificial Wrathguard (the third number, the
part of its Tavern buff the Activates added, / 2, / 4 golden). A Tasty
Lobster's numbers are the player's
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
fights share. It is a bug finder: it fixes nothing.

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
    reads off its text (Onyxia's Broodmother bonus, and Ozumat's Tentacular,
    whose number is the Tentacle's Attack now and so the sales the Tentacle
    grows by). The three powers whose fight effect only runs when the power was
    pressed in the same turn (I'll Take That!, Embrace Your Rage, Rapid
    Reanimation) are played as pressed: whoever holds one and can pay for it
    presses it before a fight. Rapid Reanimation then needs the minion it
    chose, which the file says with `reanimate`; without a marked minion the
    power is armed with nothing to destroy, as it is in a game where the
    player did not use it. Lock and Load gets the minion it holds, which the
    file carries as `hero_power.waiting`;
  - the Secret zone, from the file's `secrets`, in the order the Secrets were
    got (the order they fire in);
  - the trinkets (`TrinketDb::by_card_id`) in the slot the file marks, with
    their "improve this" number from the printed stats where a fight effect
    grows them (Fang Anklet, All-Purpose Kibble) and the small standing rules
    the fight reads (a power's fight effect running twice, the Hammer of
    Twilight aura on minions the fight summons).
- **Incomplete:** a fight is incomplete when the engine does not build
  something it carried, and its missing pieces are listed: a minion (it
  keeps its stats and keywords as a Stand-in, Era's rule for the loader), a
  hero power, a trinket, a Dark Gift, a Secret card the engine has no kind
  for, a quest reward (no quest is modelled), a hand card the log does not
  name. A file written before a key existed counts too, because the check
  cannot tell what was missing: Lock and Load without `hero_power.waiting`,
  Prestidigitation without `secrets`, Ozumat without the power's numbers. An
  incomplete fight is never flagged.
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

## The shop replay

`bgbench shopreplay` covers the other half of a game: every step Era took in a
shop phase (`boards/actions/*.json`), played through the engine and compared
with what the real game did. It is a bug finder like the fight check: it
fixes nothing.

```
./target/release/bgbench shopreplay [--actions DIR] [--seeds N] [--out DIR] [--card NAME]
```

Defaults: `boards/actions`, 64 seeds, `boards/checks` (git-ignored), no card
filter. It needs no `ref/bg-cards.json`: the file names every card it writes.
Every step in the folder (62 games, 717 turns, 15,674 steps in
`boards/actions` on 2026-11-03) takes about two seconds on 32 cores: every seed
of a random step is run, since any of them may be the log's.

- **Reset before every step.** Replaying a whole turn would let one early
  mismatch spoil every later step, and forcing the log's randomness into the
  engine would need hooks everywhere. So a step is replayed on its own: the
  engine is loaded from the logged state *before* it (the turn's `start`, else
  the `after` of the step before), the step is applied, and the result is
  compared with that step's `after`.
- **The loader** (`bgcore::shop_replay::load`) is the fight check's, extended:
  the same `RealBoard::build_one` and `Player::from_real_board` seat the board,
  the hand and the player-level numbers, then the shop phase's own fields are
  set (Gold, the Tavern tier and its price, the free Refreshes, the freeze, the
  hero power and the trinkets, the Secret zone). What it has to get right, and
  why:
  - **The hero** is found through a hero power's card id (the log names a hero
    by its game id, often a skin, which the card set does not use), walking a
    chooser power's parents back to the one the hero starts with. A state with
    no power at all is a hero whose passive power has no entity in play, and
    the player is then seated without a hero.
  - **The two power slots** are the file's own `second` mark, not the order of
    its list (a state can name the second power first: `11_02_00_10_22-g5`
    turn 11 has Patches' power in the second slot and a trinket's in the main
    one). A file without the mark is in slot order. The comparison orders the
    file's list the same way.
  - **A name that is not the card's name.** The ref data names every Dark
    Paradox version plainly ("Dark Paradox") and the file's card id says which
    one (`BG36_360t3` and its kin, golden `BG36_360_Gt3`; `cards.toml` builds
    them as "Dark Paradox t3" to "t9"), so `CardIdMap` names them by their id,
    and the shop replay's own loader does the same through
    `real_boards::resolved_name`. Floating Candle Set's Spellcraft spell is
    logged under the trinket's name (`BG36_MagicItem_208t`); `cards.toml`
    builds it as the "Floating Candle" token, and the same table maps it. The
    greater Jailer Sticker's spell prints the lesser one's name in the log
    (both are "Jailer Sticker", `BG35_MagicItem_306t` and `BG35_MagicItem_733t`)
    but says "2 random Undead" where the lesser says "a random Undead":
    `cards.toml` builds it as "Jailer Sticker (Greater)", and only the card id
    tells the two apart.
  - **The Tavern's spell slot** is the engine's own (`Player.shop_spell`), not
    a minion slot: the game's spell slot is the last spell in slot order, and
    putting it in `shop` let a card that picks a random Tavern minion pick the
    spell. A spell the game put in a minion slot (Saloon's Finest) stays in
    `shop`, and a buy of the spell slot is `Action::BuySpell`.
  - **The freeze** is one state on the player (`pl.frozen`) and off the cards,
    because the engine toggles the player's flag when Freeze is pressed.
  - **The last opponent's warband** (`last_opponent_board`, see "The action
    file") is seated twice: as that opponent's seen board (`Player.seen`, with
    `Player.last_opponent` naming a seat, which Bob's Burgles and Time Thief
    copy from) and as `Player.next_board`, so that I Spy's button is legal. The
    log keeps no state of the warband I am about to fight, so I Spy is pressed
    against the last one I fought; the offers it opens are the file's own (the
    replay forces a pick's options), so only the press's own state is read. A
    card the card set cannot build is left out of the warband (it is copied by
    card only).
  - **The standing rules** a power or trinket sets are recomputed on the seat
    (`Game::sync_seat_rules` for the powers; for a trinket,
    `Game::seat_trinket_pick_state` runs its `on_pick` acts that leave a rule
    standing, through the pick's own code), or a Tavern dealt by a Refresh
    would ignore Stay Frosty, Enhancification, Glowing Gauntlet and the like.
    A trinket's `on_pick` acts that hand out a card, Gold or stats once are
    skipped: the logged board, hand and numbers are already theirs.
  - **Magnetic** is put back on a loaded card (the board loader drops it, since
    a fight never reads it), and a Lockbox's turns left are its `{0}`.
  - **The counters a card keeps** are read off the logged numbers, card by
    card, because the same number means something else on each card:
    - a minion's `on_event` trigger counts its events in `improvements`
      ("After you spend 9 Gold, get a random Bounty. (6 Gold left!)"), and the
      card data says which cards have one and how many events it takes: the
      logged first number is what is left of it (Sky Admiral Rogers, Gunpowder
      Courier, Felboar, Eredar Escapist, Enterprising Escapee, Air Revenant,
      Auto Reveille, Unbound Tempest). A card whose first number is something
      else (Living Azerite's buff pair), whose trigger is every 1, or whose
      count the log never fills (Doubloon Grifter's "@ left!") keeps 0;
    - a trinket's "left this turn" or "every N" count: Warcry Totem's free
      Battlecry minions (`tp.free_buys`, its `{0}`), Transcribing Typewriter's
      copies (`tp.copy_buys`), Magicfin Sticker's uses (`uses_left`), Guiding
      Candle's Tier 6 Refreshes (`tp.refresh_tier6`, its `{0}`) and Wand of
      Divination's count toward the third spell on a minion (`count`, three
      minus its `{0}`), and Snarky Portrait's "improve this" (`value`, whose
      `{0}` is the buff the next Fishbait death gives: 4 at the pick, then 4
      more per death). Every other trinket's numbers are a stat, an
      improvement count or a card id (Portable Factory) and are left alone;
    - a hero power's count: Tae'thelan's Reliquary Research counts Tavern
      spells bought in `HeroCounts.tavern_spells_bought`, which the log shows
      as how many more spells it takes, and Gone Fishing counts the minions
      sold in `PowerState.progress`, which the log shows the other way round
      as the sells left ("5" is none since the last Murloc, then 4, 3, 2, 1
      and a Murloc at 0: `Hearthstone_2026_10_28_22_11_50-g4` turn 12);
    - Goldenizer Supply's "every 3 turns" count (`TrinketState.count`, from
      the turns left in its `{0}`: 3 left is the turn after the pick, 1 left
      is the turn it fires on), and a Magicfin Mycologist's teachings left
      this turn (`uses_left`, its `{0}`, and no number at all means none).
  - **A Magicfin token's taught spell** is a dbf id in the token's `{0}`, not
    a name: `shop_replay::seat_taught_spells` turns it into the card through
    the ref file's own dbf ids (before the replay runs, since `load` has no
    ref data) and the loader hands it to `Minion.taught_spell`.
  - **"The last Tavern spell you cast"** (`Player.last_spell`, which
    Cataclysmic Harbinger, the Galaxy's Lens and Lens Case copy) is in no
    state: `replay_step` reads it off the file's history, the last `play`
    step of a Tavern spell before the step being replayed. A spell a minion
    cast for the player and a Blood Gem are not it.
  - **Linked cards** (a sold Faceless Operative or Wandering Willbreaker,
    Kith'ix's Dark Ritual) are a pair the state carries only through the card
    each came from: `shop_replay::seat_links` links two hand cards with one
    `from` that came from a source handing out a pair, so playing one
    discards the other (`Minion.linked`, docs/rules/design.md "Linked
    cards"). Four cards from one source (two presses, a golden source sold
    twice) do not say who goes with whom and stay unlinked.
  - **A start-of-turn card a full hand refused** (a Spellcraft spell, a
    Lockbox) waits for room and joins at the first step that frees a slot
    (docs/rules/design.md "A full hand"), and no state can show a card that is
    not in the hand yet: `shop_replay::seat_queued_start_of_turn` queues it
    again from the trinket the state does show, unless the hand holds it or
    the turn already shows it arriving or being played
    (`Hearthstone_2026_10_28_22_11_50-g6` turn 8's Lockbox). The same log
    shows the gap this cannot close: a trinket's start-of-turn trigger the
    game resolves after a trinket pick or a pending Discover fires at
    `MAIN_START`, behind the logged step boundary
    (`Hearthstone_2026_10_26_00_48_15-g2` turn 9's Floating Candle,
    `Hearthstone_2026_10_30_19_45_15-g2` turn 9's Token of the Old Gods), and
    that one is the collector's boundary to move.

      as how many more spells it takes; Buried Treasure's number is the digs
      left ("(1 left!)" is the fourth dig) and the engine counts the digs done
      in `PowerState.progress`; and the price on the button is the log's own
      (`ShopPowerSpec::cost`), which seats `PowerState.cost_delta` as its
      difference from the card's cost (Pirate Parrrrty!'s Pirates bought, The
      Galaxy's Lens's turns waited).
  - **The last Tavern spell you cast** (The Galaxy's Lens) is in no state, and
    the log's press only shows the copy it gave: `replay_step` reads it off the
    steps before the press, the last `play` of a Tavern spell in the game,
    earlier turns included (docs/rules/heroes.md, Era 2026-11-03).
  - **A chosen power that completes at the start of a turn** (Galewing's
    flightpaths) is the one turn-start trigger the counters cannot carry: the
    log's turn-start state is taken before the countdown. A state whose chosen
    power shows one turn left, after a turn that ended on the same number, is
    the turn the count runs out, and the loader seats the parent the power
    completes into (the Discover the chooser's effect opened is the step).
  - **Steady Growth's turn** ("At the end of your turn, gain +X/+Y", the pair
    fixed by the turn the gift landed) is the first turn the file shows that
    card with that gift (`parse_action_files` seats `ShopGiftSpec::landed`
    from the file's own history, by card name and golden flag: the game gives
    a minion a new entity id every turn, so the ids cannot follow it). The
    growth then comes from darkgifts.toml's table for that turn, and the
    doubling for Drakkari the engine already does.
  - **A Discover that takes a slot** is seated from the pick's `moved.to` when
    the file writes one, and otherwise from the state it left (a pick that
    turns a minion into itself adds no copy to the hand, and the state shows
    the pick where another card stood: the transformed card keeps its entity
    id): the engine's `discover_into_shop` for a Tavern slot (Galakrond's Greed
    and Token of the Old Gods on a Tavern minion) and `tp.transform_slot` for a
    board minion. The slot is the file's, which counts the spell slot; the
    engine's shop holds the minion slots alone. The old card is still in the
    Tavern in the state before the pick (the game keeps it on show while the
    Discover is open) and the engine's pick puts it back in the pool, so
    nothing is removed twice.
  - **A played minion's slot** (`play_slot`) is where it stands in the state
    the step left, among the cards the loader already knows: the file's
    `moved.to` is the position the log showed when it landed, which is before
    a Battlecry's summons push it and before the minions it killed leave, so
    with a full board it is even past the board. A card the step destroyed and
    brought back to the slot it stood in (a Reborn minion, whose new entity id
    the step's `left` and `entered` say) still holds that slot, and so does
    one the game rebuilt under a new id as it landed (Scarlet Skull in
    `10_24_15_25_26-g6` turn 12 step 29: the same card id, an id the state
    before did not hold).
  - **The game's own triple check** ("Check Triples") runs as an action ends,
    not at the summon or the gain that made the third copy, so a logged state
    can hold three plain copies a summon left on the board: the Maw Caster's
    Skeletons of `10_24_15_25_26-g6` turn 8 sit there for two steps while its
    Discover is answered. `load` asks for the check on the step's own action
    (`Game::note_triples_due`) when `board_triple_due` reads it in the two
    states: three plain copies of one name on the board before, and a Golden
    card of that name in the hand after that the state before did not hold.
  - **What a pick still owes the action that opened it.** The game resolves a
    Discover or a Choose One inside the play that asked for it, but the log
    makes the pick a step of its own, and `replay_step` replays the pick alone
    from the state before it: nothing of the play is owed any more. The loader
    seats it back from the step's `source` card, for a pick that ends a spell
    the player cast (`shop_replay::seat_cast_after_pick`: the "after you cast a
    spell" triggers and the Spell Siphon Dark Gift count, log
    `10_24_17_59_26-g1` turn 10 step 43 to 44) and for a Discover a Battlecry
    opened (`owe_battlecry_count`, "for each Battlecry" counting at the pick).
    A pick that is not the play's last word (a Discover a Battlecry opened and
    a target it asks for after it) is still one step here, so the rest of the
    play is behind the boundary.
  - **A Magnetized minion's host** (`magnetize_host`) is the card the played
    minion's own stats went into: the step's `changed` list is in no order of
    its own, and a stat static the play set off comes before the host. It must
    be a card the played minion can Magnetize onto and its stats must have
    grown by at least the played card's.
  - A game with one player is a sandbox game, so nothing else acts in it. The
    pool is the lobby's minus the copies this player holds, so a Refresh draws
    from what the game had left, and the cards hold those copies so a sale
    gives them back.
- **Not compared:** a hero power's presses left (the client's counter moves at
  whichever of a press's clicks it chooses, and not at all for a chooser), the
  MAGNETIC keyword (a card property), which card of a zone a random change
  landed on (a random step's shape is the set of changes, not the cards they
  fell on), and the client's freeze button (the per-card flags say the same
  thing).
- **A minion's target is a place on the board before it lands.** A Battlecry
  or Choose One picks its target before the played minion is in its slot, so
  the log's board place for the target is the board as it stood; the engine has
  already put the minion where the file says it landed, and `actions_for` moves
  every board pick from that slot on one place along. Without it the pick lands
  on the played minion itself (Disguised Graverobber in
  `10_24_00_56_06-g1` turn 11, Sprightly Scarab's Choose One).
- **Steps the file cannot carry.** A prompt with one answer is no prompt: the
  engine applies the only target itself, so the step's own target click has
  nothing left to answer and the state it left is compared instead. A Discover
  the one target opened (Kelp Keeper triggering a Battlecry that Discovers,
  Trash for Treasure) is not a target prompt and does not change that. A pick of a
  two-step effect (Spirit Swap's second minion) answers the prompt the game
  opened in the step before, which one state cannot carry: that step is
  incomplete. So is that power's second click when the log writes it as a
  press of its own (`BG20_HERO_201p2`): the game counts each click as a use
  and swaps the button to the power's second half, while the engine presses
  once, so the file shows a power the engine does not build. The Warband Whistle's one-shot "your next Refresh holds copies of
  your warband" is armed from the steps before it (the pick armed it, the first
  Refresh after that spent it), since no state says so.
- **The step to the engine's action:** `buy` slot, `sell` position, `play`
  index with its target (and a Choose One's `sub_option`), `activate`, `move`,
  `roll`, `freeze`, `level`, `hero_power` (main or second) with its target,
  `dark_discovery`, `discover`/`trinket_pick` (the log's options are loaded as
  the engine's offer, then the same one is picked), `end_turn` (the end of turn
  effects alone, through `Game::shop_end_of_turn`, without the fight). A step
  whose target is a card in the hand is a Discard (`Action::Discard`, the
  card's hand slot): an Activate with `discard = true` (Mangled Bandit) or a
  power that discards (Drest'agath's Incubate).
- **Deterministic or random.** Every step is run with `--seeds` seeds. If every
  seed gives the same state the step is deterministic and is compared exactly.
  If not, it is random, and every seed's outcome is looked at, not the first
  one's: it passes when one of them is the log's, or, for a large random space
  (a Refresh, a random minion out of the pool), when one of them has the same
  shape: Gold, Health, armor, the tier and its price, the free Refreshes, how
  many cards each zone holds, how many of them are spells, and every change
  the step made to a card that was there before and still is, as a multiset
  per zone: the stat delta, the keywords it gained or lost, and whether the
  card is golden. Which card of a zone a change landed on is not part of it
  (one seed picks the Tea Set's Dragon, another the other one; one gives a
  buff to the first of two same-named minions, another to the second), and a
  card is paired with the copies of its own name rather than by position, so
  a minion that moved, or that another minion was sold from under, is still
  the same card. A change the other side does not have, or one of another
  size, is still a difference.
- **Incomplete, not flagged.** A step is incomplete when the state or the step
  involves something the engine does not build (a minion, a hero power, a
  trinket, a Dark Gift, an option no card set has) or something the log does
  not say. That is the same rule as the fight check's, and an incomplete step
  is never flagged.
- **Outputs** (`boards/checks/`):
  - `steps.tsv`: one row per step (game, turn, step, kind, card, verdict,
    deterministic, the first difference);
  - `shop-cards.tsv`: per card, hero power, trinket and Dark Gift: the steps it
    was in, the flagged ones and the ratio, most suspicious first;
  - `shop-report.md`: the summary, the steps by kind, why steps were
    incomplete, the most common differences grouped by cause, the 30 most
    suspicious cards, and 30 flagged steps with the state before, the action,
    the log's result and the engine's.
- `--card NAME` limits a run to the steps that involve that card (by name or
  card id), so a later build run can check the cards it touched.

What the replay cannot see, and so cannot blame on the engine: the lobby's real
tribes (the file's `tribes` are the ones the log showed), the client's own
prices where a discount is invisible (a Tavern spell is priced from the card's
own marks or, failing those, from what its buy step paid, so a discount the
log applied is the log's number; a minion's price comes from the engine's own
rule, and so does a standing "minions cost (1)" from a played Friends and
Family Discount or the Chillmere Mosaic spell, because a logged Tavern minion
carries no price unless the game wrote one on it), a
trinket's per-option price at a pick (the file only says what the pick paid), a
Dark Gift's countdown (Incubation, Replication and Affinity keep "turns left" on
the gift's enchantment, which the log does not number, so a start of turn and an
end of turn step that is waiting on it is short by one card), and anything that
happens between two of Era's decisions (the file writes it into the step
before).

Measures for the two checks on one machine are comparable: `fightcheck` reads
a fight, `shopreplay` a step.
