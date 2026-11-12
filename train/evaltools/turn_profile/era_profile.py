"""Profile Era's logged shop turns 7 to 12.

Measurement for docs/briefs/turn-profile.md. It reads `boards/actions/*.json`
and `boards/real/*.json`, joins them on source and game, and writes one JSON
row per game with per-turn counts. Nothing here changes the engine or the
logs.

    python3 -I train/evaltools/turn_profile/era_profile.py

The fields mirror `collect_bot.py`:

- `actions`: logged steps, End Turn included.
- `actions_grouped`: the same; Era's log already folds a target, a discard and
  a Choose One answer into the step that opened them.
- `kinds`: steps by the bot's action-kind names.
- `rolls`, `buys` (minions), `sells`, `cycles`, `spells_bought`,
  `spells_played`, `hero_powers`, `level_gold`.
- `tier_end`, `gold_end` from the turn's `end`.
- `ended_by_cap`: no End Turn step, at least 200 steps and an `end` state.
  Era has no turn like that; the test is there so the column means the same
  thing on both sides.
- `gold_end_exact`: false when the turn has no `end` state.

A buy-then-sell cycle is a sell step whose card entity id is one this turn
bought. When an id is missing, the card id is used instead. The log keeps
entity ids, so two copies of a card are told apart.
"""

from __future__ import annotations

import glob
import json
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
ACTIONS = ROOT / "boards" / "actions"
REAL = ROOT / "boards" / "real"
OUT = ROOT / "train" / "runs" / "turn-profile" / "raw-era.json"
TARGETS = tuple(range(7, 13))

KIND = {
    "buy": "buy",
    "sell": "sell",
    "play": "play",
    "roll": "roll",
    "freeze": "freeze",
    "level": "level",
    "hero_power": "hero_power",
    "activate": "activate",
    "move": "move",
    "discover": "choose",
    "dark_discovery": "dark_gift",
    "trinket_pick": "choose_trinket",
    "end_turn": "end",
}


def spell_hand(pre: dict, index: int, card: dict) -> bool:
    """Whether the card a play step names was a spell in the pre-state."""
    for c in pre.get("hand", []):
        if c.get("index") == index and c.get("id") == card.get("id"):
            return c.get("kind") == "spell"
    # The index sometimes moved: fall back to the entity id anywhere in hand.
    for c in pre.get("hand", []):
        if c.get("id") == card.get("id"):
            return c.get("kind") == "spell"
    return False


def buy_is_spell(step: dict, pre: dict) -> bool:
    if step.get("spell"):
        return True
    slot = step.get("slot")
    card = step.get("card", {})
    for c in pre.get("tavern", []):
        if c.get("slot") == slot and c.get("id") == card.get("id"):
            return c.get("kind") == "spell"
    return card.get("kind") == "spell"


def profile_turn(t: dict) -> dict | None:
    turn = int(t["turn"])
    if turn not in TARGETS:
        return None
    actions = t["actions"]
    end = t.get("end")
    pre = t["start"]
    kinds = Counter()
    rolls = buys = sells = cycles = spells_bought = spells_played = 0
    hero_powers = 0
    level_gold = 0
    bought: list[dict] = []
    play_unknown = 0
    for a in actions:
        step = a["step"]
        kind = KIND.get(step, step)
        if step == "hero_power" and a.get("which") == "second":
            kind = "hero_power2"
        if step == "buy":
            if buy_is_spell(a, pre):
                kind = "buy_spell"
        kinds[kind] += 1
        if step == "roll":
            rolls += 1
        elif step == "sell":
            sells += 1
            card = a.get("card") or {}
            cid = card.get("id")
            card_id = card.get("card_id")
            hit = None
            if cid is not None:
                for i, b in enumerate(bought):
                    if b["id"] == cid:
                        hit = i
                        break
            if hit is None and card_id is not None:
                for i, b in enumerate(bought):
                    if b["card_id"] == card_id:
                        hit = i
                        break
            if hit is not None:
                del bought[hit]
                cycles += 1
        elif step == "buy":
            if buy_is_spell(a, pre):
                spells_bought += 1
            else:
                buys += 1
                card = a.get("card") or {}
                bought.append({"id": card.get("id"),
                               "card_id": card.get("card_id")})
        elif step == "play":
            card = a.get("card") or {}
            if spell_hand(pre, a.get("index"), card):
                spells_played += 1
            elif card.get("kind") == "spell":
                spells_played += 1
            elif card.get("attack", 0) == 0 and card.get("health", 0) == 0:
                # A played card with no stats and no hand kind: almost always
                # a spell, but keep it out and count it as unknown instead.
                play_unknown += 1
        elif step in ("hero_power",):
            hero_powers += 1
        elif step == "level":
            paid = (a.get("paid") or {}).get("gold")
            if paid is None:
                paid = a.get("cost", 0)
            if paid and paid > 0:
                level_gold += int(paid)
        pre = a.get("after") or pre

    ended = end is not None
    return {
        "actions": len(actions),
        "actions_grouped": len(actions),
        "kinds": dict(sorted(kinds.items())),
        "rolls": rolls,
        "buys": buys,
        "sells": sells,
        "cycles": cycles,
        "spells_bought": spells_bought,
        "spells_played": spells_played,
        "play_unknown": play_unknown,
        "hero_powers": hero_powers,
        "level_gold": level_gold,
        "tier_start": (t.get("start") or {}).get("tier"),
        "tier_end": end.get("tier") if ended else None,
        "gold_start": (t.get("start") or {}).get("gold"),
        "gold_end": end.get("gold") if ended else None,
        "gold_end_exact": ended,
        "ended_by_cap": (not any(a["step"] == "end_turn" for a in actions)
                         and len(actions) >= 200 and ended),
        "ended_by_end": any(a["step"] == "end_turn" for a in actions),
    }


def main():
    real = {}
    for p in glob.glob(str(REAL / "*.json")):
        d = json.load(open(p))
        real[(d["source"], d["game"])] = d
    rows = []
    for p in sorted(glob.glob(str(ACTIONS / "*.json"))):
        d = json.load(open(p))
        key = (d["source"], d["game"])
        r = real.get(key)
        turns = {}
        for t in d["turns"]:
            if int(t["turn"]) in TARGETS:
                turns[str(t["turn"])] = profile_turn(t)
        rows.append({
            "source": d["source"],
            "game": d["game"],
            "place": (r or {}).get("place"),
            "hero": (d.get("hero") or {}).get("name"),
            "turns": turns,
        })
    payload = {
        "source": str(ACTIONS),
        "games": len(rows),
        "targets": list(TARGETS),
        "rows": rows,
    }
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(json.dumps(payload) + "\n")
    places = Counter(r["place"] for r in rows)
    print(f"wrote {OUT} ({len(rows)} games, places {dict(sorted(places.items(), key=lambda x: (x[0] is None, x[0])))})")


if __name__ == "__main__":
    main()
