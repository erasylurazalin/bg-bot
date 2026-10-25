#!/usr/bin/env python3
"""Export every Battlegrounds card from the installed Hearthstone files.

Run with the venv that has UnityPy:

    .venv-unity/bin/python tools/export_cards.py

Reads (never writes) ~/hearthstone-linux/hearthstone/. Writes
ref/bg-cards-<build>.json, a copy at ref/bg-cards.json, and
ref/bg-disagreements.json (comparison with ref/hsjson-cards.json and with
the Power.log pool tags, only if those exist). Also writes
ref/bg-meta-<build>.json and a copy at ref/bg-meta.json: heroes, hero powers,
trinkets and Dark Gifts (see docs/card-data.md). No network.
"""
import argparse, collections, glob, json, os, re, shutil, sys, warnings

warnings.simplefilter("ignore")
import UnityPy

GAME = os.path.expanduser("~/hearthstone-linux/hearthstone")
BUNDLE = GAME + "/Data/OSX/dbf.unity3d"
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

# Tag ids (names from HearthstoneJSON CardDefs.xml).
T_ATTACK, T_HEALTH, T_COST = 47, 45, 48
T_RACE, T_CARDTYPE = 200, 202
T_TECH_LEVEL = 1440
T_POOL_MINION, T_POOL_SPELL = 1456, 3081
T_DUOS_ONLY = 3166
T_GOLDEN = 1429          # BACON_TRIPLE_UPGRADE_MINION_ID: dbf id of the golden card
T_BASE = 1471            # BACON_TRIPLED_BASE_MINION_ID: dbf id of the normal card
T_BUDDY = 2154
T_SPELLCRAFT = 2359
NUM_TAGS = [2, 3, 2889, 2919]   # TAG_SCRIPT_DATA_NUM_1..4

CARDTYPES = {3: "HERO", 4: "MINION", 5: "SPELL", 6: "ENCHANTMENT", 10: "HERO_POWER",
             40: "BATTLEGROUND_QUEST_REWARD", 42: "BATTLEGROUND_SPELL",
             43: "BATTLEGROUND_ANOMALY", 44: "BATTLEGROUND_TRINKET"}
RACES = {11: "UNDEAD", 14: "MURLOC", 15: "DEMON", 17: "MECHANICAL", 18: "ELEMENTAL",
         20: "BEAST", 23: "PIRATE", 24: "DRAGON", 26: "ALL", 43: "QUILBOAR",
         92: "NAGA", 126: "ABERRATION"}
KEYWORDS = {190: "TAUNT", 194: "DIVINE_SHIELD", 363: "POISONOUS", 2853: "VENOMOUS",
            189: "WINDFURY", 1207: "MEGA_WINDFURY", 1085: "REBORN", 217: "DEATHRATTLE",
            218: "BATTLECRY", 1531: "START_OF_COMBAT", 191: "STEALTH", 849: "MAGNETIC",
            2129: "AVENGE", 4204: "RALLY", 3046: "UPBEAT", 2247: "COLOSSAL",
            362: "AURA", 32: "TRIGGER_VISUAL", 227: "CANT_ATTACK", 415: "DISCOVER",
            388: "SPARE_PART", 2359: "SPELLCRAFT", 2154: "BUDDY", 4689: "CHROMADRAKE",
            4744: "OLD_GOD", 4922: "DEITY_SIGIL", 114: "ELITE"}
SUBSETS = {1591: "DRAGON", 1592: "MURLOC", 1593: "DEMON", 1594: "BEAST", 1595: "MECH",
           1596: "PIRATE", 1688: "ELEMENTAL", 1845: "QUILBOAR", 2272: "NAGA",
           2347: "UNDEAD", 4757: "ABERRATION"}
ID_RE = re.compile(r"^(BG|TB_Bacon|EBG|BGS)")

# Meta export (heroes, hero powers, trinkets, Dark Gifts).
T_HERO_POWER = 380           # HERO_POWER: dbf id of the hero's power
T_ARMOR = 292
T_SPELL_SCHOOL = 1635        # 11 = Lesser trinket, 12 = Greater trinket
T_HERO_DRAFTABLE = 1491      # BACON_HERO_CAN_BE_DRAFTED
T_HERO_BASE = 2376           # BACON_HEROPOWER_BASE_HERO_ID: dbf id of the hero a power belongs to
T_SKIN, T_SKIN_PARENT = 2038, 2039
T_COMPANION = 2130           # BACON_COMPANION_ID: the hero's buddy
T_HERO_ARMOR_2 = 3206        # unnamed in CardDefs, mostly ARMOR minus 2 (unresolved)
T_TRINKET = 3407             # BACON_TRINKET
T_DARK_GIFT_MENTION = 3627   # DARK_GIFT: card grants or mentions a Dark Gift
T_NIGHTMARE_BONUS = 3567     # IS_NIGHTMARE_BONUS: set on every Dark Gift card
T_HIDE_COST = 684
GIFT_ID_RE = re.compile(r"^BG36_MidGameEffect_000t\d*t?$")
GIFT_ENCH_RE = re.compile(r"^(BG36_MidGameEffect_000t\d*t?)e\d*$")   # as in bg/battlegrounds.py
TRINKET_SCHOOLS = {11: "LESSER", 12: "GREATER"}


def split_alt(text):
    """Split a card text at the '@' that separates normal and alternate text.
    An '@' after a space, sign, bracket or quote (or inside <b>@</b>) is a
    number placeholder that the game fills at runtime, not a separator."""
    parts, cur = [], 0
    for i, ch in enumerate(text):
        if ch != "@":
            continue
        prev = text[i - 1] if i else ""
        nxt = text[i + 1] if i + 1 < len(text) else ""
        if i == 0 or prev in " +-(\"/,\n#" or text[max(0, i - 3):i] == "<b>" \
                or (prev == "," or nxt in ",)"):
            continue
        parts.append(text[cur:i])
        cur = i + 1
    parts.append(text[cur:])
    return parts


def fill(text, nums):
    """Fill {0}..{3} from script data and resolve |4(singular, plural)."""
    unresolved = []

    def sub(m):
        i = int(m.group(1))
        v = nums[i] if i < len(nums) else None
        if v is None:
            unresolved.append(i)
            return m.group(0)
        return str(v)
    out = re.sub(r"\{(\d)\}", sub, text)

    def plural(m):
        # "3 |4(turn, turns)" picks by the number just before the macro.
        return m.group(1) + (m.group(2) if m.group(1) == "1" else m.group(3))
    out = re.sub(r"(\d+ )\|4\(([^,)]*),\s*([^)]*)\)", plural, out)
    return out, sorted(set(unresolved))


def plain(text):
    t = re.sub(r"<[^>]+>", "", text)
    t = t.replace("[x]", "").replace("\n", " ")
    t = re.sub(r"_{2,}", " ", t)
    return re.sub(r"\s+", " ", t).strip()


def client_build():
    best = None
    for f in sorted(glob.glob(GAME + "/Logs/*/Power.log")):
        with open(f, errors="replace") as fh:
            for line in fh:
                m = re.search(r"BuildNumber=(\d+)", line)
                if m:
                    best = int(m.group(1))
                    break
    return best


def unity_version():
    import subprocess
    raw = open(GAME + "/Bin/Hearthstone_Data/globalgamemanagers", "rb").read(64)
    m = re.search(rb"\d+\.\d+\.\d+[a-z]\d+", raw)
    return m.group().decode() if m else "6000.3.11f1"


def load_tables():
    UnityPy.config.FALLBACK_UNITY_VERSION = unity_version()
    env = UnityPy.load(BUNDLE)
    objs = {o.peek_name(): o for o in env.objects if o.type.name == "MonoBehaviour"}
    t = {}
    for n in ("CARD_TAG", "CARD", "CARD_RACE"):
        t[n] = objs[n].read_typetree()["Records"]   # parse each big table once
    return t


def power_log_evidence():
    """Per card id: entity count, IS_BACON_POOL_* tag values logged, creator count."""
    res = collections.defaultdict(lambda: {"entities": 0, "pool_minion": set(), "pool_spell": set(), "with_creator": 0})
    head = re.compile(r"GameState\.DebugPrintPower\(\) -\s+(?:FULL_ENTITY - Creating|SHOW_ENTITY - Updating)\b.*?CardID=(\S*)\s*$")
    tag = re.compile(r"GameState\.DebugPrintPower\(\) -\s{6,}tag=(\S+) value=(\S+)")
    for f in glob.glob(GAME + "/Logs/*/Power.log"):
        cur = None
        with open(f, errors="replace") as fh:
            for line in fh:
                if "GameState.DebugPrintPower() -" not in line:
                    continue
                m = head.search(line)
                if m:
                    cur = m.group(1) or None
                    if cur:
                        res[cur]["entities"] += 1
                    continue
                if cur:
                    m = tag.search(line)
                    if not m:
                        cur = None
                        continue
                    k, v = m.groups()
                    if k == "IS_BACON_POOL_MINION":
                        res[cur]["pool_minion"].add(int(v))
                    elif k in ("IS_BACON_POOL_SPELL", "3081"):
                        res[cur]["pool_spell"].add(int(v))
                    elif k == "CREATOR":
                        res[cur]["with_creator"] += 1
    return {k: {"entities": v["entities"], "pool_minion": sorted(v["pool_minion"]),
                "pool_spell": sorted(v["pool_spell"]), "with_creator": v["with_creator"]}
            for k, v in res.items()}


def loc(v):
    return v["m_locValues"][0] if isinstance(v, dict) else (v[0] if v else "")


def log_meta():
    """Per card id from every Power.log: entity count, CARDTYPE values and zones logged."""
    res = collections.defaultdict(lambda: {"entities": 0, "types": collections.Counter(),
                                           "zones": collections.Counter()})
    head = re.compile(r"GameState\.DebugPrintPower\(\) -\s+(?:FULL_ENTITY - Creating|SHOW_ENTITY - Updating)\b.*?CardID=(\S*)\s*$")
    tag = re.compile(r"GameState\.DebugPrintPower\(\) -\s{6,}tag=(\S+) value=(\S+)")
    for f in sorted(glob.glob(GAME + "/Logs/*/Power.log")):
        cur = None
        with open(f, errors="replace") as fh:
            for line in fh:
                if "GameState.DebugPrintPower() -" not in line:
                    continue
                m = head.search(line)
                if m:
                    cur = m.group(1) or None
                    if cur:
                        res[cur]["entities"] += 1
                    continue
                if cur:
                    m = tag.search(line)
                    if not m:
                        cur = None
                        continue
                    k, v = m.groups()
                    if k == "CARDTYPE":
                        res[cur]["types"][v] += 1
                    elif k == "ZONE":
                        res[cur]["zones"][v] += 1
    return res


def text_fields(raw, nums):
    """Same text rules as for cards: split at the real '@', fill {n}, plain versions."""
    parts = split_alt(raw)
    nt, un1 = fill(parts[0], nums)
    alt = parts[1] if len(parts) > 1 else None
    at, un2 = fill(alt, nums) if alt is not None else (None, [])
    return {"text_template": raw, "text": nt, "text_alt": at,
            "text_plain": plain(nt), "text_alt_plain": plain(at) if at is not None else None,
            "text_extra_parts": parts[2:],
            "unresolved_numbers": sorted(set(un1 + un2))}


def export_meta(build, tags, recs, refdir, no_log):
    cid = lambda d: recs[d]["m_noteMiniGuid"]
    by_cid = {cid(d): d for d in recs}
    log = None if no_log else log_meta()

    def seen(card_id):
        if log is None or card_id not in log:
            return None if log is None else 0
        return log[card_id]["entities"]

    def common(d):
        t = tags[d]
        nums = [t.get(k) for k in NUM_TAGS]
        o = {"card_id": cid(d), "dbf_id": d, "name": loc(recs[d]["m_name"]),
             "cost": t.get(T_COST)}
        o.update(text_fields(loc(recs[d]["m_textInHand"]), nums))
        o["script_data"] = {"num_1": nums[0], "num_2": nums[1], "num_3": nums[2], "num_4": nums[3]}
        o["keywords"] = sorted(KEYWORDS[k] for k in KEYWORDS if t.get(k) == 1)
        o["lobby_subsets"] = sorted(SUBSETS[k] for k in SUBSETS if t.get(k) == 1)
        o["duos_exclusive"] = t.get(T_DUOS_ONLY, 0) == 1
        return o

    def rawtags(d):
        return {str(k): v for k, v in sorted(tags[d].items())}

    # Heroes: CARDTYPE 3, a BG id or the draftable tag, not a cosmetic skin.
    is_skin = lambda d: T_SKIN in tags[d] or T_SKIN_PARENT in tags[d]
    hero_ids = [d for d in sorted(recs) if tags[d].get(T_CARDTYPE) == 3 and not is_skin(d)
                and (T_HERO_DRAFTABLE in tags[d] or ID_RE.match(cid(d)))]
    skins = collections.defaultdict(list)
    for d in recs:
        if tags[d].get(T_CARDTYPE) == 3 and T_SKIN_PARENT in tags[d]:
            skins[tags[d][T_SKIN_PARENT]].append(cid(d))
    hero_by_power = {}
    heroes = []
    for d in hero_ids:
        t = tags[d]
        duos = t.get(T_DUOS_ONLY, 0) == 1
        draft = t.get(T_HERO_DRAFTABLE, 0)
        hp = t.get(T_HERO_POWER)
        hp_card = recs[hp] if hp in recs else None
        if hp is not None:
            hero_by_power.setdefault(hp, d)
        heroes.append({
            "card_id": cid(d), "dbf_id": d, "name": loc(recs[d]["m_name"]),
            "health": t.get(T_HEALTH), "armor": t.get(T_ARMOR),
            "armor_tag_3206": t.get(T_HERO_ARMOR_2), "armor_tiers": None,
            "hero_power_dbf_id": hp,
            "hero_power_card_id": hp_card["m_noteMiniGuid"] if hp_card else None,
            "buddy_dbf_id": t.get(T_COMPANION),
            "buddy_card_id": cid(t[T_COMPANION]) if t.get(T_COMPANION) in recs else None,
            "lobby_subsets": sorted(SUBSETS[k] for k in SUBSETS if t.get(k) == 1),
            "pool_tag": "BACON_HERO_CAN_BE_DRAFTED (1491)", "pool_tag_value": t.get(T_HERO_DRAFTABLE),
            "duos_exclusive": duos, "in_solo_pool": draft == 1 and not duos,
            "skin_card_ids": sorted(skins.get(d, [])),
            "log_entities": seen(cid(d)) if log is not None else None,
            "log_entities_with_skins": (sum(seen(c) for c in [cid(d)] + skins.get(d, []))
                                        if log is not None else None),
            "tags": rawtags(d)})

    # Hero powers: CARDTYPE 10, a BG id or the power of a BG hero.
    hero_of = {h["dbf_id"]: h for h in heroes}
    hp_ids = [d for d in sorted(recs) if tags[d].get(T_CARDTYPE) == 10
              and (ID_RE.match(cid(d)) or d in hero_by_power)]
    powers = []
    for d in hp_ids:
        t = tags[d]
        owner = hero_of.get(t.get(T_HERO_BASE)) or hero_of.get(hero_by_power.get(d))
        o = common(d)
        o["hide_cost"] = t.get(T_HIDE_COST, 0) == 1
        # No passive tag exists in the data. Only the card text says it.
        o["passive_by_text"] = bool(re.search(r"passive", o["text_template"], re.I))
        o["is_base_power"] = d in hero_by_power
        o["hero_card_id"] = owner["card_id"] if owner else None
        o["hero_dbf_id"] = owner["dbf_id"] if owner else None
        o["hero_link"] = ("BACON_HEROPOWER_BASE_HERO_ID (2376)" if t.get(T_HERO_BASE) in hero_of
                          else "HERO_POWER (380) of the hero" if owner else None)
        o["hero_in_solo_pool"] = owner["in_solo_pool"] if owner else None
        o["log_entities"] = seen(cid(d)) if log is not None else None
        o["tags"] = rawtags(d)
        powers.append(o)

    # Trinkets: CARDTYPE 44.
    trinkets = []
    for d in sorted(recs):
        t = tags[d]
        if t.get(T_CARDTYPE) != 44:
            continue
        c = cid(d)
        o = common(d)
        placeholder = "_Trinket_" in c
        base = c[:-1] if c.endswith("t") else None
        o["slot"] = TRINKET_SCHOOLS.get(t.get(T_SPELL_SCHOOL), "SCHOOL_%s" % t.get(T_SPELL_SCHOOL))
        o["slot_tag"] = "SPELL_SCHOOL (1635): 11 Lesser, 12 Greater"
        o["is_placeholder"] = placeholder
        o["upgrade_of"] = base if base in by_cid and tags[by_cid[base]].get(T_SPELL_SCHOOL) == 11 else None
        o["pool_tag"] = "BACON_TRINKET (3407)"
        o["pool_tag_value"] = t.get(T_TRINKET)
        o["mentions_dark_gift"] = t.get(T_DARK_GIFT_MENTION, 0) == 1
        o["in_solo_pool"] = t.get(T_TRINKET) == 1 and not placeholder and not o["duos_exclusive"]
        o["log_entities"] = seen(c) if log is not None else None
        o["tags"] = rawtags(d)
        trinkets.append(o)

    # Dark Gifts: the SPELL card the gift is made from, id BG36_MidGameEffect_000t*.
    ench = collections.defaultdict(list)
    for d in recs:
        m = GIFT_ENCH_RE.match(cid(d))
        if m and tags[d].get(T_CARDTYPE) == 6:
            ench[m.group(1)].append(cid(d))
    ench_seen = collections.Counter()
    if log is not None:
        for c, v in log.items():
            m = GIFT_ENCH_RE.match(c)
            if m:
                ench_seen[m.group(1)] += v["entities"]
    gifts = []
    for d in sorted(recs):
        c = cid(d)
        t = tags[d]
        if t.get(T_CARDTYPE) != 5 or not GIFT_ID_RE.match(c):
            continue
        o = common(d)
        o["gift_tag"] = "IS_NIGHTMARE_BONUS (3567)"
        o["gift_tag_value"] = t.get(T_NIGHTMARE_BONUS)
        o["is_variant"] = c.endswith("t") and c[len("BG36_MidGameEffect_000"):] != "t"
        o["enchantment_card_ids"] = sorted(ench.get(c, []))
        o["in_solo_pool"] = t.get(T_NIGHTMARE_BONUS) == 1 and not o["duos_exclusive"]
        o["pool_tag"] = "IS_NIGHTMARE_BONUS (3567), no separate pool flag found"
        o["log_entities"] = seen(c) if log is not None else None
        o["log_enchantment_entities"] = ench_seen.get(c, 0) if log is not None else None
        o["tags"] = rawtags(d)
        gifts.append(o)
    placeholders = [{"card_id": cid(d), "name": loc(recs[d]["m_name"])} for d in sorted(recs)
                    if cid(d) == "BG36_MidGameEffect_010"]

    # Cards that mention or grant a Dark Gift (tag 3627), by type, for the doc.
    mention = collections.Counter(CARDTYPES.get(tags[d].get(T_CARDTYPE), str(tags[d].get(T_CARDTYPE)))
                                  for d in recs if tags[d].get(T_DARK_GIFT_MENTION) == 1)
    n = lambda xs, k="in_solo_pool": sum(1 for x in xs if x[k])
    counts = {
        "heroes": len(heroes), "heroes_in_solo_pool": n(heroes),
        "heroes_duos_exclusive": sum(1 for h in heroes if h["duos_exclusive"]),
        "hero_skins_not_exported": sum(len(v) for v in skins.values()),
        "hero_powers": len(powers), "hero_powers_of_solo_pool_heroes": sum(1 for p in powers if p["hero_in_solo_pool"]),
        "hero_powers_passive_by_text": sum(1 for p in powers if p["passive_by_text"]),
        "hero_powers_without_hero": sum(1 for p in powers if p["hero_card_id"] is None),
        "trinkets": len(trinkets), "trinkets_in_solo_pool": n(trinkets),
        "trinkets_lesser_in_solo_pool": sum(1 for x in trinkets if x["in_solo_pool"] and x["slot"] == "LESSER"),
        "trinkets_greater_in_solo_pool": sum(1 for x in trinkets if x["in_solo_pool"] and x["slot"] == "GREATER"),
        "dark_gifts": len(gifts), "dark_gifts_in_solo_pool": n(gifts),
        "cards_with_dark_gift_tag_3627": dict(mention),
    }
    out = {"game_build": build, "unity_version": UnityPy.config.FALLBACK_UNITY_VERSION,
           "source": "Data/OSX/dbf.unity3d", "power_log_used": log is not None, "counts": counts,
           "heroes": heroes, "hero_powers": powers, "trinkets": trinkets, "dark_gifts": gifts,
           "dark_gift_placeholders": placeholders}
    path = os.path.join(refdir, "bg-meta-%s.json" % build)
    with open(path, "w") as fh:
        json.dump(out, fh, indent=1, ensure_ascii=False)
    shutil.copyfile(path, os.path.join(refdir, "bg-meta.json"))
    print("wrote", path, counts)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--build", help="override the build number used in the file name")
    ap.add_argument("--no-log", action="store_true", help="skip Power.log evidence")
    args = ap.parse_args()

    build = args.build or client_build() or "unknown"
    tabs = load_tables()
    tags = collections.defaultdict(dict)
    for r in tabs["CARD_TAG"]:
        tags[r["m_cardId"]][r["m_tagId"]] = r["m_tagValue"]
    race_tag = {r["m_isRaceTagId"]: r["m_ID"] for r in tabs["CARD_RACE"] if r["m_isRaceTagId"]}
    recs = {r["m_ID"]: r for r in tabs["CARD"]}
    del tabs

    def in_scope(dbf, r):
        t = tags[dbf]
        if t.get(T_CARDTYPE) not in (4, 5, 42):
            return False
        return bool(ID_RE.match(r["m_noteMiniGuid"]) or any(
            k in t for k in (T_TECH_LEVEL, T_POOL_MINION, T_POOL_SPELL, T_GOLDEN, T_BASE)))

    def loc(v):
        return v["m_locValues"][0] if isinstance(v, dict) else (v[0] if v else "")

    def stat(t, k):
        return t.get(k)

    cards = []
    for dbf, r in sorted(recs.items()):
        if not in_scope(dbf, r):
            continue
        t = tags[dbf]
        nums = [t.get(k) for k in NUM_TAGS]
        raw = loc(r["m_textInHand"])
        parts = split_alt(raw)
        normal = parts[0]
        alt = parts[1] if len(parts) > 1 else None
        extra = parts[2:]
        nt, un1 = fill(normal, nums)
        at, un2 = fill(alt, nums) if alt is not None else (None, [])
        races = []
        if T_RACE in t and t[T_RACE]:
            races.append(t[T_RACE])
        for tid, rid in race_tag.items():
            if t.get(tid) == 1 and rid not in races:
                races.append(rid)
        gold_dbf, base_dbf = t.get(T_GOLDEN), t.get(T_BASE)
        gold = recs.get(gold_dbf)
        ctype = CARDTYPES.get(t.get(T_CARDTYPE), str(t.get(T_CARDTYPE)))
        pm, ps = t.get(T_POOL_MINION, 0), t.get(T_POOL_SPELL, 0)
        duos = t.get(T_DUOS_ONLY, 0) == 1
        solo_pool = (ctype == "MINION" and pm == 1 and not duos) or \
                    (ctype == "BATTLEGROUND_SPELL" and ps == 1 and not duos)
        card = {
            "dbf_id": dbf, "card_id": r["m_noteMiniGuid"], "name": loc(r["m_name"]),
            "type": ctype, "tier": t.get(T_TECH_LEVEL),
            "tribes": [RACES.get(x, "RACE_%d" % x) for x in races],
            "cost": stat(t, T_COST), "attack": stat(t, T_ATTACK), "health": stat(t, T_HEALTH),
            "is_golden": base_dbf is not None,
            "golden_dbf_id": gold_dbf,
            "golden_card_id": gold["m_noteMiniGuid"] if gold else None,
            "golden_attack": tags[gold_dbf].get(T_ATTACK) if gold else None,
            "golden_health": tags[gold_dbf].get(T_HEALTH) if gold else None,
            "normal_dbf_id": base_dbf,
            "normal_card_id": recs[base_dbf]["m_noteMiniGuid"] if base_dbf in recs else None,
            "keywords": sorted(KEYWORDS[k] for k in KEYWORDS if t.get(k) == 1),
            "lobby_subsets": sorted(SUBSETS[k] for k in SUBSETS if t.get(k) == 1),
            "text_template": raw,
            "text": nt, "text_alt": at,
            "text_plain": plain(nt), "text_alt_plain": plain(at) if at is not None else None,
            "text_extra_parts": extra,
            "script_data": {"num_1": nums[0], "num_2": nums[1], "num_3": nums[2], "num_4": nums[3]},
            "unresolved_numbers": sorted(set(un1 + un2)),
            "pool_minion_tag": t.get(T_POOL_MINION), "pool_spell_tag": t.get(T_POOL_SPELL),
            "duos_exclusive": duos,
            "in_solo_pool": solo_pool,
            "tags": {str(k): v for k, v in sorted(t.items())},
        }
        cards.append(card)

    out = {"game_build": build, "unity_version": UnityPy.config.FALLBACK_UNITY_VERSION,
           "source": "Data/OSX/dbf.unity3d", "counts": {}, "cards": cards}
    c = collections.Counter()
    for k in cards:
        c[(k["type"], "golden" if k["is_golden"] else "normal")] += 1
    out["counts"] = {"%s_%s" % k: v for k, v in sorted(c.items())}
    out["counts"]["in_solo_pool"] = sum(1 for k in cards if k["in_solo_pool"])
    out["counts"]["total"] = len(cards)

    refdir = os.path.join(ROOT, "ref")
    os.makedirs(refdir, exist_ok=True)
    path = os.path.join(refdir, "bg-cards-%s.json" % build)
    with open(path, "w") as fh:
        json.dump(out, fh, indent=1, ensure_ascii=False)
    shutil.copyfile(path, os.path.join(refdir, "bg-cards.json"))
    print("wrote", path, out["counts"])
    export_meta(build, tags, recs, refdir, args.no_log)

    # Comparison with HearthstoneJSON and with the live Power.log tags.
    dis = {"pool_flags": [], "texts_with_separator": [], "log": {}}
    hs_path = os.path.join(refdir, "hsjson-cards.json")
    if os.path.exists(hs_path):
        hs = {c["dbfId"]: c for c in json.load(open(hs_path))}
        for k in cards:
            h = hs.get(k["dbf_id"])
            if not h:
                continue
            ours = k["in_solo_pool"] if not k["duos_exclusive"] else False
            hflag = bool(h.get("isBattlegroundsPoolMinion") or h.get("isBattlegroundsPoolSpell"))
            if k["type"] in ("MINION", "BATTLEGROUND_SPELL") and ours != hflag:
                dis["pool_flags"].append({"card_id": k["card_id"], "name": k["name"], "type": k["type"],
                    "game_pool_minion_tag": k["pool_minion_tag"], "game_pool_spell_tag": k["pool_spell_tag"],
                    "duos_exclusive": k["duos_exclusive"], "hsjson_flag": hflag, "game_says_in_solo_pool": ours})
            if k["text_alt"] is not None:
                dis["texts_with_separator"].append({"card_id": k["card_id"], "name": k["name"],
                    "hsjson_text": h.get("text"), "normal": k["text_plain"], "alt": k["text_alt_plain"]})
    if not args.no_log:
        ev = power_log_evidence()
        byid = {k["card_id"]: k for k in cards}
        seen_pool = [i for i, v in ev.items() if i in byid and (1 in v["pool_minion"] or 1 in v["pool_spell"])]
        mism = []
        for i, v in ev.items():
            k = byid.get(i)
            if not k:
                continue
            lm = max(v["pool_minion"], default=0)
            ls = max(v["pool_spell"], default=0)
            if (lm, ls) != (k["pool_minion_tag"] or 0, k["pool_spell_tag"] or 0):
                mism.append({"card_id": i, "log": [lm, ls], "bundle": [k["pool_minion_tag"], k["pool_spell_tag"]]})
        dis["log"] = {"cards_seen_with_pool_tag": len(seen_pool), "mismatch_with_bundle": mism,
                      "evidence": {i: ev[i] for i in ("BG25_013", "EBG_Spell_014", "BG28_604") if i in ev}}
    with open(os.path.join(refdir, "bg-disagreements.json"), "w") as fh:
        json.dump(dis, fh, indent=1, ensure_ascii=False)
    print("pool flag disagreements:", len(dis["pool_flags"]), "texts with separator:", len(dis["texts_with_separator"]))


if __name__ == "__main__":
    main()
