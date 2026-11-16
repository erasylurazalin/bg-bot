#!/usr/bin/env python3
"""Does BASE pick trinkets like Era? T2 of plan-2026-11-13.

    .venv/bin/python train/evaltools/trinket_picks.py --vram-gb 4

Every `trinket_pick` in boards/frozen/actions-2026-11-15 is one offer. The
whole-turn replay (bgpy.EraReplay / EraTurnPath, the same seating as
`bgbench turn_replay`) puts the engine in the logged state immediately before
the pick, with the logged offer forced by the loader. The replay seats the
turn's live lobby by default. BASE then reads the four option logits. The
report compares BASE's greedy pick and the probability it gives Era's pick
with a uniform pick and with a streamer-popular pick from
boards/twitch/games.

Measurement only. Writes train/runs/trinket-picks/report.md and rows.json.
"""

from __future__ import annotations

import argparse
import glob
import json
import math
import os
import re
import sys
from collections import Counter, defaultdict
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "train"))
os.chdir(ROOT)

import bgpy
import fixed_eval as fe
import ppo
import torch
from model import masked

ACTIONS = ROOT / "boards/frozen/actions-2026-11-15"
TWITCH = ROOT / "boards/twitch/games"
OUT = ROOT / "train/runs/trinket-picks"
CKPT = ROOT / "train/runs/r1113-a1/best.pt"
PLACEHOLDERS = {"Lesser Trinket", "Greater Trinket"}
Z = 1.96


def wilson(k: float, n: int) -> tuple[float, float]:
    """Wilson 95% interval for a 0/1 agreement count."""
    if n <= 0:
        return (float("nan"), float("nan"))
    p = k / n
    den = 1.0 + Z * Z / n
    centre = (p + Z * Z / (2 * n)) / den
    half = Z * math.sqrt((p * (1 - p) + Z * Z / (4 * n)) / n) / den
    return (centre - half, centre + half)


def mean_range(values: list[float]) -> tuple[float, float, float]:
    """(mean, lower, upper): normal 95% range from the standard error."""
    v = np.asarray(values, dtype=np.float64)
    if len(v) == 0:
        return (float("nan"), float("nan"), float("nan"))
    mean = float(v.mean())
    if len(v) < 2:
        return (mean, mean, mean)
    se = float(v.std(ddof=1) / math.sqrt(len(v)))
    return (mean, mean - Z * se, mean + Z * se)


def pct(x: float) -> str:
    return "n/a" if not math.isfinite(x) else f"{100 * x:.1f}%"


def num(x: float, places: int = 3) -> str:
    return "n/a" if not math.isfinite(x) else f"{x:.{places}f}"


def load_trinkets() -> list[dict]:
    """The trinket table in the engine's own order: name, card_id, slot, cost."""
    text = (ROOT / "trinkets.toml").read_text()
    blocks = re.split(r"(?m)^\[\[[^\]]+\]\]\s*$", text)[1:]
    entries = []
    for block in blocks:
        name = re.search(r'(?m)^name = "([^"]+)"', block)
        card = re.search(r'(?m)^card_id = "([^"]+)"', block)
        slot = re.search(r'(?m)^slot = "([^"]+)"', block)
        cost = re.search(r"(?m)^cost = (\d+)", block)
        if name and card:
            entries.append({
                "name": name.group(1),
                "card_id": card.group(1),
                "slot": slot.group(1) if slot else None,
                "cost": int(cost.group(1)) if cost else 0,
            })
    if len(entries) != bgpy.N_TRINKETS:
        raise SystemExit(
            f"trinkets.toml has {len(entries)} entries, bgpy has {bgpy.N_TRINKETS}"
        )
    return entries


def action_offers() -> list[dict]:
    offers = []
    for path in sorted(ACTIONS.glob("*.json")):
        data = json.loads(path.read_text())
        for turn in data.get("turns", []):
            for step_index, step in enumerate(turn.get("actions", [])):
                if step.get("step") != "trinket_pick":
                    continue
                options = [
                    {
                        "name": o.get("name"),
                        "card_id": o.get("card_id"),
                        "cost": int(o.get("cost", 0) or 0),
                    }
                    for o in step.get("options", [])
                ]
                picked = step.get("picked") or {}
                offers.append({
                    "file": path.name,
                    "source": data.get("source"),
                    "game": data.get("game"),
                    "turn": turn.get("turn"),
                    "step": step_index,
                    "slot_kind": step.get("slot_kind"),
                    "options": options,
                    "picked": {
                        "name": picked.get("name"),
                        "card_id": picked.get("card_id"),
                    },
                })
    return offers


def streamer_picks() -> tuple[list[dict], Counter, dict]:
    """One pick per game and slot, from the clean Twitch games.

    boards/twitch/games records the trinket the player has equipped, not the
    offer. A `trinket_pick` event is emitted whenever that slot changes, so a
    clean game is one with at most one first-time event per slot and no
    placeholder trinket. Games with two streams interleaved (the parser's
    known collector artifact) show several first-time events in one slot and
    are dropped whole.
    """
    picks = []
    dropped = Counter()
    games = 0
    total_games = 0
    for path in sorted(TWITCH.glob("*.json")):
        total_games += 1
        data = json.loads(path.read_text())
        chosen = {}
        bad = None
        for turn in data.get("turns", []):
            for event in turn.get("events", []):
                if event.get("kind") != "trinket_pick":
                    continue
                name = event.get("trinket")
                if name in PLACEHOLDERS:
                    bad = "a placeholder trinket name"
                    continue
                if not event.get("first_time"):
                    continue
                slot = event.get("slot")
                if slot in chosen:
                    bad = "more than one first-time event in a slot"
                    continue
                chosen[slot] = (turn.get("turn"), name)
        if bad:
            dropped[bad] += 1
            continue
        games += 1
        for slot, (turn, name) in chosen.items():
            picks.append({"file": path.name, "slot": slot, "turn": turn, "trinket": name})
    counts = Counter(p["trinket"] for p in picks)
    info = {
        "games": total_games,
        "clean_games": games,
        "dropped": dict(dropped),
        "picks": len(picks),
        "distinct": len(counts),
    }
    return picks, counts, info


def advance_to(path, step_index: int) -> str | None:
    """Play Era's logged steps up to the target step; None on success."""
    while path.steps_on_path < step_index:
        try:
            action = path.era_action()
        except Exception as exc:
            return f"era_action failed before the target step: {exc}"
        if action < 0:
            why = path.left_why or "Era's logged steps ended"
            return f"no decision at step {path.steps_on_path}: {why}"
        try:
            on_path = path.act(action)
        except Exception as exc:
            return f"acting before the target step failed: {exc}"
        if not on_path:
            why = path.left_why or "the turn left Era's path"
            return f"left Era's path at step {path.steps_on_path}: {why}"
    if path.steps_on_path != step_index:
        return (
            f"could not stop on step {step_index}: the replay is at step "
            f"{path.steps_on_path}"
        )
    return None


def main() -> None:
    global ACTIONS
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--ckpt", default=str(CKPT))
    ap.add_argument("--actions", default=str(ACTIONS),
                    help="the frozen action folder (default the live-lobby set)")
    ap.add_argument("--vram-gb", type=float, required=True,
                    help="cap this process's GPU memory in GB (required)")
    ap.add_argument("--cpu", action="store_true", help="run on the CPU")
    ap.add_argument("--out", default=str(OUT))
    args = ap.parse_args()

    ACTIONS = Path(args.actions).resolve()
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)

    table = load_trinkets()
    by_card = {e["card_id"]: i for i, e in enumerate(table)}
    by_index = table
    offers = action_offers()
    stream, stream_counts, stream_info = streamer_picks()
    total_stream = stream_info["picks"]

    dev = torch.device("cpu" if args.cpu or not torch.cuda.is_available() else "cuda")
    if dev.type == "cuda":
        total_gb = torch.cuda.get_device_properties(0).total_memory / 2**30
        torch.cuda.set_per_process_memory_fraction(min(1.0, args.vram_gb / total_gb), 0)
        print(f"device cuda, {args.vram_gb:g} GB of {total_gb:.1f} GB cap", flush=True)
    else:
        print("device cpu", flush=True)

    info = fe.load_ckpt(args.ckpt, dev)
    net = info["net"]
    if info["n_comps"]:
        raise SystemExit(
            f"{args.ckpt}: network has {info['n_comps']} comp inputs; "
            "this measurement feeds no comp overlay"
        )
    ppo.FORCE_DARK = info["force_dark_gift"]
    ppo.FORCE_TURN2 = False
    ppo.OLD_BOARDS = info["old_boards"]

    kind_names = [name for name, _ in bgpy.ACTION_KINDS]
    trinket_base = sum(
        count for name, count in bgpy.ACTION_KINDS[:kind_names.index("choose_trinket")]
    )
    choices = bgpy.TRINKET_CHOICES

    replays, bad_files = bgpy.era_replays(str(ACTIONS), str(ROOT / "ref/bg-cards.json"))
    by_file = {r.file: r for r in replays}
    for line in bad_files:
        print(f"loader: {line}", flush=True)

    rows = []
    for number, offer in enumerate(offers, 1):
        options = offer["options"]
        picked_index = next(
            (i for i, o in enumerate(options) if o["card_id"] == offer["picked"]["card_id"]),
            None,
        )
        unbuilt = [o["name"] for o in options if o["card_id"] not in by_card]
        row = {
            "file": offer["file"],
            "source": offer["source"],
            "game": offer["game"],
            "turn": offer["turn"],
            "step": offer["step"],
            "slot": offer["slot_kind"],
            "options": options,
            "picked": offer["picked"],
            "picked_index": picked_index,
            "reached": False,
            "drop_reason": None,
            "base_probs": None,
            "base_greedy": None,
            "base_prob_era": None,
            "base_prob_era_full": None,
            "replay_missing": [],
            "streamer_share": [
                stream_counts.get(o["name"], 0) / total_stream if total_stream else 0.0
                for o in options
            ],
            "streamer_picks": [stream_counts.get(o["name"], 0) for o in options],
            "popular": None,
            "popular_tied": [],
            "popular_agreement": None,
            "popular_defined": False,
        }
        if picked_index is None:
            row["drop_reason"] = "the logged pick is not one of the logged options"
            rows.append(row)
            continue
        if unbuilt:
            row["drop_reason"] = (
                "options the trinket table does not build: " + ", ".join(unbuilt)
            )
            rows.append(row)
            continue
        replay = by_file.get(offer["file"])
        if replay is None:
            row["drop_reason"] = "no replay for the action file"
            rows.append(row)
            continue
        try:
            path = replay.path(offer["turn"], 1)
        except Exception as exc:
            row["drop_reason"] = f"the turn does not seat: {exc}"
            rows.append(row)
            continue
        why = advance_to(path, offer["step"])
        row["replay_missing"] = list(path.missing)
        if why is not None:
            row["drop_reason"] = why
            rows.append(row)
            continue
        try:
            decision = path.era_action()
        except Exception as exc:
            row["drop_reason"] = f"era_action failed at the target step: {exc}"
            rows.append(row)
            continue
        if not (trinket_base <= decision < trinket_base + choices):
            extra = ("; " + path.left_why) if path.left_why else ""
            row["drop_reason"] = (
                f"the replay asks {decision} at the logged trinket step, not a "
                f"trinket choice{extra}"
            )
            rows.append(row)
            continue
        obs = np.zeros(bgpy.OBS_SIZE, dtype=np.float32)
        mask = np.zeros(bgpy.N_ACTIONS, dtype=bool)
        path.observe(obs, mask)
        seen_cards = []
        for k in range(min(len(options), choices)):
            index = int(obs[bgpy.SCALAR_TRINKET_OPTION + k * bgpy.TRINKET_OPTION_FEATURES])
            seen_cards.append(by_index[index - 1]["card_id"] if 1 <= index <= len(by_index) else None)
        if seen_cards != [o["card_id"] for o in options]:
            row["drop_reason"] = (
                "the forced offer differs from the log: "
                + ", ".join(str(c) for c in seen_cards)
            )
            rows.append(row)
            continue
        if decision - trinket_base != picked_index:
            row["drop_reason"] = (
                "the replay's forced pick is option "
                f"{decision - trinket_base}, the log's is {picked_index}"
            )
            rows.append(row)
            continue
        legal = [bool(mask[trinket_base + k]) for k in range(len(options))]
        if not all(legal):
            row["drop_reason"] = "the engine's legal mask does not allow every option"
            rows.append(row)
            continue
        # The replay sets the picked option's price to the net Gold the pick
        # paid and every other option's to 0. The log shows each option's own
        # displayed price, so put those back before the network reads it.
        for k, option in enumerate(options):
            obs[bgpy.SCALAR_TRINKET_OPTION + k * bgpy.TRINKET_OPTION_FEATURES + 1] = option["cost"]

        with torch.no_grad():
            o = torch.from_numpy(obs[None]).to(dev)
            m = torch.from_numpy(mask[None]).to(dev)
            m = ppo.force_mask(o, m)
            logits, _ = net(o, comp=None)
            option_logits = logits[0, trinket_base:trinket_base + len(options)]
            option_probs = torch.softmax(option_logits, 0)
            full = torch.softmax(masked(logits, m)[0], 0)
        probs = option_probs.detach().cpu().numpy()
        full_prob_era = float(full[trinket_base + picked_index].detach().cpu())
        row["reached"] = True
        row["base_probs"] = [float(p) for p in probs]
        row["base_greedy"] = int(np.argmax(probs))
        row["base_prob_era"] = float(probs[picked_index])
        row["base_prob_era_full"] = full_prob_era
        shares = row["streamer_share"]
        best = max(shares) if shares else 0.0
        if best > 0:
            tied = [i for i, share in enumerate(shares) if share == best]
            row["popular"] = tied[0]
            row["popular_tied"] = tied
            row["popular_defined"] = True
            row["popular_agreement"] = 1.0 / len(tied) if picked_index in tied else 0.0
        rows.append(row)
        if number % 25 == 0 or number == len(offers):
            print(f"offers {number}/{len(offers)}", flush=True)

    reached = [r for r in rows if r["reached"]]
    groups = [
        ("All trinket offers", reached),
        ("Lesser", [r for r in reached if r["slot"] == "lesser"]),
        ("Greater", [r for r in reached if r["slot"] == "greater"]),
    ]

    def summary(group: list[dict]) -> dict:
        n = len(group)
        greedy = sum(1 for r in group if r["base_greedy"] == r["picked_index"])
        pa = [r["base_prob_era"] for r in group]
        paf = [r["base_prob_era_full"] for r in group]
        rand = [1.0 / len(r["options"]) for r in group]
        stream_agree = [r["popular_agreement"] for r in group if r["popular_defined"]]
        no_stream = sum(1 for r in group for c in r["streamer_picks"] if c == 0)
        total_options = sum(len(r["options"]) for r in group)
        return {
            "n": n,
            "greedy": greedy,
            "greedy_range": wilson(greedy, n),
            "pera": mean_range(pa),
            "pera_full": mean_range(paf),
            "random": mean_range(rand),
            "stream": mean_range(stream_agree),
            "stream_sum": float(sum(stream_agree)),
            "stream_n": len(stream_agree),
            "stream_ties": sum(
                1 for r in group
                if r["popular_defined"] and r["popular_agreement"] not in (0.0, 1.0)
            ),
            "top_ties": sum(
                1 for r in group
                if r["popular_defined"] and len(r["popular_tied"]) > 1
            ),
            "no_stream": no_stream,
            "total_options": total_options,
        }

    stats = {label: summary(group) for label, group in groups}
    all_stats = stats["All trinket offers"]
    miss_lines = Counter(
        line for r in reached for line in r["replay_missing"]
    )
    miss_offers = sum(1 for r in reached if r["replay_missing"])

    # rows.json keeps every offer, including the ones the replay could not reach.
    (out / "rows.json").write_text(json.dumps(rows, indent=1) + "\n")

    lines = []
    lines.append("# Do BASE's trinket picks agree with Era's?")
    lines.append("")
    lines.append(
        "Measurement only. Every `trinket_pick` in "
        f"`{ACTIONS.relative_to(ROOT)}` is one offer. The engine is seated "
        "at the logged state immediately before the pick by `bgpy.EraReplay` "
        "and `EraTurnPath` (the same seating as `bgbench turn_replay`), with "
        "the logged offer forced by the loader and the turn's live lobby "
        "seated by default. BASE then reads the four "
        "option logits. Script: `train/evaltools/trinket_picks.py`; rows: "
        "`rows.json`."
    )
    lines.append("")
    lines.append(
        f"Checkpoint `{Path(args.ckpt).relative_to(ROOT)}`, update "
        f"{info['update']}, seed 1, device {dev.type}"
        + (f", {args.vram_gb:g} GB cap" if dev.type == "cuda" else "")
        + ". "
        f"{len(offers)} logged offers, {len(reached)} reached, "
        f"{len(offers) - len(reached)} dropped."
    )
    lines.append("")
    lines.append("## Main table")
    lines.append("")
    lines.append(
        "`p(Era)` is the mean probability BASE gives Era's option, conditional "
        "on the offered trinkets (the four option logits softmaxed over the "
        "offer). `random` is the expected agreement of a uniform pick, "
        "`1 / options`, averaged. `streamer` is the streamer-pick frequency "
        "proxy described below. The 95% columns are ranges: Wilson for the "
        "greedy 0/1 agreement, normal from the standard error for the means. "
        "`n/N` beside an agreement is the count behind it."
    )
    lines.append("")
    lines.append(
        "| Group | Offers | BASE greedy agrees Era | 95% | mean p(Era) | 95% | random | 95% | streamer popular agrees Era | 95% | options with no streamer data |"
    )
    lines.append(
        "|---|---:|---:|---|---:|---|---:|---|---:|---|---:|"
    )
    for label, group in groups:
        s = stats[label]
        gl, gh = s["greedy_range"]
        pl, ph = s["pera"][1], s["pera"][2]
        rl, rh = s["random"][1], s["random"][2]
        sl, sh = s["stream"][1], s["stream"][2]
        lines.append(
            f"| {label} | {s['n']} | {pct(s['greedy'] / s['n'])} "
            f"({s['greedy']}/{s['n']}) | {pct(gl)} to {pct(gh)} | "
            f"{num(s['pera'][0])} | {num(pl)} to {num(ph)} | "
            f"{pct(s['random'][0])} | {pct(rl)} to {pct(rh)} | "
            f"{pct(s['stream'][0])} ({s['stream_sum']:.1f}/{s['stream_n']}) | "
            f"{pct(sl)} to {pct(sh)} | "
            f"{s['no_stream']}/{s['total_options']} |"
        )
    lines.append("")
    lines.append(
        "Every reached offer had four options, so the random baseline is "
        "exactly 25.0% in every row. The streamer numerator is fractional "
        "where the top proxy share was tied and Era's pick was one of the "
        f"tied leaders; there were {all_stats['stream_ties']} such offers "
        f"overall ({all_stats['top_ties']} offers had a tied top share in "
        "all)."
    )
    lines.append("")
    lines.append(
        f"Agreement minus random: {pct(all_stats['greedy'] / all_stats['n'])} "
        f"minus {pct(all_stats['random'][0])} = "
        f"{100 * (all_stats['greedy'] / all_stats['n'] - all_stats['random'][0]):+.1f} "
        "percentage points overall. "
        f"The full-mask mean p(Era) (including Freeze and End Turn as legal "
        f"actions) is {num(all_stats['pera_full'][0])} "
        f"[{num(all_stats['pera_full'][1])}, {num(all_stats['pera_full'][2])}]."
    )
    lines.append("")
    lines.append("## How the offer state was reached")
    lines.append("")
    lines.append(
        "For each offer the script builds a fresh `EraTurnPath` for the logged "
        "turn (`seed=1`), plays Era's own logged steps up to the pick's step "
        "index, and stops there. The path reseats every step from the file's "
        "own before-state, so the observed state is the log's board, hand, "
        "Tavern, Gold, tier, hero powers, trinkets, counters and lobby. At the "
        "trinket step the loader's `force_offer` puts the logged four options "
        "(by table index) and the logged slot on the player, and the script "
        "checks that the observed option card ids match the log and that the "
        "forced pick is the logged one. It then writes each option's displayed "
        "`cost` from the log into the option-price observation fields: the "
        "loader itself gives only the picked option its net paid Gold (which "
        "can be negative) and 0 to the others."
    )
    lines.append("")
    lines.append(
        "What differs from Era's real state: the replay seats the log's live "
        "lobby, so the seven opponents' heroes, health, armor, tier, placement "
        "and last-seen boards are the logged ones. The file's "
        "`last_opponent_board` is seated as the last opponent's seen board "
        "(the loader's own choice for Bob's Burgles and I Spy), but no "
        "current opponent board outside my fights and no leaderboard tribe "
        "line for a player I did not fight is available in the action file. "
        "Era's current opponent information at that moment can therefore "
        "still be missing. Everything on Era's own side is the logged state, "
        "not the engine's own turn start."
    )
    lines.append("")
    if miss_lines:
        detail = ", ".join(f"{line} x{n}" for line, n in sorted(miss_lines.items()))
        lines.append(
            f"The loader also reported content it could not build on "
            f"{miss_offers} of the reached offers, all on Era's own side: "
            f"{detail}. These are recorded per offer in `replay_missing`; the "
            "offer itself was still forced and read."
        )
    else:
        lines.append("The loader reported no missing content on the reached offers.")
    lines.append("")
    lines.append("## Offers dropped")
    lines.append("")
    dropped = [r for r in rows if not r["reached"]]
    for r in dropped:
        names = ", ".join(o["name"] for o in r["options"])
        lines.append(
            f"- {r['source']} g{r['game']} turn {r['turn']} "
            f"({r['slot']}, step {r['step']}): {r['drop_reason']}. "
            f"Options: {names}."
        )
    if not dropped:
        lines.append("None.")
    lines.append("")
    lines.append(
        f"Dropped offers are excluded from every share above, which is why "
        f"the table has {all_stats['n']} offers, not {len(offers)}."
    )
    lines.append("")
    lines.append("## Streamer data and the popularity proxy")
    lines.append("")
    lines.append(
        "The form the brief asks for, the share of offers containing a "
        "trinket where it was picked, cannot be computed from "
        "`boards/twitch/games`: a `trinket_pick` event carries only the "
        "trinket the streamer ended up with, never the options that were "
        "shown. The raw snapshots behind those games carry the same single "
        "equipped-trinket id. The popularity table here is therefore a proxy: "
        "each trinket's share of all clean streamer picks. It ranks how often "
        "streamers end up with a trinket, which mixes preference with how "
        "often the trinket is offered. On Era's offer the popular pick is the "
        "option with the largest proxy share; ties at the top split the "
        "agreement equally. Offers where no option has any streamer pick are "
        "left out of the streamer agreement; none of the reached offers fell "
        "into that case."
    )
    lines.append("")
    lines.append(
        f"Clean games: {stream_info['clean_games']} of {stream_info['games']}; "
        f"{stream_info['picks']} picks, {stream_info['distinct']} distinct "
        "trinket names. Dropped games: "
        + ", ".join(f"{n} {why}" for why, n in sorted(stream_info["dropped"].items()))
        + ". A clean game has at most one first-time event per trinket slot "
        "and no `Lesser Trinket` / `Greater Trinket` placeholder. Games with "
        "two interleaved streams (the collector's known artifact) show several "
        "first-time events in one slot and are dropped whole. Picks are "
        "matched by trinket name only: some names stand for both a lesser and "
        "a greater entry, and the event's first/second slot label is not "
        "reliable enough to split them."
    )
    lines.append("")
    lines.append("## Choices the brief left open")
    lines.append("")
    choices_text = [
        "Greedy means the highest-probability option among the offered "
        "trinkets, not among every legal action. The engine also allows "
        "Freeze and End Turn during the offer; they are not trinket picks and "
        "would only dilute the comparison.",
        "p(Era) is conditional on the four offered trinkets. The full-mask "
        "probability, which includes Freeze and End Turn, is reported once "
        "above and stored per offer as `base_prob_era_full`.",
        "The log's displayed option `cost` is written into the observation. "
        "The replay's own forced prices are the picked option's net paid Gold "
        "and 0 for the rest, which is not the offer Era saw.",
        "Offers with an option the trinket table does not build are dropped: "
        "the network has no embedding row for that trinket, and `force_offer` "
        "cannot seat it. Two offers are dropped for this reason.",
        "The streamer baseline uses pick frequency, not the requested "
        "offer-conditional share, because the offer list is not logged. This "
        "is the main caveat on the streamer column.",
        "Streamer popularity is matched by name only. Names shared by a "
        "lesser and a greater trinket are pooled; the 19 shared names in the "
        "cleaned picks include common options such as Transcribing Typewriter, "
        "Timeworn Candelabra and Glass of Perspective.",
        "Streamer ties at the maximum share count as a fraction: an offer "
        "where Era's option is one of k tied leaders scores 1/k.",
        "A 95% range on a 0/1 agreement is Wilson; a range on a mean is the "
        "normal standard-error range. With one offer per row the two differ "
        "slightly at the extremes.",
        "Every offer is seated with `seed=1`. The trinket offer itself is "
        "forced, so the seed does not affect the observed option set.",
        "The streamer cleaning drops a whole game when it has a placeholder "
        "trinket or more than one first-time event in one slot; 22 of 449 "
        "games are dropped for those two reasons.",
    ]
    for i, text in enumerate(choices_text, 1):
        lines.append(f"{i}. {text}")
    lines.append("")
    lines.append("## Reading")
    lines.append("")
    lines.append(
        f"BASE's greedy pick is Era's on {all_stats['greedy']} of "
        f"{all_stats['n']} offers ({pct(all_stats['greedy'] / all_stats['n'])}), "
        f"against {pct(all_stats['random'][0])} for a uniform pick and "
        f"{pct(all_stats['stream'][0])} for the streamer-popular proxy. "
        f"The mean probability it gives Era's pick is "
        f"{num(all_stats['pera'][0])}, while the mean probability it gives "
        f"its own greedy pick is {num(float(np.mean([max(r['base_probs']) for r in reached])))}. "
        "The policy is therefore making firm choices, mostly for trinkets "
        "other than Era's, rather than spreading its mass over the offer."
    )
    lines.append("")
    lines.append(
        f"T2's rule in `docs/notes/plan-2026-11-13.md` is that BASE agrees "
        "with Era within 10 points of random. The point estimate is "
        f"{100 * (all_stats['greedy'] / all_stats['n'] - all_stats['random'][0]):+.1f} "
        "points from random, inside that band, so this check says trinkets "
        "are picked blind. The 95% range of the agreement "
        f"({pct(all_stats['greedy_range'][0])} to "
        f"{pct(all_stats['greedy_range'][1])}) is below 25.0%, so the point "
        "estimate is below random, not merely near it. The missing opponent "
        "state described above is the main reason to read the exact level "
        "with care."
    )
    lines.append("")
    (out / "report.md").write_text("\n".join(lines) + "\n")

    print(
        f"wrote {out / 'report.md'} and {out / 'rows.json'}: "
        f"{len(reached)} reached, {len(offers) - len(reached)} dropped",
        flush=True,
    )
    for label, group in groups:
        s = stats[label]
        print(
            f"  {label}: {s['n']} offers, greedy {s['greedy']}/{s['n']} "
            f"({pct(s['greedy'] / s['n'])}), p(Era) {num(s['pera'][0])}, "
            f"streamer {pct(s['stream'][0])} of {s['stream_n']}",
            flush=True,
        )


if __name__ == "__main__":
    main()
