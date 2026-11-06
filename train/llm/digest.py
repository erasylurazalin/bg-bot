#!/usr/bin/env python3
"""One of Era's games -> a compact plain-text digest for a language model.

Everything here is read from the files the collector already writes:

- boards/actions/<log folder>-g<N>.json: the shop phases of one game, every
  step Era took with what it paid and the state it left (docs/real-boards.md
  "The action file").
- boards/real/<same name>.json: the same game with Era's final `place`
  (0 while the game was still going).
- ref/bg-cards-<build>.json and ref/bg-meta-<build>.json: the card texts of
  the game's own patch. bg-cards holds minions and Tavern spells, bg-meta the
  heroes, hero powers, trinkets and Dark Gifts.
- the game's own Power.log, for the client build (`BuildNumber=`), which says
  which patch the game was played on.

The digest carries only Era's side of the game: Era's shop, board and hand,
and whether the fight after the turn was won. Opponents' boards are
deliberately left out (they are not to be analysed).

Run it as a script:

    python3 -I train/llm/digest.py --all        # every finished game
    python3 -I train/llm/digest.py <game file>  # one digest to stdout

`--all` writes train/llm/digests/<source>-g<N>.txt and prints one line per
game with its patch, where the build came from and the size estimate
(characters / 4, the usual rough token count).
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
ACTIONS_DIR = ROOT / "boards" / "actions"
REAL_DIR = ROOT / "boards" / "real"
REF_DIR = ROOT / "ref"
OUT_DIR = HERE / "digests"

OLD_BUILD = 254477
NEW_BUILD = 255790
PATCH_OF_BUILD = {OLD_BUILD: "37.0", NEW_BUILD: "37.0.2"}

# The client that carries 37.0.2 was installed on 2026-11-04 around 14:38.
# Games on that date without a log are unknown; earlier ones are 37.0.
PATCH_DATE = "2026_11_04"

LOG_DIRS = (
    Path.home() / "bg-logs",
    Path.home() / "bg-logs-laptop",
    Path.home() / "hearthstone-linux" / "hearthstone" / "Logs",
)

BUILD_RE = re.compile(r"GameState\.DebugPrintGame\(\) - BuildNumber=(\d+)")
SOURCE_RE = re.compile(r"^Hearthstone_(\d{4})_(\d{2})_(\d{2})_(\d{2})_(\d{2})_(\d{2})$")

_CACHE: dict[str, dict] = {}


# --------------------------------------------------------------------------
# The files


def load_json(path: Path):
    with path.open() as f:
        return json.load(f)


def game_files() -> list[Path]:
    """Every game in boards/actions, sorted by log folder and game number."""
    return sorted(ACTIONS_DIR.glob("*-g*.json"))


def places() -> dict[tuple[str, int], int]:
    """(source, game) -> Era's final place; 0 means the game was unfinished."""
    out = {}
    for path in REAL_DIR.glob("*-g*.json"):
        if path.name.startswith("."):
            continue
        try:
            data = load_json(path)
        except (OSError, json.JSONDecodeError):
            continue
        if "source" in data and "game" in data:
            out[(data["source"], int(data["game"]))] = int(data.get("place") or 0)
    return out


def export(build: int) -> tuple[dict, dict]:
    """(card id -> card entry, card id -> meta entry) for one build."""
    name = str(build)
    if name in _CACHE:
        return _CACHE[name]
    cards = {}
    path = REF_DIR / f"bg-cards-{name}.json"
    if path.exists():
        for card in load_json(path)["cards"]:
            cards[card["card_id"]] = card
    meta = {}
    path = REF_DIR / f"bg-meta-{name}.json"
    if path.exists():
        data = load_json(path)
        for key, kind in (
            ("heroes", "hero"),
            ("hero_powers", "hero_power"),
            ("trinkets", "trinket"),
            ("dark_gifts", "dark_gift"),
        ):
            for card in data.get(key, []):
                meta[card["card_id"]] = dict(card, kind=kind)
    _CACHE[name] = (cards, meta)
    return cards, meta


def plain_card_id(card_id: str) -> str:
    """A golden card's own id (BG25_008_G) as the normal card's (BG25_008)."""
    if card_id.endswith("_G"):
        base = card_id[:-2]
        cards, _ = export(NEW_BUILD)
        old_cards, _ = export(OLD_BUILD)
        if base in cards or base in old_cards:
            return base
    return card_id


# --------------------------------------------------------------------------
# Which patch a game was played on


def read_build(source: str) -> int | None:
    """The client build from the game's own Power.log, or None."""
    folder = source[len("laptop-"):] if source.startswith("laptop-") else source
    for logs in LOG_DIRS:
        path = logs / folder / "Power.log"
        if not path.exists():
            continue
        try:
            with path.open("r", errors="replace") as f:
                # The line stands near the top of the file, in the first game
                # the client started; 20,000 lines is far past it and keeps
                # a 400 MB log from being read whole.
                for number, line in enumerate(f):
                    if number > 20000:
                        break
                    found = BUILD_RE.search(line)
                    if found:
                        return int(found.group(1))
        except OSError:
            continue
    return None


def source_date(source: str) -> str:
    """The date in the log folder name as 2026-10-24, or the name itself."""
    folder = source[len("laptop-"):] if source.startswith("laptop-") else source
    found = SOURCE_RE.match(folder)
    if not found:
        return folder
    y, m, d = found.group(1), found.group(2), found.group(3)
    return f"{y}-{m}-{d}"


def source_stamp(source: str) -> str:
    """2026-10-24 00:56 from the log folder name."""
    folder = source[len("laptop-"):] if source.startswith("laptop-") else source
    found = SOURCE_RE.match(folder)
    if not found:
        return folder
    y, mo, d, h, mi = found.group(1), found.group(2), found.group(3), found.group(4), found.group(5)
    return f"{y}-{mo}-{d} {h}:{mi}"


def patch_of(source: str) -> dict:
    """{'build', 'patch', 'from_log', 'guess'} for one game's log folder."""
    build = read_build(source)
    guess = False
    if build is None:
        guess = True
        date = source_date(source).replace("-", "_")
        if date < PATCH_DATE:
            build = OLD_BUILD
        elif date > PATCH_DATE:
            build = NEW_BUILD
        else:
            build = None
    if build is None:
        patch = "unknown"
    else:
        patch = PATCH_OF_BUILD.get(build, f"build {build} (not 254477 or 255790)")
    return {
        "build": build,
        "patch": patch,
        "from_log": build is not None and not guess,
        "guess": guess,
    }


# --------------------------------------------------------------------------
# What 37.0.2 changed, card by card


def _stat_pair(card: dict) -> str | None:
    a, h = card.get("attack"), card.get("health")
    if a is None or h is None:
        return None
    return f"{a}/{h}"


def _changes(old: dict, new: dict) -> list[str]:
    """The short then/now list of what the patch changed on one card."""
    parts = []
    if old.get("tier") != new.get("tier"):
        parts.append(f"tier {old.get('tier')} -> {new.get('tier')}")
    if old.get("cost") != new.get("cost"):
        parts.append(f"cost {old.get('cost')} -> {new.get('cost')}")
    ao, an = _stat_pair(old), _stat_pair(new)
    if ao and an and ao != an:
        parts.append(f"{ao} -> {an}")
    go, gn = old.get("golden_attack"), old.get("golden_health")
    gno, gnn = new.get("golden_attack"), new.get("golden_health")
    if None not in (go, gn, gno, gnn) and (go, gn) != (gno, gnn):
        parts.append(f"golden {go}/{gn} -> {gno}/{gnn}")
    if (old.get("text_plain") or "") != (new.get("text_plain") or ""):
        parts.append("text changed")
    return parts


def changed_by_patch() -> dict[str, list[str]]:
    """card id -> what 37.0.2 changed, over cards, trinkets and Dark Gifts."""
    if "changed" in _CACHE:
        return _CACHE["changed"]
    old_cards, old_meta = export(OLD_BUILD)
    new_cards, new_meta = export(NEW_BUILD)
    out: dict[str, list[str]] = {}
    for card_id, old in old_cards.items():
        # Golden copies are their own entries but hold the same text; the
        # normal entry carries the golden numbers too, so one entry says it all.
        if old.get("is_golden"):
            continue
        new = new_cards.get(card_id)
        if not new:
            continue
        parts = _changes(old, new)
        if parts:
            out[card_id] = parts
    for card_id, old in old_meta.items():
        new = new_meta.get(card_id)
        if not new:
            continue
        parts = _changes(old, new)
        if parts:
            out[card_id] = parts
    _CACHE["changed"] = out
    return out


# --------------------------------------------------------------------------
# Small formatters


def stat_numbers(numbers) -> str:
    """The card's own script numbers (the {0} {1} of its text), trailing
    zeros dropped: [3, 3, 1, 0] -> 3/3/1."""
    if not numbers:
        return ""
    values = [n for n in numbers if isinstance(n, int)]
    while values and values[-1] == 0:
        values.pop()
    return "/".join(str(n) for n in values)


def keywords_text(card: dict) -> str:
    words = [k.lower().replace("_", " ") for k in card.get("keywords") or []]
    return ", ".join(words)


def gifts_text(card: dict) -> str:
    names = [g.get("name") or g.get("card_id") for g in card.get("dark_gifts") or []]
    return ", ".join(n for n in names if n)


def card_line(card: dict) -> str:
    """Name, stats, golden, keywords, Dark Gifts, script numbers."""
    name = card.get("name") or card.get("card_id") or "?"
    if card.get("kind") == "spell":
        cost = card.get("cost")
        return f"{name} (Tavern spell, {cost} Gold)" if cost is not None else f"{name} (Tavern spell)"
    bits = []
    a, h = card.get("attack"), card.get("health")
    if a is not None and h is not None:
        bits.append(f"{a}/{h}")
    if card.get("golden"):
        bits.append("golden")
    words = keywords_text(card)
    if words:
        bits.append(words)
    gifts = gifts_text(card)
    if gifts:
        bits.append(f"Dark Gift: {gifts}")
    numbers = stat_numbers(card.get("numbers"))
    if numbers:
        bits.append(f"numbers {numbers}")
    return f"{name} " + "; ".join(bits) if bits else name


def listed(cards, empty: str = "none") -> str:
    """A numbered list of board or hand cards, or a word for nothing."""
    rows = [c for c in cards or [] if c]
    if not rows:
        return empty
    out = []
    for i, card in enumerate(rows, 1):
        position = card.get("position") or card.get("index") or card.get("slot")
        label = position if isinstance(position, int) and position > 0 else i
        out.append(f"{label}. {card_line(card)}")
    return "; ".join(out)


def card_name(card_id: str, fallback: str = "?") -> str:
    """A card's printed name, from either export."""
    cards, meta = export(NEW_BUILD)
    entry = cards.get(card_id) or meta.get(card_id)
    if entry and entry.get("name"):
        return entry["name"]
    cards, meta = export(OLD_BUILD)
    entry = cards.get(plain_card_id(card_id)) or meta.get(plain_card_id(card_id))
    return (entry or {}).get("name") or fallback


def target_text(target) -> str:
    if not target:
        return ""
    if isinstance(target, dict):
        name = target.get("name") or target.get("card_id") or "?"
        where = target.get("kind") or target.get("where")
        if where == "slot":
            return f"{name} in Tavern slot {target.get('slot')}"
        if where == "board" and target.get("position"):
            return f"{name} at position {target.get('position')}"
        return str(name)
    return str(target)


def price_text(paid: dict) -> str:
    """What a step took: Gold and Health, in words."""
    gold = (paid or {}).get("gold") or 0
    health = (paid or {}).get("health") or 0
    bits = []
    if gold:
        bits.append(f"{abs(gold)} Gold" + (" back" if gold < 0 else ""))
    if health:
        bits.append(f"{abs(health)} Health" + (" back" if health < 0 else ""))
    return ", ".join(bits)


def export_type(card_id: str) -> str:
    cards, meta = export(NEW_BUILD)
    entry = cards.get(card_id) or meta.get(card_id)
    return (entry or {}).get("type") or ""


def is_spell(card: dict) -> bool:
    if card.get("kind") == "spell":
        return True
    if export_type(card.get("card_id") or "") in ("BATTLEGROUND_SPELL", "SPELL"):
        return True
    return False


def position_of(card: dict, before: dict, after: dict) -> int | None:
    """Where a minion Era played landed on Era's board."""
    for minion in after.get("board") or []:
        if card.get("id") and minion.get("id") == card.get("id"):
            return minion.get("position")
    was = {m.get("id") for m in before.get("board") or []}
    for minion in after.get("board") or []:
        if minion.get("card_id") == card.get("card_id") and minion.get("id") not in was:
            return minion.get("position")
    return None


# --------------------------------------------------------------------------
# One step, in plain words


def action_line(action: dict, before: dict) -> str:
    step = action.get("step")
    paid = action.get("paid") or {}
    price = price_text(paid)
    card = action.get("card") or {}
    name = card.get("name") or card.get("card_id") or "?"
    if step == "buy":
        what = "Tavern spell " if is_spell(card) else ""
        bits = [f"bought {what}{name}"]
        if price:
            bits.append(f"({price})")
        slot = action.get("slot")
        if slot:
            bits.append(f"from slot {slot}")
        return " ".join(bits)
    if step == "sell":
        bits = [f"sold {name}"]
        if action.get("position"):
            bits.append(f"at position {action['position']}")
        if price:
            bits.append(f"({price})")
        return " ".join(bits)
    if step == "play":
        if (card.get("card_id") or "") == "TB_BaconShop_Triples_01":
            return "used a Triple Reward"
        if is_spell(card):
            on = target_text(action.get("target"))
            return f"cast {name}" + (f" on {on}" if on else "")
        line = f"played {name}"
        where = position_of(card, before, action.get("after") or {})
        if where:
            line += f" to position {where}"
        choice = action.get("sub_option")
        if isinstance(choice, int) and choice >= 0:
            line += f" (Choose One option {choice + 1})"
        target = target_text(action.get("target"))
        if target:
            line += f" targeting {target}"
        return line
    if step == "activate":
        bits = [f"activated {name}"]
        if price:
            bits.append(f"({price})")
        target = target_text(action.get("target"))
        if target:
            bits.append(f"targeting {target}")
        return " ".join(bits)
    if step == "move":
        return (f"moved {name} from position {action.get('position')} "
                f"to position {action.get('to')}")
    if step == "roll":
        free = action.get("free")
        if free:
            left = (action.get("free_refreshes") or {}).get("after")
            tail = f", {left} free left" if isinstance(left, int) else ""
            return f"refreshed (free{tail})"
        return "refreshed" + (f" ({price})" if price else "")
    if step == "freeze":
        return "froze the Tavern" if action.get("on") else "unfroze the Tavern"
    if step == "level":
        tier = action.get("tier_after")
        line = f"levelled to tier {tier}" if tier else "levelled up"
        return line + (f" ({price})" if price else "")
    if step == "hero_power":
        which = action.get("which") or "main"
        line = "used the second hero power" if which == "second" else "used the hero power"
        target = target_text(action.get("target"))
        if target:
            line += f" on {target}"
        return line + (f" ({price})" if price else "")
    if step == "dark_discovery":
        line = "used the Dark Gift button"
        return line + (f" ({price})" if price else "")
    if step == "discover":
        return discover_line(action, price)
    if step == "trinket_pick":
        return trinket_line(action, price)
    if step == "end_turn":
        return "ended the turn"
    return f"{step} " + (name if card else "")


def discover_line(action: dict, price: str) -> str:
    kind = action.get("kind") or "discover"
    source = (action.get("source") or {}).get("name") or ""
    options = action.get("options") or []
    offered = []
    for option in options:
        text = option.get("name") or option.get("card_id") or "?"
        gift = gifts_text(option)
        if gift:
            text += f" (Dark Gift: {gift})"
        if option.get("golden"):
            text += " (golden)"
        offered.append(text)
    picked = action.get("picked") or {}
    took = picked.get("name") or picked.get("card_id") or "nothing"
    if kind == "dark_gift":
        head = "Dark Gift Discover"
    elif kind == "triple_reward":
        head = "triple reward"
    elif kind == "choose_one":
        head = "Choose One"
    else:
        head = f"Discover from {source}" if source else "Discover"
    line = f"{head}: offered " + " / ".join(offered) + f"; took {took}"
    gift = gifts_text(picked)
    if gift:
        line += f" (Dark Gift: {gift})"
    return line + (f" ({price})" if price else "")


def trinket_line(action: dict, price: str) -> str:
    kind = action.get("slot_kind") or "trinket"
    options = action.get("options") or []
    offered = []
    for option in options:
        text = option.get("name") or option.get("card_id") or "?"
        cost = option.get("cost")
        if cost is not None:
            text += f" ({cost} Gold)"
        offered.append(text)
    picked = action.get("picked") or {}
    took = picked.get("name") or picked.get("card_id") or "nothing"
    cost = picked.get("cost")
    if cost is not None:
        took += f" ({cost} Gold)"
    line = f"{kind} trinket pick: offered " + " / ".join(offered) + f"; took {took}"
    paid_gold = (action.get("paid") or {}).get("gold") or 0
    if price and cost != abs(paid_gold):
        line += f" ({price})"
    return line


# --------------------------------------------------------------------------
# Turns


def turn_block(turn: dict, prev_start: dict | None, next_start: dict | None) -> list[str]:
    start = turn.get("start") or {}
    end = turn.get("end") or {}
    tier = start.get("tier")
    gold = start.get("gold")
    gold_max = start.get("gold_max")
    gold_text = f"{gold} Gold" if gold == gold_max else f"{gold} Gold of {gold_max}"
    health = start.get("health")
    armor = start.get("armor")
    vitals = f"Health {health}" + (f" + Armor {armor}" if armor else "")
    upgrade = start.get("upgrade_cost")
    up_text = f", the tier up costs {upgrade} Gold" if upgrade else ""

    lines = [f"## Turn {turn.get('turn')}", f"Start: tier {tier}, {gold_text}, {vitals}{up_text}"]
    if start.get("frozen"):
        lines.append("The Tavern was frozen at the start of the turn.")
    free = start.get("free_refreshes")
    if free:
        lines.append(f"Free Refreshes at the start: {free}.")
    trinkets = [t.get("name") or t.get("card_id") for t in start.get("trinkets") or []]
    was = [t.get("name") or t.get("card_id") for t in (prev_start or {}).get("trinkets") or []]
    if trinkets and trinkets != was:
        lines.append("Trinkets now: " + ", ".join(n for n in trinkets if n))
    tavern = listed(start.get("tavern"))
    if tavern != "none":
        lines.append(f"Tavern: {tavern}")

    before = start
    items: list[str | None] = []
    rolls = 0          # consecutive Refreshes, counted as one line
    roll_gold = 0
    roll_free = True
    roll_left = None

    def flush_rolls():
        nonlocal rolls, roll_gold, roll_free, roll_left
        if not rolls:
            return
        if roll_free:
            tail = f", {roll_left} free left" if isinstance(roll_left, int) else ""
            items.append(f"refreshed {rolls} times (all free{tail})" if rolls > 1
                         else f"refreshed (free{tail})")
        else:
            items.append(f"refreshed {rolls} times ({roll_gold} Gold)" if rolls > 1
                         else f"refreshed ({roll_gold} Gold)")
        rolls, roll_gold, roll_free, roll_left = 0, 0, True, None

    for action in turn.get("actions") or []:
        step = action.get("step")
        if step == "end_turn":
            before = action.get("after") or before
            continue
        if step == "roll":
            rolls += 1
            roll_gold += abs((action.get("paid") or {}).get("gold") or 0)
            if not action.get("free"):
                roll_free = False
            roll_left = (action.get("free_refreshes") or {}).get("after")
        else:
            flush_rolls()
            items.append(action_line(action, before))
        after = action.get("after")
        if isinstance(after, dict):
            before = after
        for changed in action.get("changed") or []:
            was = changed.get("was") or {}
            now = changed.get("now") or {}
            if now.get("golden") and not was.get("golden"):
                flush_rolls()
                name = now.get("name") or card_name(now.get("card_id") or "")
                items.append(f"triple made: {name} golden")
    flush_rolls()

    actions = []
    index = 0
    while index < len(items):
        text = items[index]
        same = index
        while same + 1 < len(items) and items[same + 1] == text:
            same += 1
        count = same - index + 1
        actions.append("- " + text + (f" (x{count})" if count > 1 else ""))
        index = same + 1
    if not actions:
        actions = ["- (the log has no steps of Era's in this turn)"]
    lines.append("Actions:")
    lines.extend(actions)

    if end:
        lines.append(f"End board ({len(end.get('board') or [])}): {listed(end.get('board'))}")
        lines.append(f"End hand: {listed(end.get('hand'))}")
        gold_left = end.get("gold")
        if isinstance(gold_left, int):
            lines.append(f"Gold left: {gold_left}")
    else:
        lines.append("End state: (the log ends inside this turn)")

    if next_start:
        won = next_start.get("won_last_combat")
        if won is True:
            lines.append("Fight: won.")
        elif won is False:
            lines.append("Fight: lost.")
        elif turn.get("turn") == 1:
            lines.append("Fight: (turn 1 has no combat)")
        else:
            lines.append("Fight: (the log does not say)")
    else:
        lines.append("Fight: (not logged: the game ends with this turn)")
    return lines


def raw_turn(game: dict, number: int) -> str:
    """One turn as the collector wrote it, trimmed to the actions and the
    end of turn state (for the one follow-up question a model may ask)."""
    for turn in game.get("turns") or []:
        if turn.get("turn") != number:
            continue
        steps = []
        for action in turn.get("actions") or []:
            step = {k: v for k, v in action.items() if k not in ("after", "tavern", "changed", "entered", "left", "moved")}
            steps.append(step)
        trimmed = {
            "turn": turn.get("turn"),
            "start": turn.get("start"),
            "actions": steps,
            "end": turn.get("end"),
        }
        return json.dumps(trimmed, indent=1)
    return json.dumps({"error": f"this game has no turn {number}"})


# --------------------------------------------------------------------------
# The patch line


def patch_line(info: dict, seen_cards: set[str]) -> str:
    patch = info["patch"]
    build = info["build"]
    if patch == "37.0.2":
        return f"Patch: 37.0.2 (build {build}), the current patch."
    changed = changed_by_patch()
    cards, meta = export(NEW_BUILD)
    rows = []
    for plain in sorted({plain_card_id(c) for c in seen_cards}):
        if plain not in changed:
            continue
        new = cards.get(plain) or meta.get(plain) or {}
        name = new.get("name") or plain
        rows.append(f"{name} ({'; '.join(changed[plain])})")
    listed_text = ", ".join(rows) if rows else "none of the cards in this game"
    if patch == "37.0":
        head = (
            f"Patch: 37.0 (build {build}). This game was played BEFORE patch "
            "37.0.2. The patch changed numbers (stats, prices, a few texts), "
            "not the structure of the game."
        )
    elif patch == "unknown":
        head = (
            "Patch: unknown (the log is missing and the game is dated 2026-11-04, "
            "the day 37.0.2 arrived, so either patch is possible)."
        )
    else:
        head = (
            f"Patch: {patch}. This game's client build is {build}, which is "
            "neither 254477 (37.0) nor 255790 (37.0.2), so the numbers may be "
            "from either patch."
        )
    return f"{head} Cards in this game that 37.0.2 changed: {listed_text}."


# --------------------------------------------------------------------------
# The digest


def collect_card_ids(node, out: set[str]) -> None:
    """Every card id anywhere in a piece of the game's JSON."""
    if isinstance(node, dict):
        card_id = node.get("card_id")
        if isinstance(card_id, str):
            out.add(card_id)
        for value in node.values():
            collect_card_ids(value, out)
    elif isinstance(node, list):
        for value in node:
            collect_card_ids(value, out)


# The shop's own buttons, which are "cards" in the log but not in the game.
UI_CARDS = {
    "BG30_Trinket_1st",
    "BG30_Trinket_2nd",
    "TB_BaconShop_Triples_01",
    "BG36_MidGameEffect_010",
}


def used_card_ids(game: dict, info: dict) -> set[str]:
    """The cards Era had or took in this game.

    Tavern and Discover offers are left out on purpose: they are named with
    their stats in the turns and their texts would double the digest (a game
    offers hundreds of cards). Everything Era bought, picked, played, kept or
    was offered by name where it mattered has its text in the section.
    """
    out: set[str] = set()
    if info.get("hero_id"):
        out.add(info["hero_id"])
    out.update(info.get("hero_power_ids") or [])
    for turn in game.get("turns") or []:
        for key in ("start", "end"):
            state = turn.get(key) or {}
            collect_card_ids(state.get("board"), out)
            collect_card_ids(state.get("hand"), out)
            collect_card_ids(state.get("hero_powers"), out)
            collect_card_ids(state.get("trinkets"), out)
        for action in turn.get("actions") or []:
            collect_card_ids(action.get("card"), out)
            collect_card_ids(action.get("picked"), out)
            collect_card_ids(action.get("target"), out)
            source = action.get("source") or {}
            if source.get("card_id") not in UI_CARDS:
                collect_card_ids(source, out)
    return out


def cards_section(seen_cards: set[str], info: dict) -> list[str]:
    """Every card Era had or took in this game, with its text once, from the
    game's own patch export."""
    cards, meta = export(info["build"] or NEW_BUILD)
    minions, spells, trinkets, gifts, powers, unknown = [], [], [], [], [], []
    seen_names: set[str] = set()
    for plain in sorted({plain_card_id(c) for c in seen_cards}):
        meta_entry = meta.get(plain)
        entry = cards.get(plain) or cards.get(plain.replace("TB_BaconUps_", ""))
        if meta_entry is None and entry is None:
            if not plain.startswith("TB_BaconShop_"):
                unknown.append(f"- {plain}")
            continue
        if meta_entry is not None and meta_entry.get("kind") in ("trinket", "dark_gift"):
            name = meta_entry.get("name") or plain
            text = (meta_entry.get("text_plain") or "").strip()
            cost = meta_entry.get("cost")
            head = f"{name} ({plain})" + (f", {cost} Gold" if cost is not None else "")
            if meta_entry["kind"] == "trinket":
                head += f" ({ (meta_entry.get('slot') or 'trinket').lower()})"
                rows = trinkets
            else:
                rows = gifts
            if name not in seen_names:
                seen_names.add(name)
                rows.append(f"- {head}: {text}" if text else f"- {head}.")
            continue
        if entry is None:
            name = meta_entry.get("name") or plain
            text = (meta_entry.get("text_plain") or "").strip()
            cost = meta_entry.get("cost")
            head = f"{name} ({plain})" + (f", {cost} Gold" if cost is not None else "")
            if name not in seen_names:
                seen_names.add(name)
                powers.append(f"- {head}: {text}" if text else f"- {head}.")
            continue
        name = entry.get("name") or plain
        if "[DNT]" in name or plain.startswith("TB_BaconShop_"):
            # Shop buttons and log-only helper cards are not game cards.
            continue
        if name in seen_names:
            continue
        seen_names.add(name)
        text = (entry.get("text_plain") or "").strip()
        kind = entry.get("type")
        if kind == "MINION":
            head = f"{name} ({plain}), tier {entry.get('tier')}"
            tribes = ", ".join(t.lower() for t in entry.get("tribes") or [])
            if tribes:
                head += f", {tribes}"
            stats = _stat_pair(entry)
            if stats:
                head += f", {stats}"
            golden = (entry.get("golden_attack"), entry.get("golden_health"))
            if None not in golden:
                head += f" (golden {golden[0]}/{golden[1]})"
            minions.append(f"- {head}: {text}" if text else f"- {head}.")
        else:
            cost = entry.get("cost")
            head = f"{name} ({plain})" + (f", {cost} Gold" if cost is not None else "")
            spells.append(f"- {head}: {text}" if text else f"- {head}.")
    lines = []
    for title, rows in (
        ("Minions", minions),
        ("Tavern spells", spells),
        ("Trinkets", trinkets),
        ("Dark Gifts", gifts),
        ("Heroes and hero powers", powers),
        ("Marks the export has no text for (enchantments, skins)", unknown),
    ):
        if rows:
            lines.append(f"### {title}")
            lines.extend(rows)
    return lines


_LOBBY_TRIBES = {"BEAST": "beast", "MECHANICAL": "mech", "DRAGON": "dragon",
                 "MURLOC": "murloc", "DEMON": "demon", "PIRATE": "pirate",
                 "ELEMENTAL": "elemental", "UNDEAD": "undead", "QUILBOAR": "quilboar",
                 "ABERRATION": "aberration", "NAGA": "naga"}


def lobby_tribes(game: dict) -> list[str]:
    """The lobby's types, by Era's rule (docs/rules/design.md "The lobby"): a
    type is in when a minion of only that type shows up in the Tavern. The log
    never names the lobby; minions with two types, "all types" minions and
    cards outside the Tavern don't count."""
    cards, _ = export(NEW_BUILD)
    old_cards, _ = export(OLD_BUILD)
    found = set()
    for turn in game.get("turns") or []:
        states = [turn.get("start") or {}] + [a.get("after") or {} for a in turn.get("actions") or []]
        for state in states:
            for m in state.get("tavern") or []:
                cid = plain_card_id(m.get("card_id") or "")
                c = cards.get(cid) or old_cards.get(cid)
                if not c or c.get("type") != "MINION":
                    continue
                tribes = c.get("tribes") or []
                if len(tribes) == 1 and tribes[0] in _LOBBY_TRIBES:
                    found.add(_LOBBY_TRIBES[tribes[0]])
    return sorted(found)


def game_info(path: Path) -> dict:
    """Everything the digest and the analysis need to know about one game."""
    game = load_json(path)
    source = game["source"]
    number = int(game["game"])
    where = places().get((source, number))
    hero = game.get("hero") or {}
    hero_id = hero.get("card_id") or ""
    if "_SKIN_" in hero_id:
        # The log names a hero skin; the export holds the hero itself.
        hero_id = hero_id.split("_SKIN_")[0]
    powers: list[dict] = []
    seen_powers = set()
    for turn in game.get("turns") or []:
        for power in (turn.get("start") or {}).get("hero_powers") or []:
            key = power.get("card_id")
            if key not in seen_powers:
                seen_powers.add(key)
                powers.append(power)
    if not powers:
        # Some logs start mid-game with no hero power in the state; the
        # hero's own entry in the export names its base power.
        _, meta = export(patch_of(source)["build"] or NEW_BUILD)
        hero_entry = meta.get(hero_id) or {}
        base = hero_entry.get("hero_power_card_id")
        if base and base in meta:
            powers.append({"card_id": base, "name": meta[base].get("name")})
    return {
        "path": path,
        "source": source,
        "game": number,
        "key": f"{source}-g{number}",
        "date": source_stamp(source),
        "hero": hero.get("name") or hero.get("card_id") or "?",
        "hero_id": hero_id,
        "hero_powers": [p.get("name") or p.get("card_id") for p in powers],
        "hero_power_ids": [p.get("card_id") for p in powers if p.get("card_id")],
        "tribes": list(game.get("tribes") or []),
        "lobby": lobby_tribes(game),
        "place": where,
        "unfinished": where in (None, 0),
        "turns": len(game.get("turns") or []),
        "steps": sum(len(t.get("actions") or []) for t in game.get("turns") or []),
        "digest_path": OUT_DIR / f"{source}-g{number}.txt",
        **patch_of(source),
    }


def digest(path: Path, info: dict | None = None) -> str:
    """One game's digest, as plain text."""
    info = info or game_info(path)
    game = load_json(path)
    seen = used_card_ids(game, info)

    lines = []
    lines.append(f"# Era's game: {info['hero']} on {info['date']}")
    lines.append("")
    lines.append(f"Source: {info['source']}, game {info['game']}")
    lines.append(f"Date: {source_date(info['source'])}")
    hero_powers = [p for p in info["hero_powers"] if p]
    label = "Hero power" if len(hero_powers) < 2 else "Hero powers"
    lines.append(f"Hero: {info['hero']} ({info['hero_id']})")
    lines.append(f"{label}: " + (", ".join(hero_powers) or "none seen"))
    lines.append(patch_line(info, seen))
    lobby = info.get("lobby") or []
    if len(lobby) == 5:
        lines.append("Lobby tribes: " + ", ".join(lobby) + " (found from the minions of only "
                     "one type that showed up in Era's Tavern)")
    else:
        lines.append(f"Lobby tribes: only {len(lobby)} of the 5 could be found (" +
                     (", ".join(lobby) or "none") + "); the log is too short to see them all")
    if info["unfinished"]:
        lines.append("Final place: unknown (the log ends before the game did)")
    else:
        lines.append(f"Final place: {info['place']} of 8")
    lines.append(f"Turns logged: {info['turns']}, steps Era took: {info['steps']}")
    lines.append("")

    turns = game.get("turns") or []
    for i, turn in enumerate(turns):
        before = turns[i - 1].get("start") if i else None
        after = turns[i + 1].get("start") if i + 1 < len(turns) else None
        lines.extend(turn_block(turn, before, after))
        lines.append("")

    lines.append("## Cards in this game")
    lines.append("")
    lines.append(
        "Texts from the game's own patch export: the cards Era bought, played, "
        "picked or held, plus Era's hero powers, trinkets and Dark Gifts. A "
        "Tavern or Discover offer Era did not take is named with its stats in "
        "the turn and has no text here."
    )
    lines.append("")
    lines.extend(cards_section(seen, info))
    lines.append("")
    return "\n".join(lines)


# --------------------------------------------------------------------------
# The script


def size_estimate(text: str) -> int:
    """Characters / 4, the usual rough token count."""
    return round(len(text) / 4)


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("games", nargs="*", help="game files (default: --all)")
    parser.add_argument("--all", action="store_true", help="digest every game")
    parser.add_argument("--out", type=Path, default=OUT_DIR, help="where --all writes")
    args = parser.parse_args(argv)

    if not args.all and not args.games:
        parser.error("give game files or --all")

    if args.games:
        for name in args.games:
            path = Path(name)
            if not path.exists() and ACTIONS_DIR.joinpath(path.name).exists():
                path = ACTIONS_DIR / path.name
            info = game_info(path)
            text = digest(path, info)
            print(text)
        return 0

    args.out.mkdir(parents=True, exist_ok=True)
    from_log = guessed = neither = 0
    unfinished = []
    skipped = []
    sizes = []
    patch_counts = {"37.0": 0, "37.0.2": 0, "unknown": 0}
    for path in game_files():
        info = game_info(path)
        if info["unfinished"]:
            unfinished.append(info["key"])
            continue
        text = digest(path, info)
        args.out.joinpath(info["digest_path"].name).write_text(text)
        tokens = size_estimate(text)
        sizes.append((tokens, info["key"]))
        if info["from_log"]:
            from_log += 1
        elif info["guess"]:
            guessed += 1
        if info["patch"] in patch_counts:
            patch_counts[info["patch"]] += 1
        else:
            neither += 1
        where = "log" if info["from_log"] else "date guess"
        flag = ""
        if info["patch"] == "37.0":
            flag = "  [before 37.0.2]"
        elif info["patch"] == "unknown":
            flag = "  [patch unknown]"
        elif info["patch"] not in patch_counts:
            flag = "  [other build]"
        print(f"{info['key']:<48} {info['patch']:<8} {where:<10} "
              f"{tokens:>6} tokens{flag}")

    sizes.sort()
    print()
    print(f"digested {len(sizes)} games into {args.out}")
    if sizes:
        smallest, median, largest = sizes[0], sizes[len(sizes) // 2], sizes[-1]
        print(f"tokens: smallest {smallest[0]} ({smallest[1]}), "
              f"median {median[0]} ({median[1]}), largest {largest[0]} ({largest[1]})")
    print(f"patch: 37.0 {patch_counts['37.0']}, 37.0.2 {patch_counts['37.0.2']}, "
          f"unknown {patch_counts['unknown']}, other build {neither}")
    print(f"build from the game's Power.log: {from_log}; guessed from the date: {guessed}")
    if unfinished:
        print("skipped, unfinished (place 0): " + ", ".join(unfinished))
    if skipped:
        print("skipped, unchanged: " + ", ".join(skipped))
    return 0


if __name__ == "__main__":
    sys.exit(main())
