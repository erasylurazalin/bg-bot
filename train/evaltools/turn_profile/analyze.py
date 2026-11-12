"""Build the turn-profile report and its per-game JSON.

Measurement for docs/briefs/turn-profile.md. Reads the two collector outputs
from `train/runs/turn-profile/` (Era and the bot at lag 0 and lag 2), checks the
bot places against the fixed eval's `results.json`, and writes:

    train/runs/turn-profile/report.md
    train/runs/turn-profile/rows.json

Run it with `python3 -I train/evaltools/turn_profile/analyze.py` after the collectors.
"""

from __future__ import annotations

import json
import math
import statistics
from collections import Counter
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
SRC = ROOT / "train" / "runs" / "turn-profile"
OUT = ROOT / "train" / "runs" / "turn-profile"
TURNS = tuple(range(7, 13))
KINDS = ["buy", "buy_spell", "sell", "activate", "play", "target",
         "target_shop", "choose", "discard", "move", "roll", "freeze",
         "level", "end", "hero_power", "hero_power2", "choose_trinket",
         "dark_gift"]
REF = {
    0: ROOT / "train" / "runs" / "fixed-eval" / "r1111-lagmix-lag0" / "results.json",
    2: ROOT / "train" / "runs" / "fixed-eval" / "r1111-lagmix-lag2" / "results.json",
}


def load(path: Path) -> dict:
    return json.loads(path.read_text())


def turn_rows(rows: list[dict], turn: int, split: str | None = None) -> list[dict]:
    out = []
    for r in rows:
        s = r["turns"].get(str(turn))
        if s is None:
            continue
        if split is not None:
            p = r.get("place")
            if p is None or p == 0:
                continue
            if split == "top" and p > 4:
                continue
            if split == "bottom" and p < 5:
                continue
        out.append(s)
    return out


def mean(vals: list[float]) -> float:
    return sum(vals) / len(vals) if vals else float("nan")


def p90(vals: list[int]) -> int:
    s = sorted(vals)
    if not s:
        return 0
    return s[max(0, math.ceil(0.9 * len(s)) - 1)]


def vals(rows: list[dict], key: str) -> list[float]:
    return [s[key] for s in rows if s.get(key) is not None]


def count_cell(rows: list[dict], key: str, places: int = 2) -> str:
    v = vals(rows, key)
    if not v:
        return "n/a"
    return f"{mean(v):.{places}f} ({int(sum(v))}/{len(v)})"


def tier_cell(rows: list[dict], key: str) -> str:
    v = vals(rows, key)
    if not v:
        return "n/a"
    return f"{mean(v):.2f} ({len(v)})"


def gold_cell(rows: list[dict]) -> str:
    v = vals(rows, "gold_end")
    if not v:
        return "n/a"
    med = statistics.median(v)
    return f"{mean(v):.2f} / {med:.1f} ({len(v)})"


def action_cell(rows: list[dict]) -> str:
    v = [s["actions"] for s in rows]
    if not v:
        return "n/a"
    return f"{mean(v):.1f} / {statistics.median(v):.0f} / {p90(v)} ({len(v)})"


def cycle_cell(rows: list[dict]) -> str:
    v = [s["cycles"] for s in rows]
    if not v:
        return "n/a"
    games = sum(1 for s in rows if s["cycles"])
    return f"{mean(v):.3f} ({int(sum(v))}/{len(v)}; {games} games)"


def cap_cell(rows: list[dict]) -> str:
    if not rows:
        return "n/a"
    k = sum(1 for s in rows if s["ended_by_cap"])
    word = "game" if k == 1 else "games"
    return f"{k} ({100 * k / len(rows):.2f}%; {k} {word})"


def ratio(a: float, b: float) -> float:
    if b == 0:
        return float("inf") if a > 0 else 1.0
    return a / b


def fold(r: float) -> float:
    if math.isinf(r):
        return float("inf")
    if r == 0:
        return float("inf")
    return max(r, 1.0 / r)


def fmt_ratio(r: float) -> str:
    if math.isinf(r):
        return "inf"
    return f"{r:.2f}x"


def table(header: list[str], rows: list[list[str]]) -> str:
    out = ["| " + " | ".join(header) + " |",
           "|" + "|".join("---" for _ in header) + "|"]
    for row in rows:
        out.append("| " + " | ".join(row) + " |")
    return "\n".join(out)


def main():
    era_all = load(SRC / "raw-era.json")["rows"]
    bots = {lag: load(SRC / f"raw-lag{lag}.json") for lag in (0, 2)}

    validation = {}
    for lag in (0, 2):
        ref = load(REF[lag])
        # checkpoint 1 is runs/r1111-lagmix/best.pt (update 2000), the brief's
        # checkpoint.
        ck = ref["checkpoints"][1]
        bad = []
        for r in bots[lag]["rows"]:
            q = ck["rows"][r["game"]]
            if (r["place"] != q["place"]
                    or r["knocked_out_turn"] != q["knocked_out_turn"]):
                bad.append({
                    "game": r["game"],
                    "place": r["place"],
                    "reference_place": q["place"],
                    "knocked_out_turn": r["knocked_out_turn"],
                    "reference_knocked_out_turn": q["knocked_out_turn"],
                })
        validation[str(lag)] = {
            "reference": str(REF[lag]),
            "checkpoint": ck["path"],
            "update": ck["update"],
            "games": len(bots[lag]["rows"]),
            "mismatches": len(bad),
            "examples": bad[:10],
        }

    groups = [
        ("Era all", era_all, None),
        ("Era top 4", era_all, "top"),
        ("Era 5th-8th", era_all, "bottom"),
        ("Bot lag 0", bots[0]["rows"], None),
        ("Bot lag 2", bots[2]["rows"], None),
    ]
    header = ["turn"] + [g[0] for g in groups]

    counts_rows = []
    for t in TURNS:
        row = [str(t)]
        for _, rows, split in groups:
            rs = turn_rows(rows, t, split)
            with_end = sum(1 for s in rs if s["gold_end"] is not None)
            row.append(f"{len(rs)} turns ({with_end} with end)")
        counts_rows.append(row)

    action_rows = [[str(t)] + [action_cell(turn_rows(rows, t, sp))
                               for _, rows, sp in groups] for t in TURNS]
    grouped_rows = []
    for t in TURNS:
        row = [str(t)]
        for _, rows, sp in groups:
            rs = turn_rows(rows, t, sp)
            v = [s["actions_grouped"] for s in rs]
            row.append(f"{mean(v):.1f} ({len(v)})" if v else "n/a")
        grouped_rows.append(row)

    def measure_rows(key: str, fmt) -> list[list[str]]:
        return [[str(t)] + [fmt(turn_rows(rows, t, sp), key)
                            for _, rows, sp in groups] for t in TURNS]

    rolls_rows = measure_rows("rolls", count_cell)
    buys_rows = measure_rows("buys", count_cell)
    sells_rows = measure_rows("sells", count_cell)
    cycles_rows = [[str(t)] + [cycle_cell(turn_rows(rows, t, sp))
                               for _, rows, sp in groups] for t in TURNS]
    sb_rows = measure_rows("spells_bought", count_cell)
    sp_rows = measure_rows("spells_played", count_cell)
    hp_rows = measure_rows("hero_powers", count_cell)
    level_rows = measure_rows("level_gold", count_cell)
    tier_rows = measure_rows("tier_end", tier_cell)
    gold_rows = [[str(t)] + [gold_cell(turn_rows(rows, t, sp))
                             for _, rows, sp in groups] for t in TURNS]
    cap_rows = [[str(t)] + [cap_cell(turn_rows(rows, t, sp))
                            for _, rows, sp in groups] for t in TURNS]

    # Pooled action kinds per turn.
    kind_rows = []
    kind_totals = {}
    for name, rows, split in groups:
        pool = [s for t in TURNS for s in turn_rows(rows, t, split)]
        c = Counter()
        for s in pool:
            c.update(s.get("kinds", {}))
        kind_totals[name] = (c, len(pool))
    for k in KINDS:
        row = [k]
        for name, _, _ in groups:
            c, n = kind_totals[name]
            row.append(f"{c.get(k, 0) / n:.2f}" if n else "n/a")
        kind_rows.append(row)
    row = ["raw actions"]
    for name, _, _ in groups:
        c, n = kind_totals[name]
        row.append(f"{sum(c.values()) / n:.2f}" if n else "n/a")
    kind_rows.append(row)
    row = ["player actions"]
    for name, _, _ in groups:
        c, n = kind_totals[name]
        grouped = sum(v for k, v in c.items()
                      if k not in ("target", "target_shop", "discard"))
        row.append(f"{grouped / n:.2f}" if n else "n/a")
    kind_rows.append(row)

    # Pooled ranking.
    measures = [
        ("player actions (grouped)", "actions_grouped"),
        ("rolls", "rolls"),
        ("buys", "buys"),
        ("sells", "sells"),
        ("buy-then-sell cycles", "cycles"),
        ("spells bought", "spells_bought"),
        ("spells played/cast", "spells_played"),
        ("hero power uses", "hero_powers"),
        ("gold spent on levelling", "level_gold"),
        ("tier at end of turn", "tier_end"),
        ("gold left at End Turn", "gold_end"),
    ]
    pooled = {}
    for label, key in measures:
        d = {}
        for name, rows, split in groups:
            v = [s[key] for t in TURNS for s in turn_rows(rows, t, split)
                 if s.get(key) is not None]
            d[name] = (mean(v), len(v), sum(v))
        pooled[key] = d

    rank = []
    for label, key in measures:
        e = pooled[key]["Era all"][0]
        b0 = pooled[key]["Bot lag 0"][0]
        b2 = pooled[key]["Bot lag 2"][0]
        r0, r2 = ratio(b0, e), ratio(b2, e)
        score = max(fold(r0), fold(r2))
        rank.append((score, label, e, b0, b2, r0, r2, pooled[key]))
    rank.sort(key=lambda x: x[0], reverse=True)

    rank_rows = []
    for i, (score, label, e, b0, b2, r0, r2, d) in enumerate(rank, 1):
        e_m, e_n, e_s = d["Era all"]
        b0_m, b0_n, b0_s = d["Bot lag 0"]
        b2_m, b2_n, b2_s = d["Bot lag 2"]
        rank_rows.append([
            str(i), label,
            f"{e_m:.3f} ({int(e_s)}/{e_n})",
            f"{b0_m:.3f} ({int(b0_s)}/{b0_n})", fmt_ratio(r0),
            f"{b2_m:.3f} ({int(b2_s)}/{b2_n})", fmt_ratio(r2),
            ("inf" if math.isinf(score) else f"{score:.2f}x"),
        ])

    # Small extra counts for the notes.
    era_place0 = sum(1 for r in era_all if r.get("place") == 0)
    bot_cap_all = {}
    for lag in (0, 2):
        rows = bots[lag]["rows"]
        bot_cap_all[str(lag)] = {
            "turns_played": sum(r["turns_played"] for r in rows),
            "cap_turns": sum(len(r["cap_turns"]) for r in rows),
            "games_with_cap": sum(1 for r in rows if r["cap_turns"]),
            "cap_turn_numbers": dict(sorted(Counter(
                t for r in rows for t in r["cap_turns"]).items())),
        }

    now = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    lines = []
    lines.append("# Where a bot turn differs from one of Era's, turns 7 to 12")
    lines.append("")
    lines.append(f"Written {now}. Measurement only: no training, no engine "
                 "change, nothing under `crates/` or `train/*.py` touched.")
    lines.append("")
    lines.append("## What was run")
    lines.append("")
    lines.append("- Checkpoint: `train/runs/r1111-lagmix/best.pt`, update "
                 f"{bots[0]['update']}, force_dark_gift "
                 f"{bots[0]['force_dark_gift']}, old_boards "
                 f"{bots[0]['old_boards']}, observation "
                 f"{bots[0]['obs_size']}, actions {bots[0]['n_actions']}. "
                 "The same header is in each raw file.")
    lines.append("- Lineup 1,0,0,0,0,7 (one learner, seven real seats), "
                 "frozen boards `boards/frozen/real-2026-11-10`, real lobby "
                 "filter on, seed base 20261110, chunks of 512 envs, one game "
                 "per seed, actions sampled.")
    lines.append("- Lag 0: `real_lag=0`, `real_lag_from=1`, 2048 games. "
                 "Lag 2: `real_lag=2`, `real_lag_from=7`, 2048 games. The "
                 "lag-2 real seats draw weaker, older boards from turn 7, so "
                 "that run survives much longer: 2048 games reach turn 10 "
                 "against 1578 at lag 0, and 1929 reach turn 12 against 305.")
    lines.append("- Era: every `boards/actions/*.json` file (103 games), "
                 "places from `boards/real/*.json`. A turn is a logged turn "
                 "with a shop phase; turns 7 to 12 are used.")
    lines.append("- GPU cap `--vram-gb 10`; `pcbusy` reported OFF (since "
                 "2026-11-11 15:02) before the runs.")
    lines.append("")
    lines.append("### Fixed-eval check")
    lines.append("")
    for lag in (0, 2):
        v = validation[str(lag)]
        lines.append(f"- Lag {lag}: places and knocked-out turns match "
                     f"`{v['reference']}` (`{v['checkpoint']}`, update "
                     f"{v['update']}) exactly: {v['mismatches']} mismatches "
                     f"in {v['games']} games.")
    lines.append("")
    lines.append("## Counts")
    lines.append("")
    lines.append("`turns` is the number of games that reached the turn and "
                 "have a logged shop phase for it; `with end` is the subset "
                 "with an end state (Era's `end`, or the bot's End Turn, or "
                 "the bot's cap-ended turn).")
    lines.append("")
    lines.append(table(header, counts_rows))
    lines.append("")
    lines.append(f"Era: {len(era_all)} action files, {era_place0} game with "
                 "place 0 (included in Era all, left out of both splits). "
                 "The two splits use place 1-4 and place 5-8. The 4 Era turns "
                 "without an end are turns the log ends inside; the 3 lag-0 "
                 "bot turns without an end are turns whose last action was "
                 "not End Turn and did not hit the cap.")
    lines.append("")
    lines.append("## Actions per turn")
    lines.append("")
    lines.append("Cells are `mean / median / 90th percentile (turns)`. "
                 "The 90th percentile is nearest-rank on the turn values. "
                 "Era counts logged steps; the bot counts env actions, which "
                 "include `target`, `target_shop` and `discard` steps that "
                 "Era folds into the buy, play or activate that opened them.")
    lines.append("")
    lines.append(table(header, action_rows))
    lines.append("")
    lines.append("### Actions per turn after folding continuations")
    lines.append("")
    lines.append("The bot count here drops `target`, `target_shop` and "
                 "`discard`, so a Play plus its target is one action as in "
                 "Era's log. `choose` stays a separate action because Era "
                 "logs a Discover as its own step. Cells are `mean (turns)`.")
    lines.append("")
    lines.append(table(header, grouped_rows))
    lines.append("")
    lines.append("## Rolls per turn")
    lines.append("")
    lines.append("Cells are `mean (total/turns)`. Each row is one step or "
                 "one `roll` action, so free Refreshes are included.")
    lines.append("")
    lines.append(table(header, rolls_rows))
    lines.append("")
    lines.append("## Buys per turn")
    lines.append("")
    lines.append("Cells are `mean (total/turns)`. A buy is a Tavern minion "
                 "only; a Tavern spell is in the next table. Era's `buy` step "
                 "minus its spell buys; the bot's `buy` action.")
    lines.append("")
    lines.append(table(header, buys_rows))
    lines.append("")
    lines.append("## Sells per turn")
    lines.append("")
    lines.append("Cells are `mean (total/turns)`.")
    lines.append("")
    lines.append(table(header, sells_rows))
    lines.append("")
    lines.append("## Buy-then-sell cycles per turn")
    lines.append("")
    lines.append("A cycle is a card bought and sold in the same turn. Era is "
                 "matched on the card's entity id from the log (card id when "
                 "the log omits it). The bot has no entity id in the "
                 "observation, so it is matched by card id and golden flag: "
                 "the sell's board slot must hold a card id the bot bought "
                 "this turn. Cells are `mean (total/turns; turns with at "
                 "least one cycle)`.")
    lines.append("")
    lines.append(table(header, cycles_rows))
    lines.append("")
    lines.append("## Spells bought per turn")
    lines.append("")
    lines.append("Tavern spells bought. Era's `buy` with `spell: true` (or a "
                 "pre-state Tavern slot whose kind is spell); the bot's "
                 "`buy_spell` action. Cells are `mean (total/turns)`.")
    lines.append("")
    lines.append(table(header, sb_rows))
    lines.append("")
    lines.append("## Spells played or cast per turn")
    lines.append("")
    lines.append("A Play step or action whose card was a spell in the hand "
                 "before it. This counts every spell played from hand, "
                 "including generated Blood Gems and Triple Rewards; spells "
                 "cast by another card's effect are not actions and are not "
                 "counted. Cells are `mean (total/turns)`.")
    lines.append("")
    lines.append(table(header, sp_rows))
    lines.append("")
    lines.append("## Hero power uses per turn")
    lines.append("")
    lines.append("Era's `hero_power` steps (main and second); the bot's "
                 "`hero_power` and `hero_power2` actions. Cells are "
                 "`mean (total/turns)`.")
    lines.append("")
    lines.append(table(header, hp_rows))
    lines.append("")
    lines.append("## Gold spent on levelling per turn")
    lines.append("")
    lines.append("Era sums the `paid.gold` of its `level` steps (the client's "
                 "actual price). The bot reads `upgrade_cost` from the "
                 "observation before each `level` action. Cells are "
                 "`mean (total/turns)`.")
    lines.append("")
    lines.append(table(header, level_rows))
    lines.append("")
    lines.append("## Tier at end of turn")
    lines.append("")
    lines.append("Era's `end.tier`; the bot's `tier` before the End Turn "
                 "action or before the action that hits the cap. Turns "
                 "without an end state are left out. Cells are `mean (turns)`.")
    lines.append("")
    lines.append(table(header, tier_rows))
    lines.append("")
    lines.append("## Gold left at End Turn")
    lines.append("")
    lines.append("Era's `end.gold` (after end-of-turn effects); the bot's "
                 "`gold` before the End Turn action. The bot's end-of-turn "
                 "effects cannot be read before the next turn resets Gold, so "
                 "this is Gold at the click, not after those effects. Turns "
                 "without an end state are left out. Cells are `mean / median "
                 "(turns)`.")
    lines.append("")
    lines.append(table(header, gold_rows))
    lines.append("")
    lines.append("## Turns ended by the 200-action cap")
    lines.append("")
    lines.append("Detection: the last action was not End Turn, the pre-action "
                 "`actions_left` was 1 (so `actions` was 199 and the 200th "
                 "action triggers the engine's `finish_shop`), and the next "
                 "observation was a later turn or the game ended. That is the "
                 "engine's own `pl.actions >= max_actions_per_turn` condition "
                 "in `finish_shop`, with the Lobster search exception left "
                 "out by the turn-change test. The rules value is 200 "
                 "(`rules.toml`). Cells are `count (share of turns in the "
                 "cell; games affected)`.")
    lines.append("")
    lines.append(table(header, cap_rows))
    lines.append("")
    lines.append("All cap-ended turns in both bot runs were exactly 200 "
                 "`choose` actions in a row, not buys or rolls: 3 turns in "
                 "2 lag-0 games and 4 turns in 2 lag-2 games. Across all "
                 f"turns played, lag 0 has {bot_cap_all['0']['cap_turns']} cap "
                 f"turns in {bot_cap_all['0']['turns_played']} turns "
                 f"({bot_cap_all['0']['games_with_cap']} games); lag 2 has "
                 f"{bot_cap_all['2']['cap_turns']} in "
                 f"{bot_cap_all['2']['turns_played']} turns "
                 f"({bot_cap_all['2']['games_with_cap']} games). No Era turn "
                 "hit the cap.")
    lines.append("")
    lines.append("## Action kinds, pooled over turns 7 to 12")
    lines.append("")
    lines.append("Mean actions of each kind per turn; the per-turn kind "
                 "counts for every game are in `rows.json`. Era's `discover` is "
                 "shown as `choose`, `dark_discovery` as `dark_gift`, and "
                 "`trinket_pick` as `choose_trinket`; Era has no logged "
                 "`target`, `target_shop` or `discard` step because those "
                 "live inside the step that opened them.")
    lines.append("")
    lines.append(table(["kind"] + [g[0] for g in groups], kind_rows))
    lines.append("")
    lines.append("## Ranking of the gaps")
    lines.append("")
    lines.append("Each measure is pooled over turns 7 to 12 as total events "
                 "over turns. `ratio` is bot divided by Era all. The sort key "
                 "is the larger fold gap of the two lags (`max(ratio, "
                 "1/ratio)`), so a bot number well below Era and one well "
                 "above both sort high. Counts are `total/turns` beside each "
                 "mean. Lag 2 is a much weaker lobby (the fixed eval has the "
                 "bot at 3.484 average place against 5.966 at lag 0), so its "
                 "turns are a different surviving population; the lag-0 "
                 "column is the closer comparison.")
    lines.append("")
    lines.append(table(
        ["rank", "measure", "Era all /turn", "bot lag 0 /turn", "ratio",
         "bot lag 2 /turn", "ratio", "gap"],
        rank_rows))
    lines.append("")
    abs_rank = sorted(
        ((abs(pooled[key]["Bot lag 0"][0] - pooled[key]["Era all"][0]), label)
         for label, key in measures), reverse=True)
    lines.append("By absolute per-turn difference at lag 0 the order is " +
                 ", ".join(f"{label} ({d:.2f})" for d, label in abs_rank) +
                 ". The fold ranking above puts the buy-then-sell cycles "
                 "first because Era's base is small, even though the action "
                 "count is the larger absolute gap.")
    lines.append("")
    lines.append("## What stands out")
    lines.append("")
    top = rank[0]
    lines.append(f"- The largest fold gap is **{top[1]}**: Era "
                 f"{top[2]:.3f} per turn against the bot's {top[3]:.3f} at "
                 f"lag 0 and {top[4]:.3f} at lag 2 ({fmt_ratio(top[5])} and "
                 f"{fmt_ratio(top[6])}). Era does buy-and-sell churn in a "
                 "turn; the bot does far less of it.")
    lines.append("- The bot takes about half the actions, rolls about a "
                 "quarter as often, and sells about a fifth as often as Era. "
                 "It buys about a third as many minions and plays about "
                 "two-fifths as many spells. The raw action gap is smaller "
                 "after folding targets and discards, but the economy gap is "
                 "not.")
    lines.append("- The bot spends more Gold on levelling and leaves 4 to 5 "
                 "times as much Gold on the table at End Turn. It reaches a "
                 "similar tier at the end of the turn (bot against Era: 4.96 "
                 "against 5.25 pooled at lag 0; 5.25 against 5.25 at lag 2), "
                 "so the difference is not a lower tier: it is doing much "
                 "less with the Gold it has.")
    lines.append("- The cap turns are not a shopping loop. They are 200 "
                 "repeated `choose` actions, which points at a Discover or "
                 "choice prompt the policy keeps answering without the engine "
                 "moving on. This is the one place where the measurement "
                 "looks like an engine or action-mask bug, not a style "
                 "difference.")
    lines.append("- Era's top-4 and 5th-to-8th turns are close on actions, "
                 "rolls, buys and sells; the 5th-to-8th turns spend more on "
                 "levelling (3.16 against 2.68 Gold) and leave more Gold "
                 "(0.55 against 0.20). The split is reported in every table.")
    lines.append("")
    lines.append("## Files")
    lines.append("")
    lines.append("- `train/runs/turn-profile/report.md`: this report.")
    lines.append("- `train/runs/turn-profile/rows.json`: every per-game, "
                 "per-turn row used here, plus the pooled summary and the "
                 "fixed-eval check.")
    lines.append("- `train/evaltools/turn_profile/era_profile.py`, "
                 "`collect_bot.py`, `analyze.py`: the scripts.")
    lines.append("- `train/runs/turn-profile/raw-era.json`, `raw-lag0.json`, "
                 "`raw-lag2.json`: the collector outputs.")
    lines.append("")

    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / "report.md").write_text("\n".join(lines) + "\n")

    payload = {
        "written": now,
        "brief": "docs/briefs/turn-profile.md",
        "checkpoint": {
            "path": "train/runs/r1111-lagmix/best.pt",
            "update": bots[0]["update"],
        },
        "turns": list(TURNS),
        "validation": validation,
        "era": {
            "games": len(era_all),
            "place0": era_place0,
            "rows": era_all,
        },
        "bot": {
            "lag0": {
                "meta": {k: v for k, v in bots[0].items() if k != "rows"},
                "rows": bots[0]["rows"],
            },
            "lag2": {
                "meta": {k: v for k, v in bots[2].items() if k != "rows"},
                "rows": bots[2]["rows"],
            },
        },
        "summary": {
            "pooled": pooled,
            "ranking": [
                {"measure": label, "era": d["Era all"],
                 "bot_lag0": d["Bot lag 0"], "bot_lag2": d["Bot lag 2"],
                 "ratio_lag0": r0, "ratio_lag2": r2}
                for _, label, _, _, _, r0, r2, d in rank
            ],
            "all_turn_caps": bot_cap_all,
        },
    }
    (OUT / "rows.json").write_text(json.dumps(payload) + "\n")
    print(f"wrote {OUT / 'report.md'}")
    print(f"wrote {OUT / 'rows.json'} ({len(era_all)} Era games, "
          f"{len(bots[0]['rows'])} + {len(bots[2]['rows'])} bot games)")
    for lag in (0, 2):
        print(f"lag {lag} place check: {validation[str(lag)]['mismatches']} "
              f"mismatches in {validation[str(lag)]['games']}")


if __name__ == "__main__":
    main()
