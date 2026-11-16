#!/usr/bin/env python3
"""Where BASE picks trinkets: offer position, cost, and the gap against Era.

    .venv/bin/python train/evaltools/trinket_where.py --vram-gb 13.5

The fixed eval's seeds at lag 0 (the same 2,048 games as
`train/fixed_eval.py` with no lag), one learner against seven real seats. The
tool logs every trinket offer the learner resolves, with the offer's four
trinkets and the one it took, then compares:

- the share of picks by offer position and by the picked trinket's cost in
  `trinkets.toml`, lesser and greater separately;
- the ten trinkets the bot picks most often, each with its pick rate when
  offered, Era's own pick rate when offered from the frozen action set, and
  the streamers' clean pick count from `boards/twitch/games`;
- the ten largest bot-minus-Era pick-rate gaps with at least 5 offers on each
  side.

Measurement only. Writes train/runs/trinket-check/where.md and
train/runs/trinket-check/where-rows.json.
"""

from __future__ import annotations

import argparse
import json
import sys
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "train"))
sys.path.insert(0, str(ROOT / "train" / "evaltools"))

import real_boards
import torch

import fixed_eval as fe
import trinket_picks as tp

BASE = ROOT / "train/runs/r1115-school-text/best.pt"
OUT = ROOT / "train/runs/trinket-check"


def pct(k: float, n: float) -> str:
    return "n/a" if n <= 0 else f"{100 * k / n:.1f}%"


def label(table: list[dict], dup: set[str], i: int) -> str:
    e = table[i]
    return f"{e['name']} ({e['slot']})" if e["name"] in dup else e["name"]


def load_eval_args(args) -> argparse.Namespace:
    """The namespace eval_checkpoint reads, fixed to the lag-0 own-pick run."""
    return argparse.Namespace(
        ckpts=[str(args.ckpt)], games=args.games, temp=1.0, name=None,
        power_text=False, boards=fe.FROZEN, seed_base=args.seed_base,
        vram_gb=args.vram_gb, cpu=args.cpu, no_lobby_filter=False,
        real_lag=0, real_lag_from=1, learner_heroes=[],
        era_starts=None, era_turns="7,11", era_split=None, era_folder=None,
        era_turn_min=0, era_turn_max=0, trinket_pick="own")


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--ckpt", default=str(BASE),
                    help=f"checkpoint (default {BASE.relative_to(ROOT)})")
    ap.add_argument("--games", type=int, default=2048, help="games (default 2048)")
    ap.add_argument("--seed-base", type=int, default=fe.SEED_BASE,
                    help=f"first chunk's seed (default {fe.SEED_BASE})")
    ap.add_argument("--vram-gb", type=float,
                    help="cap this process's GPU memory; required when CUDA is used")
    ap.add_argument("--cpu", action="store_true", help="run on the CPU")
    ap.add_argument("--out", default=str(OUT))
    args = ap.parse_args()
    if args.games < 1:
        raise SystemExit("--games must be at least 1")

    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)

    table = tp.load_trinkets()
    by_card = {e["card_id"]: i for i, e in enumerate(table)}
    names = [e["name"] for e in table]
    slots = [e["slot"] for e in table]
    costs = [e["cost"] for e in table]
    dup = {n for n, c in Counter(names).items() if c > 1}

    _, stream_counts, stream_info = tp.streamer_picks()
    era_offers = tp.action_offers()
    era_offer_count: Counter = Counter()
    era_pick_count: Counter = Counter()
    unknown_options = 0
    for offer in era_offers:
        seen = set()
        for option in offer["options"]:
            i = by_card.get(option["card_id"])
            if i is None:
                unknown_options += 1
                continue
            if i not in seen:
                era_offer_count[i] += 1
                seen.add(i)
        i = by_card.get((offer["picked"] or {}).get("card_id"))
        if i is not None:
            era_pick_count[i] += 1

    dev = torch.device("cpu" if args.cpu or not torch.cuda.is_available() else "cuda")
    if dev.type == "cuda":
        if args.vram_gb is None:
            raise SystemExit("--vram-gb is required when CUDA is used")
        total = torch.cuda.get_device_properties(0).total_memory / 2 ** 30
        torch.cuda.set_per_process_memory_fraction(min(1.0, args.vram_gb / total), 0)
        print(f"device cuda, {args.vram_gb:g} GB of {total:.1f} GB cap", flush=True)
    else:
        print("device cpu", flush=True)

    real, unknown = real_boards.load(fe.FROZEN)
    if not real:
        raise SystemExit(f"{fe.FROZEN}: no real boards")
    info = fe.load_ckpt(args.ckpt, dev)
    print(f"update {info['update']}, force_dark_gift {info['force_dark_gift']}, "
          f"old_boards {info['old_boards']}, comp inputs {info['n_comps']}", flush=True)

    ns = load_eval_args(args)
    old_log = fe.TRINKET_LOG
    fe.TRINKET_LOG = []
    try:
        rows = fe.eval_checkpoint(info, ns, real, dev)
    finally:
        log = fe.TRINKET_LOG
        fe.TRINKET_LOG = old_log
    if len(rows) != args.games:
        raise SystemExit(f"got {len(rows)} games, wanted {args.games}")
    print(f"{len(log)} learner trinket offers logged", flush=True)

    offers: Counter = Counter()
    picks: Counter = Counter()
    positions: Counter = Counter()
    cost_counts: Counter = Counter()
    auto = 0
    for rec in log:
        i = rec["pick"]
        for offered in rec["offer"]:
            offers[offered] += 1
        picks[i] += 1
        positions[rec["pick_slot"]] += 1
        cost_counts[(costs[i], slots[i])] += 1
        if rec["auto"]:
            auto += 1
    n_picks = sum(picks.values())

    lines = [
        "# Where BASE picks trinkets",
        "",
        f"- checkpoint: `{args.ckpt}` (update {info['update']})",
        f"- games: {len(rows)} on the fixed eval's seeds at lag 0 "
        f"(seed base {args.seed_base}, {fe.CHUNK} envs a chunk)",
        f"- offers logged: {len(log)}, picks: {n_picks}, "
        f"picked because the turn ended with the offer open: {auto}",
        f"- distinct trinkets offered: {len(offers)}, picked: {len(picks)}",
        f"- Era offers: {len(era_offers)} from `{tp.ACTIONS.relative_to(ROOT)}`; "
        f"streamers: {stream_info['picks']} clean picks from "
        f"{stream_info['clean_games']} of {stream_info['games']} games",
        f"- unbuilt Era options skipped: {unknown_options}",
        f"- trinkets are keyed by table entry (card id); {len(dup)} names stand for "
        f"both a lesser and a greater entry, so those show their slot",
        "",
        "## Share of picks by offer position",
        "",
        "| offer position | picks | share of picks |",
        "|---|---|---|",
    ]
    for k in range(fe.TRINKET_CHOICES):
        lines.append(f"| {k + 1} | {positions.get(k, 0)} | "
                     f"{pct(positions.get(k, 0), n_picks)} |")
    lines.append("")
    over = [k + 1 for k in range(fe.TRINKET_CHOICES)
            if n_picks and positions.get(k, 0) / n_picks > 0.60]
    if over:
        lines.append(f"**Offer position {', '.join(map(str, over))} takes over 60% "
                     f"of the picks.** That is a bug or an untrained head, not a "
                     f"preference: the network is not choosing among the four slots.")
    else:
        lines.append("No offer position is over 60% of the picks.")
    lines.append("")

    lesser_total = sum(v for (c, s), v in cost_counts.items() if s == "lesser")
    greater_total = sum(v for (c, s), v in cost_counts.items() if s == "greater")
    lines += [
        "## Share of picks by cost (the card data's cost)",
        "",
        "| cost | lesser picks | lesser share | greater picks | greater share | "
        "all picks | all share |",
        "|---|---|---|---|---|---|---|",
    ]
    for cost in sorted({c for c, _ in cost_counts}):
        lp = cost_counts.get((cost, "lesser"), 0)
        gp = cost_counts.get((cost, "greater"), 0)
        lines.append(f"| {cost} | {lp} | {pct(lp, lesser_total)} | {gp} | "
                     f"{pct(gp, greater_total)} | {lp + gp} | "
                     f"{pct(lp + gp, n_picks)} |")
    lines += ["", f"Lesser picks: {lesser_total}; greater picks: {greater_total}.", ""]

    top = sorted(picks, key=lambda i: (-picks[i], label(table, dup, i)))[:10]
    lines += [
        "## The ten trinkets the bot picks most often",
        "",
        "Pick rate when offered is picks divided by the offers that included the "
        "trinket. Era's rate is the same count over her frozen offers, and the "
        "streamer count is the clean `trinket_pick` events by name.",
        "",
        "| trinket | bot picks | bot offers | bot rate | Era picks | Era offers | "
        "Era rate | streamer picks |",
        "|---|---|---|---|---|---|---|---|",
    ]
    for i in top:
        er_n = era_offer_count.get(i, 0)
        er_k = era_pick_count.get(i, 0)
        lines.append(f"| {label(table, dup, i)} | {picks[i]} | {offers[i]} | "
                     f"{pct(picks[i], offers[i])} | {er_k} | {er_n} | "
                     f"{pct(er_k, er_n)} | {stream_counts.get(names[i], 0)} |")
    lines.append("")

    gaps = []
    for i in set(offers) | set(era_offer_count):
        if offers.get(i, 0) < 5 or era_offer_count.get(i, 0) < 5:
            continue
        bot_rate = picks.get(i, 0) / offers[i]
        era_rate = era_pick_count.get(i, 0) / era_offer_count[i]
        gaps.append((bot_rate - era_rate, i, bot_rate, era_rate))
    gaps.sort(key=lambda g: (-g[0], label(table, dup, g[1])))
    positive = [g for g in gaps if g[0] > 0][:10]
    lines += [
        "## The ten largest gaps where the bot picks it and Era does not",
        "",
        "At least 5 offers on each side. Gap is the bot's pick rate minus Era's, "
        "in percentage points.",
        "",
        "| trinket | gap (pp) | bot rate | Era rate | bot offers | Era offers | "
        "streamer picks |",
        "|---|---|---|---|---|---|---|",
    ]
    if not positive:
        lines.append("| none | | | | | | |")
    for gap, i, bot_rate, era_rate in positive:
        lines.append(f"| {label(table, dup, i)} | {100 * gap:+.1f} | "
                     f"{100 * bot_rate:.1f}% | {100 * era_rate:.1f}% | "
                     f"{offers[i]} | {era_offer_count[i]} | "
                     f"{stream_counts.get(names[i], 0)} |")
    lines.append("")

    report = "\n".join(lines) + "\n"
    (out / "where.md").write_text(report)
    payload = {
        "ckpt": args.ckpt,
        "update": info["update"],
        "games": len(rows),
        "seed_base": args.seed_base,
        "chunk": fe.CHUNK,
        "lag": 0,
        "streamers": stream_info,
        "era_offers": len(era_offers),
        "unbuilt_era_options": unknown_options,
        "offers": log,
    }
    (out / "where-rows.json").write_text(json.dumps(payload, indent=2) + "\n")
    print()
    print(report)
    print(f"written to {out / 'where.md'} and {out / 'where-rows.json'}")


if __name__ == "__main__":
    main()
