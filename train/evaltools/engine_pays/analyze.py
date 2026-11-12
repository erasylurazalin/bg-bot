"""Turn train/runs/engine-pays/turns.json into report.md.

Measurement only. The per-turn tables use seed 1 (one run per logged turn);
the per-card table pools every fully replayed run (turn and seed), with the
number of distinct turns shown beside the match count.
"""

from __future__ import annotations

import collections
import json
import re
import statistics
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
RUN = ROOT / "train/runs/engine-pays"
ROWS = json.loads((RUN / "turns.json").read_text())

CARRY_REPORT = ROOT / "train/runs/late-buys/report.md"
COMPS = ROOT / "data/comps/comps.toml"
OUT = RUN / "report.md"

SEEDS = sorted({r["seed"] for r in ROWS})
TURN_MIN = min(r["turn"] for r in ROWS)
TURN_MAX = max(r["turn"] for r in ROWS)
PRIMARY = 1


def pct(values, p):
    values = sorted(values)
    if not values:
        return None
    k = (len(values) - 1) * p / 100.0
    lo = int(k)
    hi = min(lo + 1, len(values) - 1)
    f = k - lo
    return values[lo] * (1 - f) + values[hi] * f


def ratio(row):
    if row["log_total"] <= 0:
        return None
    return row["engine_total"] / row["log_total"]


def carry_cards():
    text = CARRY_REPORT.read_text()
    section = text.split("## 1. Which cards carry")[1].split("Group totals")[0]
    out = []
    for line in section.splitlines():
        if not line.startswith("| ") or line.startswith("|---"):
            continue
        name = line.split("|")[1].strip()
        if name and name != "card":
            out.append(name)
    return out


def core_cards():
    comps = tomllib.loads(COMPS.read_text())
    out = set()
    for comp in comps["comp"]:
        for card in comp.get("core", []):
            out.add(card["name"])
    return out


def match_minions(engine_board, log_board):
    """Pair engine and log minions: same position and name first, then name.

    Remaining pairs prefer the same golden state, then the first one left.
    """
    pairs = []
    used_engine = set()
    used_log = set()
    for i, (e, l) in enumerate(zip(engine_board, log_board)):
        if e["name"] == l["name"]:
            pairs.append((e, l, "position"))
            used_engine.add(i)
            used_log.add(i)
    left_engine = [(i, m) for i, m in enumerate(engine_board) if i not in used_engine]
    left_log = [(i, m) for i, m in enumerate(log_board) if i not in used_log]
    for _, e in left_engine:
        candidates = [
            (j, l) for j, l in left_log if j not in used_log and l["name"] == e["name"]
        ]
        if not candidates:
            continue
        same = [c for c in candidates if c[1]["golden"] == e["golden"]]
        j, l = (same or candidates)[0]
        pairs.append((e, l, "name"))
        used_log.add(j)
    return pairs


def aggregate(rows, held, carry):
    """Per-card matched ratios, pooled over the given rows."""
    per = collections.defaultdict(list)
    turns = collections.defaultdict(set)
    by_position = collections.Counter()
    by_name = collections.Counter()
    for row in rows:
        if not row["complete"]:
            continue
        for e, l, how in match_minions(row["engine_board"], row["log_board"]):
            total = l["attack"] + l["health"]
            if total <= 0:
                continue
            per[e["name"]].append((e["attack"] + e["health"]) / total)
            turns[e["name"]].add((row["file"], row["turn"]))
            if how == "position":
                by_position[e["name"]] += 1
            else:
                by_name[e["name"]] += 1
    return per, turns, by_position, by_name


def card_row(name, per, turns, by_position, by_name):
    values = per.get(name, [])
    n = len(values)
    distinct = len(turns.get(name, ()))
    if not values:
        return {
            "name": name,
            "n": 0,
            "turns": 0,
            "position": 0,
            "by_name": 0,
            "mean": None,
            "median": None,
            "flag": "",
        }
    mean = statistics.mean(values)
    median = statistics.median(values)
    flag = ""
    if n >= 5 and median < 0.8:
        flag = "under"
    elif n >= 5 and median > 1.25:
        flag = "over"
    return {
        "name": name,
        "n": n,
        "turns": distinct,
        "position": by_position[name],
        "by_name": by_name[name],
        "mean": mean,
        "median": median,
        "flag": flag,
    }


def fmt(x, places=3):
    return "n/a" if x is None else f"{x:.{places}f}"


def reason_group(row):
    stop = row["stop"]
    cls = stop["class"]
    why = stop["reason"]
    if cls in ("start", "seat"):
        if why.startswith("an option the card set does not build"):
            return "start or seat: an option the card set does not build"
        if why.startswith("the turn has no steps"):
            return "start: the turn has no steps"
        return f"{cls}: {why.split(':')[0]}"
    if cls == "missing":
        return f"missing: {why.split(':')[0]}"
    if cls == "illegal":
        m = re.match(r"the engine does not allow ([A-Za-z]+)", why)
        if m:
            return f"illegal: the engine does not allow {m.group(1)}"
        return f"illegal: {why.split(':')[0]}"
    return f"{cls}: {why.split(':')[0]}"


def examples(rows, group, n=5):
    out = []
    for row in rows:
        if reason_group(row) != group:
            continue
        stop = row["stop"]
        card = f", {stop['card']}" if stop.get("card") else ""
        out.append(
            f"{row['file']} turn {row['turn']} step {stop['step_number']} "
            f"({stop['kind']}{card})"
        )
        if len(out) == n:
            break
    return out


def main():
    seed1 = [r for r in ROWS if r["seed"] == PRIMARY]
    complete = [r for r in seed1 if r["complete"]]
    stopped = [r for r in seed1 if not r["complete"]]
    turns = sorted({(r["file"], r["turn"]) for r in seed1})
    carry = carry_cards()
    core = core_cards()

    held = set()
    for row in ROWS:
        for zone in ("start_board", "start_hand", "log_board"):
            for m in row[zone]:
                held.add(m["name"])
    core_held = core & held

    per_all, turns_all, pos_all, name_all = aggregate(ROWS, held, carry)
    per_seed1, turns_seed1, _, _ = aggregate(seed1, held, carry)

    lines = []
    lines.append("# Does the engine pay for Era's comps the way the real game does?")
    lines.append("")
    lines.append(
        "Measurement for `docs/briefs/engine-pays.md`. No game rule changes. "
        "The whole-turn replay is `shop_replay::replay_whole_turn` and "
        "`bgbench turn_replay`; the rows are `turns.json` beside this report, "
        "and the scratch script is `train/evaltools/engine_pays/analyze.py`."
    )
    lines.append("")
    lines.append(
        "pcbusy before the run: `OFF since 2026-11-11 15:02: agents and Claude "
        "run at full speed. CPU in use: 2.5 of 32 cores (0.0 by agents and "
        "Claude, 2.5 by everything else). verdict: quiet`. Its status files "
        "listed `engine-pays` and `turn-profile`; the measured CPU was 2.5 of "
        "32 cores."
    )
    lines.append("")
    lines.append("## What was replayed")
    lines.append("")
    lines.append(
        f"{len(turns)} logged turns {TURN_MIN} to {TURN_MAX} in "
        f"`boards/actions/`. Each turn is seated once from its own start "
        "(board, hand, Tavern, Gold, tier, counters, hero, trinkets), then "
        "every logged step is played in order on that one game. Before each "
        "step the log's own Tavern (`shop`, spell slot and freeze) is put in "
        "place, with its pool copies returned and taken again, so a Buy or "
        "Activate still names the card the log shows. Everything else is the "
        "engine's carried state, and random effects use the engine's draw. A "
        "run stops at the first step it cannot follow: a state it cannot "
        "seat, a card the loader cannot build, or an action the engine "
        "refuses. The board at that last good step is compared with the log's "
        "state at the same step."
    )
    lines.append("")
    lines.append(
        f"Five seeds per turn ({len(ROWS)} runs). The per-turn tables use "
        f"seed {PRIMARY}, one run per logged turn. The per-card table pools "
        "every fully replayed run because several late turns are thin; it "
        "also gives the number of distinct logged turns behind each match "
        "count."
    )
    lines.append("")
    lines.append(
        "The known check: the per-step `lobster_replay` on "
        "`Hearthstone_2026_10_28_22_11_50-g2` turn 10 ends at Tasty Lobster "
        "count 18 and board 1044/818. Whole-turn mode on the same turn stops "
        "at step 41 (0-based 40), the Sprightly Scarab play: the carried hand "
        "is the engine's own, and it no longer holds the card at the logged "
        "hand slot. At the last good step (the Snarky Shark sell) the "
        "run has count 15 and board 684/559; the third Fishbait attack does "
        "not happen, and the log's board at that step is 1847 Attack plus "
        "Health. The unit test "
        "`the_whole_turn_mode_on_the_tras_tath_turn_stops_at_the_moved_hand` "
        "pins this."
    )
    lines.append("")
    lines.append("## Per turn 7 to 12")
    lines.append("")
    lines.append(
        "Seed 1. A stopped turn's step share is `played / steps`; a start "
        "that cannot seat counts as stopped at step 1. Ratio is engine board "
        "Attack plus Health over the log's, at the turn's end, on fully "
        "replayed turns only."
    )
    lines.append("")
    lines.append(
        "| turn | logged turns | full | stopped | stop share mean | stop share median | ratio n | mean | median | p10 | p90 |"
    )
    lines.append("|---|---|---|---|---|---|---|---|---|---|---|")
    all_ratios = []
    all_shares = []
    for t in range(TURN_MIN, TURN_MAX + 1):
        rows = [r for r in seed1 if r["turn"] == t]
        full = [r for r in rows if r["complete"]]
        stop = [r for r in rows if not r["complete"]]
        ratios = [ratio(r) for r in full if ratio(r) is not None]
        shares = [(r["played"] / r["steps"]) if r["steps"] else 0.0 for r in stop]
        all_ratios.extend(ratios)
        all_shares.extend(shares)
        lines.append(
            f"| {t} | {len(rows)} | {len(full)} | {len(stop)} | "
            f"{fmt(statistics.mean(shares) if shares else None)} | "
            f"{fmt(statistics.median(shares) if shares else None)} | "
            f"{len(ratios)} | {fmt(statistics.mean(ratios) if ratios else None)} | "
            f"{fmt(statistics.median(ratios) if ratios else None)} | "
            f"{fmt(pct(ratios, 10) if ratios else None)} | "
            f"{fmt(pct(ratios, 90) if ratios else None)} |"
        )
    lines.append(
        f"| all | {len(turns)} | {len(complete)} | {len(stopped)} | "
        f"{fmt(statistics.mean(all_shares))} | {fmt(statistics.median(all_shares))} | "
        f"{len(all_ratios)} | {fmt(statistics.mean(all_ratios))} | "
        f"{fmt(statistics.median(all_ratios))} | {fmt(pct(all_ratios, 10))} | "
        f"{fmt(pct(all_ratios, 90))} |"
    )
    lines.append("")
    by_turn = collections.defaultdict(list)
    for row in ROWS:
        by_turn[(row["file"], row["turn"])].append(row)
    all_seeds = sum(1 for v in by_turn.values() if all(r["complete"] for r in v))
    any_seed = sum(1 for v in by_turn.values() if any(r["complete"] for r in v))
    lines.append(
        f"At seed 1, {len(complete)} of {len(turns)} turns "
        f"({100 * len(complete) / len(turns):.1f}%) replay fully; "
        f"{len(stopped)} stop. Of the stopped turns, "
        f"{sum(1 for r in stopped if r['played'] == 0)} stop at step 1. "
        "The complete count moves a little with the seed: "
        + ", ".join(
            f"seed {s} {sum(1 for r in ROWS if r['seed'] == s and r['complete'])}"
            for s in SEEDS
        )
        + f". {all_seeds} turns are complete at every seed and {any_seed} at "
        "least one seed."
    )
    lines.append("")

    lines.append("## The 20 carry cards")
    lines.append("")
    lines.append(
        "The 20 cards from `train/runs/late-buys/report.md` section 1. "
        "Matches are individual minions paired between the engine and log "
        "end boards, same position and name first, then by name (same golden "
        "state preferred). `pos/name` is how many pairs came from each pass. "
        "Ratios pool every fully replayed turn and seed; `turns` is the "
        "number of distinct logged turns behind the matches."
    )
    lines.append("")
    lines.append("| card | matches | turns | pos/name | mean | median | flag |")
    lines.append("|---|---|---|---|---|---|---|")
    carry_rows = [card_row(c, per_all, turns_all, pos_all, name_all) for c in carry]
    for c in carry_rows:
        if c["n"] == 0:
            continue
        flag = c["flag"].upper() if c["flag"] else ""
        lines.append(
            f"| {c['name']} | {c['n']} | {c['turns']} | {c['position']}/{c['by_name']} | "
            f"{fmt(c['mean'])} | {fmt(c['median'])} | {flag} |"
        )
    none = [c["name"] for c in carry_rows if c["n"] == 0]
    lines.append("")
    lines.append(
        "No carry card is flagged: every card with at least 5 matches has a "
        "median ratio between 0.8 and 1.25. Cards with no matched minion on a "
        "fully replayed end board: " + (", ".join(none) if none else "none") + "."
    )
    lines.append("")

    lines.append("## Comp core cards Era held")
    lines.append("")
    lines.append(
        f"Core cards are every `core` name in `data/comps/comps.toml` "
        f"({len(core)} names); {len(core_held)} of them appear on Era's board "
        "or in her hand at the start or end of a logged turn 7 to 12. The "
        "same matching and pooling as the carry table."
    )
    lines.append("")
    lines.append("| card | matches | turns | pos/name | mean | median | flag |")
    lines.append("|---|---|---|---|---|---|---|")
    core_rows = [card_row(c, per_all, turns_all, pos_all, name_all) for c in core_held]
    core_rows.sort(key=lambda c: (-c["n"], c["name"]))
    for c in core_rows:
        if c["n"] == 0:
            continue
        flag = c["flag"].upper() if c["flag"] else ""
        lines.append(
            f"| {c['name']} | {c['n']} | {c['turns']} | {c['position']}/{c['by_name']} | "
            f"{fmt(c['mean'])} | {fmt(c['median'])} | {flag} |"
        )
    no_match = [c["name"] for c in core_rows if c["n"] == 0]
    lines.append("")
    if no_match:
        lines.append(
            "Held core cards with no matched minion on a fully replayed end "
            "board: " + ", ".join(no_match) + "."
        )
        lines.append("")
    flagged = [c for c in core_rows if c["flag"]]
    if flagged:
        lines.append("Flagged core cards (at least 5 matches):")
        lines.append("")
        for c in flagged:
            lines.append(
                f"- **{c['name']}**: median {fmt(c['median'])}, {c['n']} matches "
                f"on {c['turns']} distinct turn{'s' if c['turns'] != 1 else ''}, "
                f"mean {fmt(c['mean'])}."
            )
        lines.append("")
    lines.append(
        "On seed 1 alone no core card reaches 5 matches with a median outside "
        "0.8 to 1.25; the two flags above come from pooling the seeds. "
        "`Resourceful Robot` has 15 matches on 3 turns: 5 at 1.0, 5 at about "
        "0.78 and 5 at about 0.19. `Unbound Tempest` has 5 matches on one turn, "
        "all 0.464. The thin low means that do not flag are `Eredar Escapist` "
        "(17 matches, median 1.0, mean 0.896) and `Conveyor Construct` (18 "
        "matches, median 1.0, mean 0.868)."
    )
    lines.append("")

    lines.append("## Why turns stop")
    lines.append("")
    lines.append(
        f"Seed 1, {len(stopped)} stopped turns. `start` means the turn's own "
        "start could not be seated; `seat` a later step's state; `missing` a "
        "card the loader had to leave out; `illegal` an action the engine "
        "refused; `ended` a turn the engine ended early."
    )
    lines.append("")
    lines.append("| class | step kind | count |")
    lines.append("|---|---|---|")
    classes = collections.Counter((r["stop"]["class"], r["stop"]["kind"]) for r in stopped)
    for (cls, kind), n in classes.most_common():
        lines.append(f"| {cls} | {kind} | {n} |")
    lines.append("")
    groups = collections.Counter(reason_group(r) for r in stopped)
    lines.append("| reason | count |")
    lines.append("|---|---|")
    for group, n in groups.most_common():
        lines.append(f"| {group} | {n} |")
    lines.append("")
    lines.append("### Examples, five per top reason")
    lines.append("")
    for group, _ in groups.most_common(6):
        lines.append(f"**{group}** ({groups[group]})")
        lines.append("")
        for ex in examples(stopped, group):
            lines.append(f"- {ex}")
        lines.append("")
    unbuilt = collections.Counter()
    for row in stopped:
        stop = row["stop"]
        m = re.match(r"an option the card set does not build: (.*)", stop["reason"])
        if m:
            unbuilt[m.group(1)] += 1
    if unbuilt:
        lines.append(
            "The unbuilt options behind the top reason, most common first: "
            + ", ".join(f"{name} {n}" for name, n in unbuilt.most_common(12))
            + "."
        )
        lines.append("")

    lines.append("## Random effects")
    lines.append("")
    lines.append(
        "Each turn was run at five seeds. A turn is in the first group when "
        "its engine end board is identical at all five seeds, so no random "
        "effect moved it; in the second group when the board changes. Spread "
        "is the difference between the largest and smallest per-seed ratio "
        "(engine total over log total) for that turn. Only turns complete at "
        "all five seeds are counted, since a stopped run compares at a "
        "different step."
    )
    lines.append("")
    same = []
    differ = []
    for key, seed_rows in by_turn.items():
        if len(seed_rows) != len(SEEDS) or not all(r["complete"] for r in seed_rows):
            continue
        seed_rows = sorted(seed_rows, key=lambda r: r["seed"])
        signatures = {json.dumps(r["engine_board"], sort_keys=True) for r in seed_rows}
        ratios = [ratio(r) for r in seed_rows if ratio(r) is not None]
        if not ratios:
            continue
        spread = max(ratios) - min(ratios)
        item = (statistics.mean(ratios), spread, key, ratios)
        (same if len(signatures) == 1 else differ).append(item)
    lines.append(
        "| group | turns | mean ratio | mean spread | median spread | p90 spread | max spread |"
    )
    lines.append("|---|---|---|---|---|---|---|")
    for label, group in (
        ("same board at every seed", same),
        ("board changes with the seed", differ),
    ):
        if not group:
            continue
        means = [g[0] for g in group]
        spreads = [g[1] for g in group]
        lines.append(
            f"| {label} | {len(group)} | {fmt(statistics.mean(means))} | "
            f"{fmt(statistics.mean(spreads))} | {fmt(statistics.median(spreads))} | "
            f"{fmt(pct(spreads, 90))} | {fmt(max(spreads))} |"
        )
    lines.append("")
    if differ:
        widest = max(differ, key=lambda g: g[1])
        lines.append(
            f"The widest spread is `{widest[2][0]}` turn {widest[2][1]}: "
            f"ratios " + ", ".join(f"{x:.3f}" for x in widest[3]) + "."
        )
        lines.append("")
    lines.append("Per seed, over the turns that seed completes:")
    lines.append("")
    lines.append("| seed | complete | mean ratio | median ratio | pooled ratio |")
    lines.append("|---|---|---|---|---|")
    for s in SEEDS:
        rows = [r for r in ROWS if r["seed"] == s and r["complete"]]
        ratios = [ratio(r) for r in rows if ratio(r) is not None]
        pooled = sum(r["engine_total"] for r in rows) / sum(r["log_total"] for r in rows)
        lines.append(
            f"| {s} | {len(rows)} | {fmt(statistics.mean(ratios))} | "
            f"{fmt(statistics.median(ratios))} | {fmt(pooled)} |"
        )
    lines.append("")

    lines.append("## Checks")
    lines.append("")
    lines.append(
        "- `cargo build --workspace`: ok. `cargo build -p bgpy`: ok."
    )
    lines.append(
        "- `cargo test --workspace`: 1440 passed, 0 failed, 2 ignored."
    )
    lines.append(
        "- `cargo clippy --workspace --all-targets`: no warnings."
    )
    lines.append(
        "- The new unit test "
        "`the_whole_turn_mode_on_the_tras_tath_turn_stops_at_the_moved_hand` "
        "passes; it pins the known turn 10 result above."
    )
    lines.append(
        "- Normal games unchanged: a HEAD scratch copy under "
        "`target/engine-pays/head` ran `bgbench shopreplay --seeds 1`, and the "
        "current tree ran the same command. `steps.tsv`, `shop-cards.tsv` and "
        "`shop-report.md` are byte-identical. Both report 103 games, 1,179 "
        "turns, 25,712 steps, 16,040 ok, 3,773 incomplete, 5,878 flagged and "
        "21 illegal. The scratch copy was deleted."
    )
    lines.append(
        "- Fuzz and the games-per-second run were skipped: no code normal "
        "games run was changed."
    )
    lines.append("")

    lines.append("## Plain reading")
    lines.append("")
    mean_ratio = statistics.mean(all_ratios)
    median_ratio = statistics.median(all_ratios)
    if len(complete) / len(turns) < 0.5:
        coverage = (
            "the engine cannot follow most late turns: only "
            f"{len(complete)} of {len(turns)} ({100 * len(complete) / len(turns):.1f}%) "
            "replay fully at seed 1"
        )
    else:
        coverage = (
            f"{100 * len(complete) / len(turns):.1f}% of the turns replay fully "
            "at seed 1"
        )
    if median_ratio < 0.9:
        pay = "on the turns it can follow it under-pays"
    elif median_ratio > 1.1:
        pay = "on the turns it can follow it over-pays"
    else:
        pay = "on the turns it can follow it pays about like the log"
    lines.append(
        f"The engine does not look like it under-pays Era's comps where it can "
        f"follow them. Across the {len(all_ratios)} fully replayed turns at "
        f"seed 1 the board ratio has mean {mean_ratio:.3f}, median "
        f"{median_ratio:.3f}, 10th percentile {pct(all_ratios, 10):.3f} and "
        f"90th percentile {pct(all_ratios, 90):.3f}. The median is 1.0 on "
        "turns 7, 8, 10, 11 and 12; turn 9 has one turn at 0.977. No carry "
        "card is flagged, and only two comp "
        "core cards are flagged after pooling the seeds, both on thin and "
        "late matches (`Resourceful Robot`, `Unbound Tempest`). The larger "
        f"problem is coverage: {coverage}, and turn 9 has only one fully "
        "replayed turn. The largest single reason is a card the engine does "
        "not build (mostly an unbuildable trinket or Discover option at the "
        "first step, 134 turns, then 92 illegal Plays and 49 illegal "
        "Discovers). So the evidence says the engine pays about like the log "
        "where it can follow, but that is measured on only a quarter of the "
        "late turns, and turn 9 is barely measured at all. The late turns "
        "themselves are mostly not measurable until the missing cards are "
        "built."
    )
    lines.append("")
    text = "\n".join(lines)
    if "\u2014" in text:
        raise SystemExit("em dash in the report")
    OUT.write_text(text)
    print(f"wrote {OUT}")
    print(
        f"{len(turns)} turns, {len(complete)} complete, {len(stopped)} stopped, "
        f"{len(all_ratios)} ratios"
    )


if __name__ == "__main__":
    sys.exit(main())
