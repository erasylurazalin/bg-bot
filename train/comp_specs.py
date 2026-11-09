"""The comp library for evaluation: names, enabler groups and ending checks.

`ppo.py` and `eval_overlay.py` share this so a game's final board is judged
the same way by training's wheels-off line and by the overlay eval.
"""

import tomllib
from pathlib import Path


def load_comps(path):
    """Every comp's enabler groups, core and payoff/filler names."""
    data = tomllib.loads(Path(path).read_text())
    out = []
    for c in data.get("comp", []):
        groups = [list(e.get("all", [])) for e in c.get("enabler", [])]
        names = sorted({name for entry in groups for group in entry for name in group})
        out.append({
            "name": c.get("name", "?"),
            "enabler_groups": groups,
            "enabler_names": names,
            "core": [core["name"] for core in c.get("core", [])],
            "payoff": c.get("payoff", []),
            "filler": c.get("filler", []),
        })
    return out


def count(path):
    """The number of comps in the file."""
    return len(tomllib.loads(Path(path).read_text()).get("comp", []))


def health_penalty(path):
    """`[defaults] health_loss_penalty`, the overlay's negative bonus."""
    data = tomllib.loads(Path(path).read_text())
    return float(data.get("defaults", {}).get("health_loss_penalty", 0.0))


def names_to_ids(names, wanted):
    out = []
    for name in wanted:
        if name not in names:
            raise SystemExit(f"card {name!r} is not in the card set")
        out.append(names.index(name))
    return out


def ids(specs, names):
    """The loaded specs with card ids resolved against `names`."""
    return [
        {
            **spec,
            "core_ids": names_to_ids(names, spec["core"]),
            "enabler_groups_ids": [
                [names_to_ids(names, group) for group in entry]
                for entry in spec["enabler_groups"]
            ],
            "payoff_ids": names_to_ids(names, spec["payoff"]),
            "filler_ids": names_to_ids(names, spec["filler"]),
        }
        for spec in specs
    ]


def ends_as_comp(final, spec):
    """A final board that is the comp: one enabler entry complete and at
    least two of its core cards."""
    final_set = set(final)
    if not spec["enabler_groups_ids"]:
        return False
    complete = any(
        all(any(card in final_set for card in group) for group in entry)
        for entry in spec["enabler_groups_ids"]
    )
    return complete and sum(1 for card in spec["core_ids"] if card in final_set) >= 2
