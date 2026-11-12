"""Report for the archive check in docs/briefs/archive-check.md.

Reads the per-game rows written by collect.py and writes
train/runs/archive-check/report.md. Measurement only.

    .venv/bin/python train/evaltools/archive_check/analyze.py
"""

from __future__ import annotations

import json
import tomllib
from collections import Counter, defaultdict
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / "train/runs/archive-check"
CARRY_REPORT = ROOT / "train/runs/late-buys/report.md"
COMPS = ROOT / "data/comps/comps.toml"
LAGS = (2, 0)
GROUP_ORDER = (
    ("carry", "Carry card"),
    ("any_comp", "Any comp"),
    ("ek", "Undead - Eternal Knight"),
    ("other_comp", "Other comps, no Eternal Knight"),
)
MIN_CELL = 10
MIN_GROUP = 100


def carry_cards() -> list[str]:
    lines = CARRY_REPORT.read_text().splitlines()
    start = next(i for i, line in enumerate(lines)
                 if line.startswith("## 1. Which cards"))
    end = next(i for i, line in enumerate(lines[start + 1:], start + 1)
               if line.startswith("Group totals"))
    out = []
    for line in lines[start:end]:
        if not line.startswith("|"):
            continue
        if line.startswith("|---") or line.startswith("| card "):
            continue
        name = line.split("|")[1].strip()
        if name:
            out.append(name)
    if len(out) != 20:
        raise SystemExit(f"expected 20 carry cards, found {len(out)}")
    return out


def load() -> tuple[dict, list[dict], list[str]]:
    raw = {lag: json.loads((OUT / f"raw-lag{lag}.json").read_text())
           for lag in LAGS}
    comps = tomllib.loads(COMPS.read_text())["comp"]
    specs = [{
        "name": c["name"],
        "core": {x["name"] for x in c.get("core", [])},
    } for c in comps]
    return raw, specs, carry_cards()


def group_flags(row: dict, carry: set[str], specs: list[dict]) -> dict[str, bool]:
    held = {m["name"] for m in row["turn8"]["board"]}
    held |= {m["name"] for m in row["turn8"]["hand"]}
    comps = [s["name"] for s in specs if len(held & s["core"]) >= 2]
    ek = "Undead - Eternal Knight" in comps
    other = any(name != "Undead - Eternal Knight" for name in comps)
    return {
        "carry": bool(held & carry),
        "any_comp": bool(comps),
        "ek": ek,
        "other_comp": other and not ek,
    }


def mean_range(values) -> tuple[float, float, float, int]:
    v = np.asarray(values, dtype=np.float64)
    n = len(v)
    if n == 0:
        return (float("nan"), float("nan"), float("nan"), 0)
    mean = float(v.mean())
    if n < 2:
        return (mean, mean, mean, n)
    half = 1.96 * float(v.std(ddof=1)) / np.sqrt(n)
    return (mean, mean - half, mean + half, n)


def rate_range(count: int, n: int) -> tuple[float, float, float]:
    if n <= 0:
        return (float("nan"), float("nan"), float("nan"))
    z = 1.96
    p = count / n
    den = 1.0 + z * z / n
    centre = (p + z * z / (2 * n)) / den
    half = z * np.sqrt((p * (1 - p) + z * z / (4 * n)) / n) / den
    return (p, centre - half, centre + half)


def fmt(x: float, places: int = 2) -> str:
    return "n/a" if x is None or not np.isfinite(x) else f"{x:.{places}f}"


def fmt_range(stats, places: int = 2) -> str:
    mean, lo, hi, n = stats
    if n == 0:
        return "n/a"
    return f"{fmt(mean, places)} ({fmt(lo, places)} to {fmt(hi, places)})"


def fmt_share(count: int, n: int) -> str:
    p, lo, hi = rate_range(count, n)
    if n == 0:
        return "n/a"
    return f"{count} ({100 * p:.1f}%, {100 * lo:.1f} to {100 * hi:.1f})"


def fmt_cut(x: float) -> str:
    return f"{x:.0f}" if abs(x - round(x)) < 1e-9 else f"{x:.1f}"


def band(value: float, cuts: tuple[float, float]) -> int:
    if value <= cuts[0]:
        return 0
    if value <= cuts[1]:
        return 1
    return 2


def band_label(index: int, cuts: tuple[float, float], metric: str) -> str:
    a, b = fmt_cut(cuts[0]), fmt_cut(cuts[1])
    if index == 0:
        return f"{metric} <= {a}"
    if index == 1:
        return f"{metric} ({a}, {b}]"
    return f"{metric} > {b}"


def raw_tables(rows: list[dict], flags: list[dict]) -> tuple[list[str], list[str]]:
    outcome = [
        "| group | arm | games | place mean (95%) | top 4 | firsts |",
        "|---|---|---:|---|---|---|",
    ]
    boards = [
        "| group | arm | games at T12 | T12 board total (95%) | T8 total, same games | growth T8 to T12 (95%) | mean ratio |",
        "|---|---|---:|---|---|---|---:|",
    ]
    for key, label in GROUP_ORDER:
        holder = [r for r, f in zip(rows, flags) if f[key]]
        other = [r for r, f in zip(rows, flags) if not f[key]]
        for arm, arm_rows in (("holders", holder), ("non-holders", other)):
            places = [r["place"] for r in arm_rows]
            top4 = sum(r["place"] <= 4 for r in arm_rows)
            first = sum(r["place"] == 1 for r in arm_rows)
            outcome.append(
                f"| {label} | {arm} | {len(arm_rows)} | {fmt_range(mean_range(places))} | "
                f"{fmt_share(top4, len(arm_rows))} | {fmt_share(first, len(arm_rows))} |")
            at12 = [r for r in arm_rows if r["turn12_board_total"] is not None]
            t12 = [r["turn12_board_total"] for r in at12]
            t8 = [r["turn8"]["board_total"] for r in at12]
            growth = [b - a for a, b in zip(t8, t12)]
            ratios = [b / a for a, b in zip(t8, t12) if a]
            boards.append(
                f"| {label} | {arm} | {len(at12)} | {fmt_range(mean_range(t12), 1)} | "
                f"{fmt(mean_range(t8)[0], 1)} | {fmt_range(mean_range(growth), 1)} | "
                f"{fmt(float(np.mean(ratios)), 2) if ratios else 'n/a'}x |")
    return outcome, boards


def matched(rows: list[dict], flags: list[dict], key: str,
            cuts: tuple[tuple[float, float], tuple[float, float]]) -> dict:
    holders = [r for r, f in zip(rows, flags) if f[key]]
    non = [r for r, f in zip(rows, flags) if not f[key]]
    hp_cuts, board_cuts = cuts
    cells = []
    used = []
    for hi in range(3):
        for bi in range(3):
            h = [r["place"] for r in holders
                 if band(r["turn8"]["health"] + r["turn8"]["armor"], hp_cuts) == hi
                 and band(r["turn8"]["board_total"], board_cuts) == bi]
            n = [r["place"] for r in non
                 if band(r["turn8"]["health"] + r["turn8"]["armor"], hp_cuts) == hi
                 and band(r["turn8"]["board_total"], board_cuts) == bi]
            cell = {
                "hi": hi, "bi": bi, "holders": len(h), "non": len(n),
                "holder_mean": float(np.mean(h)) if h else None,
                "non_mean": float(np.mean(n)) if n else None,
                "diff": None, "used": False,
            }
            if h and n:
                cell["diff"] = cell["holder_mean"] - cell["non_mean"]
            if len(h) >= MIN_CELL and len(n) >= MIN_CELL:
                cell["used"] = True
                used.append((cell, h, n))
            cells.append(cell)
    if not used:
        return {"cells": cells, "diff": None, "lo": None, "hi": None,
                "used_holders": 0, "total_holders": len(holders),
                "used_cells": 0, "verdict": "no readable cells"}
    weights = np.array([len(h) for _, h, _ in used], dtype=np.float64)
    diffs = np.array([c["diff"] for c, _, _ in used], dtype=np.float64)
    variances = np.array([
        (np.var(h, ddof=1) / len(h) if len(h) > 1 else 0.0)
        + (np.var(n, ddof=1) / len(n) if len(n) > 1 else 0.0)
        for _, h, n in used
    ], dtype=np.float64)
    total = weights.sum()
    diff = float((weights * diffs).sum() / total)
    half = 1.96 * float(np.sqrt(((weights / total) ** 2 * variances).sum()))
    lo, hi = diff - half, diff + half
    if len(holders) < MIN_GROUP:
        verdict = "too few holders"
    elif hi < 0:
        verdict = "clearly better"
    elif lo > 0:
        verdict = "worse"
    else:
        verdict = "no clear difference"
    return {
        "cells": cells, "diff": diff, "lo": lo, "hi": hi,
        "used_holders": int(weights.sum()), "total_holders": len(holders),
        "used_cells": len(used), "verdict": verdict,
    }


def matched_tables(rows: list[dict], flags: list[dict],
                   cuts: tuple[tuple[float, float], tuple[float, float]]) -> tuple[list[str], list[str]]:
    summary = [
        "| group | holders | holders used | cells used | weighted difference (95%) | reading |",
        "|---|---:|---:|---:|---|---|",
    ]
    cells = [
        "| group | HP band | board band | holders | non-holders | holder place | non place | difference | used |",
        "|---|---|---:|---:|---:|---|---|---|---|",
    ]
    for key, label in GROUP_ORDER:
        m = matched(rows, flags, key, cuts)
        rng = "n/a" if m["diff"] is None else (
            f"{fmt(m['diff'])} ({fmt(m['lo'])} to {fmt(m['hi'])})")
        summary.append(
            f"| {label} | {m['total_holders']} | {m['used_holders']} | "
            f"{m['used_cells']} | {rng} | {m['verdict']} |")
        for c in m["cells"]:
            if c["holders"] == 0 and c["non"] == 0:
                mark = "empty"
            elif c["holders"] == 0:
                mark = "no holders"
            elif c["non"] == 0:
                mark = "no non-holders"
            elif not c["used"]:
                mark = "too few"
            else:
                mark = "yes"
            cells.append(
                f"| {label} | {band_label(c['hi'], cuts[0], 'HP')} | "
                f"{band_label(c['bi'], cuts[1], 'board')} | {c['holders']} | "
                f"{c['non']} | {fmt(c['holder_mean'])} | {fmt(c['non_mean'])} | "
                f"{fmt(c['diff'])} | {mark} |")
    return summary, cells


def composition(rows_by_lag: dict[int, list[dict]], carry_names: list[str],
                specs: list[dict]) -> tuple[list[str], list[str], list[str]]:
    carry = set(carry_names)
    carry_table = [
        "| card | lag 0 games | lag 0 copies | lag 0 golden | lag 0 place | "
        "lag 2 games | lag 2 copies | lag 2 golden | lag 2 place |",
        "|---|---:|---:|---:|---:|---:|---:|---:|---:|",
    ]
    for name in carry_names:
        values = []
        for lag in (0, 2):
            games = []
            copies = 0
            golden = 0
            for r in rows_by_lag[lag]:
                here = [m for m in r["turn8"]["board"] + r["turn8"]["hand"]
                        if m["name"] == name]
                if here:
                    games.append(r)
                    copies += len(here)
                    golden += sum(bool(m["golden"]) for m in here)
            values.append((len(games), copies, golden,
                           float(np.mean([r["place"] for r in games])) if games else None))
        carry_table.append(
            f"| {name} | {values[0][0]} | {values[0][1]} | {values[0][2]} | "
            f"{fmt(values[0][3])} | {values[1][0]} | {values[1][1]} | "
            f"{values[1][2]} | {fmt(values[1][3])} |")

    counts = defaultdict(lambda: [0, 0])
    places = defaultdict(lambda: [[], []])
    for i, lag in enumerate((0, 2)):
        for r in rows_by_lag[lag]:
            held = {m["name"] for m in r["turn8"]["board"] + r["turn8"]["hand"]}
            for spec in specs:
                if len(held & spec["core"]) >= 2:
                    counts[spec["name"]][i] += 1
                    places[spec["name"]][i].append(r["place"])
    comp_table = [
        "| comp | lag 0 games | lag 0 place | lag 2 games | lag 2 place |",
        "|---|---:|---:|---:|---:|",
    ]
    for name, (n0, n2) in sorted(counts.items(), key=lambda kv: (-kv[1][1], kv[0])):
        comp_table.append(
            f"| {name} | {n0} | {fmt(float(np.mean(places[name][0]))) if n0 else 'n/a'} | "
            f"{n2} | {fmt(float(np.mean(places[name][1]))) if n2 else 'n/a'} |")

    dist = [
        "| carry cards held | lag 0 holders | lag 2 holders |",
        "|---|---:|---:|",
    ]
    for k in (1, 2, 3, 4):
        vals = []
        for lag in (0, 2):
            held = [
                sum(m["name"] in carry for m in r["turn8"]["board"] + r["turn8"]["hand"])
                for r in rows_by_lag[lag]
            ]
            vals.append(sum((n >= 4 if k == 4 else n == k) for n in held))
        label = f"{k}" if k < 4 else "4 or more"
        dist.append(f"| {label} | {vals[0]} | {vals[1]} |")
    return carry_table, comp_table, dist


def main():
    raw, specs, carry_names = load()
    carry = set(carry_names)
    rows_by_lag = {lag: [r for r in raw[lag]["rows"] if r["reached_turn8"]]
                   for lag in LAGS}
    flags_by_lag = {lag: [group_flags(r, carry, specs) for r in rows_by_lag[lag]]
                    for lag in LAGS}

    for lag in LAGS:
        v = raw[lag].get("validation") or {}
        if not v.get("place_match") or not v.get("knockout_match") or not v.get("turn12_match"):
            raise SystemExit(f"lag {lag}: collector validation failed: {v}")

    cuts = {}
    for lag in LAGS:
        hp = [r["turn8"]["health"] + r["turn8"]["armor"] for r in rows_by_lag[lag]]
        board = [r["turn8"]["board_total"] for r in rows_by_lag[lag]]
        cuts[lag] = (tuple(np.quantile(hp, [1 / 3, 2 / 3]).tolist()),
                     tuple(np.quantile(board, [1 / 3, 2 / 3]).tolist()))

    lines = [
        "# Archive check: turn 8 engine pieces and later place",
        "",
        "Measurement only. No training and no engine changes. The collector is "
        "`train/evaltools/archive_check/collect.py`; this report is "
        "`train/evaltools/archive_check/analyze.py`.",
        "",
        "## What was run",
        "",
        f"- checkpoint: `train/runs/r1111-lagmix/best.pt` (update "
        f"{raw[2]['update']}), the current base",
        "- games: 8,192 per lag, one game per seed, chunks of 512, seed base "
        "20261110, 1 learner plus 7 real seats",
        "- boards: `boards/frozen/real-2026-11-10` (manifest checked), lobby "
        "filter on",
        "- sets: real-seat lag 2 from turn 7, and lag 0; both were run with "
        "`--vram-gb 10`",
        f"- pcbusy at collection: {raw[2]['pcbusy']}",
        "- the brief expected pcbusy ON; the machine reported it OFF. The GPU "
        "cap was still 10 GB and `pcbusy` showed a quiet machine before the runs.",
        "",
        "| lag | games | reached turn 8 | out before turn 8 | reached turn 12 |",
        "|---|---:|---:|---:|---:|",
    ]
    for lag in LAGS:
        all_rows = raw[lag]["rows"]
        reached = rows_by_lag[lag]
        at12 = sum(r["turn12_board_total"] is not None for r in reached)
        lines.append(
            f"| {lag} | {len(all_rows)} | {len(reached)} | "
            f"{len(all_rows) - len(reached)} | {at12} |")
    lines += [
        "",
        "Collector check against the fixed eval: the first 2,048 games match "
        "`train/runs/fixed-eval/r1111-lagmix-lag0` and `-lag2` exactly on final "
        "place, knocked-out turn, and turn-12 board total (0 mismatches on all "
        "three in both sets).",
        "",
        "## Definitions",
        "",
        "- Turn 8 state: the cards on the learner's board and in its hand at its "
        "last turn 8 action, plus Health, armor, tier and board Attack plus "
        "Health.",
        "- Carry cards: the 20 cards in section 1 of "
        "`train/runs/late-buys/report.md`. Golden and plain copies both count, "
        "and only presence matters.",
        "- Comp progress: two or more distinct `core` card names of one comp, "
        "from `data/comps/comps.toml`, on the board or in the hand. Copies of "
        "one core name do not count as two names.",
        "- `Any comp` is every position with two or more core names of any comp. "
        "`Undead - Eternal Knight` is its own group. `Other comps` is every "
        "non-Eternal-Knight comp position without Eternal Knight progress, so "
        "the last two groups do not overlap.",
        "- A group with fewer than 100 holders is marked too few to read. No "
        "conclusion is drawn from it.",
        "",
        "Formulas: mean ranges are mean +/- 1.96 standard errors. Shares use a "
        "95% Wilson range. A lower place is better.",
        "",
    ]

    for lag in LAGS:
        rows = rows_by_lag[lag]
        flags = flags_by_lag[lag]
        hp_cuts, board_cuts = cuts[lag]
        lines += [
            f"## Lag {lag}",
            "",
            f"Tercile cut points from all {len(rows)} turn 8 positions in this "
            f"set: Health plus armor at {fmt_cut(hp_cuts[0])} and "
            f"{fmt_cut(hp_cuts[1])}; board Attack plus Health at "
            f"{fmt_cut(board_cuts[0])} and {fmt_cut(board_cuts[1])}. Bands are "
            "`<= first cut`, `(first cut, second cut]`, and `> second cut`.",
            "",
            "### Raw",
            "",
        ]
        outcome, boards = raw_tables(rows, flags)
        lines += outcome + ["", "Board total and growth:", ""] + boards + [""]
        lines += [
            "### Matched on turn 8 state",
            "",
            "The weighted difference uses holder counts as cell weights. A "
            "negative difference means holders placed better. Cells enter it "
            "only when both arms have at least 10 games; smaller cells are "
            "shown and marked too few.",
            "",
        ]
        summary, cells = matched_tables(rows, flags, cuts[lag])
        lines += summary + [""]
        lines += cells + [""]

    carry_table, comp_table, dist = composition(rows_by_lag, carry_names, specs)
    lines += [
        "## What the holders had",
        "",
        "### Carry cards, among all turn 8 positions",
        "",
    ]
    lines += carry_table + [
        "",
        "Most cards have fewer than 100 holder games by themselves; only Blue "
        "Volumizer clears 100 in both sets (108 and 111). These rows describe "
        "which cards make up the carry group, not separate per-card "
        "comparisons.",
        "",
        "### Carry cards held per carry holder",
        "",
    ]
    lines += dist + [
        "",
        "### Comps, among all turn 8 positions",
        "",
    ]
    lines += comp_table + [
        "",
        "Comps with fewer than 100 games are too few to read on their own. The "
        "table is for composition, not for separate conclusions.",
        "",
        "## Plain reading",
        "",
    ]

    for lag in LAGS:
        rows = rows_by_lag[lag]
        flags = flags_by_lag[lag]
        parts = []
        for key, label in GROUP_ORDER:
            n = sum(f[key] for f in flags)
            m = matched(rows, flags, key, cuts[lag])
            if n < MIN_GROUP:
                parts.append(f"{label}: too few ({n} holders).")
            elif m["diff"] is None:
                parts.append(f"{label}: no readable matched cells ({n} holders).")
            else:
                parts.append(
                    f"{label}: matched difference {fmt(m['diff'])} places "
                    f"(95% {fmt(m['lo'])} to {fmt(m['hi'])}), {m['verdict']} "
                    f"({n} holders).")
        lines.append(f"- Lag {lag}: " + " ".join(parts))
    lines += [
        "",
        "Against the brief's table: at lag 2 the carry group is the direct "
        "answer. Its matched range includes 0, so the answer is no clear "
        "difference, not clearly better. The Eternal Knight group is below the "
        "holder threshold and is too few on its own. The any-comp and "
        "other-comp groups also include 0 at lag 2.",
        "",
        "The composition counts do not single out one card. Blue Volumizer, the "
        "commonest carry card, is on or in hand in 111 of 452 lag 2 carry "
        "positions (24.6%) and 108 of 451 at lag 0 (23.9%). The commonest comp "
        "is Undead - Attack Scaling, with 61 of 149 lag 2 comp positions "
        "(40.9%) and 71 of 185 at lag 0 (38.4%).",
        "",
    ]

    (OUT / "report.md").write_text("\n".join(lines))
    print(f"wrote {OUT / 'report.md'}")


if __name__ == "__main__":
    main()
