# Card data export

`tools/export_cards.py` reads the installed game files and writes the card
data the engine should trust. HearthstoneJSON (`ref/hsjson-cards.json`) stays
as a cross-reference only.

## Run it

```
.venv-unity/bin/python tools/export_cards.py        # about 3 seconds
```

The venv is git-ignored. To rebuild it: `python3 -m venv .venv-unity &&
.venv-unity/bin/pip install UnityPy` (the only step that needs network).

Output (all under git-ignored `ref/`):

| File | What |
|---|---|
| `bg-cards-<build>.json` | every Battlegrounds minion and spell, golden versions included |
| `bg-cards.json` | copy of the file above |
| `bg-disagreements.json` | pool flags and `@` texts versus HearthstoneJSON, plus Power.log evidence |

Read them with `python3 -I`. The export reads
`~/hearthstone-linux/hearthstone/Data/OSX/dbf.unity3d` (tables `CARD_TAG`,
`CARD`, `CARD_RACE`) and the Power.log files. Nothing is written there and
the engine never touches the network.

## Build

- Power.log `BuildNumber`: **254477**. This is the number in the file name.
- Unity version of the bundle: 6000.3.11f1 (from `globalgamemanagers`).
- `~/hearthstone-linux/craft.log` lists the last install as 37.0.0.255130.254477
  and HearthstoneJSON's latest CardDefs is build 255130. I compared 36,390 cards
  (tags 45, 47, 48, 200, 1429, 1440, 1456, 3081, 2, 3, 2889, 2919) between the
  bundle and that CardDefs: identical, except that the bundle writes an
  explicit 0 where CardDefs leaves the tag out (and LOOT_091 health). So the
  card data is the 255130 data. I could not tell from the files alone which
  number is "the build"; both are recorded here.

## Counts (this build)

Minions 1356 normal and 1236 golden,
Battleground spells 208, plain SPELL cards with a BG id 369, 345 cards in the
solo pool: 272 minions (tier 1 to 7: 24, 37, 45, 62, 56, 35, 13) and 73
spells (tier 1 to 7: 8, 7, 15, 18, 16, 5, 4). Not exported: heroes, hero
powers, enchantments, trinkets, quest rewards, anomalies.

## Tag ids used

| Id | Name | Id | Name |
|---|---|---|---|
| 47 / 45 / 48 | ATK / HEALTH / COST | 1440 | TECH_LEVEL |
| 200 | RACE (first tribe) | 202 | CARDTYPE (4 minion, 5 spell, 42 BG spell) |
| 1456 | IS_BACON_POOL_MINION | 3081 | IS_BACON_POOL_SPELL |
| 3166 | IS_BACON_DUOS_EXCLUSIVE | 1429 | BACON_TRIPLE_UPGRADE_MINION_ID (golden dbf id) |
| 1471 | BACON_TRIPLED_BASE_MINION_ID (normal dbf id) | 2154 | BACON_BUDDY |
| 2, 3, 2889, 2919 | TAG_SCRIPT_DATA_NUM_1..4 | 2347 and others | BACON_SUBSET_* (see below) |

- Tribes: tag 200 plus every tribe whose own tag (from `CARD_RACE`, for example
  2534 for Undead) is 1. The `CARD_RACE` id is the race enum (11 Undead, 14
  Murloc, 15 Demon, 17 Mech, 18 Elemental, 20 Beast, 23 Pirate, 24 Dragon, 26
  All, 43 Quilboar, 92 Naga, 126 Aberration).
- `BACON_SUBSET_*` (2347 Undead, 1591 Dragon and so on) is not a pool flag. It
  says which tribe subset a card belongs to for the lobby's tribe selection
  (Butchering has 2347 and is also a pool spell). Exported as `lobby_subsets`.
- Keyword ids are listed in `KEYWORDS` in the script (Taunt 190, Divine
  Shield 194, Reborn 1085, Venomous 2853, Deathrattle 217, Battlecry 218,
  Start of Combat 1531 and so on). Raw tags are kept in each card's `tags`.
- USE_ALTERNATE_CARD_TEXT is a runtime tag. It is not in the card data, only
  in Power.log.

## Text rules

- `m_textInHand`, first locale (enUS), is the template. Kept as `text_template`.
- An `@` is the separator between normal text and alternate text only when it
  is not a number placeholder. An `@` after a space, `+`, `(`, quote, `/`,
  newline or inside `<b>@</b>` is a placeholder the game fills at runtime.
  The first part is `text`, the second `text_alt` (shown when
  USE_ALTERNATE_CARD_TEXT is 1). Never joined.
- `{0}`..`{3}` are filled from TAG_SCRIPT_DATA_NUM_1..4 in the card data. If
  the card has no such tag, `{n}` stays in the text and `unresolved_numbers`
  lists it (70 cards, mostly values the game sets at runtime).
- `|4(one, many)` after a number is resolved. `text_plain` and
  `text_alt_plain` drop `<b>`/`<i>`, `[x]`, newlines and the `__` padding.
- The script data in the files is the default. Real games set it at runtime
  (Butchering showed NUM_1=8 in Power.log, the same as the file).

## What decides the solo pool

Rule that matches every check I could make:

- Minion: `IS_BACON_POOL_MINION` (1456) **equals 1**, and not 3166 Duos
  exclusive.
- Spell: `IS_BACON_POOL_SPELL` (3081) equals 1, and not 3166.
- Tier is `TECH_LEVEL` (1440). Golden versions carry no pool flag.
- Which tribes are in a lobby is decided per game by `BACON_SUBSET_*`, not by
  the card data.

Evidence:

- 1456 takes the values 0, 1 and 2. Only one card has 2: Rot Hide Gnoll
  (BG25_013). HearthstoneJSON turns any non-zero into `true`, so it lists the
  Gnoll as a pool minion. Power.log never shows that card in the logged games
  (four) (212 of the other pool minions do appear). Treat 2 as not in the pool,
  as the owner says.
- 106 minions have an explicit 1456 of 0 (rotated out, for example Tough Tusk,
  Pack Leader, Murozond) and are not in the pool in either source.
- `CARDS_SHOWN_IN_SHOP` is not Battlegrounds (it holds 23 CATA cards).
  `m_battlegroundsActiveEvent` and the other event fields do not separate pool
  from non-pool: pool minions have both 164 and 203 there (Tarecgosa and
  Insatiable Ur'zul have 203 and are in the pool).
- Power.log logs `IS_BACON_POOL_MINION` and `IS_BACON_POOL_SPELL` (as tag
  3081) on live entities. For the 271 cards seen with a pool tag, the logged
  value equals the bundle value in every case (0 mismatches).
- Pointy Arrow (EBG_Spell_014, tier 1 spell, cost 1): 3081 is 0 in the game
  files, and HearthstoneJSON also says false. Power.log has it six times,
  always in a hand, always with a CREATOR (Weapons Forge, BG36_884), and with
  no pool tag. So the logs show it as a generated spell, not a shop spell. I
  could not confirm that it is a pool spell. If the owner saw it in a shop,
  it was not in these logs, or it is newer than this build.

## Disagreements with HearthstoneJSON

Pool flags (full list in `ref/bg-disagreements.json`): 34 cards, all of one
kind or the other.

- Rot Hide Gnoll, BG25_013: HearthstoneJSON true, game value 2 (not in the
  solo pool).
- 33 Duos exclusive cards that HearthstoneJSON flags as pool cards:
  30 BGDUO minions (BGDUO_100, 104, 107 to 112, 114, 115, 117 to 122, 125, 700,
  701, BGDUO31_201 to 203, 205, 207 to 209, 211, 212, BGDUO33_140, 150) and the
  3 spells BG31_242, BG31_243, BG31_244. They are in the Duos pool, so
  HearthstoneJSON is not wrong, they are just not solo cards (tag 3166).
- No card has the reverse (game says pool, HearthstoneJSON says not). Pointy
  Arrow is false in both.

Texts with `@` (113 cards in `bg-disagreements.json`, with the HearthstoneJSON
text next to ours). HearthstoneJSON's `text` drops the `@` (leaving
"...8Destroy..." for Butchering BG28_604) and fills missing numbers with 0. 13
of the 113 have no line break to make it obvious. The 17 solo pool cards with
an alternate text:

BG26_814 Lovesick Balladist, BG27_005 Timecap'n Hooktail, BG28_503 Fortify,
BG28_604 Butchering, BG31_035 Groundbreaker, BG31_924 Thaumaturgist,
BG31_925 Showy Cyclist, BG31_816 Fire Baller, BG31_818 Snow Baller,
BG32_235 Surfing Sylvar, BG33_811 Healthy Bounty, BG33_812 Hostile Bounty,
BG34_322 Stalwart Kodo, BG35_814 Scarlet Survivor, BG36_100 Wandering
Willbreaker, BG36_308 Faceless Operative, BG36_763 Treasure Parrot.

Butchering, correctly: normal "Destroy a friendly Undead. Your Undead have +8
Attack this game (wherever they are)." and alternate "... have +8/+0 this game
(wherever they are)."

## Not resolved

- Which of 254477 or 255130 is "the build" (see Build).
- Whether Pointy Arrow is ever in a shop (see above).
- What value 2 of IS_BACON_POOL_MINION means. Only the Gnoll has it. Rule
  used: only 1 counts.
- Alternate-text numbers that the game sets at runtime and the file lacks
  (70 cards) are left as `{n}`.
- Power.log covers two sessions on 2026-10-18 (four games), so absence there
  is weak evidence.
