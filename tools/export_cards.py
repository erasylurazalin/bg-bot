#!/usr/bin/env python3
"""Export every Battlegrounds card from the installed Hearthstone files.

Run with the venv that has UnityPy:

    .venv-unity/bin/python tools/export_cards.py

Reads (never writes) ~/hearthstone-linux/hearthstone/. Writes
ref/bg-cards-<build>.json, a copy at ref/bg-cards.json, and
ref/bg-disagreements.json (comparison with ref/hsjson-cards.json and with
the Power.log pool tags, only if those exist). No network.
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
