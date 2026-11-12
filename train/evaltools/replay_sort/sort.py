#!/usr/bin/env python3
"""Sort every stopped whole-turn replay by the first step that diverged.

The input is train/runs/engine-pays/turns.json (seed 1). The first diverging
step is not in that file, so this script runs a logging build of turn_replay
that replays every prefix of a stopped turn and compares the engine's state
after each step with the state the log records after it. The logging build is
target/replay-sort-bin/turn_replay_trace; it was built from a scratch copy of
the tree under target/replay-sort-src/repo (only the trace output was added).

Usage, from the repo root:

    python3 train/evaltools/replay_sort/sort.py
    python3 train/evaltools/replay_sort/sort.py --run-trace

--run-trace rebuilds the trace from boards/actions instead of reusing
train/runs/replay-sort/trace.json. The action files are pinned to the ones the
engine-pays run used, so the 490 turns stay the same even after Era adds more
games to boards/actions.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import tomllib
from collections import Counter, defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
TRACE_BIN = ROOT / "target/replay-sort-bin/turn_replay_trace"


def blocks(text: str) -> list[str]:
    return re.split(r"(?m)^\[\[[^\]]+\]\]\s*$", text)[1:]


def name_card_id(block: str):
    n = re.search(r'(?m)^name = "([^"]+)"', block)
    c = re.search(r'(?m)^card_id = "([^"]+)"', block)
    return (n.group(1), c.group(1) if c else None) if n else None


def load_engine_data(root: Path):
    cards = tomllib.loads((root / "cards.toml").read_text())
    card_kind = {}
    for section, kind in (("minion", "minion"), ("token", "token"), ("spell", "spell")):
        for entry in cards.get(section, []):
            card_kind[entry["name"]] = kind

    trinkets = {}
    for block in blocks((root / "trinkets.toml").read_text()):
        found = name_card_id(block)
        if found:
            trinkets[found[0]] = found[1]

    powers = {}
    heroes_text = (root / "heroes.toml").read_text()
    for block in blocks(heroes_text):
        found = name_card_id(block)
        if found:
            powers[found[0]] = found[1]
    for m in re.finditer(
        r'power\s*=\s*\{\s*name\s*=\s*"([^"]+)",\s*card_id\s*=\s*"([^"]+)"',
        heroes_text,
    ):
        powers.setdefault(m.group(1), m.group(2))

    gifts = {}
    for block in blocks((root / "darkgifts.toml").read_text()):
        found = name_card_id(block)
        if found:
            gifts[found[0]] = found[1]

    hs = json.loads((root / "ref/hsjson-cards.json").read_text())
    by_name = defaultdict(list)
    for card in hs:
        if card.get("name"):
            by_name[card["name"]].append(card)

    def kind(name):
        if name in trinkets:
            return "trinket"
        if name in powers:
            return "hero power"
        if name in gifts:
            return "Dark Gift"
        if name in card_kind:
            return card_kind[name]
        typed = [c.get("type") for c in by_name.get(name, []) if c.get("type")]
        for t in typed:
            if t == "BATTLEGROUND_TRINKET":
                return "trinket"
            if t == "HERO_POWER":
                return "hero power"
            if t == "MINION":
                return "minion"
            if t in ("BATTLEGROUND_SPELL", "SPELL"):
                return "spell"
        return "unknown"

    def text(name):
        found = by_name.get(name, [])
        texts = []
        for card in found:
            body = card.get("text")
            if body:
                body = re.sub(r"<[^>]+>", "", body)
                body = body.replace("[x]", "").replace("&nbsp;", " ")
                body = " ".join(body.split())
                if body not in texts:
                    texts.append(body)
        return " / ".join(texts)

    return {"trinkets": trinkets, "powers": powers, "gifts": gifts,
            "card_kind": card_kind, "kind": kind, "text": text}


def parse_names(reason: str | None) -> list[str]:
    if not reason:
        return []
    out = []
    pattern = (
        r"(?:an option the card set does not build|"
        r"a trinket the trinket table does not build|"
        r"trinket not built|hero power not built|minion not built|card not built): "
        r"([^;()]+)"
    )
    for m in re.finditer(pattern, reason):
        out.append(m.group(1).strip())
    return out


ACTION_CACHE = {}


def load_action_file(path: Path):
    if path not in ACTION_CACHE:
        ACTION_CACHE[path] = json.loads(path.read_text())
    return ACTION_CACHE[path]


def turn_spec(actions: Path, trace):
    spec = load_action_file(actions / trace["file"])
    return next(t for t in spec["turns"] if t["turn"] == trace["turn"])


def hero_power_name(actions: Path, trace):
    turn = turn_spec(actions, trace)
    step = turn["actions"][trace["first_step"]]
    which = step.get("which", "main")
    powers = turn["start"].get("hero_powers", [])
    for power in powers:
        if bool(power.get("second")) == (which == "second"):
            return power.get("name", "")
    return powers[0].get("name", "") if powers else ""


def diff_text(trace) -> str:
    if trace["diffs"]:
        return trace["diffs"][0]
    if trace["log_missing"]:
        return trace["log_missing"][0]
    if trace["engine_missing"]:
        return trace["engine_missing"][0]
    return ""


def pick_a_name(trace, engine) -> str | None:
    names = parse_names(trace["stop_reason"])
    log_names = []
    for item in trace["log_missing"]:
        log_names += parse_names(item)
    engine_names = []
    for item in trace["engine_missing"]:
        engine_names += parse_names(item)
    all_names = list(dict.fromkeys(names + log_names + engine_names))
    if not all_names:
        return None
    joined = " ".join(trace["diffs"])
    for name in all_names:
        if name in joined:
            return name
    if "trinkets" in joined:
        for name in all_names:
            if engine["kind"](name) == "trinket":
                return name
    if "hero power" in joined:
        for name in all_names:
            if engine["kind"](name) == "hero power":
                return name
    for name in names:
        if name in all_names:
            return name
    return all_names[0]


# First steps whose engine effect draws at random. The value names the draw,
# for the report's group (c) table.
RANDOM_STEP = {
    "Friendly Bounty": "random minion of your most common type",
    "Abyssal Envoy": "random Tavern spell",
    "Draconic Warden": "random Chromadrake",
    "Cloning Conch": "random Murloc and its copy",
    "Mindbending Recruiter": "random Aberration",
    "Auto Accelerator": "random Magnetic Volumizer",
    "Shipwrecked Rascal": "random Bounty",
    "Snare Trapper": "random Quilboar",
    "Wandering Willbreaker": "random Tavern spells",
    "Jailer Sticker": "random Undead",
    "Robust Evolution": "random higher-Tier transform",
    "Chef's Choice": "random minion of the same type",
    "Fearless Foodie": "random Blood Gem targets",
    "Hired Mount": "random Chromadrake",
    "Chromatic Tear": "random Chromadrake",
    "Rockin' Music Box": "random Battlecry minion",
    "Privateer Portrait": "random Bounty",
    "Incubate": "random Aberration",
    "Dark Ritual": "random minions",
    "Arcane Alteration": "random same-Tier card",
}

# First steps whose engine effect is not a random draw, and the first state
# difference the trace found. The value is the one-line difference.
DETERMINISTIC_STEP = {
    "Fortify": "the log buys it for 0 Gold; the engine refuses BuySpell",
    "Boundless Potential": "the stat changes after the play land on different minions",
    "Blue Volumizer": "the log's numbers (0,6) give Spark Snapper +19 Health; the engine gives +16",
    "Locked-up Mutineer": "the hand's Blue Volumizers are 1/3 lower and one is not golden after the play",
    "Gearfin": "the play leaves Gearfin 19 Health short (43 vs 62)",
    "Misplaced Tea Set": "each buffed minion is 1 Attack and 1 Health short",
    "Decoy Conjurer": "the Activate steals a different Tavern minion (Geomagus Roogug vs the log's card)",
    "Lurking Lionfish": "the Activate's Fishbait leaves far smaller board stat gains (Tasty Lobster 2897/1662 vs 2905/1666)",
    "Green Chromadrake": "the Chromatic Tear copies do not arrive",
    "Blood Gem Barrage": "the log's numbers (7,9) give Felboar +11/+15; the engine leaves different stats and Tavern",
    "Might of Stormwind": "the log's numbers give +10/+7 to four minions; the engine gives +1/+2",
    "Healthy Bounty": "the log's numbers give +6/+12 or +3/+6; the engine gives less",
}

# Game and turn specific overrides where the first step names one card but the
# difference belongs to another (a trinket or a board trigger).
SPECIFIC = {
    ("Hearthstone_2026_11_03_00_29_06-g1.json", 8): (
        "b", "Stuffed Coin Purse",
        "Tier 6 gives 14 Gold in the engine, the log shows 12 (Gold 16 vs 14)",
    ),
    ("Hearthstone_2026_11_02_00_10_22-g5.json", 7): (
        "b", "Clever Castaway (Toughened Shield)",
        "two Toughened Shield gifts give +6 Health per card in the log, +4 in the engine",
    ),
    ("Hearthstone_2026_11_02_00_10_22-g5.json", 8): (
        "b", "Clever Castaway (Toughened Shield)",
        "two Toughened Shield gifts give +6 Health per card in the log, +4 in the engine",
    ),
    ("Hearthstone_2026_11_02_00_10_22-g5.json", 10): (
        "b", "Clever Castaway (Toughened Shield)",
        "two Toughened Shield gifts give +6 Health per card in the log, +4 in the engine",
    ),
    ("Hearthstone_2026_11_02_00_10_22-g5.json", 11): (
        "b", "Clever Castaway (Toughened Shield)",
        "two Toughened Shield gifts give +6 Health per card in the log, +4 in the engine",
    ),
    ("Hearthstone_2026_11_02_00_10_22-g5.json", 12): (
        "b", "Clever Castaway (Toughened Shield)",
        "two Toughened Shield gifts give +6 Health per card in the log, +4 in the engine",
    ),
    ("Hearthstone_2026_11_09_03_05_42-g1.json", 12): (
        "b", "Chromatic Tear",
        "the 7th Battlecry repeat does not fire; the log gets two Blue Chromadrakes",
    ),
    ("Hearthstone_2026_10_26_00_48_15-g4.json", 10): (
        "c", "Lockbox (random Golden minion and Bounty)",
        "a Lockbox opened on a different step, so the hand holds different random cards",
    ),
    ("Hearthstone_2026_11_05_21_38_00-g1.json", 10): (
        "c", "Lockbox (random Golden minion and Bounty)",
        "a Lockbox opened on the level step; the log's random Golden minion and Bounty differ",
    ),
    ("Hearthstone_2026_11_02_00_10_22-g1.json", 10): (
        "c", "Lurking Lionfish (Fishbait)",
        "the Fishbait's attack leaves different random Beasts in the hand",
    ),
    ("Hearthstone_2026_10_24_15_25_26-g5.json", 8): (
        "b", "Lurking Lionfish (Toughened Shield)",
        "the log gives Lurking Lionfish +3 Health from Toughened Shield, the engine +2",
    ),
}


def classify(trace, engine, actions):
    key = (trace["file"], trace["turn"])
    if key in SPECIFIC:
        return SPECIFIC[key]

    names = parse_names(trace["stop_reason"])
    log_names = []
    for item in trace["log_missing"]:
        log_names += parse_names(item)
    engine_names = []
    for item in trace["engine_missing"]:
        engine_names += parse_names(item)
    has_missing = bool(names or log_names or engine_names)

    def built(name):
        return (
            name in engine["trinkets"]
            or name in engine["powers"]
            or name in engine["gifts"]
            or name in engine["card_kind"]
        )

    def a_or_loader(name):
        # A name the engine builds as a trinket or hero power, offered as a
        # Discover option, is a replay loader miss, not missing content:
        # force_offer resolves every option as a card. A name the engine does
        # not build is group (a).
        if not built(name):
            return ("a", name, "")
        kind = engine["kind"](name)
        return (
            "c",
            kind + " option: " + name,
            name + " is built, but the replay's force_offer resolves the option as a card",
        )

    # A missing source (Alternate Timeline) is the thing to build, not each
    # Timewarped option it offers.
    source = trace["step_source"]
    if (
        trace["step_kind"] == "discover"
        and source
        and source in (names + log_names + engine_names)
        and not built(source)
    ):
        return ("a", source, "the Discover source itself is not built: " + source)

    # (a) The turn's own start, or the step itself, could not be built.
    if trace["at_start"] and has_missing:
        name = pick_a_name(trace, engine)
        if name:
            group, label, why = a_or_loader(name)
            if group == "a":
                return (group, label, "the turn's start cannot be seated: " + trace["stop_reason"])
            return (group, label, why)
    if (
        trace["stop_class"] in ("start", "seat", "missing")
        and trace["first_step"] == trace["stop_step"]
        and has_missing
    ):
        name = pick_a_name(trace, engine)
        if name:
            group, label, why = a_or_loader(name)
            if group == "a":
                return (group, label, trace["stop_reason"])
            return (group, label, why)
    if trace["log_missing"]:
        name = pick_a_name(trace, engine)
        if name:
            group, label, why = a_or_loader(name)
            if group == "a":
                return (group, label, trace["log_missing"][0])
            return (group, label, why)

    kind = trace["step_kind"]
    first_name = trace["step_card"] or trace["step_source"] or ""
    if kind == "hero_power" and not first_name:
        first_name = hero_power_name(actions, trace)
    first_diff = diff_text(trace)

    # (c) The engine draws its own random shop, offer or card.
    if kind == "roll":
        return ("c", "shop refresh (Roll)",
                "the engine's Refreshed Tavern differs from the log's")
    if kind == "discover":
        source = trace["step_source"] or "a Discover"
        if source == "Battlegrounds Dark Gift [DNT]":
            source = "Dark Gift offer"
        else:
            source = "Discover from " + source
        if not trace["diffs"]:
            return ("c", source,
                    "the engine is not holding the log's second offer (multi-pick Discover)")
        return ("c", source,
                "the engine's own offer or pick differs from the log's")
    if kind == "trinket_pick" and trace["diffs"]:
        return ("c", "trinket offer",
                "the engine's trinket offer differs from the log's")
    if first_name in RANDOM_STEP:
        return ("c", first_name, RANDOM_STEP[first_name] + " differs from the log")
    if first_name in DETERMINISTIC_STEP:
        return ("b", first_name, DETERMINISTIC_STEP[first_name])
    if kind == "level" and first_diff.startswith(("hand a card", "hand count")):
        if trace["file"] == "Hearthstone_2026_10_24_04_08_00-g1.json" and trace["turn"] == 8:
            return ("b", "Lockbox",
                    "the log opens a Lockbox on the level step; the engine's hand still has it")
        return ("c", "Lockbox (random Golden minion and Bounty)",
                "the level step opens a Lockbox; the log's random cards differ")
    if kind in ("play", "buy", "sell", "activate", "hero_power", "level", "dark_discovery", "move"):
        if first_diff.startswith(("board stats", "hand stats", "hand golden", "hand gifts")):
            return ("b", first_name or "the step", first_diff)
        if first_diff.startswith(("Gold ", "Health ", "armor ", "tier ", "upgrade cost", "free Refreshes")):
            return ("b", first_name or "the step", first_diff)
        if first_diff.startswith(("hand a card", "board a card", "hand count", "board count", "Tavern a card")):
            # A named hand or board difference after a deterministic step is a
            # different card or trigger. Keep it in (b) unless the card is a
            # known random source above.
            return ("b", first_name or "the step", first_diff)
    return ("unclear", first_name or "unknown", first_diff or "no state difference was found")


def turn_ref(row):
    source = row["source"].replace("Hearthstone_", "")
    return f"{source}-g{row['game']} t{row['turn']}"


def blocked_turns(rows):
    out = defaultdict(list)
    for row in rows:
        out[(row["group"], row["name"])].append(turn_ref(row))
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--turns", default=str(ROOT / "train/runs/engine-pays/turns.json"))
    ap.add_argument("--actions", default=str(ROOT / "boards/actions"))
    ap.add_argument("--out", default=str(ROOT / "train/runs/replay-sort"))
    ap.add_argument("--trace", default=str(ROOT / "train/runs/replay-sort/trace.json"))
    ap.add_argument("--run-trace", action="store_true")
    args = ap.parse_args()

    turns_path = Path(args.turns)
    actions = Path(args.actions)
    out = Path(args.out)
    trace_path = Path(args.trace)
    out.mkdir(parents=True, exist_ok=True)

    all_rows = json.loads(turns_path.read_text())
    stopped = [r for r in all_rows if r["seed"] == 1 and not r["complete"]]
    files = sorted({r["file"] for r in stopped})

    if args.run_trace or not trace_path.exists():
        if not TRACE_BIN.exists():
            sys.exit(
                "the traced replay is missing: "
                + str(TRACE_BIN)
                + " (build it from target/replay-sort-src/repo)"
            )
        pinned = ROOT / "target/replay-sort-src/actions-replay-sort"
        pinned.mkdir(parents=True, exist_ok=True)
        for old in pinned.glob("*.json"):
            old.unlink()
        for name in files:
            os.symlink((actions / name).resolve(), pinned / name)
        raw = ROOT / "target/replay-sort-src/trace-replay-sort"
        subprocess.run(
            [
                str(TRACE_BIN),
                "--actions", str(pinned),
                "--out", str(raw),
                "--seeds", "1",
                "--turns", "7,12",
                "--trace",
            ],
            cwd=ROOT,
            check=True,
        )
        trace_path.write_text((raw / "trace.json").read_text())

    traces = {(t["file"], t["turn"]): t for t in json.loads(trace_path.read_text())}
    engine = load_engine_data(ROOT)

    rows = []
    for row in stopped:
        trace = traces[(row["file"], row["turn"])]
        group, name, reason = classify(trace, engine, actions)
        entry = {
            "file": row["file"],
            "source": row["source"],
            "game": row["game"],
            "turn": row["turn"],
            "stop_step": row["stop"]["step"],
            "stop_step_number": row["stop"]["step_number"],
            "stop_kind": row["stop"]["kind"],
            "stop_class": row["stop"]["class"],
            "stop_reason": row["stop"]["reason"],
            "first_diverging_step": trace["first_step"],
            "first_diverging_step_number": trace["first_step_number"],
            "at_start": trace["at_start"],
            "first_kind": trace["step_kind"],
            "first_card": trace["step_card"],
            "first_source": trace["step_source"],
            "group": group,
            "name": name,
            "reason": reason,
            "diffs": trace["diffs"],
            "log_missing": trace["log_missing"],
            "engine_missing": trace["engine_missing"],
        }
        if group == "a":
            entry["kind"] = engine["kind"](name)
            entry["text"] = engine["text"](name)
        rows.append(entry)

    rows.sort(key=lambda r: (r["file"], r["turn"]))
    (out / "turns.json").write_text(json.dumps(rows, indent=1) + "\n")

    counts = Counter(r["group"] for r in rows)
    by_name = blocked_turns(rows)
    lines = []
    lines.append("# Every stopped replay turn, sorted by its first cause")
    lines.append("")
    lines.append("Replay: `bgbench turn_replay`, seed 1, turns 7 to 12. "
                 "Input `train/runs/engine-pays/turns.json`; the first "
                 "diverging step comes from the traced prefix replay. "
                 "Script: `train/evaltools/replay_sort/sort.py`. In `turns.json`, "
                 "`stop_step` and `first_diverging_step` are 0 based as the replay "
                 "reports them; the `*_step_number` fields are the same steps 1 based.")
    lines.append("")
    lines.append("## Counts")
    lines.append("")
    lines.append("| group | turns | meaning |")
    lines.append("|---|---|---|")
    lines.append(f"| (a) not in the engine | {counts.get('a', 0)} | a card, trinket, Dark Gift, hero power or option the engine does not build |")
    lines.append(f"| (b) in the engine, different effect | {counts.get('b', 0)} | the engine has it and its state differs from the log |")
    lines.append(f"| (c) only the replay cannot follow | {counts.get('c', 0)} | the engine has it; the replay draws its own shop, offer or card, or cannot offer a built option |")
    lines.append(f"| unclear | {counts.get('unclear', 0)} | what was seen is written out below |")
    lines.append(f"| **all** | {len(rows)} | |")
    lines.append("")
    walked = sum(1 for r in rows if r["first_diverging_step"] != r["stop_step"])
    lines.append(f"In {walked} of {len(rows)} stopped turns the first diverging step is earlier than the step the replay stopped at, so the stop reason alone would have named the wrong cause. The first diverging step is found by replaying every prefix of the turn and comparing the engine state after each step with the state the log records after it.")
    lines.append("")
    lines.append("A Refreshed Tavern is put back from the log before the next step, so a roll-first difference is often transient in the Tavern itself. It still counts as the first cause because the engine's own roll has already advanced its RNG and pool, and later generated cards follow from that. `Hearthstone_2026_10_28_22_11_50-g2` turn 10 is the known example: its first diverging step is the turn's first roll, while the stop is the Sprightly Scarab play at step 41.")
    lines.append("")

    def table(group, title, note=None, with_text=False, with_reason=False, intro=None):
        items = defaultdict(list)
        for row in rows:
            if row["group"] == group:
                items[row["name"]].append(row)
        lines.append("## " + title)
        lines.append("")
        if intro:
            lines.append(intro)
            lines.append("")
        if note:
            lines.append(note)
            lines.append("")
        if not items:
            lines.append("None.")
            lines.append("")
            return
        if with_text:
            lines.append("| name | kind | card text | turns | turns blocked |")
            lines.append("|---|---|---|---|---|")
            for name, group_rows in sorted(items.items(), key=lambda kv: (-len(kv[1]), kv[0])):
                text = group_rows[0].get("text", "").replace("|", "/")
                if len(text) > 220:
                    text = text[:217].rstrip() + "..."
                turns = ", ".join(turn_ref(r) for r in group_rows)
                lines.append(f"| {name} | {group_rows[0].get('kind', '')} | {text} | {len(group_rows)} | {turns} |")
        elif with_reason:
            lines.append("| name | what differs | turns | turns blocked |")
            lines.append("|---|---|---|---|")
            for name, group_rows in sorted(items.items(), key=lambda kv: (-len(kv[1]), kv[0])):
                reason = group_rows[0].get("reason", "").replace("|", "/")
                if len(reason) > 240:
                    reason = reason[:237].rstrip() + "..."
                turns = ", ".join(turn_ref(r) for r in group_rows)
                lines.append(f"| {name} | {reason} | {len(group_rows)} | {turns} |")
        else:
            lines.append("| name | turns | turns blocked |")
            lines.append("|---|---|---|")
            for name, group_rows in sorted(items.items(), key=lambda kv: (-len(kv[1]), kv[0])):
                turns = ", ".join(turn_ref(r) for r in group_rows)
                lines.append(f"| {name} | {len(group_rows)} | {turns} |")
        lines.append("")

    table("a", "(a) Not in the engine at all", "Name, kind and card text. The name is the first unbuilt thing the state needed.", True)
    table("b", "(b) In the engine but behaving differently", "The one-line difference for each name; the per-turn state is in `turns.json`.", with_reason=True)
    loader = [r for r in rows if r["group"] == "c" and r["name"].startswith(("trinket option:", "hero power option:"))]
    random_c = [r for r in rows if r["group"] == "c" and r not in loader]
    raw_option_stops = sum(
        1 for r in rows if "an option the card set does not build" in (r["stop_reason"] or "")
    )
    c_intro = ("(c) has two kinds. " + str(len(random_c)) + " turns first diverge at a draw the engine makes itself (a Refreshed Tavern, a Discover or a generated card). "
               + str(len(loader)) + " turns first diverge at an option the engine does build but the replay cannot offer: `force_offer` resolves every option as a card, so a trinket or hero power option fails before the trinket branch. "
               + "Those " + str(len(loader)) + " turns across " + str(len({r['name'] for r in loader})) + " names are a replay-tool problem, not missing content. "
               + "The raw stop reason says an option the card set does not build on " + str(raw_option_stops) + " turns; " + str(len(loader)) + " of those first diverge at the option, and the other " + str(raw_option_stops - len(loader)) + " first diverged earlier.")
    table("c", "(c) In the engine, only the replay cannot follow", intro=c_intro, with_reason=True)
    table("unclear", "Unclear", "These were looked at and left alone rather than guessed.", with_reason=True)

    lines.append("## Top 10 names by turns blocked")
    lines.append("")
    lines.append("| name | group | turns |")
    lines.append("|---|---|---|")
    grouped = defaultdict(list)
    for (group, name), turns in by_name.items():
        label = name
        if name.startswith(("trinket option:", "hero power option:")):
            label = "loader: built option the replay cannot offer"
        grouped[(group, label)] += turns
    top = sorted(grouped.items(), key=lambda kv: (-len(kv[1]), kv[0]))[:10]
    for (group, name), turns in top:
        lines.append(f"| {name} | {group} | {len(turns)} |")
    lines.append("")
    lines.append("The loader row pools the 65 `trinket option:` and `hero power option:` names; the (c) table lists them one by one.")
    lines.append("")
    lines.append("## Replay tool problems seen")
    lines.append("")
    lines.append(
        "1. `force_offer` resolves every offer option with `resolve_card`, which "
        "only knows `cards.toml`. A trinket option (Trinket pick, Mystery Cube, "
        "Trip Vouchers, Eastern Plaguelands, Growing Collection) or a hero power "
        "option (Conviction, Shift your Hero Power) therefore fails as 'an option "
        "the card set does not build' even though `trinkets.toml` or `heroes.toml` "
        "builds it. This is the largest single cause in the run: " + str(len(loader))
        + " turns. The trinket branch of `force_offer` is never reached because the "
        "card resolution above it fails first."
    )
    lines.append("")
    lines.append(
        "2. A multi-pick effect (`Book of Medivh` with two Discovers, `Token of the "
        "Old Gods`, `Kel'Thuzad's Kitty`, `Triple Reward` after a play) leaves the "
        "engine's own Discover offer open. `force_offer` adds the log's options "
        "without clearing that offer, so the `Choose` index can pick an engine "
        "option or fall outside the log's offer. That shows up as the group (c) "
        "Discover rows and as some illegal `Choose` stops. This is replay plumbing, "
        "not a game rule: a traced prefix replay shows it."
    )
    lines.append("")
    (out / "report.md").write_text("\n".join(lines) + "\n")
    print(f"wrote {out / 'turns.json'} and {out / 'report.md'}")
    print("  " + ", ".join(f"{g} {counts.get(g, 0)}" for g in ("a", "b", "c", "unclear")))


if __name__ == "__main__":
    main()
