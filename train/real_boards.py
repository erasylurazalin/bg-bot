"""Load the boards from Era's real games for training (docs/real-boards.md).

Reads every board file in boards/real/ (one JSON file per game, written by
tools/collect_boards.py; load whatever is there when a run starts, new games
arrive every 15 minutes), maps every minion's card id to a card name through
ref/bg-cards.json (a golden id, the game's `_G` and TB_BaconUps_* forms, maps
to the base card's name and sets golden), and prints what was loaded: boards
per turn and the unmatched minions by name.

The specs go straight to bgpy.VecEnv(real_boards=..). A real seat (lineup
kind 5) then draws a fresh random board of the current turn at the start of
each of its turns: turn 13 and later from every board of turn 13 or later
alike, and a turn before that with no boards of its own from the nearest
earlier turn that has some (the engine, bgcore::real_boards, holds the rule).

Standalone, to check a folder (stdlib only, no venv needed):

    python3 -I train/real_boards.py [DIR]
"""

from __future__ import annotations

import argparse
import json
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
REF_CARDS = ROOT / "ref" / "bg-cards.json"
CARDS_TOML = ROOT / "cards.toml"
DEFAULT_DIR = ROOT / "boards" / "real"

# Turn 13 and later all draw from one shared pool of boards.
LATE_TURN = 13

# Name aliases the engine's CardDb::by_name also accepts.
ALIASES = {
    "Goldrinn the Great Wolf": "Goldrinn, the Great Wolf",
    "Goldrinn": "Goldrinn, the Great Wolf",
    "Beast": "Foraging Bat",
}


STAND_IN = "Stand-in"


def built_cards(cards_toml: Path = CARDS_TOML) -> set[str]:
    """The card names the engine can build (cards.toml holds names, not ids)."""
    data = tomllib.loads(cards_toml.read_text())
    return {c["name"] for key in ("minion", "token", "spell") for c in data.get(key, [])}


def card_map(ref: Path = REF_CARDS) -> dict[str, tuple[str, bool]]:
    """card_id -> (card name, whether the id itself is a golden card), from
    ref/bg-cards.json (docs/card-data.md)."""
    cards = json.loads(ref.read_text())["cards"]
    names = {c["card_id"]: c["name"] for c in cards if not c["is_golden"]}
    out: dict[str, tuple[str, bool]] = {}
    for c in cards:
        if c["is_golden"]:
            # The golden card's name is the base's; where the export points
            # at the base, take that name for sure.
            base = c.get("normal_card_id")
            out[c["card_id"]] = (names.get(base, c["name"]), True)
        else:
            out[c["card_id"]] = (c["name"], False)
    return out


def load(directory=DEFAULT_DIR, ref: Path = REF_CARDS, cards_toml: Path = CARDS_TOML):
    """Read every board file in `directory` (dot files like .state.json skip).

    Returns (specs, unknown): specs is the list for bgpy.VecEnv(real_boards=..),
    one (turn, tier, minions, hand, counters, deity) per board; a minion is
    (name, attack, health, golden, keywords, numbers, dark_gifts), counters is the side's
    player-level numbers by their file keys (a number the log did not show is
    left out, and the engine uses the base value), and deity is
    (name, attack, health, golden) or None. unknown lists every minion that
    matched no card we have, by name: card ids the ref data doesn't know
    (buddies, timewarped minions) and cards the engine hasn't built (Dark
    Paradox). Those minions are dropped; the boards around them stay.

    A file from before the counters and hands were collected still loads: it
    has no `counters`, `hand`, `deity` or `numbers`, so its boards play with
    the base values and no hand.
    """
    ids = card_map(ref)
    built = built_cards(cards_toml)
    specs, unknown = [], []

    def minions(rows):
        out = []
        for m in rows:
            # A minion we don't build becomes a Stand-in with its stats and
            # keywords and no effect (Era, 2026-10-25).
            hit = ids.get(m["card_id"])
            if hit is None:
                unknown.append(m["name"] or m["card_id"])
                name, id_golden = STAND_IN, False
            else:
                name, id_golden = hit
                name = ALIASES.get(name, name)
                if name not in built:
                    unknown.append(name)
                    name = STAND_IN
            out.append((name, m["attack"], m["health"],
                        bool(m["golden"]) or id_golden, list(m["keywords"]),
                        list(m.get("numbers") or ()),
                        list(m.get("dark_gifts") or ())))
        return out

    def deity(d):
        """(name, attack, health, golden), or None for no Deity (or one whose
        card we don't build)."""
        if not d:
            return None
        hit = ids.get(d["card_id"])
        name = ALIASES.get(hit[0], hit[0]) if hit else d.get("name", "")
        if name not in built:
            return None
        return (name, d["attack"], d["health"], bool(hit and hit[1]))

    for f in sorted(Path(directory).glob("*.json")):
        if f.name.startswith("."):
            continue
        for b in json.loads(f.read_text())["boards"]:
            specs.append((b["turn"], b["tier"], minions(b["minions"]),
                          minions(b.get("hand") or ()), dict(b.get("counters") or {}),
                          deity(b.get("deity"))))
    return specs, unknown


def print_summary(specs, unknown) -> None:
    counts: dict[int, int] = {}
    for turn, *_ in specs:
        t = min(turn, LATE_TURN)
        counts[t] = counts.get(t, 0) + 1
    turns = ", ".join(
        (f"t{t}:" if t < LATE_TURN else f"{LATE_TURN}+:") + f" {counts.get(t, 0)}"
        for t in sorted(set(counts) | set(range(1, LATE_TURN + 1)))
        if counts.get(t, 0) > 0 or t == LATE_TURN
    )
    # What the collector found for the sides themselves (docs/real-boards.md):
    # a number the log did not show plays at its base value, so a board
    # without counters is a board whose player had nothing beyond the base.
    counted = sum(1 for s in specs if s[4])
    handed = sum(1 for s in specs if s[3])
    gods = sum(1 for s in specs if s[5])
    print(f"real boards: {len(specs)} ({turns})")
    print(f"  with counters: {counted}, with a hand: {handed}, with a Deity: {gods}")
    if unknown:
        names = sorted(set(unknown))
        print(f"unmatched minions ({len(unknown)}): {', '.join(names)}")


def main() -> None:
    a = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    a.add_argument("directory", nargs="?", default=DEFAULT_DIR, type=Path)
    specs, unknown = load(a.parse_args().directory)
    print_summary(specs, unknown)


if __name__ == "__main__":
    main()
