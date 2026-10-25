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
| `bg-meta-<build>.json` | heroes, hero powers, trinkets and Dark Gifts (see the last section) |
| `bg-meta.json` | copy of the file above |

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
spells (tier 1 to 7: 8, 7, 15, 18, 16, 5, 4). Not in `bg-cards.json`: heroes,
hero powers, trinkets and Dark Gifts (they are in `bg-meta.json`), enchantments,
quest rewards, anomalies (not exported).

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
- The card-pool sections above were checked against two sessions on 2026-10-18
  (four games). Power.log now holds four sessions on 2026-10-24 (the meta
  section below uses those). Absence in a log is weak evidence either way.

# Heroes, hero powers, trinkets, Dark Gifts (`bg-meta.json`)

Same source (`dbf.unity3d`), same text rules and script-data rules as for cards
(`text`, `text_alt`, `text_template`, `script_data`, `unresolved_numbers`,
`lobby_subsets`, `keywords`, raw `tags`). `bg-cards.json` is unchanged. Top
level keys: `counts`, `heroes`, `hero_powers`, `trinkets`, `dark_gifts`,
`dark_gift_placeholders`. Power.log evidence is the four 2026-10-24 sessions
(build 254477), field `log_entities` (entities created with that card id; for
heroes `log_entities_with_skins` adds the cosmetic skins, since the game plays
a skin id).

Extra tag ids (names from CardDefs.xml unless said):

| Id | Name | Id | Name |
|---|---|---|---|
| 292 | ARMOR | 380 | HERO_POWER (dbf id of the hero's power) |
| 1491 | BACON_HERO_CAN_BE_DRAFTED | 2376 | BACON_HEROPOWER_BASE_HERO_ID (hero dbf id, on a hero power) |
| 2038 / 2039 | BACON_SKIN / BACON_SKIN_PARENT_ID (hero dbf id) | 2130 | BACON_COMPANION_ID (buddy dbf id) |
| 1635 | SPELL_SCHOOL (11 Lesser trinket, 12 Greater trinket) | 3407 | BACON_TRINKET |
| 3567 | IS_NIGHTMARE_BONUS (on every Dark Gift) | 3627 | DARK_GIFT (see below) |
| 3166 | IS_BACON_DUOS_EXCLUSIVE | 3206 | no name, see Heroes |

## Heroes

Counts: **143** heroes exported: **118 in the solo pool**, 5 Duos only, 20 not
in the pool (see below). 654 cosmetic skins (cards with tag 2038 or 2039) are
not exported; each hero lists its `skin_card_ids`.

- Membership of the list: CARDTYPE 3, not a skin, and tag 1491 present or a
  BG card id.
- Solo pool: **1491 equals 1 and not 3166**. Sure about the tag (123 heroes
  have 1491 = 1, the value is never anything else). Log check: 73 of the 118
  were created in the 2026-10-24 games (counting skins), 0 of the heroes seen
  are outside the rule except the game's own helper heroes (PH placeholder,
  Bartender Bob, Kel'Thuzad, Aranna Unleashed, which have no 1491).
  The other 45 were simply not seen.
- Duos only: tag 3166 = 1. The 5 are BGDUO_HERO_100, 101, 104, 222, 223 (they
  also carry 1491 = 1, so 1491 alone would put them in a solo pool).
- The 20 not in the pool: placeholders and helpers (TB_BaconShop_HERO_PH,
  TB_BaconShopBob, TB_BaconShopBob_TWT, Kel'Thuzad, Lady Deathwhisper), the
  `_Tutorial` heroes (8), and old or transformed heroes without 1491 (Giantfin,
  Nefarian, Bartendotron, Tirion Fordring, Lady Vashj TB_61, Aranna Unleashed
  TB_59t, Naga Queen Azshara BG22_HERO_007t). Why each is out is my reading of
  the ids, not a tag.
- Health: tag 45, 30 for every pool hero (60: Bartender Bob and Patchwerk,
  TB_BaconShop_HERO_34, which is in the pool and has no ARMOR tag).
- Starting armor: tag 292. Matches Power.log (Forest Warden Omu TB_74: 6 in
  both). No armor tiers found: I could not find a tag that holds per-tier
  armor. Tag 3206 (unnamed) is ARMOR minus 2 for 93 of the 123
  draftable heroes (12 lack it, the rest differ). `armor_tiers` is always null.
- Hero power: tag 380 (dbf id), always present for the 123 draftable heroes.

## Hero powers

Counts: **172** exported (CARDTYPE 10 with a BG card id, or the power of a BG
hero): 129 are a hero's own base power (tag 380), 152 belong to a solo pool
hero (base power or an alternate form), 11 have no hero (below).

- Hero: tag 2376 (base hero dbf id) first, else the hero whose tag 380 points
  at the power (`hero_link` says which). Sure for 161 of 172. Powers whose hero
  is not a pool hero have `hero_in_solo_pool` false.
- Alternate forms of a power (ids like `..._t1` to `_t4`, `...p2`) are separate cards with 2376 and no link from tag 380. They are
  exported with their hero.
- `cost`: tag 48. Only 58 of 172 have it, the rest show no cost in the data
  (shown null, not 0). `hide_cost` is tag 684.
- Passive or active: **no tag says it**. `passive_by_text` is true when the
  text contains "passive" (8 cards: BG21_HERO_000p, BG22_HERO_000p_t1 to t4,
  BG27_HERO_801p2, BG31_HERO_811p, BG31_HERO_811p2). Not reliable the other
  way: Mister Clocksworth's Double Time and Everbloom read as passive but are
  false. Treat false as "unknown".
- Text: 22 have an alternate text. 4 have `{n}` left unresolved (the game sets
  those numbers at runtime). Hero powers keep their current numbers in
  TAG_SCRIPT_DATA_NUM_1..4 at runtime, as `bg/battlegrounds.py` reads them.
- Log: 87 of 172 were created in the 2026-10-24 games, all of them belong to
  a pool hero.

## Trinkets

Counts: **420** CARDTYPE 44 cards. **255 in the solo pool**: 122 Lesser, 133
Greater. The rest: 2 placeholders, 9 Duos only, 128 with 3407 = 0 and 26 with
no 3407 (rotated out or event cards).

- Lesser or Greater: **SPELL_SCHOOL (1635) 11 = Lesser, 12 = Greater**. Sure:
  in the log all 111 trinkets seen carry that school, and the Lesser ones sit
  in NUM_6 slot 1, the Greater in slot 2 (as `bg/battlegrounds.py` reads it).
- Pool: **BACON_TRINKET (3407) equals 1, not a placeholder, not 3166**. Fairly
  sure: all 111 non-placeholder trinkets seen in the logs have 3407 = 1 and
  none with 0 or missing was seen. The 3407 = 0 ones have not been seen so I
  cannot prove they are out of the pool, only that nothing contradicts it.
- Placeholders: `BG30_Trinket_1st` and `BG30_Trinket_2nd` ("Lesser Trinket" /
  "Greater Trinket", empty slots) carry 3407 = 1 and are excluded.
- `upgrade_of`: a Greater card whose id is a Lesser card's id plus `t` (46
  cards, 28 of them with 3407 = 1 so counted in the 133). They looked like
  the Greater form a Lesser trinket turns into, but 11 of them were created in
  the logs (zones SETASIDE and PLAY, with a CREATOR), so I left them in the
  pool. I could not tell whether they are offered directly. Unresolved.
- `lobby_subsets`: 184 of the pool trinkets carry BACON_SUBSET tags, i.e. they
  are tied to a tribe being in the lobby (same rule as for cards).
- `cost` null (74 pool trinkets) means no COST tag, not free.
- 8 have an unresolved `{n}`.
- 4 trinkets carry tag 3627 (they hand out a Dark Gift minion).

## Dark Gifts

Counts: **47** Dark Gifts, all marked in the pool.

- What they are: a Dark Gift is a SPELL card (CARDTYPE 5) with id
  `BG36_MidGameEffect_000t<n>` (4 variants end in a second `t`: 28t, 29t, 30t,
  64t). Its enchantments are CARDTYPE 6, ids `<gift id>e`, `e2`, `e3`. This
  matches `GIFT_ENCHANTMENT` and `gifts_by_minion` in `bg/battlegrounds.py`,
  which names the gift from the enchantment id on the minion.
- Tag: **IS_NIGHTMARE_BONUS (3567) = 1** on 47 of those 48 cards. The 48th,
  `BG36_MidGameEffect_010` ("Battlegrounds Dark Gift [DNT]"), is a placeholder
  with no tags (32 times in the logs, always SETASIDE). It is listed under
  `dark_gift_placeholders`, not as a gift.
- Tag 3627 (DARK_GIFT) is **not** the gift marker. It is on 29 cards that give
  or offer a Dark Gift (14 minions, 8 spells, 4 trinkets, 2 hero powers, 1 of
  type 7). The 10 minions with tag 3880 (HAS_DARK_GIFT) are the Dark Paradox
  forms (BG36_360t*), which carry a gift.
- Pool flag: **none found**. `in_solo_pool` is just "3567 = 1". Tags 4853 and
  4854 (unnamed, 3 to 12) look like a stat range and 4855 is 1 on all 47. I
  did not work out what any of them mean, so no pool or weight is exported.
- Log: 37 of the 47 had an enchantment in the logs. The 10 with none
  (`log_enchantment_entities` = 0): t13, t28, t28t, t60, t62, t69, t72, t78,
  t88, t89. Absence is weak evidence.
- 8 gift texts keep an `{n}` (a value the game sets on the minion).

## Not resolved (meta)

- Armor tiers per hero: none found. Starting armor is tag 292 only. Tag 3206
  (about armor minus 2) is exported raw as `armor_tag_3206`.
- No tag for passive hero powers (only `passive_by_text`).
- Whether the Greater `...t` trinkets are directly offered or only made by
  upgrading a Lesser one (28 are flagged pool).
- Whether 3407 = 0 trinkets are really out of the pool (never seen).
- Whether 3567 = 1 means every Dark Gift is in the pool, or there is a pool tag
  I could not find (4853, 4854, 4855, 4874 are unnamed).
- 11 hero powers have no hero (anomaly and quest powers such as Rage Potion,
  Match Fixing, Eject!): `hero_card_id` is null.
- Why 20 heroes without 1491 exist is guessed from their ids.
- 45 of the 118 pool heroes were not in the logs, so the rule is checked for
  the other 73 only.
