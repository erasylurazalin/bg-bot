"""Collect turn 8 positions and late outcomes for the archive check.

Measurement for docs/briefs/archive-check.md. This is a copy of
train/evaltools/final_boards.py narrowed to turn 8, with hand and Health
added. It plays the fixed eval's games at the requested real-seat lag.

    .venv/bin/python train/evaltools/archive_check/collect.py \
        --ckpt train/runs/r1111-lagmix/best.pt --name r1111-lagmix \
        --games 8192 --real-lag 2 --vram-gb 10 \
        --fixed-eval train/runs/fixed-eval/r1111-lagmix-lag2/results.json \
        --out train/runs/archive-check/raw-lag2.json
"""

from __future__ import annotations

import argparse
import json
import sys
import time
from pathlib import Path

import numpy as np
import torch

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "train"))

import bgpy
import fixed_eval
import ppo
import real_boards
from ppo import Bufs, F_COMP, LEARNER, act, comp_onehot, force_mask

FROZEN = ROOT / "boards" / "frozen" / "real-2026-11-10"
SEED_BASE = 20261110
CHUNK = 512
LINEUP = [1, 0, 0, 0, 0, 7]


def parse_args():
    a = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    a.add_argument("--ckpt", required=True)
    a.add_argument("--name", required=True)
    a.add_argument("--games", type=int, default=8192)
    # Named --real-lag, not --lag: ppo is imported before this parses and
    # would read --lag as its --lag-in-obs abbreviation, adding a scalar.
    a.add_argument("--real-lag", type=int, required=True)
    a.add_argument("--real-lag-from", type=int, default=7)
    a.add_argument("--seed-base", type=int, default=SEED_BASE)
    a.add_argument("--boards", type=Path, default=FROZEN)
    a.add_argument("--vram-gb", type=float, required=True)
    a.add_argument("--fixed-eval", type=Path,
                   help="results.json whose first 2048 rows must match")
    a.add_argument("--out", type=Path, required=True)
    return a.parse_args()


def slots(row, start: int, n: int):
    """The card slots at `start`: (id, is_spell, attack, health, golden)."""
    out = []
    for i in range(n):
        b = start + i * bgpy.SLOT_FEATURES
        if row[b] <= 0.5:
            out.append(None)
            continue
        out.append((
            int(row[b + 1]) - 1,
            bool(row[b + 2] > 0.5),
            int(row[b + 3]),
            int(row[b + 4]),
            bool(row[b + 7] > 0.5),
        ))
    return out


def cards(row, start: int, n: int, names) -> list[dict]:
    out = []
    for i, m in enumerate(slots(row, start, n)):
        if m is None:
            continue
        card, spell, attack, health, golden = m
        out.append({
            "name": names[card],
            "card": card,
            "position": i + 1,
            "spell": spell,
            "golden": golden,
            "attack": attack,
            "health": health,
        })
    return out


def position(row, names) -> dict:
    board = cards(row, bgpy.BOARD_START, bgpy.BOARD, names)
    hand = cards(row, bgpy.HAND_START, bgpy.HAND, names)
    return {
        "board": board,
        "hand": hand,
        "health": int(round(float(row[bgpy.SCALAR_HEALTH]))),
        "armor": int(round(float(row[bgpy.SCALAR_ARMOR]))),
        "tier": int(round(float(row[bgpy.SCALAR_TIER]))),
        "board_attack": int(sum(m["attack"] for m in board)),
        "board_health": int(sum(m["health"] for m in board)),
        "board_total": int(sum(m["attack"] + m["health"] for m in board)),
    }


def validate(rows: list[dict], path: Path, games: int = 2048) -> dict:
    """Compare the first fixed-eval games by place, knockout and turn 12 total."""
    data = json.loads(path.read_text())
    ckpt = next((c for c in data["checkpoints"]
                 if c["path"].endswith("r1111-lagmix/best.pt")), None)
    if ckpt is None:
        raise SystemExit(f"{path}: no r1111-lagmix/best.pt checkpoint")
    ref = {int(r["game"]): r for r in ckpt["rows"][:games]}
    ours = [r for r in rows if r["game"] < games]
    result = {
        "reference": str(path),
        "games": min(games, len(ours)),
        "place_mismatches": [],
        "knockout_mismatches": [],
        "turn12_mismatches": [],
    }
    for r in ours:
        q = ref.get(r["game"])
        if q is None:
            continue
        if r["place"] != q["place"]:
            result["place_mismatches"].append(
                {"game": r["game"], "ours": r["place"], "reference": q["place"]})
        if r["knocked_out_turn"] != q["knocked_out_turn"]:
            result["knockout_mismatches"].append(
                {"game": r["game"], "ours": r["knocked_out_turn"],
                 "reference": q["knocked_out_turn"]})
        want = q["board_totals"].get("12")
        if r["turn12_board_total"] != want:
            result["turn12_mismatches"].append(
                {"game": r["game"], "ours": r["turn12_board_total"],
                 "reference": want})
    result["place_match"] = not result["place_mismatches"]
    result["knockout_match"] = not result["knockout_mismatches"]
    result["turn12_match"] = not result["turn12_mismatches"]
    return result


def main():
    args = parse_args()
    folder = args.boards.resolve()
    fixed_eval.check_boards(folder)
    real, unknown = real_boards.load(folder)
    if not real:
        raise SystemExit(f"{folder}: no real boards")
    print(f"real boards: {len(real)}" +
          (f", {len(set(unknown))} unmatched names" if unknown else ""), flush=True)

    dev = torch.device("cuda" if torch.cuda.is_available() else "cpu")
    if dev.type == "cuda":
        total = torch.cuda.get_device_properties(0).total_memory / 2 ** 30
        torch.cuda.set_per_process_memory_fraction(min(1.0, args.vram_gb / total), 0)
        print(f"device cuda, {args.vram_gb:g} GB of {total:.1f} GB cap", flush=True)
    else:
        print("device cpu", flush=True)

    info = fixed_eval.load_ckpt(args.ckpt, dev)
    ppo.FORCE_DARK = info["force_dark_gift"]
    ppo.FORCE_TURN2 = False
    ppo.OLD_BOARDS = info["old_boards"]
    ppo.REAL_LOBBY_FILTER = True
    net = info["net"]
    print(f"{args.ckpt}: update {info['update']}, force_dark_gift "
          f"{info['force_dark_gift']}, old_boards {info['old_boards']}, "
          f"comps {info['n_comps']}", flush=True)

    names = None
    rows = []
    started = time.time()
    chunk = 0
    while chunk * CHUNK < args.games:
        start = chunk * CHUNK
        n = min(CHUNK, args.games - start)
        env = bgpy.VecEnv(n, args.seed_base + chunk, LINEUP, real_boards=real,
                          old_boards=info["old_boards"], real_lobby_filter=True,
                          real_lag=args.real_lag, real_lag_from=args.real_lag_from)
        if info["overlay_comps"]:
            env.set_overlay(info["overlay_comps"], 0.0)
        if names is None:
            names = env.card_names()
        b = Bufs(n)
        env.kinds(b.kinds)
        done = np.zeros(n, dtype=bool)
        rec = [None] * n
        turn8 = [None] * n
        turn12 = [None] * n
        ar = np.arange(n)
        torch.manual_seed(args.seed_base + chunk)
        steps = 0
        while not done.all():
            env.observe(b.obs, b.mask, b.seat, b.kind)
            turns = b.obs[:, bgpy.SCALAR_TURN].astype(np.int64)
            current = (~done) & (b.kind == LEARNER)
            for e in np.nonzero(current)[0]:
                e = int(e)
                turn = int(turns[e])
                if turn == 8:
                    turn8[e] = position(b.obs[e], names)
                elif turn == 12:
                    turn12[e] = position(b.obs[e], names)["board_total"]
            o = torch.from_numpy(b.obs).to(dev)
            mask = force_mask(o, torch.from_numpy(b.mask).to(dev))
            comp = None
            if net.n_comps:
                env.overlay_probe(b.forced1, b.flags)
                comp_idx = b.flags[ar, b.seat, F_COMP] - 1
                comp = comp_onehot(comp_idx, net.n_comps, dev)
            acts, _, _, _ = act(net, o, mask, comp=comp)
            acts_np = acts.cpu().numpy()
            kinds = b.kinds.copy()
            env.step(acts_np, b.places, b.tier_turns, b.tribes, b.reset)
            for e, s in zip(*np.nonzero(b.places)):
                e, s = int(e), int(s)
                if kinds[e, s] != LEARNER or done[e]:
                    continue
                place = int(b.places[e, s])
                rec[e] = {
                    "game": start + e,
                    "chunk": chunk,
                    "env": e,
                    "place": place,
                    "knocked_out_turn": None if place == 1 else int(turns[e]),
                    "reached_turn8": turn8[e] is not None,
                    "turn8": turn8[e],
                    "turn12_board_total": turn12[e],
                }
                done[e] = True
            if b.reset.any():
                env.kinds(b.kinds)
            steps += 1
            if steps % 2000 == 0 and not done.all():
                print(f"    chunk {chunk}: {int(done.sum())}/{n} first games done, "
                      f"{time.time() - started:.0f}s", flush=True)
        rows.extend(r for r in rec if r is not None)
        print(f"  chunk {chunk} done: {int(done.sum())}/{n} games, "
              f"{len(rows)}/{args.games} total, {steps} steps, "
              f"{time.time() - started:.0f}s", flush=True)
        chunk += 1

    rows.sort(key=lambda r: r["game"])
    validation = None
    if args.fixed_eval is not None:
        validation = validate(rows, args.fixed_eval)
        print(f"validation: {validation['games']} games, "
              f"places {len(validation['place_mismatches'])} mismatches, "
              f"knockouts {len(validation['knockout_mismatches'])}, "
              f"turn12 totals {len(validation['turn12_mismatches'])}", flush=True)
        for key in ("place_mismatches", "knockout_mismatches",
                    "turn12_mismatches"):
            if validation[key]:
                print(f"  first {key}: {validation[key][:5]}", flush=True)

    payload = {
        "checkpoint": str(args.ckpt),
        "update": info["update"],
        "name": args.name,
        "games": args.games,
        "seed_base": args.seed_base,
        "chunk": CHUNK,
        "lineup": LINEUP,
        "boards": str(args.boards),
        "lobby_filter": True,
        "real_lag": args.real_lag,
        "real_lag_from": args.real_lag_from,
        "pcbusy": fixed_eval.pcbusy_line(),
        "elapsed_seconds": round(time.time() - started, 1),
        "validation": validation,
        "rows": rows,
    }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(payload) + "\n")
    print(f"wrote {args.out} ({len(rows)} games, {payload['elapsed_seconds']}s)",
          flush=True)
    if validation is not None and not (
            validation["place_match"] and validation["knockout_match"]
            and validation["turn12_match"]):
        raise SystemExit("collector does not match fixed eval")


if __name__ == "__main__":
    main()
