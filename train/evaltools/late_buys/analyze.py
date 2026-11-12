"""Turn the collected bot rows and Era's logged games into the late-buys report.

    .venv/bin/python train/evaltools/late_buys/analyze.py
    .venv/bin/python train/evaltools/late_buys/analyze.py --verdict train/runs/late-buys/verdict.txt

Reads train/runs/late-buys/raw-<name>.json (collect_bot.py), boards/actions and
boards/real, and writes train/runs/late-buys/report.md and data.json.
"""

from __future__ import annotations

import argparse
import glob
import json
import math
from collections import Counter, defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
RAW = ROOT / "train" / "runs" / "late-buys"
OUT = ROOT / "train" / "runs" / "late-buys"
TRANSITIONS = ((8, 9), (9, 10), (10, 11), (11, 12))
TURNS = (8, 9, 10, 11, 12)
TIER_TURNS = (10, 11, 12)


def load_json(path: Path):
    return json.loads(path.read_text())


def pct(x, places=1) -> str:
    if x is None:
        return "n/a"
    return f"{100 * x:.{places}f}%"


def num(x, places=2) -> str:
    if x is None:
        return "n/a"
    return f"{x:.{places}f}"


def mean(values):
    return sum(values) / len(values) if values else None


def median(values):
    if not values:
        return None
    v = sorted(values)
    n = len(v)
    if n % 2:
        return v[n // 2]
    return 0.5 * (v[n // 2 - 1] + v[n // 2])


def load_era():
    real = {}
    for path in glob.glob(str(ROOT / "boards" / "real" / "*.json")):
        d = load_json(Path(path))
        real[(d["source"], d["game"])] = d
    actions = {}
    for path in glob.glob(str(ROOT / "boards" / "actions" / "*.json")):
        d = load_json(Path(path))
        actions[(d["source"], d["game"])] = d
    games = []
    for key, d in actions.items():
        place = real.get(key, {}).get("place")
        turns = {}
        for t in d.get("turns") or []:
            end = t.get("end")
            if not end:
                continue
            board = []
            for pos, m in enumerate(end.get("board") or [], 1):
                board.append({
                    "name": m.get("name") or m.get("card_id") or "unknown",
                    "golden": bool(m.get("golden")),
                    "attack": int(m.get("attack") or 0),
                    "health": int(m.get("health") or 0),
                    "position": int(m.get("position") or pos),
                })
            turns[int(t["turn"])] = {"board": board, "tier": end.get("tier")}
        games.append({"key": key, "place": place, "turns": turns})
    return real, games


def real_me_turn12(real, place_ok):
    boards = []
    for d in real.values():
        place = d.get("place")
        if not place_ok(place):
            continue
        for b in d.get("boards") or []:
            if b.get("turn") == 12 and b.get("who") == "me":
                boards.append([{
                    "name": m.get("name") or m.get("card_id") or "unknown",
                    "golden": bool(m.get("golden")),
                    "attack": int(m.get("attack") or 0),
                    "health": int(m.get("health") or 0),
                } for m in b.get("minions") or []])
    return boards


def real_all_turn12(real):
    boards = []
    for d in real.values():
        for b in d.get("boards") or []:
            if b.get("turn") != 12:
                continue
            boards.append([{
                "name": m.get("name") or m.get("card_id") or "unknown",
                "golden": bool(m.get("golden")),
                "attack": int(m.get("attack") or 0),
                "health": int(m.get("health") or 0),
            } for m in b.get("minions") or []])
    return boards


def bot_boards(raw, turn=12):
    boards = []
    for r in raw["rows"]:
        b = r.get("boards", {}).get(str(turn))
        if b is not None:
            boards.append(b)
    return boards


def board_stats(boards):
    total = 0
    card_sum = Counter()
    card_boards = Counter()
    for board in boards:
        names = Counter()
        for m in board:
            s = int(m["attack"]) + int(m["health"])
            total += s
            name = m["name"]
            card_sum[name] += s
            names[name] += 1
        for name in names:
            card_boards[name] += 1
    cards = {}
    for name, s in card_sum.items():
        cards[name] = {
            "stat_sum": s,
            "stat_share": s / total if total else None,
            "boards": card_boards[name],
            "board_share": card_boards[name] / len(boards) if boards else None,
        }
    return {
        "boards": len(boards),
        "total_stats": total,
        "cards": cards,
    }


def match_minions(prev, curr):
    """Pair the same minion between two consecutive turn-end boards.

    The step log's entity ids are not stable across turns in these files, so
    the match is positional: first the same position with the same name and
    golden state, then the same position with the same name (a triple changes
    the golden flag), then a name and golden state that is unique on both
    boards, then the rest of the same name and golden state in position order.
    """
    used_a, used_b = set(), set()
    pairs = []

    def take(i, j):
        used_a.add(i)
        used_b.add(j)
        pairs.append((prev[i], curr[j]))

    for i, a in enumerate(prev):
        for j, b in enumerate(curr):
            if j in used_b:
                continue
            if (a["position"] == b["position"] and a["name"] == b["name"]
                    and a["golden"] == b["golden"]):
                take(i, j)
                break
    for i, a in enumerate(prev):
        if i in used_a:
            continue
        for j, b in enumerate(curr):
            if j in used_b:
                continue
            if a["position"] == b["position"] and a["name"] == b["name"]:
                take(i, j)
                break
    left_a = [i for i in range(len(prev)) if i not in used_a]
    left_b = [j for j in range(len(curr)) if j not in used_b]
    count_a = Counter((prev[i]["name"], prev[i]["golden"]) for i in left_a)
    count_b = Counter((curr[j]["name"], curr[j]["golden"]) for j in left_b)
    for key, n in count_a.items():
        if n != 1 or count_b.get(key, 0) != 1:
            continue
        i = next(i for i in left_a if (prev[i]["name"], prev[i]["golden"]) == key)
        j = next(j for j in left_b if (curr[j]["name"], curr[j]["golden"]) == key)
        if i not in used_a and j not in used_b:
            take(i, j)
    for i in sorted((i for i in left_a if i not in used_a),
                    key=lambda i: prev[i]["position"]):
        key = (prev[i]["name"], prev[i]["golden"])
        for j in sorted((j for j in left_b if j not in used_b),
                        key=lambda j: curr[j]["position"]):
            if (curr[j]["name"], curr[j]["golden"]) == key:
                take(i, j)
                break
    return pairs


def growth(games, keep):
    """Mean and median Attack plus Health ratio per consecutive turn pair."""
    out = {
        "overall": {f"{a}->{b}": {"ratios": []} for a, b in TRANSITIONS},
        "by_card": defaultdict(lambda: {f"{a}->{b}": {"ratios": []}
                                        for a, b in TRANSITIONS}),
    }
    for g in games:
        if not keep(g):
            continue
        for a, b in TRANSITIONS:
            ta = g.get("turns", {}).get(a) if "turns" in g else None
            tb = g.get("turns", {}).get(b) if "turns" in g else None
            if ta is None or tb is None:
                continue
            prev, curr = ta["board"], tb["board"]
            for ma, mb in match_minions(prev, curr):
                s0 = ma["attack"] + ma["health"]
                s1 = mb["attack"] + mb["health"]
                if s0 <= 0:
                    continue
                r = s1 / s0
                label = f"{a}->{b}"
                out["overall"][label]["ratios"].append(r)
                out["by_card"][ma["name"]][label]["ratios"].append(r)
    def finish(d):
        for label, d2 in d.items():
            v = d2.pop("ratios")
            d2["n"] = len(v)
            d2["mean"] = mean(v)
            d2["median"] = median(v)
    finish(out["overall"])
    for d in out["by_card"].values():
        finish(d)
    return out


def bot_games(raw):
    games = []
    for r in raw["rows"]:
        turns = {}
        for t in TURNS:
            b = r.get("boards", {}).get(str(t))
            if b is not None:
                turns[t] = {
                    "board": [{**m, "name": m["name"],
                               "golden": bool(m["golden"]),
                               "attack": int(m["attack"]),
                               "health": int(m["health"]),
                               "position": int(m["position"])}
                              for m in b],
                    "tier": r.get("tiers", {}).get(str(t)),
                }
        games.append({"turns": turns, "place": r.get("place")})
    return games


def tier_stats(games, turn, keep):
    vals = []
    for g in games:
        if not keep(g):
            continue
        t = g.get("turns", {}).get(turn)
        if t is not None and t.get("tier") is not None:
            vals.append(int(t["tier"]))
    dist = Counter(vals)
    return {
        "n": len(vals),
        "mean": mean(vals),
        "distribution": {str(k): dist[k] for k in sorted(dist)},
    }


def bot_buy_offer(raw, names):
    rows = raw["rows"]
    n = len(rows)
    out = {}
    for name in names:
        buys = 0
        games_buy = 0
        offers = 0
        games_offer = 0
        for r in rows:
            b = int(r.get("buys", {}).get(name, 0))
            o = int(r.get("offers", {}).get(name, 0))
            # A top-20 card is a minion; keep only minion offers and buys.
            if r.get("buy_spell", {}).get(name):
                b = 0
            if r.get("offer_spell", {}).get(name):
                o = 0
            if b:
                buys += b
                games_buy += 1
            if o:
                offers += o
                games_offer += 1
        out[name] = {
            "games": n,
            "games_offered": games_offer,
            "offer_share": games_offer / n if n else None,
            "offers": offers,
            "offers_per_game": offers / n if n else None,
            "games_bought": games_buy,
            "buy_share": games_buy / n if n else None,
            "buys": buys,
            "buys_per_game": buys / n if n else None,
            "buys_per_offer": buys / offers if offers else None,
            "buy_given_offer": games_buy / games_offer if games_offer else None,
        }
    return out


def late_top(raw, limit=15):
    totals = Counter()
    games_with = Counter()
    for r in raw["rows"]:
        for name, c in r.get("late_buys", {}).items():
            if r.get("buy_spell", {}).get(name):
                continue
            totals[name] += int(c)
            games_with[name] += 1
    top = sorted(totals.items(), key=lambda x: (-x[1], x[0]))[:limit]
    stats = defaultdict(lambda: {"attack": [], "health": []})
    for r in raw["rows"]:
        for m in r.get("boards", {}).get("12", []) or []:
            stats[m["name"]]["attack"].append(int(m["attack"]))
            stats[m["name"]]["health"].append(int(m["health"]))
    out = []
    for name, c in top:
        a = stats[name]["attack"]
        h = stats[name]["health"]
        out.append({
            "name": name,
            "buys": c,
            "games_bought": games_with[name],
            "mean_attack_t12": mean(a),
            "mean_health_t12": mean(h),
            "mean_total_t12": mean([x + y for x, y in zip(a, h)]),
            "boards_t12": len(a),
        })
    return out


def fmt_table(headers, rows):
    lines = ["| " + " | ".join(headers) + " |",
             "|" + "|".join("---" for _ in headers) + "|"]
    for row in rows:
        lines.append("| " + " | ".join(str(x) for x in row) + " |")
    return lines


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--verdict", type=Path)
    args = ap.parse_args()

    real, era_games = load_era()
    by_key = {(g["key"]): g for g in era_games}
    era_places = {g["key"]: g["place"] for g in era_games}

    def place_ok(p):
        return p is not None and 1 <= p <= 8

    top4 = lambda p: p is not None and 1 <= p <= 4
    bottom4 = lambda p: p is not None and 5 <= p <= 8
    all_era = lambda p: place_ok(p)

    era_top4_boards = real_me_turn12(real, top4)
    era_bottom4_boards = real_me_turn12(real, bottom4)
    era_own_boards = real_me_turn12(real, all_era)
    all_real_boards = real_all_turn12(real)

    raw_lag = load_json(RAW / "raw-r1111-lagmix.json")
    raw_bundle = load_json(RAW / "raw-r1109-bundle.json")
    bot_lag_boards = bot_boards(raw_lag, 12)
    bot_bundle_boards = bot_boards(raw_bundle, 12)

    groups = {
        "Era top 4": board_stats(era_top4_boards),
        "Era 5th to 8th": board_stats(era_bottom4_boards),
        "Era all": board_stats(era_own_boards),
        "All real boards": board_stats(all_real_boards),
        "Bot r1111-lagmix": board_stats(bot_lag_boards),
        "Bot r1109-bundle": board_stats(bot_bundle_boards),
    }
    top20 = sorted(groups["Era top 4"]["cards"].items(),
                   key=lambda kv: (-(kv[1]["stat_share"] or 0),
                                   -(kv[1]["board_share"] or 0), kv[0]))[:20]
    top20_names = [name for name, _ in top20]

    def card_row(name):
        row = [name]
        for label in ("Era top 4", "Era 5th to 8th", "Era all",
                      "All real boards", "Bot r1111-lagmix", "Bot r1109-bundle"):
            c = groups[label]["cards"].get(name)
            row.append(pct(c["stat_share"]) if c else "0.0%")
            row.append(pct(c["board_share"]) if c else "0.0%")
        return row

    buy_lag = bot_buy_offer(raw_lag, top20_names)
    buy_bundle = bot_buy_offer(raw_bundle, top20_names)

    era_games_by_place = [g for g in era_games if place_ok(g["place"])]
    era_top4_games = [g for g in era_games if top4(g["place"])]
    era_bottom4_games = [g for g in era_games if bottom4(g["place"])]
    bot_lag_games = bot_games(raw_lag)
    bot_bundle_games = bot_games(raw_bundle)
    bot_lag_top4_games = [g for g in bot_lag_games if top4(g["place"])]
    bot_bundle_top4_games = [g for g in bot_bundle_games if top4(g["place"])]

    growth_groups = {
        "Era top 4": growth(era_top4_games, lambda g: True),
        "Era 5th to 8th": growth(era_bottom4_games, lambda g: True),
        "Era all": growth(era_games_by_place, lambda g: True),
        "Bot r1111-lagmix": growth(bot_lag_games, lambda g: True),
        "Bot r1109-bundle": growth(bot_bundle_games, lambda g: True),
    }

    tiers = {}
    for turn in TIER_TURNS:
        tiers[str(turn)] = {
            "Era all": tier_stats(era_games_by_place, turn, lambda g: True),
            "Era top 4": tier_stats(era_top4_games, turn, lambda g: True),
            "Era 5th to 8th": tier_stats(era_bottom4_games, turn, lambda g: True),
            "Bot r1111-lagmix": tier_stats(bot_lag_games, turn, lambda g: True),
            "Bot r1111-lagmix top 4": tier_stats(bot_lag_top4_games, turn, lambda g: True),
            "Bot r1109-bundle": tier_stats(bot_bundle_games, turn, lambda g: True),
            "Bot r1109-bundle top 4": tier_stats(bot_bundle_top4_games, turn, lambda g: True),
        }

    late = {
        "Bot r1111-lagmix": late_top(raw_lag),
        "Bot r1109-bundle": late_top(raw_bundle),
    }

    data = {
        "meta": {
            "brief": "docs/briefs/late-buys.md",
            "era_games_files": len(real),
            "era_action_games": len(era_games),
            "era_games_with_place": len(era_games_by_place),
            "era_top4_games": len(era_top4_games),
            "era_bottom4_games": len(era_bottom4_games),
            "era_place_counts": dict(sorted(Counter(
                d.get("place") for d in real.values()).items(),
                key=lambda kv: (kv[0] is None, kv[0]))),
            "bot_lagmix_games": len(raw_lag["rows"]),
            "bot_bundle_games": len(raw_bundle["rows"]),
            "bot_lagmix_elapsed_seconds": raw_lag.get("elapsed_seconds"),
            "bot_bundle_elapsed_seconds": raw_bundle.get("elapsed_seconds"),
            "top20": top20_names,
        },
        "q1": {label: groups[label] for label in groups},
        "q2": {
            "buy_offer": {
                "Bot r1111-lagmix": buy_lag,
                "Bot r1109-bundle": buy_bundle,
            },
            "growth": growth_groups,
        },
        "q3": {"tiers": tiers},
        "late_top15": late,
    }
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / "data.json").write_text(json.dumps(data, indent=2) + "\n")

    lines = []
    lines.append("# Late buys: what the bot builds on turns 9 to 12, against Era")
    lines.append("")
    lines.append("Measurement for `docs/briefs/late-buys.md`. No training and no "
                 "engine changes. The collector is `train/evaltools/late_buys/collect_bot.py`; "
                 "this report is `train/evaltools/late_buys/analyze.py`.")
    lines.append("")
    lines.append("## Data")
    lines.append("")
    lines.append(f"- Era: {len(real)} logged games in `boards/real`, "
                 f"{len(era_games)} action files, {len(era_games_by_place)} games "
                 f"with a final place (one file has place 0 and is left out of the "
                 f"place groups). Top 4: {len(era_top4_games)} games, 5th to 8th: "
                 f"{len(era_bottom4_games)}.")
    lines.append(f"- Era boards at turn 12: {len(era_own_boards)} of Era's own, "
                 f"{len(all_real_boards)} in all (Era plus opponents).")
    lines.append(f"- Bot: {len(raw_lag['rows'])} games for `r1111-lagmix/best.pt` "
                 f"(update {raw_lag['update']}, {raw_lag.get('elapsed_seconds')}s) and "
                 f"{len(raw_bundle['rows'])} for `r1109-bundle/best.pt` "
                 f"(update {raw_bundle['update']}, {raw_bundle.get('elapsed_seconds')}s), "
                 f"the fixed eval at lag 0: seed base {raw_lag['seed_base']}, lineup "
                 f"1 learner + 7 real, frozen boards `real-2026-11-10`, lobby filter on. "
                 f"2048 games fit well inside 20 minutes "
                 f"({raw_lag.get('elapsed_seconds')}s and "
                 f"{raw_bundle.get('elapsed_seconds')}s per checkpoint), so 2048 were run.")
    lines.append(f"- Bot games reaching end of turn 12: {len(bot_lag_boards)} "
                 f"(r1111-lagmix), {len(bot_bundle_boards)} (r1109-bundle), out of "
                 f"2048 each.")
    lines.append("- Card identity is the plain card name: a golden minion is counted "
                 "with its plain card on every stat, offer and buy table.")
    lines.append("- Offers are the Tavern cards seen in the observation, counted when "
                 "a card newly appears; a card staying on the shelf between decisions "
                 "is not counted again, and a Roll's fresh Tavern is. Buys are the "
                 "`Buy(slot)` actions, so they are exact. A buy of a spell that sat in "
                 "a minion slot is marked as a spell and left out of the minion tables.")
    lines.append("")
    lines.append("The collector reproduces the fixed eval's places exactly for all 2048 "
                 "games of both checkpoints; the full rows are in `data.json`.")
    lines.append("")
    lines.append("## 1. Which cards carry the stats on turn 12")
    lines.append("")
    lines.append(f"The 20 cards are Era's top 4 at turn 12, ranked by their share of "
                 f"the group's summed Attack plus Health. The table also gives the share "
                 f"of boards the card appears on. Groups: Era top 4 "
                 f"({groups['Era top 4']['boards']} boards), Era 5th to 8th "
                 f"({groups['Era 5th to 8th']['boards']}), Era all "
                 f"({groups['Era all']['boards']}), all real boards at turn 12 "
                 f"({groups['All real boards']['boards']}), bot r1111-lagmix "
                 f"({groups['Bot r1111-lagmix']['boards']}), bot r1109-bundle "
                 f"({groups['Bot r1109-bundle']['boards']}).")
    lines.append("")
    headers = ["card"]
    for label in ("Era top 4", "Era 5th to 8th", "Era all",
                  "All real boards", "Bot r1111-lagmix", "Bot r1109-bundle"):
        headers.extend([f"{label} stat", f"{label} boards"])
    lines.extend(fmt_table(headers, [card_row(n) for n in top20_names]))
    lines.append("")
    lines.append("Group totals: " + ", ".join(
        f"{label} {groups[label]['total_stats']} stats on {groups[label]['boards']} boards"
        for label in groups) + ".")
    lines.append("")
    lines.append("## 2. For each of those cards: offered, bought and growth")
    lines.append("")
    lines.append("### Offered and bought, turns 1 to 12")
    lines.append("")
    for label, table in (("Bot r1111-lagmix", buy_lag),
                         ("Bot r1109-bundle", buy_bundle)):
        lines.append(f"`{label}` ({table[top20_names[0]]['games']} games). "
                     "An offer is a card appearance in the Tavern: a card staying on "
                     "the shelf between decisions is not counted again, and a "
                     "Roll's fresh Tavern is.")
        lines.append("")
        rows = []
        for name in top20_names:
            d = table[name]
            rows.append([
                name,
                pct(d["offer_share"]), num(d["offers_per_game"]),
                pct(d["buy_share"]), num(d["buys_per_game"]),
                num(d["buys_per_offer"]), pct(d["buy_given_offer"]),
            ])
        lines.extend(fmt_table(
            ["card", "games offered", "offers/game", "games bought",
             "buys/game", "buys/offer", "bought if offered"], rows))
        lines.append("")
    lines.append("### Growth while held, turns 8 to 12")
    lines.append("")
    lines.append("A minion is matched between end of turn T and end of turn T+1 "
                 "by position and card name first, then by a unique card name and "
                 "golden state, then by card name and golden state in position order. "
                 "The action files' entity ids are not stable from turn to turn, so "
                 "ids are not usable. The per-card tables give the number of matched "
                 "minions in brackets, so thin cells are visible.")
    lines.append("")
    rows = []
    for label, g in growth_groups.items():
        row = [label]
        for a, b in TRANSITIONS:
            d = g["overall"][f"{a}->{b}"]
            row.append(f"{num(d['mean'])} (n={d['n']})")
        rows.append(row)
    lines.extend(fmt_table(["group"] + [f"{a}->{b}" for a, b in TRANSITIONS], rows))
    lines.append("")
    lines.append("Per card, by transition. `bot lagmix`, `bot bundle`, `Era top 4` "
                 "and `Era 5th to 8th` are mean ratios, with the number of matched "
                 "minions in brackets.")
    lines.append("")
    for a, b in TRANSITIONS:
        label = f"{a}->{b}"
        lines.append(f"#### Turn {a} to {b}")
        lines.append("")
        rows = []
        for name in top20_names:
            row = [name]
            for group in ("Bot r1111-lagmix", "Bot r1109-bundle",
                          "Era top 4", "Era 5th to 8th"):
                d = growth_groups[group]["by_card"].get(name, {}).get(label)
                if d is None or d["n"] == 0:
                    row.append("n/a")
                else:
                    row.append(f"{num(d['mean'])} ({d['n']})")
            rows.append(row)
        lines.extend(fmt_table(["card"] + ["Bot r1111-lagmix", "Bot r1109-bundle",
                                           "Era top 4", "Era 5th to 8th"], rows))
        lines.append("")
    lines.append("### Top 15 bot buys on turns 9 to 12")
    lines.append("")
    for label, table in late.items():
        lines.append(f"`{label}`")
        lines.append("")
        rows = []
        for i, d in enumerate(table, 1):
            rows.append([
                i, d["name"], d["buys"], d["games_bought"],
                num(d["mean_attack_t12"], 1), num(d["mean_health_t12"], 1),
                num(d["mean_total_t12"], 1), d["boards_t12"],
            ])
        lines.extend(fmt_table(["rank", "card", "buys 9-12", "games with buy",
                                "mean Attack t12", "mean Health t12",
                                "mean A+H t12", "boards t12"], rows))
        lines.append("")
    lines.append("## 3. Tier at the end of turns 10, 11 and 12")
    lines.append("")
    lines.append("Tier is the end-of-shop tier: the action files' `end.tier` for Era, "
                 "the last observation of that turn for the bot. Games that did not "
                 "reach the turn are not counted.")
    lines.append("")
    for turn in TIER_TURNS:
        lines.append(f"### Turn {turn}")
        lines.append("")
        table = tiers[str(turn)]
        all_tiers = sorted({int(k) for d in table.values() for k in d["distribution"]})
        rows = []
        for label, d in table.items():
            dist = " ".join(f"T{t}:{d['distribution'].get(str(t), 0)}"
                            for t in all_tiers)
            rows.append([label, d["n"], num(d["mean"]), dist])
        lines.extend(fmt_table(["group", "games", "mean tier", "distribution"], rows))
        lines.append("")
    verdict = None
    if args.verdict and args.verdict.is_file():
        verdict = args.verdict.read_text().strip()
    lines.append("## Verdict")
    lines.append("")
    if verdict:
        lines.append(verdict)
    else:
        lines.append("(verdict follows the numbers above)")
    lines.append("")
    (OUT / "report.md").write_text("\n".join(lines) + "\n")
    print(f"wrote {OUT / 'report.md'} and {OUT / 'data.json'}")


if __name__ == "__main__":
    main()
