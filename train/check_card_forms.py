"""Check data/card_forms.json against the card list and the allowed form.

    python3 -I train/check_card_forms.py
"""

from __future__ import annotations

import json
import sys
import tomllib
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CARDS_TOML = ROOT / "cards.toml"
FORMS = ROOT / "data" / "card_forms.json"

FIELDS = (
    "trigger", "effect", "target", "tribes", "scaling", "scales_with",
    "wants", "buff_attack", "buff_health",
)
LIST_FIELDS = ("trigger", "effect", "target", "tribes", "wants")
ALLOWED = {
    "trigger": {
        "battlecry", "deathrattle", "rally", "avenge", "start_of_combat",
        "end_of_turn", "start_of_turn", "on_buy", "on_sell", "on_play_friend",
        "on_summon_friend", "on_cast_spell", "on_gold_spent", "on_attack",
        "on_damaged", "on_kill", "activated", "aura", "passive", "on_event",
        "cast", "none",
    },
    "effect": {
        "buff_stats", "buff_self", "give_keyword", "summon", "generate_card",
        "discover", "gain_gold", "refund", "buff_shop", "buff_hand", "gain_gem",
        "deal_damage", "magnetize", "copy", "transform", "trigger_again",
        "reduce_cost", "refresh_shop", "freeze", "heal_or_armor", "other",
    },
    "target": {
        "self", "adjacent", "left", "right", "random_friend", "all_friends",
        "chosen", "shop_minion", "shop_all", "hand", "hero", "enemy", "none",
    },
    "tribes": {
        "beast", "mech", "dragon", "murloc", "demon", "pirate", "elemental",
        "undead", "quilboar", "aberration", "any_tribe", "none",
    },
    "scaling": {"none", "one_time", "linear", "compounding"},
    "scales_with": {"none", "turns", "events", "count"},
    "wants": {
        "deathrattles", "battlecries", "rally", "spells", "selling", "buying",
        "summons", "gold", "shop_buffs", "gems", "keywords", "divine_shields",
        "one_big_minion", "many_minions", "same_tribe", "position_left",
        "position_next_to", "none",
    },
}


def card_names(path=CARDS_TOML):
    data = tomllib.loads(path.read_text())
    return [card["name"] for part in ("minion", "token", "spell")
            for card in data.get(part, [])]


def main():
    names = card_names()
    forms = json.loads(FORMS.read_text())
    errors = []

    expected, seen = set(names), set(forms)
    for name in sorted(expected - seen):
        errors.append(f"missing card: {name}")
    for name in sorted(seen - expected):
        errors.append(f"extra card: {name}")
    if len(names) != len(expected):
        errors.append("duplicate card name in cards.toml")

    counts = {field: Counter() for field in FIELDS}
    for name in names:
        form = forms.get(name)
        if form is None:
            continue
        if not isinstance(form, dict):
            errors.append(f"{name}: form is not an object")
            continue
        if set(form) != set(FIELDS):
            errors.append(f"{name}: fields {sorted(form)}")
            continue
        for field in FIELDS:
            value = form[field]
            if field in LIST_FIELDS:
                if not isinstance(value, list) or not value:
                    errors.append(f"{name}: {field} is not a non-empty list")
                    continue
                if len(value) != len(set(value)):
                    errors.append(f"{name}: {field} repeats a value")
                for item in value:
                    if item not in ALLOWED[field]:
                        errors.append(f"{name}: {field} has {item!r}")
                    counts[field][item] += 1
            elif field in ("buff_attack", "buff_health"):
                if not isinstance(value, int) or isinstance(value, bool) or value < 0:
                    errors.append(f"{name}: {field} is not a non-negative integer")
                    continue
                counts[field][value] += 1
            else:
                if not isinstance(value, str):
                    errors.append(f"{name}: {field} is not a string")
                    continue
                if value not in ALLOWED[field]:
                    errors.append(f"{name}: {field} has {value!r}")
                counts[field][value] += 1
        scaling = form.get("scaling")
        scales_with = form.get("scales_with")
        if scaling in ("none", "one_time") and scales_with != "none":
            errors.append(f"{name}: scales_with {scales_with!r} with scaling {scaling!r}")
        if scaling in ("linear", "compounding") and scales_with == "none":
            errors.append(f"{name}: scales_with none with scaling {scaling!r}")

    for field in FIELDS:
        print(f"{field}:")
        for value, count in sorted(counts[field].items(), key=lambda item: str(item[0])):
            print(f"  {str(value):16} {count}")
    print(f"cards: {len(names)}, forms: {len(forms)}")
    if errors:
        for error in errors:
            print(f"ERROR: {error}")
        print(f"FAILED: {len(errors)} errors")
        return 1
    print("OK: every card has a valid form")
    return 0


if __name__ == "__main__":
    sys.exit(main())
