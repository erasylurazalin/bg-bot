#!/usr/bin/env python3
"""Compare GameState entity tags in Power.log files against cards.toml and trinkets.toml.

Reads only GameState.DebugPrintPower() FULL_ENTITY and SHOW_ENTITY blocks,
skips golden copies (via ref/hsjson-cards.json), computes the most common
TECH_LEVEL, ATK/HEALTH, and COST per card_id, and prints every difference
from cards.toml and trinkets.toml.

Usage:
    python3 -I train/evaltools/patch_check.py ~/bg-logs/Hearthstone_2026_11_18_* ...
"""

from __future__ import annotations

import argparse
import json
import re
import tomllib
from collections import Counter, defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

RE_ENTITY = re.compile(
    r"GameState\.DebugPrintPower\(\)\s*-\s*"
    r"(?:FULL_ENTITY - (?:Creating|Updating)|SHOW_ENTITY - Updating)\b"
    r".*?\bCardID=(\S*)"
)
RE_TAG = re.compile(
    r"GameState\.DebugPrintPower\(\)\s*-\s+tag=(\S+)\s+value=(\S+)"
)


def load_hsjson(path: Path) -> dict[str, dict]:
    data = json.loads(path.read_text())
    return {c["id"]: c for c in data if c.get("id")}


def load_cards_toml(path: Path) -> tuple[dict[str, dict], dict[str, dict], dict[str, dict]]:
    data = tomllib.loads(path.read_text())
    minions = {e["name"]: e for e in data.get("minion", [])}
    spells = {e["name"]: e for e in data.get("spell", [])}
    tokens = {e["name"]: e for e in data.get("token", [])}
    return minions, spells, tokens


def load_trinkets_toml(path: Path) -> dict[str, dict]:
    """Parse trinkets.toml with regex (multiline inline tables exceed TOML 1.0)."""
    text = path.read_text()
    blocks = re.split(r"(?m)^\[\[trinket\]\]\s*$", text)[1:]
    by_cid: dict[str, dict] = {}
    for block in blocks:
        name_m = re.search(r'(?m)^name = "([^"]+)"', block)
        cid_m = re.search(r'(?m)^card_id = "([^"]+)"', block)
        slot_m = re.search(r'(?m)^slot = "([^"]+)"', block)
        cost_m = re.search(r"(?m)^cost = (\d+)", block)
        if not (name_m and cid_m):
            continue
        by_cid[cid_m.group(1)] = {
            "name": name_m.group(1),
            "card_id": cid_m.group(1),
            "slot": slot_m.group(1) if slot_m else "",
            "cost": int(cost_m.group(1)) if cost_m else 0,
        }
    return by_cid


def is_golden(card_id: str, hs_by_id: dict[str, dict]) -> bool:
    card = hs_by_id.get(card_id)
    if card and "battlegroundsNormalDbfId" in card:
        return True
    return card_id.endswith("_G")


def resolve_log_paths(inputs: list[str]) -> list[Path]:
    paths: list[Path] = []
    for raw in inputs:
        p = Path(raw).expanduser()
        if p.is_dir():
            cand = p / "Power.log"
            if cand.is_file():
                paths.append(cand)
        elif p.is_file():
            paths.append(p)
    return sorted(set(paths))


def scan_logs(
    log_paths: list[Path], hs_by_id: dict[str, dict]
) -> dict[str, dict]:
    records: dict[str, dict] = defaultdict(
        lambda: {
            "count": 0,
            "cardtype": Counter(),
            "tech_level": Counter(),
            "stats": Counter(),
            "cost": Counter(),
        }
    )

    for log_path in log_paths:
        cur_cid: str | None = None
        cur_tags: dict[str, str] = {}

        def flush() -> None:
            if not cur_cid or is_golden(cur_cid, hs_by_id):
                return
            rec = records[cur_cid]
            rec["count"] += 1
            if "CARDTYPE" in cur_tags:
                rec["cardtype"][cur_tags["CARDTYPE"]] += 1
            if "TECH_LEVEL" in cur_tags:
                rec["tech_level"][int(cur_tags["TECH_LEVEL"])] += 1
            if "HEALTH" in cur_tags:
                atk = int(cur_tags.get("ATK", 0))
                rec["stats"][(atk, int(cur_tags["HEALTH"]))] += 1
            cost = int(cur_tags["COST"]) if "COST" in cur_tags else 0
            rec["cost"][cost] += 1

        with log_path.open(errors="replace") as f:
            for line in f:
                if "GameState.DebugPrintPower()" not in line:
                    continue
                m = RE_ENTITY.search(line)
                if m:
                    flush()
                    cur_cid = m.group(1) or None
                    cur_tags = {}
                    continue
                if cur_cid is not None:
                    mt = RE_TAG.search(line)
                    if mt:
                        cur_tags[mt.group(1)] = mt.group(2)
                    else:
                        flush()
                        cur_cid = None
                        cur_tags = {}
            flush()

    return records


def mode_of(counter: Counter):
    if not counter:
        return None
    return counter.most_common(1)[0][0]


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "logs",
        nargs="+",
        help="Power.log files or Hearthstone log directories containing Power.log",
    )
    parser.add_argument(
        "--all",
        action="store_true",
        help="Print all observed card_ids with their most common tags",
    )
    parser.add_argument(
        "--trinkets",
        action="store_true",
        help="Print status of every trinket in trinkets.toml (seen cost vs never seen)",
    )
    args = parser.parse_args()

    log_paths = resolve_log_paths(args.logs)
    if not log_paths:
        raise SystemExit("No Power.log files found for the given paths.")

    hs_by_id = load_hsjson(ROOT / "ref/hsjson-cards.json")
    minions, spells, tokens = load_cards_toml(ROOT / "cards.toml")
    trinkets = load_trinkets_toml(ROOT / "trinkets.toml")

    records = scan_logs(log_paths, hs_by_id)
    print(f"Scanned {len(log_paths)} log(s), {len(records)} distinct non-golden card_ids.")

    if args.all:
        print("\n=== Per card_id modes ===")
        for cid in sorted(records):
            rec = records[cid]
            name = hs_by_id.get(cid, {}).get("name", "?")
            ct = mode_of(rec["cardtype"]) or "?"
            tl = mode_of(rec["tech_level"])
            st = mode_of(rec["stats"])
            co = mode_of(rec["cost"])
            st_s = f"{st[0]}/{st[1]}" if st else "-"
            tl_s = str(tl) if tl is not None else "-"
            print(
                f"{cid:24s} {ct:20s} n={rec['count']:4d} "
                f"tier={tl_s:2s} stats={st_s:7s} cost={co}  {name}"
            )

    diffs: list[tuple[str, str, str, int, str]] = []
    for cid, rec in sorted(records.items()):
        ct = mode_of(rec["cardtype"])
        name = hs_by_id.get(cid, {}).get("name", "")
        tl = mode_of(rec["tech_level"])
        st = mode_of(rec["stats"])
        co = mode_of(rec["cost"])
        n = rec["count"]

        if cid in trinkets:
            t = trinkets[cid]
            if co != t["cost"]:
                diffs.append((
                    f"trinket ({t['slot']})",
                    cid,
                    t["name"],
                    n,
                    f"cost: toml={t['cost']} log={co} (seen={dict(rec['cost'])})",
                ))
            continue

        if ct == "MINION" and name in minions:
            m = minions[name]
            exp_tier = m.get("tier")
            exp_stats = tuple(m["stats"]) if "stats" in m else None
            parts = []
            if tl != exp_tier:
                parts.append(f"tier: toml={exp_tier} log={tl}")
            if exp_stats is not None and st != exp_stats:
                parts.append(f"stats: toml={exp_stats} log={st}")
            if parts:
                diffs.append(("minion", cid, name, n, "; ".join(parts)))
        elif ct == "MINION" and name in tokens:
            tk = tokens[name]
            exp_stats = tuple(tk["stats"]) if "stats" in tk else None
            parts = []
            if "tier" in tk and tl != tk["tier"]:
                parts.append(f"tier: toml={tk['tier']} log={tl}")
            if exp_stats is not None and st != exp_stats:
                parts.append(f"stats: toml={exp_stats} log={st}")
            if parts:
                diffs.append(("token", cid, name, n, "; ".join(parts)))
        elif ct == "BATTLEGROUND_SPELL" and name in spells:
            sp = spells[name]
            exp_tier = sp.get("tier")
            exp_cost = sp.get("cost", 0)
            parts = []
            if tl != exp_tier:
                parts.append(f"tier: toml={exp_tier} log={tl}")
            if co != exp_cost:
                parts.append(f"cost: toml={exp_cost} log={co}")
            if parts:
                diffs.append(("spell", cid, name, n, "; ".join(parts)))

    print(f"\n=== Differences from cards.toml and trinkets.toml ({len(diffs)}) ===")
    for kind, cid, name, n, detail in diffs:
        print(f"  [{kind}] {name} ({cid}, n={n}): {detail}")

    if args.trinkets:
        print("\n=== Trinkets in trinkets.toml ===")
        for cid, t in sorted(trinkets.items(), key=lambda kv: (kv[1]["slot"], kv[1]["name"])):
            rec = records.get(cid)
            if not rec:
                print(f"  [{t['slot']:7s}] {t['name']:32s} {cid:22s} toml={t['cost']}  NEVER SEEN")
            else:
                co = mode_of(rec["cost"])
                flag = " DIFF" if co != t["cost"] else ""
                print(
                    f"  [{t['slot']:7s}] {t['name']:32s} {cid:22s} toml={t['cost']} "
                    f"log={co} (n={rec['count']}, seen={dict(rec['cost'])}){flag}"
                )


if __name__ == "__main__":
    main()
